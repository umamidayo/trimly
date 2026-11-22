//! Settings stored as JSON in %APPDATA%\DiscordTrim\config.json.
//! Unknown or missing fields fall back to defaults, so old configs keep working.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    /// Tweak id -> enabled. A tweak missing here uses its own default.
    pub tweaks: BTreeMap<String, bool>,
    /// Seconds Discord's windows must stay hidden or minimized before its memory is trimmed.
    pub trim_delay_secs: u64,
    /// Seconds between repeat trims while an app stays hidden, since memory climbs back.
    pub trim_interval_secs: u64,
    /// Seconds without keyboard or mouse input before explorer.exe's memory is trimmed.
    pub explorer_idle_secs: u64,
    /// Seconds between explorer.exe trims while the user is active.
    pub explorer_interval_secs: u64,
    /// Seconds between Discord trims while its windows are shown (active trimming).
    pub active_trim_interval_secs: u64,
    /// Set after the first launch has turned on "Start with Windows".
    pub first_run_done: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tweaks: BTreeMap::new(),
            trim_delay_secs: 30,
            trim_interval_secs: 60,
            explorer_idle_secs: 10,
            explorer_interval_secs: 60,
            active_trim_interval_secs: 300,
            first_run_done: false,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn old_config_gets_new_defaults() {
        let cfg: super::Config = serde_json::from_str(r#"{"trim_delay_secs":120,"first_run_done":true}"#).unwrap();
        assert_eq!(cfg.trim_delay_secs, 120);
        assert_eq!(cfg.active_trim_interval_secs, 300);
    }
}

pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("DiscordTrim")
}

fn path() -> PathBuf {
    dir().join("config.json")
}

pub fn load() -> Config {
    match std::fs::read_to_string(path()) {
        // Some editors (and PowerShell 5.1) write a UTF-8 BOM, which serde_json rejects.
        Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_default(),
        Err(_) => {
            let cfg = Config::default();
            save(&cfg);
            cfg
        }
    }
}

pub fn save(cfg: &Config) {
    let _ = std::fs::create_dir_all(dir());
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(path(), text);
    }
}
