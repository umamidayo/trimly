//! "Start with Windows" via HKCU\Software\Microsoft\Windows\CurrentVersion\Run.

use crate::util::wide;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::*;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE: &str = "DiscordTrim";

fn open() -> Option<HKEY> {
    let mut key: HKEY = null_mut();
    let sub = wide(RUN_KEY);
    let rc = unsafe {
        RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, KEY_QUERY_VALUE | KEY_SET_VALUE, &mut key)
    };
    (rc == ERROR_SUCCESS).then_some(key)
}

pub fn is_enabled() -> bool {
    let Some(key) = open() else { return false };
    let name = wide(VALUE);
    let rc = unsafe { RegQueryValueExW(key, name.as_ptr(), null_mut(), null_mut(), null_mut(), null_mut()) };
    unsafe { RegCloseKey(key) };
    rc == ERROR_SUCCESS
}

pub fn set(enabled: bool) {
    let Some(key) = open() else { return };
    let name = wide(VALUE);
    unsafe {
        if enabled {
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            let data = wide(&format!("\"{exe}\""));
            RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, data.as_ptr() as *const u8, (data.len() * 2) as u32);
        } else {
            RegDeleteValueW(key, name.as_ptr());
        }
        RegCloseKey(key);
    }
}
