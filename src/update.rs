//! Checks GitHub Releases for a newer version on startup and downloads its installer when the user
//! asks. Uses the curl.exe that ships with Windows 10, so the app needs no HTTP library.

use serde::Deserialize;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

/// GitHub repo holding the releases, as "owner/name". Until it is set, the check finds nothing.
const REPO: &str = "OWNER/REPO";
const NO_WINDOW: u32 = 0x0800_0000; // CREATE_NO_WINDOW: no console flash

#[derive(Clone)]
pub struct Update {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Set by the startup check when a newer release exists; read by the settings window.
pub static AVAILABLE: Mutex<Option<Update>> = Mutex::new(None);

pub fn check_in_background() {
    std::thread::spawn(|| {
        if let Some(u) = check() {
            if let Ok(mut slot) = AVAILABLE.lock() {
                *slot = Some(u);
            }
        }
    });
}

/// The newest release, if it is newer than this build and has an installer attached.
fn check() -> Option<Update> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = curl(&["-sfL", "-H", "User-Agent: DiscordTrim", &url])?;
    let release: Release = serde_json::from_slice(&body).ok()?;
    let version = release.tag_name.trim_start_matches('v').to_string();
    if !newer(&version, env!("CARGO_PKG_VERSION")) {
        return None;
    }
    let asset = release.assets.into_iter().find(|a| a.name.ends_with("Setup.exe"))?;
    Some(Update { version, url: asset.browser_download_url })
}

/// Saves the installer to the temp folder and returns its path.
pub fn download(u: &Update) -> Option<PathBuf> {
    let path = std::env::temp_dir().join(format!("DiscordTrim-Setup-{}.exe", u.version));
    curl(&["-sfL", "-o", path.to_str()?, &u.url])?;
    Some(path)
}

fn curl(args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("curl.exe").args(args).creation_flags(NO_WINDOW).output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// True when version `a` is newer than `b`, comparing dot-separated numbers ("0.10.0" > "0.9.0").
fn newer(a: &str, b: &str) -> bool {
    let parts = |s: &str| s.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

#[cfg(test)]
mod tests {
    #[test]
    fn compares_versions_by_number() {
        assert!(super::newer("0.10.0", "0.9.0"));
        assert!(!super::newer("0.1.0", "0.1.0"));
        assert!(!super::newer("0.1.0", "0.2.0"));
    }
}
