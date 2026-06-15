use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BkgrndConfig {
    #[serde(default)]
    pub wopr_base_url: Option<String>,
    #[serde(default)]
    pub wopr_token: Option<String>,
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
