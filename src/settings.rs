//! Settings window, opened from the tray. Shows measured results (Discord's RAM now, trims run,
//! memory freed) and edits the feature switches and timings.

use crate::{autostart, config::{self, Config}, procs, tray::{error_box, APP}, tweaks::trim, update, util::{loword, wide}};
use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{CheckDlgButton, IsDlgButtonChecked, BST_CHECKED, BST_UNCHECKED};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

// Control ids. Ranges let WM_CTLCOLORSTATIC pick colors by role.
const FEAT_BASE: i32 = 3000; // one checkbox per tweak, in registry order
const AUTOSTART: i32 = 3100;
const TIME_BASE: i32 = 3200; // one number box per entry in TIMINGS
const STAT_VALUE: i32 = 3300; // 3 big numbers
const STAT_CAPTION: i32 = 3310; // 3 captions under them
const MUTED: i32 = 3400; // gray text: subtitle and "sec" suffixes
const UPDATE_BTN: i32 = 3500;
const TIMER_ID: usize = 1;
const MB: f64 = 1024.0 * 1024.0;

/// True while the update installer is downloading, so the button can't start a second one.
static DOWNLOADING: AtomicBool = AtomicBool::new(false);

/// Labels of the number boxes, in the order `timing_mut` maps them to config fields.
const TIMINGS: [&str; 5] = [
    "Trim hidden apps after",
    "Repeat hidden-app trim every",
    "Repeat visible Discord trim every",
    "Trim Explorer after no input for",
    "Repeat Explorer trim every",
];

// Layout in 96-DPI pixels, scaled to the monitor at creation.
const W: i32 = 480;
const PAD: i32 = 20;
const TILE_Y: i32 = 84;
const TILE_H: i32 = 76;
const TILE_GAP: i32 = 10;
const TILE_W: i32 = (W - 2 * PAD - 2 * TILE_GAP) / 3;
const FOOTER_H: i32 = 56;

// Colors (COLORREF is 0x00BBGGRR).
const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
const TEXT: u32 = rgb(28, 28, 28);
const GRAY: u32 = rgb(100, 100, 100);
const ACCENT: u32 = rgb(64, 78, 200);
const PANEL: u32 = rgb(243, 243, 243);
const WHITE: u32 = rgb(255, 255, 255);

#[derive(Clone, Copy)]
struct Fonts {
    body: HFONT,
    header: HFONT,
    title: HFONT,
    stat: HFONT,
}

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
    static FONTS: Cell<Option<Fonts>> = const { Cell::new(None) };
    static PANEL_BRUSH: Cell<HBRUSH> = const { Cell::new(null_mut()) };
    static FOOTER_Y: Cell<i32> = const { Cell::new(0) };
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

/// Windows' own UI font (Segoe UI) at this DPI, plus bigger and bolder variants of it.
unsafe fn make_fonts(dpi: u32) -> Fonts {
    let mut ncm: NONCLIENTMETRICSW = zeroed();
    ncm.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
    SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as _, 0, dpi);
    let mut base = ncm.lfMessageFont;
    if base.lfHeight == 0 {
        // Fallback if the query failed: Segoe UI 9pt.
        base.lfHeight = -(12 * dpi as i32 / 96);
        let face = wide("Segoe UI");
        base.lfFaceName[..face.len()].copy_from_slice(&face);
    }
    base.lfQuality = CLEARTYPE_QUALITY as u8;
    // The message font is 9pt; `pt` scales from it.
    let make = |pt: i32, weight: i32| {
        let mut lf = base;
        lf.lfHeight = base.lfHeight * pt / 9;
        lf.lfWeight = weight;
        CreateFontIndirectW(&lf)
    };
    Fonts { body: make(9, 400), header: make(11, 600), title: make(16, 600), stat: make(18, 600) }
}

pub fn open() {
    unsafe {
        let existing = WINDOW.get();
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
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
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hIcon: LoadIconW(hinst, 1 as _),
            ..zeroed()
        };
        RegisterClassExW(&wc); // a second call just fails harmlessly
        let title = wide("DiscordTrim");
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), style, CW_USEDEFAULT, CW_USEDEFAULT,
            0, 0, null_mut(), null_mut(), hinst, null());
        WINDOW.set(hwnd);
        let fonts = make_fonts(GetDpiForWindow(hwnd).max(96));
        FONTS.set(Some(fonts));
        PANEL_BRUSH.set(CreateSolidBrush(PANEL));

        // Header.
        ctl(hwnd, "STATIC", "DiscordTrim", 0, PAD, 16, W - 2 * PAD, 32, 0, fonts.title);
        ctl(hwnd, "STATIC", &format!("Keeps Discord's memory down in the background.   v{}", env!("CARGO_PKG_VERSION")),
            0, PAD, 50, W - 2 * PAD, 20, MUTED, fonts.body);

        // Results: three tiles with a big number and a caption.
        let captions = ["Discord RAM now", "Trims (all apps)", "Freed (all apps)"];
        for (i, caption) in captions.iter().enumerate() {
            let x = PAD + i as i32 * (TILE_W + TILE_GAP) + 14;
            ctl(hwnd, "STATIC", "", 0, x, TILE_Y + 10, TILE_W - 28, 34, STAT_VALUE + i as i32, fonts.stat);
            ctl(hwnd, "STATIC", caption, 0, x, TILE_Y + 46, TILE_W - 28, 20, STAT_CAPTION + i as i32, fonts.body);
        }
        refresh_stats(hwnd);

        // Features: the same list as the tray menu, showing what runs right now.
        let mut y = TILE_Y + TILE_H + 22;
        ctl(hwnd, "STATIC", "Features", 0, PAD, y, W - 2 * PAD, 24, 0, fonts.header);
        y += 30;
        let features: Vec<(&str, bool)> = APP.with(|a| {
            a.borrow().as_ref().map_or(Vec::new(), |app| app.tweaks.iter().map(|t| (t.name(), t.is_enabled())).collect())
        });
        let check = BS_AUTOCHECKBOX as u32 | WS_TABSTOP;
        for (i, (name, on)) in features.iter().enumerate() {
            ctl(hwnd, "BUTTON", name, check, PAD, y, W - 2 * PAD, 22, FEAT_BASE + i as i32, fonts.body);
            CheckDlgButton(hwnd, FEAT_BASE + i as i32, if *on { BST_CHECKED } else { BST_UNCHECKED });
            y += 26;
        }
        ctl(hwnd, "BUTTON", "Start with Windows", check, PAD, y, W - 2 * PAD, 22, AUTOSTART, fonts.body);
        CheckDlgButton(hwnd, AUTOSTART, if autostart::is_enabled() { BST_CHECKED } else { BST_UNCHECKED });
        y += 40;

        // Timing: label, number box, "sec".
        ctl(hwnd, "STATIC", "Timing", 0, PAD, y, W - 2 * PAD, 24, 0, fonts.header);
        y += 32;
        let mut cfg = APP.with(|a| a.borrow().as_ref().map(|app| app.cfg.clone())).unwrap_or_default();
        let edit_x = W - PAD - 40 - 80;
        for (i, label) in TIMINGS.iter().enumerate() {
            let value = *timing_mut(&mut cfg, i);
            ctl(hwnd, "STATIC", label, 0, PAD, y + 3, edit_x - PAD - 10, 20, 0, fonts.body);
            ctlx(hwnd, WS_EX_CLIENTEDGE, "EDIT", &value.to_string(), WS_TABSTOP | ES_NUMBER as u32 | ES_RIGHT as u32 | ES_AUTOHSCROLL as u32,
                edit_x, y, 80, 24, TIME_BASE + i as i32, fonts.body);
            ctl(hwnd, "STATIC", "sec", 0, edit_x + 88, y + 3, 32, 20, MUTED, fonts.body);
            y += 32;
        }
        y += 12;

        // Footer bar: update on the left, Save and Close on the right.
        FOOTER_Y.set(y);
        let by = y + (FOOTER_H - 30) / 2;
        let btn = BS_PUSHBUTTON as u32 | WS_TABSTOP;
        // refresh_update_button hides this until the startup check finds a newer release.
        ctl(hwnd, "BUTTON", "", btn, PAD, by, 170, 30, UPDATE_BTN, fonts.body);
        ctl(hwnd, "BUTTON", "Save", BS_DEFPUSHBUTTON as u32 | WS_TABSTOP, W - PAD - 88 - 8 - 88, by, 88, 30, IDOK, fonts.body);
        ctl(hwnd, "BUTTON", "Close", btn, W - PAD - 88, by, 88, 30, IDCANCEL, fonts.body);
        refresh_update_button(hwnd);

        // Size the window to fit, centered on the screen it opened on.
        let mut rc = RECT { left: 0, top: 0, right: scale(hwnd, W), bottom: scale(hwnd, y + FOOTER_H) };
        AdjustWindowRectEx(&mut rc, style, 0, 0);
        let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
        let mut mi: MONITORINFO = zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY), &mut mi);
        let work = mi.rcWork;
        let (x, y) = (work.left + (work.right - work.left - ww) / 2, work.top + (work.bottom - work.top - wh) / 2);
        SetWindowPos(hwnd, null_mut(), x, y, ww, wh, SWP_NOZORDER);

        SetTimer(hwnd, TIMER_ID, 2000, None);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}

unsafe fn ctl(parent: HWND, class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32, font: HFONT) {
    ctlx(parent, 0, class, text, style, x, y, w, h, id, font);
}

/// Creates one child control. Position and size are in 96-DPI pixels.
unsafe fn ctlx(parent: HWND, ex: u32, class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32, font: HFONT) {
    let (c, t) = (wide(class), wide(text));
    let child = CreateWindowExW(ex, c.as_ptr(), t.as_ptr(), WS_CHILD | WS_VISIBLE | style,
        scale(parent, x), scale(parent, y), scale(parent, w), scale(parent, h),
        parent, id as usize as HMENU, GetModuleHandleW(null()), null());
    SendMessageW(child, WM_SETFONT, font as usize, 1);
}

/// Measured results: Discord's RAM right now, trims run, and memory the trims freed.
fn stats() -> [String; 3] {
    let now: u64 = procs::pids(trim::DISCORD.exes)
        .into_iter()
        .filter_map(procs::open_query)
        .map(|h| {
            let bytes = trim::working_set(h);
            procs::close(h);
            bytes
        })
        .sum();
    let freed = trim::FREED_BYTES.load(Ordering::Relaxed) as f64 / MB;
    [
        if now == 0 { "Not running".to_string() } else { format!("{:.0} MB", now as f64 / MB) },
        trim::TRIMS.load(Ordering::Relaxed).to_string(),
        if freed >= 1024.0 { format!("{:.2} GB", freed / 1024.0) } else { format!("{freed:.0} MB") },
    ]
}

unsafe fn refresh_stats(hwnd: HWND) {
    for (i, s) in stats().iter().enumerate() {
        SetWindowTextW(GetDlgItem(hwnd, STAT_VALUE + i as i32), wide(s).as_ptr());
    }
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
        let edit = GetDlgItem(hwnd, TIME_BASE + i as i32);
        match edit_text(edit).trim().parse::<u64>() {
            Ok(v) if (1..=86_400).contains(&v) => vals[i] = v,
            _ => {
                error_box(&format!("\"{label}\" needs a whole number of seconds from 1 to 86400."));
                SetFocus(edit);
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

/// Shows "Install update vX" once a newer release was found at startup; hidden otherwise.
unsafe fn refresh_update_button(hwnd: HWND) {
    let version = update::AVAILABLE.lock().ok().and_then(|g| g.as_ref().map(|u| u.version.clone()));
    let btn = GetDlgItem(hwnd, UPDATE_BTN);
    let (text, enabled) = match version {
        _ if DOWNLOADING.load(Ordering::Relaxed) => ("Downloading update...".to_string(), false),
        Some(v) => (format!("Install update v{v}"), true),
        None => {
            ShowWindow(btn, SW_HIDE);
            return;
        }
    };
    SetWindowTextW(btn, wide(&text).as_ptr());
    EnableWindow(btn, enabled as i32);
    ShowWindow(btn, SW_SHOW);
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

/// Paints the gray result tiles and footer bar; the white background comes from the window class.
unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let brush = PANEL_BRUSH.get();
    for i in 0..3 {
        let x = PAD + i * (TILE_W + TILE_GAP);
        let r = RECT { left: scale(hwnd, x), top: scale(hwnd, TILE_Y), right: scale(hwnd, x + TILE_W), bottom: scale(hwnd, TILE_Y + TILE_H) };
        FillRect(hdc, &r, brush);
    }
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    client.top = scale(hwnd, FOOTER_Y.get());
    FillRect(hdc, &client, brush);
    EndPaint(hwnd, &ps);
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
        // Text color and background per control role; checkboxes come through here too.
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let hdc = wp as HDC;
            let id = GetDlgCtrlID(lp as HWND);
            let on_panel = (STAT_VALUE..STAT_CAPTION + 3).contains(&id) || id == UPDATE_BTN || id == IDOK || id == IDCANCEL;
            let color = match id {
                STAT_VALUE..=3302 => ACCENT,
                STAT_CAPTION..=3312 | MUTED => GRAY,
                _ => TEXT,
            };
            SetTextColor(hdc, color);
            SetBkColor(hdc, if on_panel { PANEL } else { WHITE });
            (if on_panel { PANEL_BRUSH.get() } else { GetSysColorBrush(COLOR_WINDOW) }) as LRESULT
        }
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_TIMER => {
            refresh_stats(hwnd);
            refresh_update_button(hwnd);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_ID);
            WINDOW.set(null_mut());
            if let Some(f) = FONTS.take() {
                for font in [f.body, f.header, f.title, f.stat] {
                    DeleteObject(font as HGDIOBJ);
                }
            }
            DeleteObject(PANEL_BRUSH.replace(null_mut()) as HGDIOBJ);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
