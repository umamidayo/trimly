//! Checks GitHub Releases for a newer version on startup, using WinHTTP (built into Windows).
//! Installing is left to the browser: "Install" opens the installer's download link, so Trimly
//! itself never downloads or runs an executable. (Doing that silently is what malware droppers
//! do, and antivirus heuristics flag it.)

use crate::util::wide;
use serde::Deserialize;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use windows_sys::Win32::Networking::WinHttp::*;

/// GitHub repo holding the releases, as "owner/name".
const REPO: &str = "umamidayo/trimly";

#[derive(Clone)]
pub struct Update {
    pub version: String,
    /// The installer's download link, opened in the browser.
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

/// Set by the startup check when a newer release exists; read by the window.
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
    let body = https_get("api.github.com", &format!("/repos/{REPO}/releases/latest"))?;
    let release: Release = serde_json::from_slice(&body).ok()?;
    let version = release.tag_name.trim_start_matches('v').to_string();
    if !newer(&version, env!("CARGO_PKG_VERSION")) {
        return None;
    }
    let asset = release.assets.into_iter().find(|a| a.name.ends_with("Setup.exe"))?;
    // Only ever hand GitHub download links to the browser.
    asset.browser_download_url.starts_with("https://github.com/")
        .then(|| Update { version, url: asset.browser_download_url })
}

/// Closes a WinHTTP handle when dropped.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn handle(h: *mut c_void) -> Option<Handle> {
    (!h.is_null()).then_some(Handle(h))
}

/// GET https://host/path. Returns the body on HTTP 200. Redirects (e.g. a renamed repo) are followed.
fn https_get(host: &str, path: &str) -> Option<Vec<u8>> {
    unsafe {
        let agent = wide("Trimly");
        let session = handle(WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0))?;
        let host = wide(host);
        let conn = handle(WinHttpConnect(session.0, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT as u16, 0))?;
        let (verb, object) = (wide("GET"), wide(path));
        let req = handle(WinHttpOpenRequest(conn.0, verb.as_ptr(), object.as_ptr(), null(), null(), null(), WINHTTP_FLAG_SECURE))?;
        let accept = wide("Accept: application/vnd.github+json");
        if WinHttpSendRequest(req.0, accept.as_ptr(), u32::MAX, null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(req.0, null_mut()) == 0
        {
            return None;
        }
        let (mut status, mut size) = (0u32, 4u32);
        WinHttpQueryHeaders(req.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, null(),
            &mut status as *mut u32 as _, &mut size, null_mut());
        if status != 200 {
            return None;
        }
        let mut body = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let mut read = 0u32;
            if WinHttpReadData(req.0, buf.as_mut_ptr() as _, buf.len() as u32, &mut read) == 0 {
                return None;
            }
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            if body.len() > 4 << 20 {
                return None; // a release listing is a few KB; refuse anything absurd
            }
        }
        Some(body)
    }
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
