//! Settings stored as JSON in %APPDATA%\Trimly\config.json.
//! Unknown or missing fields fall back to defaults, so old configs keep working.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// When an app gets trimmed.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// After none of its windows have been shown for one hidden-interval, then every interval.
    Hidden,
    /// On a timer, whether or not its windows are shown (for apps kept open on another monitor).
    Always,
    /// Not trimmed. Its memory is still shown.
    Paused,
}

/// One app the user added.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppCfg {
    pub name: String,
    /// Lowercase exe names of all its processes, e.g. ["steam.exe", "steamwebhelper.exe"].
    /// The first one is the app's id.
    pub exes: Vec<String>,
    pub mode: Mode,
    /// Windows efficiency mode (EcoQoS) while none of its windows are shown.
    pub eco: bool,
}

impl AppCfg {
    pub fn key(&self) -> &str {
        self.exes.first().map(String::as_str).unwrap_or("")
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub apps: Vec<AppCfg>,
    /// Seconds between trims of hidden apps (also how long an app must be hidden first).
    pub hidden_every_secs: u64,
    /// Seconds between trims of apps in "Always" mode.
    pub always_every_secs: u64,
    /// Stops all trimming (and efficiency mode) without changing any app's settings.
    pub paused: bool,
    /// "system", "mint", "ember" or "daylight".
    pub theme: String,
    /// Settings window size in 96-DPI pixels, saved when it closes.
    pub window_size: Option<(i32, i32)>,
    /// Pre-0.1 feature switches, read once to build `apps`; never written again.
    #[serde(skip_serializing)]
    tweaks: BTreeMap<String, bool>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            apps: vec![discord(Mode::Always)],
            hidden_every_secs: 60,
            always_every_secs: 300,
            paused: false,
            theme: "system".into(),
            window_size: None,
            tweaks: BTreeMap::new(),
        }
    }
}

fn discord(mode: Mode) -> AppCfg {
    AppCfg { name: "Discord".into(), exes: vec!["discord.exe".into()], mode, eco: true }
}

/// Builds the app list from the old fixed feature switches. A switch missing from the old
/// file meant its default, which was "on" for every Discord and Explorer feature.
fn apps_from_tweaks(t: &BTreeMap<String, bool>) -> Vec<AppCfg> {
    let on = |id: &str, default: bool| t.get(id).copied().unwrap_or(default);
    let mut apps = Vec::new();
    let discord_mode = if on("discord_trim_active", true) {
        Some(Mode::Always)
    } else if on("discord_trim_idle", true) {
        Some(Mode::Hidden)
    } else {
        None
    };
    if let Some(mode) = discord_mode {
        apps.push(AppCfg { eco: on("discord_eco_qos", true), ..discord(mode) });
    }
    if on("steam_trim_idle", false) {
        apps.push(AppCfg {
            name: "Steam".into(),
            exes: vec!["steam.exe".into(), "steamwebhelper.exe".into()],
            mode: Mode::Hidden,
            eco: false,
        });
    }
    if on("explorer_trim_idle", true) {
        apps.push(AppCfg { name: "Windows Explorer".into(), exes: vec!["explorer.exe".into()], mode: Mode::Always, eco: false });
    }
    if on("ctf_trim_idle", false) {
        apps.push(AppCfg { name: "CTF Loader".into(), exes: vec!["ctfmon.exe".into()], mode: Mode::Always, eco: false });
    }
    apps
}

/// Parses config text. Files from before the app list existed get one built from their switches.
fn parse(text: &str) -> Config {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) else {
        return Config::default();
    };
    let has_apps = value.get("apps").is_some();
    let mut cfg: Config = serde_json::from_value(value).unwrap_or_default();
    if !has_apps {
        cfg.apps = apps_from_tweaks(&cfg.tweaks);
    }
    cfg
}

fn appdata() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn dir() -> PathBuf {
    appdata().join("Trimly")
}

/// The app was called DiscordTrim before 0.1.0 shipped; move its settings folder over once.
/// shortcut: only the dev machine had the old name; delete after the first public release.
pub fn migrate_old_dir() {
    let (old, new) = (appdata().join("DiscordTrim"), dir());
    if old.is_dir() && !new.exists() {
        let _ = std::fs::rename(old, new);
    }
}

fn path() -> PathBuf {
    dir().join("config.json")
}

pub fn load() -> Config {
    match std::fs::read_to_string(path()) {
        Ok(text) => {
            let cfg = parse(&text);
            save(&cfg); // writes the migrated app list, if one was just built
            cfg
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_switches_become_apps() {
        // The dev machine's real pre-0.1 config: Steam and CTF off, everything else default.
        let cfg = parse(r#"{"tweaks":{"ctf_trim_idle":false,"steam_trim_idle":false},"first_run_done":true,"trim_delay_secs":30}"#);
        let names: Vec<_> = cfg.apps.iter().map(|a| (a.name.as_str(), a.mode, a.eco)).collect();
        assert_eq!(names, [("Discord", Mode::Always, true), ("Windows Explorer", Mode::Always, false)]);
    }

    #[test]
    fn saved_app_list_is_kept_even_when_empty() {
        let cfg = parse(r#"{"apps":[],"theme":"ember"}"#);
        assert!(cfg.apps.is_empty());
        assert_eq!(cfg.theme, "ember");
        assert_eq!(cfg.hidden_every_secs, 60);
    }

    #[test]
    fn bom_and_garbage_fall_back_safely() {
        assert_eq!(parse("\u{feff}{\"paused\":true}").paused, true);
        assert_eq!(parse("not json").apps.len(), 1); // defaults: Discord
    }
}
