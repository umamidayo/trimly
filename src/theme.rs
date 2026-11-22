//! Color themes. The page (ui.html) has the same palettes as CSS; this copy colors the native
//! parts: the tray menu and the window background behind the page.

use crate::util::wide;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

/// Colors as COLORREF (0x00BBGGRR).
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: u32,
    pub surface: u32,
    pub surface2: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub track: u32,
}

const fn hex(rgb: u32) -> u32 {
    ((rgb >> 16) & 0xff) | (rgb & 0xff00) | ((rgb & 0xff) << 16)
}

const MINT: Palette = Palette {
    dark: true, bg: hex(0x0c1115), surface: hex(0x141b21), surface2: hex(0x1f2a33),
    text: hex(0xe6edf2), muted: hex(0x8696a3), accent: hex(0x34d399), track: hex(0x2a3640),
};
const EMBER: Palette = Palette {
    dark: true, bg: hex(0x120d0c), surface: hex(0x1c1513), surface2: hex(0x2c201c),
    text: hex(0xf3ebe8), muted: hex(0xa8948d), accent: hex(0xfb923c), track: hex(0x3a2b26),
};
const DAYLIGHT: Palette = Palette {
    dark: false, bg: hex(0xf4f6f8), surface: hex(0xffffff), surface2: hex(0xe9eef2),
    text: hex(0x12181d), muted: hex(0x5b6872), accent: hex(0x0e9f6e), track: hex(0xd5dbe0),
};

/// True when Windows is set to dark mode for apps.
pub fn system_dark() -> bool {
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name = wide("AppsUseLightTheme");
    let (mut v, mut size) = (1u32, 4u32);
    let rc = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, null_mut(),
            &mut v as *mut u32 as _, &mut size)
    };
    rc == ERROR_SUCCESS && v == 0
}

/// The theme the page should use: "system" resolves to Mint or Daylight.
pub fn resolve(name: &str) -> &'static str {
    match name {
        "mint" => "mint",
        "ember" => "ember",
        "daylight" => "daylight",
        _ if system_dark() => "mint",
        _ => "daylight",
    }
}

pub fn palette(name: &str) -> Palette {
    match resolve(name) {
        "ember" => EMBER,
        "daylight" => DAYLIGHT,
        _ => MINT,
    }
}

/// Makes native popup menus (their frame and shadow) follow a dark or light theme. Uses
/// uxtheme's SetPreferredAppMode/FlushMenuThemes (ordinals 135/136, Windows 10 1903+).
/// shortcut: undocumented API; if it is missing the menu keeps a light frame, nothing breaks.
pub fn set_menu_dark(dark: bool) {
    unsafe {
        let lib = wide("uxtheme.dll");
        let h = LoadLibraryW(lib.as_ptr());
        if h.is_null() {
            return;
        }
        if let Some(set_mode) = GetProcAddress(h, 135 as _) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(set_mode);
            set_mode(if dark { 2 } else { 3 }); // ForceDark / ForceLight
        }
        if let Some(flush) = GetProcAddress(h, 136 as _) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(flush);
            flush();
        }
    }
}
