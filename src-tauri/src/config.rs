use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BkgrndConfig {
    #[serde(default)]
    pub wopr_base_url: Option<String>,
    #[serde(default)]
    pub wopr_token: Option<String>,
    // Spotify Web API credentials for playlist/album conversion. These live in
    // config.yaml because a .app launched from Finder never sees shell env
    // vars; the BKGRND_SPOTIFY_* env vars still win when present.
    #[serde(default)]
    pub spotify_client_id: Option<String>,
    #[serde(default)]
    pub spotify_client_secret: Option<String>,
    #[serde(default)]
    pub spotify_access_token: Option<String>,
    #[serde(default)]
    pub spotify_max_tracks: Option<usize>,
}

fn data_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".bkgrnd")
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.yaml")
}

pub fn load_config() -> BkgrndConfig {
    let path = config_path();
    match std::fs::read_to_string(path) {
        Ok(raw) => serde_yaml::from_str(&raw).unwrap_or_default(),
        Err(_) => BkgrndConfig::default(),
    }
}

/// Env var override with config.yaml fallback, treating blank values as unset.
pub fn setting(env_key: &str, config_value: Option<&str>) -> Option<String> {
    if let Ok(value) = std::env::var(env_key) {
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    config_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
