use log::info;
use ron::ser::{to_string_pretty, PrettyConfig};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const CONFIG_FILE: &str = "deploy_config.ron";

/// Using serde(default) means we can add new values, and load old config files, without it being
/// a breaking change.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// This is the host the rss_r program will be deployed to.
    /// Either hostname, or ip.
    pub target_host: String,
    pub target_ip: u32,
    /// Username to log in as on the target.
    pub username: String,
    pub private_key_file: PathBuf,

    /// Local zip file that contains the built `rss_r` executable and `resources` direcory.
    pub rss_r_zip: PathBuf,
    /// Directory on the target that the rss_r script will be deployed to in test mode.
    /// This directory will be emptied upon test deployment.
    pub rss_r_target_test_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            target_host: String::new(),
            target_ip: 22,
            username: String::new(),
            private_key_file: PathBuf::new(),
            rss_r_zip: PathBuf::new(),
            rss_r_target_test_dir: PathBuf::new(),
        }
    }
}

impl Config {
    pub fn save(&self) {
        let path = PathBuf::from(CONFIG_FILE);

        let serialized = to_string_pretty(self, PrettyConfig::default())
            .expect("Could not convert config to RON");
        fs::write(&path, serialized).expect("Could not save config file");
    }

    pub fn load() -> Option<Self> {
        info!("Loading configuration from `{}`", CONFIG_FILE);

        let path = PathBuf::from(CONFIG_FILE);

        if let Ok(contents) = fs::read_to_string(path) {
            let result = ron::from_str(&contents);
            result.ok()
        } else {
            None
        }
    }

    pub fn host_and_port(&self) -> String {
        format!("{}:{}", self.target_host, self.target_ip)
    }
}
