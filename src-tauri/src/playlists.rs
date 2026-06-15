use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::history;

const DEFAULT_PLAYLIST_ID: &str = "streams";
const DEFAULT_PLAYLIST_NAME: &str = "Streams";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDoc {
    pub version: u32,
    pub updated_at: String,
    pub playlists: Vec<Playlist>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub items: Vec<PlaylistItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItem {
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub thumbnail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default)]
    pub added_at: String,
}

fn data_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".bkgrnd")
}

pub fn playlists_path() -> PathBuf {
    data_dir().join("playlists.yaml")
}

fn ensure_dir() {
    let dir = data_dir();
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }
}

pub fn load_playlists() -> PlaylistDoc {
    ensure_dir();
    let path = playlists_path();
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_yaml::from_str(&raw).unwrap_or_else(|_| empty_doc()),
        Err(_) => empty_doc(),
    }
}

pub fn save_playlists(doc: &PlaylistDoc) {
    ensure_dir();
    let path = playlists_path();
    if let Ok(yaml) = serde_yaml::to_string(doc) {
        let _ = std::fs::write(path, yaml);
    }
}

pub fn empty_doc() -> PlaylistDoc {
    PlaylistDoc {
        version: 1,
        updated_at: chrono_now(),
        playlists: Vec::new(),
    }
}

pub fn load_or_derive_playlists() -> PlaylistDoc {
    let mut doc = load_playlists();
    if doc.playlists.is_empty() {
        doc = doc_from_history();
    } else {
        normalize_playlist_names(&mut doc);
    }
    doc
}

pub fn save_item(mut item: PlaylistItem) -> PlaylistDoc {
    let mut doc = load_or_derive_playlists();
    if doc.playlists.is_empty() {
        doc.playlists.push(Playlist {
            id: DEFAULT_PLAYLIST_ID.to_string(),
            name: DEFAULT_PLAYLIST_NAME.to_string(),
            items: Vec::new(),
        });
    }
    normalize_playlist_names(&mut doc);

    if item.added_at.trim().is_empty() {
        item.added_at = chrono_now();
    }

    let playlist_index = doc
        .playlists
        .iter()
        .position(|playlist| playlist.id == DEFAULT_PLAYLIST_ID)
        .unwrap_or(0);
    let playlist = &mut doc.playlists[playlist_index];

    playlist.items.retain(|existing| existing.url != item.url);
    playlist.items.insert(0, item);
    playlist.items.truncate(50);
    doc.updated_at = chrono_now();
    save_playlists(&doc);
    doc
}

pub fn remove_item(url: &str) -> PlaylistDoc {
    let mut doc = load_or_derive_playlists();
    for playlist in &mut doc.playlists {
        playlist.items.retain(|item| item.url != url);
    }
    doc.updated_at = chrono_now();
    save_playlists(&doc);
    doc
}

pub fn doc_from_history() -> PlaylistDoc {
    let history_items = history::load_history();
    let items: Vec<PlaylistItem> = history_items
        .into_iter()
        .map(|h| PlaylistItem {
            url: h.url,
            title: h.title,
            channel: String::new(),
            thumbnail: h.thumbnail,
            duration: h.duration,
            added_at: h.played_at,
        })
        .collect();

    PlaylistDoc {
        version: 1,
        updated_at: chrono_now(),
        playlists: vec![Playlist {
            id: DEFAULT_PLAYLIST_ID.to_string(),
            name: DEFAULT_PLAYLIST_NAME.to_string(),
            items,
        }],
    }
}

fn normalize_playlist_names(doc: &mut PlaylistDoc) {
    let legacy_id = format!("{}-{}", "recent", "mixes");
    let legacy_name = format!("{} {}", "Recent", "Mixes");
    for playlist in &mut doc.playlists {
        if playlist.id == legacy_id {
            playlist.id = DEFAULT_PLAYLIST_ID.to_string();
        }

        if playlist.name == legacy_name {
            playlist.name = DEFAULT_PLAYLIST_NAME.to_string();
        }

        for item in &mut playlist.items {
            if item.channel == legacy_name {
                item.channel.clear();
            }
        }
    }
}

fn chrono_now() -> String {
    // ISO 8601 timestamp without chrono dependency (matches history.rs style)
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

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
    let month = m + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        y, month, d, hours, minutes, seconds
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}
