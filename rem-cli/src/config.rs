// ── config.rs ── cli was written in rust so dont write in py same design but redesign the current cli
//
// JSON-backed configuration persistence for the rem CLI. Stores the user's
// theme, mode, and active model. Writes are atomic: tmp file + os rename,
// so a crash mid-write never corrupts the on-disk config.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const CONFIG_DIR_NAME: &str = "rem-cli";
const CONFIG_FILE_NAME: &str = "config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_model")]
    pub model: String,
}

fn default_theme() -> String {
    "GHOST".to_string()
}
fn default_mode() -> String {
    "CHAT".to_string()
}
fn default_model() -> String {
    "rem-coder".to_string()
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            mode: default_mode(),
            model: default_model(),
        }
    }
}

/// Returns the absolute path of the config file: `$XDG_CONFIG_HOME/rem-cli/config.json`,
/// falling back to `$HOME/.config/rem-cli/config.json`. The directory is created on demand.
pub fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))?;
    Some(base.join(CONFIG_DIR_NAME).join(CONFIG_FILE_NAME))
}

pub fn load_config() -> UiConfig {
    let Some(path) = config_path() else {
        return UiConfig::default();
    };
    if !path.exists() {
        let cfg = UiConfig::default();
        let _ = save_config(&cfg);
        return cfg;
    }
    match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<UiConfig>(&text) {
            Ok(cfg) => cfg,
            Err(_) => {
                let cfg = UiConfig::default();
                let _ = save_config(&cfg);
                cfg
            }
        },
        Err(_) => UiConfig::default(),
    }
}

pub fn save_config(cfg: &UiConfig) -> io::Result<()> {
    let Some(path) = config_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        let serialized = serde_json::to_string_pretty(cfg)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        file.write_all(serialized.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(&tmp, &path)
}
