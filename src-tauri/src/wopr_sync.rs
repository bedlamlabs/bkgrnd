use crate::config;
use crate::player::{self, PlayerStatus, SharedState};
use crate::playlists;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use tauri::AppHandle;

const DEFAULT_BASE_URL: &str = "https://bkgrnd.bedl.am";
const PLAYLISTS_PATH: &str = "/api/v1/playlists";
const LOCAL_STATUS_PATH: &str = "/api/v1/local/status";
const LOCAL_COMMAND_NEXT_PATH: &str = "/api/v1/local/commands/next";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalPlaybackCommand {
    action: String,
    #[serde(default)]
    url: String,
}

fn base_url() -> String {
    // Priority: env var > config.yaml > default
    if let Ok(v) = std::env::var("BKGRND_WOPR_BASE_URL") {
        return v;
    }
    let cfg = config::load_config();
    cfg.wopr_base_url
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

fn auth_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    // Priority: env var > config.yaml
    let token = std::env::var("BKGRND_WOPR_TOKEN").ok().or_else(|| {
        let cfg = config::load_config();
        cfg.wopr_token
    });
    if let Some(token) = token.filter(|s| !s.trim().is_empty()) {
        if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", token)) {
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
    }
    headers
}

fn parse_ts(ts: &str) -> i64 {
    // Very small parser for "YYYY-MM-DDTHH:MM:SS.000Z"
    // Returns seconds since epoch, or 0 on parse failure.
    let bytes = ts.as_bytes();
    if bytes.len() < 20 {
        return 0;
    }
    let s = |start: usize, len: usize| -> i64 {
        std::str::from_utf8(&bytes[start..start + len])
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    };
    let year = s(0, 4);
    let month = s(5, 2);
    let day = s(8, 2);
    let hour = s(11, 2);
    let min = s(14, 2);
    let sec = s(17, 2);

    // Convert Y-M-D to days since epoch (same approach as playlists.rs)
    let mut days = 0i64;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let mdays = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    for m in 0..(month.saturating_sub(1) as usize).min(12) {
        days += mdays[m] as i64;
    }
    days += (day - 1).max(0);

    days * 86400 + hour * 3600 + min * 60 + sec
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

pub async fn sync_once() {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let base = base_url();
    let url = format!("{}{}", base.trim_end_matches('/'), PLAYLISTS_PATH);

    // Load local doc (or derive from history if missing)
    let mut local = playlists::load_playlists();
    if local.playlists.is_empty() {
        local = playlists::doc_from_history();
        playlists::save_playlists(&local);
    }

    // Fetch remote doc, distinguishing "server unreachable" (keep local,
    // try later) from "server reachable but doc corrupt" (heal it by
    // pushing local). Without that distinction a torn playlists.yaml on
    // the server wedges sync forever.
    enum RemoteDoc {
        Ok(playlists::PlaylistDoc),
        Corrupt,
        Unreachable,
    }

    let remote = match client.get(&url).headers(auth_headers()).send().await {
        Ok(resp) if resp.status().is_success() => match resp.text().await {
            Ok(text) => match serde_yaml::from_str(&text) {
                Ok(doc) => RemoteDoc::Ok(doc),
                Err(e) => {
                    eprintln!("[wopr-sync] remote playlists corrupt ({e}); pushing local copy");
                    RemoteDoc::Corrupt
                }
            },
            Err(_) => RemoteDoc::Unreachable,
        },
        _ => RemoteDoc::Unreachable,
    };

    let push_local = match remote {
        RemoteDoc::Unreachable => return,
        RemoteDoc::Corrupt => true,
        RemoteDoc::Ok(remote_doc) => {
            let local_ts = parse_ts(&local.updated_at);
            let remote_ts = parse_ts(&remote_doc.updated_at);
            if remote_ts > local_ts {
                playlists::save_playlists(&remote_doc);
                return;
            }
            // Push local if newer or equal (idempotent for a single-user doc)
            true
        }
    };

    if push_local {
        let _ = client
            .put(&url)
            .headers(auth_headers())
            .body(serde_yaml::to_string(&local).unwrap_or_default())
            .send()
            .await;
    }
}

async fn publish_status(client: &reqwest::Client, app_state: SharedState) {
    let status: PlayerStatus = match player::get_status(app_state).await {
        Ok(status) => status,
        Err(_) => PlayerStatus::empty(),
    };

    let base = base_url();
    let url = format!("{}{}", base.trim_end_matches('/'), LOCAL_STATUS_PATH);
    let _ = client
        .put(url)
        .headers(auth_headers())
        .json(&status)
        .send()
        .await;
}

// The server long-polls this endpoint (holds up to ~25s waiting for a
// command), so the request needs a timeout comfortably above the hold window.
async fn poll_command(client: &reqwest::Client) -> Result<Option<LocalPlaybackCommand>, ()> {
    let base = base_url();
    let url = format!("{}{}", base.trim_end_matches('/'), LOCAL_COMMAND_NEXT_PATH);
    let resp = client
        .get(url)
        .headers(auth_headers())
        .timeout(std::time::Duration::from_secs(35))
        .send()
        .await
        .map_err(|_| ())?;
    if !resp.status().is_success() {
        return Err(());
    }
    resp.json::<Option<LocalPlaybackCommand>>()
        .await
        .map_err(|_| ())
}

async fn execute_command(command: LocalPlaybackCommand, app: AppHandle, app_state: SharedState) {
    match command.action.as_str() {
        "play" if !command.url.trim().is_empty() => {
            if is_allowed_play_url(&command.url) {
                let _ = player::play(&command.url, app, app_state).await;
            }
        }
        "pause_toggle" => {
            let _ = player::toggle_pause(app_state).await;
        }
        "stop" => {
            let _ = player::stop(app_state).await;
        }
        _ => {}
    }
}

fn is_allowed_play_url(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.starts_with("https://www.youtube.com/")
        || trimmed.starts_with("https://youtube.com/")
        || trimmed.starts_with("https://youtu.be/")
        || trimmed.starts_with("https://music.youtube.com/")
        || trimmed.starts_with("https://open.spotify.com/")
        || trimmed.starts_with("spotify:playlist:")
        || trimmed.starts_with("spotify:album:")
        || trimmed.starts_with("spotify:track:")
}

pub async fn sync_loop(app: AppHandle, app_state: SharedState) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    sync_once().await;

    // Status heartbeat: the server marks the Mac offline after 15s without a
    // status PUT, so publish on a fixed cadence independent of command polling.
    {
        let client = client.clone();
        let state = app_state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                publish_status(&client, state.clone()).await;
            }
        });
    }

    // Playlist sync every 60s (unchanged cadence).
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            sync_once().await;
        }
    });

    // Command channel: long-poll continuously. The server holds each request
    // until a command arrives (or ~25s passes), so phone taps land in
    // sub-second time while idle traffic stays low.
    loop {
        match poll_command(&client).await {
            Ok(Some(command)) => {
                execute_command(command, app.clone(), app_state.clone()).await;
                publish_status(&client, app_state.clone()).await;
            }
            Ok(None) => {} // hold window lapsed; re-poll immediately
            Err(()) => {
                // Server unreachable; back off so we don't hammer it.
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}
