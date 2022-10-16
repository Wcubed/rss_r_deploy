use log::{error, warn};
use ron::ser::{to_string_pretty, PrettyConfig};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const CONFIG_FILE: &str = "deploy_config.ron";

/// Using serde(default) means we can add new values, and load old config files, without it being
/// a breaking change.
#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {}

impl Config {
    pub fn save(&self) {
        let path = PathBuf::from(CONFIG_FILE);

        let serialized = to_string_pretty(self, PrettyConfig::default())
            .expect("Could not convert config to RON");
        fs::write(&path, serialized).expect("Could not save config file");
    }

    pub fn load() -> Option<Self> {
        let path = PathBuf::from(CONFIG_FILE);

        if let Ok(contents) = fs::read_to_string(path) {
            let result = ron::from_str(&contents);
            result.ok()
        } else {
            None
        }
    }
}
