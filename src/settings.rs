//! Settings window, opened from the tray menu. Edits the feature switches and timings, and shows
//! measured results: Discord's current RAM use and how much the trims have freed so far.

use crate::{autostart, config::{self, Config}, procs, tray::{error_box, APP}, tweaks::trim, util::{loword, wide}};
use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::update;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::{GetStockObject, COLOR_BTNFACE, DEFAULT_GUI_FONT, HBRUSH, HGDIOBJ};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{CheckDlgButton, IsDlgButtonChecked, BST_CHECKED, BST_UNCHECKED};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const FEAT_BASE: i32 = 3000; // one checkbox per tweak, in registry order
const AUTOSTART: i32 = 3100;
const TIME_BASE: i32 = 3200; // one number box per entry in TIMINGS
const STATUS: i32 = 3300;
const UPDATE_BTN: i32 = 3400;
const TIMER_ID: usize = 1;
/// True while the update installer is downloading, so the button can't start a second one.
static DOWNLOADING: AtomicBool = AtomicBool::new(false);
const MB: f64 = 1024.0 * 1024.0;

/// Labels of the number boxes, in the order `timing_mut` maps them to config fields.
const TIMINGS: [&str; 5] = [
    "Hidden apps: trim after hidden for (seconds)",
    "Hidden apps: repeat trim every (seconds)",
    "Visible Discord: repeat trim every (seconds)",
    "Explorer: trim after no input for (seconds)",
    "Explorer: repeat trim every (seconds)",
];

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
}

/// The open settings window, or null. The message loop uses it for Tab, Enter and Esc.
pub fn hwnd() -> HWND {
    WINDOW.get()
}

fn timing_mut(cfg: &mut Config, i: usize) -> &mut u64 {
    match i {
        0 => &mut cfg.trim_delay_secs,
        1 => &mut cfg.trim_interval_secs,
        2 => &mut cfg.active_trim_interval_secs,
        3 => &mut cfg.explorer_idle_secs,
        _ => &mut cfg.explorer_interval_secs,
    }
}

/// Scales a size designed at 96 DPI to the monitor the window is on.
unsafe fn scale(hwnd: HWND, px: i32) -> i32 {
    let dpi = GetDpiForWindow(hwnd).max(96) as i32;
    px * dpi / 96
}

// Layout in 96-DPI pixels; the window is 440 x 400 on the client side.
const PAD: i32 = 12;
const ROW: i32 = 24;
const WIDTH: i32 = 416;

pub fn open() {
    unsafe {
        let existing = WINDOW.get();
        if !existing.is_null() {
            SetForegroundWindow(existing);
            return;
        }
        let hinst = GetModuleHandleW(null());
        let class = wide("DiscordTrimSettings");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst,
            lpszClassName: class.as_ptr(),
            hbrBackground: (COLOR_BTNFACE + 1) as HBRUSH,
            ..zeroed()
        };
        RegisterClassExW(&wc); // a second call just fails harmlessly
        let title = wide("DiscordTrim settings");
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU;
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), style, CW_USEDEFAULT, CW_USEDEFAULT,
            0, 0, null_mut(), null_mut(), hinst, null());
        WINDOW.set(hwnd);

        let mut rc = RECT { left: 0, top: 0, right: scale(hwnd, 440), bottom: scale(hwnd, 400) };
        AdjustWindowRectEx(&mut rc, style, 0, 0);
        SetWindowPos(hwnd, null_mut(), 0, 0, rc.right - rc.left, rc.bottom - rc.top, SWP_NOMOVE | SWP_NOZORDER);

        let font = GetStockObject(DEFAULT_GUI_FONT);
        let mut y = PAD;

        // Feature switches: the same list as the tray menu, showing what runs right now.
        let features: Vec<(&str, bool)> = APP.with(|a| {
            a.borrow().as_ref().map_or(Vec::new(), |app| app.tweaks.iter().map(|t| (t.name(), t.is_enabled())).collect())
        });
        for (i, (name, on)) in features.iter().enumerate() {
            ctl(hwnd, "BUTTON", name, BS_AUTOCHECKBOX as u32, PAD, y, WIDTH, ROW, FEAT_BASE + i as i32, font);
            CheckDlgButton(hwnd, FEAT_BASE + i as i32, if *on { BST_CHECKED } else { BST_UNCHECKED });
            y += 26;
        }
        ctl(hwnd, "BUTTON", "Start with Windows", BS_AUTOCHECKBOX as u32, PAD, y, WIDTH, ROW, AUTOSTART, font);
        CheckDlgButton(hwnd, AUTOSTART, if autostart::is_enabled() { BST_CHECKED } else { BST_UNCHECKED });
        y += 36;

        // Timings: label on the left, number box on the right.
        let mut cfg = APP.with(|a| a.borrow().as_ref().map(|app| app.cfg.clone())).unwrap_or_default();
        for (i, label) in TIMINGS.iter().enumerate() {
            let value = *timing_mut(&mut cfg, i);
            ctl(hwnd, "STATIC", label, 0, PAD, y + 2, 280, ROW, 0, font);
            ctl(hwnd, "EDIT", &value.to_string(), WS_BORDER | ES_NUMBER as u32 | ES_AUTOHSCROLL as u32, PAD + 290, y, 90, ROW, TIME_BASE + i as i32, font);
            y += 28;
        }
        y += 10;

        ctl(hwnd, "STATIC", &status_text(), 0, PAD, y + 2, WIDTH, ROW, STATUS, font);
        y += 38;
        ctl(hwnd, "BUTTON", "No update available", BS_PUSHBUTTON as u32 | WS_DISABLED, PAD, y, 200, 28, UPDATE_BTN, font);
        ctl(hwnd, "BUTTON", "Save", BS_DEFPUSHBUTTON as u32, 440 - PAD - 190, y, 90, 28, IDOK, font);
        ctl(hwnd, "BUTTON", "Close", BS_PUSHBUTTON as u32, 440 - PAD - 90, y, 90, 28, IDCANCEL, font);

        refresh_update_button(hwnd);
        SetTimer(hwnd, TIMER_ID, 2000, None);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}

/// Creates one child control with the UI font. Position and size are in 96-DPI pixels.
unsafe fn ctl(parent: HWND, class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32, font: HGDIOBJ) {
    let (c, t) = (wide(class), wide(text));
    let child = CreateWindowExW(0, c.as_ptr(), t.as_ptr(), WS_CHILD | WS_VISIBLE | WS_TABSTOP | style,
        scale(parent, x), scale(parent, y), scale(parent, w), scale(parent, h),
        parent, id as usize as HMENU, GetModuleHandleW(null()), null());
    SendMessageW(child, WM_SETFONT, font as usize, 1);
}

/// One line of measured results: Discord's RAM right now, and what the trims have freed.
fn status_text() -> String {
    let now: u64 = procs::pids(trim::DISCORD.exes)
        .into_iter()
        .filter_map(procs::open)
        .map(|h| {
            let bytes = trim::working_set(h);
            procs::close(h);
            bytes
        })
        .sum();
    format!(
        "Discord RAM now: {:.0} MB   Trims: {}   Freed by trims: {:.1} MB",
        now as f64 / MB,
        trim::TRIMS.load(Ordering::Relaxed),
        trim::FREED_BYTES.load(Ordering::Relaxed) as f64 / MB,
    )
}

unsafe fn edit_text(h: HWND) -> String {
    let mut buf = [0u16; 16];
    let n = GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32);
    String::from_utf16_lossy(&buf[..n as usize])
}

/// Validates the number boxes, then applies everything to the running app. False on bad input.
unsafe fn save(hwnd: HWND) -> bool {
    let mut vals = [0u64; 5];
    for (i, label) in TIMINGS.iter().enumerate() {
        let text = edit_text(GetDlgItem(hwnd, TIME_BASE + i as i32));
        match text.trim().parse::<u64>() {
            Ok(v) if (1..=86_400).contains(&v) => vals[i] = v,
            _ => {
                error_box(&format!("{label}\n\nEnter a whole number of seconds from 1 to 86400."));
                return false;
            }
        }
    }
    let autostart_on = IsDlgButtonChecked(hwnd, AUTOSTART) == BST_CHECKED;
    let count = APP.with(|a| a.borrow().as_ref().map_or(0, |app| app.tweaks.len()));
    let checks: Vec<bool> = (0..count).map(|i| IsDlgButtonChecked(hwnd, FEAT_BASE + i as i32) == BST_CHECKED).collect();

    APP.with(|a| {
        if let Some(app) = a.borrow_mut().as_mut() {
            for (i, v) in vals.iter().enumerate() {
                *timing_mut(&mut app.cfg, i) = *v;
            }
            for (i, on) in checks.iter().enumerate() {
                let id = app.tweaks[i].id().to_string();
                app.cfg.tweaks.insert(id, *on);
            }
            config::save(&app.cfg);
            app.sync();
        }
    });
    autostart::set(autostart_on);
    true
}

/// Shows the update button as "Install vX" when a newer release was found at startup.
unsafe fn refresh_update_button(hwnd: HWND) {
    let version = update::AVAILABLE.lock().ok().and_then(|g| g.as_ref().map(|u| u.version.clone()));
    let (text, on) = match version {
        _ if DOWNLOADING.load(Ordering::Relaxed) => ("Downloading update...".to_string(), false),
        Some(v) => (format!("Install v{v}"), true),
        None => ("No update available".to_string(), false),
    };
    let btn = GetDlgItem(hwnd, UPDATE_BTN);
    SetWindowTextW(btn, wide(&text).as_ptr());
    EnableWindow(btn, on as i32);
}

/// Downloads the newest installer on a worker thread, then runs it. The installer closes the
/// running app itself (Inno Setup's CloseApplications), so this window doesn't exit anything.
unsafe fn start_update(hwnd: HWND) {
    let Some(u) = update::AVAILABLE.lock().ok().and_then(|g| g.clone()) else { return };
    DOWNLOADING.store(true, Ordering::Relaxed);
    refresh_update_button(hwnd);
    std::thread::spawn(move || {
        match update::download(&u) {
            Some(path) => {
                let file = wide(&path.display().to_string());
                let open = wide("open");
                ShellExecuteW(null_mut(), open.as_ptr(), file.as_ptr(), null(), null(), SW_SHOWNORMAL);
                if let Ok(mut slot) = update::AVAILABLE.lock() {
                    *slot = None;
                }
            }
            None => error_box("Could not download the update. Check your internet connection, then try again."),
        }
        DOWNLOADING.store(false, Ordering::Relaxed);
    });
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match loword(wp) as i32 {
                IDOK => {
                    if save(hwnd) {
                        DestroyWindow(hwnd);
                    }
                }
                IDCANCEL => {
                    DestroyWindow(hwnd);
                }
                UPDATE_BTN => start_update(hwnd),
                _ => {}
            }
            0
        }
        WM_TIMER => {
            let text = wide(&status_text());
            SetWindowTextW(GetDlgItem(hwnd, STATUS), text.as_ptr());
            refresh_update_button(hwnd);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_ID);
            WINDOW.set(null_mut());
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
