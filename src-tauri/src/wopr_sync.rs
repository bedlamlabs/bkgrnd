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

    // Fetch remote doc (best-effort)
    let remote: Option<playlists::PlaylistDoc> =
        match client.get(&url).headers(auth_headers()).send().await {
            Ok(resp) if resp.status().is_success() => match resp.text().await {
                Ok(text) => serde_yaml::from_str(&text).ok(),
                Err(_) => None,
            },
            _ => None,
        };

    match remote {
        None => {
            // If we can't reach WOPR, just keep local as-is.
            return;
        }
        Some(remote_doc) => {
            let local_ts = parse_ts(&local.updated_at);
            let remote_ts = parse_ts(&remote_doc.updated_at);

            if remote_ts > local_ts {
                playlists::save_playlists(&remote_doc);
                return;
            }

            // Push local if newer or equal (idempotent for a single-user doc)
            let _ = client
                .put(&url)
                .headers(auth_headers())
                .body(serde_yaml::to_string(&local).unwrap_or_default())
                .send()
                .await;
        }
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

async fn poll_command(client: &reqwest::Client) -> Option<LocalPlaybackCommand> {
    let base = base_url();
    let url = format!("{}{}", base.trim_end_matches('/'), LOCAL_COMMAND_NEXT_PATH);
    let resp = client.get(url).headers(auth_headers()).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Option<LocalPlaybackCommand>>()
        .await
        .ok()
        .flatten()
}

async fn control_once(client: &reqwest::Client, app: AppHandle, app_state: SharedState) {
    publish_status(client, app_state.clone()).await;

    let Some(command) = poll_command(client).await else {
        return;
    };

    match command.action.as_str() {
        "play" if !command.url.trim().is_empty() => {
            if is_allowed_play_url(&command.url) {
                let _ = player::play(&command.url, app, app_state.clone()).await;
            }
        }
        "pause_toggle" => {
            let _ = player::toggle_pause(app_state.clone()).await;
        }
        "stop" => {
            let _ = player::stop(app_state.clone()).await;
        }
        _ => {}
    }

    publish_status(client, app_state).await;
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

    // Run immediately, then keep playlist sync and remote control heartbeats independent.
    sync_once().await;
    control_once(&client, app.clone(), app_state.clone()).await;

    let mut control_interval = tokio::time::interval(std::time::Duration::from_secs(2));
    let mut control_ticks = 0u32;
    loop {
        control_interval.tick().await;
        control_ticks = control_ticks.saturating_add(1);
        control_once(&client, app.clone(), app_state.clone()).await;
        if control_ticks >= 30 {
            control_ticks = 0;
            sync_once().await;
        }
    }
}
