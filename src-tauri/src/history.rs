use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub thumbnail: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_count: Option<usize>,
    pub played_at: String,
}

const MAX_ENTRIES: usize = 50;

fn data_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".bkgrnd")
}

fn history_path() -> PathBuf {
    data_dir().join("history.json")
}

fn ensure_dir() {
    let dir = data_dir();
    if !dir.exists() {
        // Migrate from ~/.play/ if it exists
        let old_dir = dir.parent().unwrap().join(".play");
        if old_dir.exists() {
            let _ = std::fs::rename(&old_dir, &dir);
        } else {
            let _ = std::fs::create_dir_all(&dir);
        }
    }
}

pub fn load_history() -> Vec<HistoryEntry> {
    ensure_dir();
    let path = history_path();
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn add_to_history(
    url: &str,
    title: &str,
    thumbnail: &str,
    entry_type: &str,
    track_count: Option<usize>,
) {
    ensure_dir();
    let mut history = load_history();

    // Remove existing entry with same URL (dedup)
    history.retain(|h| h.url != url);

    // Prepend the new entry
    let entry = HistoryEntry {
        url: url.to_string(),
        title: title.to_string(),
        thumbnail: thumbnail.to_string(),
        entry_type: entry_type.to_string(),
        track_count,
        played_at: chrono_now(),
    };
    history.insert(0, entry);

    // Cap at MAX_ENTRIES
    history.truncate(MAX_ENTRIES);

    let path = history_path();
    if let Ok(json) = serde_json::to_string_pretty(&history) {
        let _ = std::fs::write(&path, json);
    }
}

fn chrono_now() -> String {
    // ISO 8601 timestamp without chrono dependency
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Convert to ISO 8601 manually
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Days since epoch to Y-M-D (simplified)
    let mut y = 1970i64;
    let mut remaining_days = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        y += 1;
    }
    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining_days < md as i64 {
            m = i;
            break;
        }
        remaining_days -= md as i64;
    }
    let d = remaining_days + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        y,
        m + 1,
        d,
        hours,
        minutes,
        seconds
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
