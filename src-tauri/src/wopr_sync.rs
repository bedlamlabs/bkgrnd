use crate::playlists;
use crate::config;
use reqwest::header::{HeaderMap, HeaderValue};

const DEFAULT_BASE_URL: &str = "http://worp.thriveos.pro:808";
const PLAYLISTS_PATH: &str = "/api/v1/playlists";

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
    let remote: Option<playlists::PlaylistDoc> = match client
        .get(&url)
        .headers(auth_headers())
        .send()
        .await
    {
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

pub async fn sync_loop() {
    // Run immediately, then every ~60s.
    sync_once().await;
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        interval.tick().await;
        sync_once().await;
    }
}
