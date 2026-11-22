//! Tray icon, its menu (owner-drawn in the current theme) and the app's message loop.

use crate::{autostart, config, engine, settings, theme, update, util::{fmt_size, loword, wide}};
use std::cell::{Cell, RefCell};
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_SELECTED};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi};
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 1;
/// Posted by a second launch (e.g. the Start menu shortcut) to open the window in this instance.
pub const WM_OPEN_SETTINGS: u32 = WM_APP + 2;
pub const TRAY_CLASS: &str = "TrimlyTray";
const ID_OPEN: usize = 1;
const ID_TRIM_ALL: usize = 2;
const ID_PAUSE: usize = 3;
const ID_AUTOSTART: usize = 4;
const ID_ADMIN: usize = 5;
const ID_EXIT: usize = 6;

pub struct App {
    pub cfg: config::Config,
}

impl App {
    /// Saves the config and hands it to the engine.
    pub fn commit(&self) {
        config::save(&self.cfg);
        engine::apply(&self.cfg);
    }
}

thread_local! {
    pub static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

pub fn error_box(text: &str) {
    let t = wide(text);
    let c = wide("Trimly");
    unsafe { MessageBoxW(null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONWARNING) };
}

fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

unsafe fn add_icon(hwnd: HWND) {
    let hinst = GetModuleHandleW(null());
    let mut icon = LoadImageW(hinst, 1 as _, IMAGE_ICON, GetSystemMetrics(SM_CXSMICON),
        GetSystemMetrics(SM_CYSMICON), LR_DEFAULTCOLOR) as HICON;
    if icon.is_null() {
        icon = LoadIconW(null_mut(), IDI_APPLICATION);
    }
    let mut nid = icon_data(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = icon;
    copy(&mut nid.szTip, "Trimly");
    Shell_NotifyIconW(NIM_ADD, &nid);
}

unsafe fn remove_icon(hwnd: HWND) {
    Shell_NotifyIconW(NIM_DELETE, &icon_data(hwnd));
}

/// Shows a Windows notification from the tray icon.
unsafe fn notify(hwnd: HWND, text: &str) {
    let mut nid = icon_data(hwnd);
    nid.uFlags = NIF_INFO;
    copy(&mut nid.szInfoTitle, "Trimly");
    copy(&mut nid.szInfo, text);
    Shell_NotifyIconW(NIM_MODIFY, &nid);
}

/// Copies text into a fixed UTF-16 buffer, cut to fit with its terminating NUL.
fn copy(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

/// True when this process runs with an elevated (administrator) token.
fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev: TOKEN_ELEVATION = zeroed();
        let mut len = 0u32;
        let ok = GetTokenInformation(token, TokenElevation, &mut elev as *mut _ as *mut _,
            size_of::<TOKEN_ELEVATION>() as u32, &mut len);
        CloseHandle(token);
        ok != 0 && elev.TokenIsElevated != 0
    }
}

/* ---------- Owner-drawn menu ---------- */

enum Row {
    Header { title: String, sub: String },
    Separator,
    Item { glyph: char, label: String, toggle: Option<bool>, enabled: bool },
}

struct Menu {
    rows: Vec<Row>,
    pal: theme::Palette,
    dpi: i32,
    body: HFONT,
    bold: HFONT,
    small: HFONT,
    icons: HFONT,
}

impl Menu {
    fn s(&self, px: i32) -> i32 {
        px * self.dpi / 96
    }
}

impl Drop for Menu {
    fn drop(&mut self) {
        for f in [self.body, self.bold, self.small, self.icons] {
            unsafe { DeleteObject(f as HGDIOBJ) };
        }
    }
}

unsafe fn make_font(dpi: u32, pt10: i32, weight: i32, face: Option<&str>) -> HFONT {
    let mut ncm: NONCLIENTMETRICSW = zeroed();
    ncm.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
    SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as _, 0, dpi);
    let mut lf = ncm.lfMenuFont;
    // Menu font is 9pt; pt10 is tenths of a point.
    lf.lfHeight = -(pt10 * dpi as i32 / 720);
    lf.lfWeight = weight;
    lf.lfQuality = CLEARTYPE_QUALITY as u8;
    if let Some(face) = face {
        lf.lfFaceName = [0; 32];
        copy(&mut lf.lfFaceName, face);
    }
    CreateFontIndirectW(&lf)
}

unsafe fn fill_round(hdc: HDC, r: RECT, radius: i32, color: u32) {
    let brush = CreateSolidBrush(color);
    let old_b = SelectObject(hdc, brush as HGDIOBJ);
    let old_p = SelectObject(hdc, GetStockObject(NULL_PEN));
    RoundRect(hdc, r.left, r.top, r.right + 1, r.bottom + 1, radius, radius);
    SelectObject(hdc, old_b);
    SelectObject(hdc, old_p);
    DeleteObject(brush as HGDIOBJ);
}

unsafe fn text(hdc: HDC, s: &str, mut r: RECT, font: HFONT, color: u32, flags: u32) {
    let old = SelectObject(hdc, font as HGDIOBJ);
    SetTextColor(hdc, color);
    SetBkMode(hdc, TRANSPARENT as _);
    let w: Vec<u16> = s.encode_utf16().collect();
    DrawTextW(hdc, w.as_ptr(), w.len() as i32, &mut r, flags | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(hdc, old);
}

fn measure(m: &mut MEASUREITEMSTRUCT) {
    MENU.with(|menu| {
        let menu = menu.borrow();
        let Some(menu) = menu.as_ref() else { return };
        let Some(row) = menu.rows.get(m.itemData) else { return };
        m.itemWidth = menu.s(260) as u32;
        m.itemHeight = menu.s(match row {
            Row::Header { .. } => 62,
            Row::Separator => 9,
            Row::Item { .. } => 36,
        }) as u32;
    });
}

unsafe fn draw(d: &DRAWITEMSTRUCT) {
    MENU.with(|menu| {
        let menu = menu.borrow();
        let Some(menu) = menu.as_ref() else { return };
        let Some(row) = menu.rows.get(d.itemData) else { return };
        let (p, hdc, rc) = (menu.pal, d.hDC, d.rcItem);
        let bg = CreateSolidBrush(p.surface);
        FillRect(hdc, &rc, bg);
        DeleteObject(bg as HGDIOBJ);
        let s = |px| menu.s(px);
        let left = DT_LEFT | DT_VCENTER;
        match row {
            Row::Separator => {
                let y = (rc.top + rc.bottom) / 2;
                let line = RECT { left: rc.left + s(14), top: y, right: rc.right - s(14), bottom: y + 1.max(s(1)) };
                fill_round(hdc, line, 0, p.track);
            }
            Row::Header { title, sub } => {
                let tile = RECT { left: rc.left + s(12), top: rc.top + s(12), right: rc.left + s(50), bottom: rc.top + s(50) };
                fill_round(hdc, tile, s(12), p.surface2);
                text(hdc, "\u{E945}", tile, menu.icons, p.accent, DT_CENTER | DT_VCENTER);
                let x = tile.right + s(12);
                text(hdc, title, RECT { left: x, top: rc.top + s(12), right: rc.right - s(8), bottom: rc.top + s(32) }, menu.bold, p.text, left);
                text(hdc, sub, RECT { left: x, top: rc.top + s(31), right: rc.right - s(8), bottom: rc.top + s(50) }, menu.small, p.muted, left | DT_END_ELLIPSIS);
            }
            Row::Item { glyph, label, toggle, enabled } => {
                let hot = d.itemState & ODS_SELECTED != 0 && d.itemState & ODS_DISABLED == 0 && *enabled;
                if hot {
                    let hl = RECT { left: rc.left + s(6), top: rc.top + s(2), right: rc.right - s(6), bottom: rc.bottom - s(2) };
                    fill_round(hdc, hl, s(10), p.surface2);
                }
                let (fg, icon) = if *enabled { (p.text, p.accent) } else { (p.muted, p.muted) };
                let icon_box = RECT { left: rc.left + s(16), top: rc.top, right: rc.left + s(38), bottom: rc.bottom };
                text(hdc, &glyph.to_string(), icon_box, menu.icons, icon, DT_CENTER | DT_VCENTER);
                text(hdc, label, RECT { left: rc.left + s(48), top: rc.top, right: rc.right - s(60), bottom: rc.bottom }, menu.body, fg, left);
                if let Some(on) = toggle {
                    // A small switch, like the ones in the window.
                    let cy = (rc.top + rc.bottom) / 2;
                    let track = RECT { left: rc.right - s(50), top: cy - s(8), right: rc.right - s(18), bottom: cy + s(8) };
                    fill_round(hdc, track, s(16), if *on { p.accent } else { p.track });
                    let kx = if *on { track.right - s(14) } else { track.left + s(2) };
                    let knob = RECT { left: kx, top: cy - s(6), right: kx + s(12), bottom: cy + s(6) };
                    fill_round(hdc, knob, s(12), if *on && !p.dark { 0xffffff } else if *on { p.bg } else { p.muted });
                }
            }
        }
    });
}

/// Header line: what Trimly has done this session.
fn header_sub(cfg: &config::Config) -> String {
    let (freed, running) = engine::STATS.lock().map_or((0, 0), |s| (s.freed, s.apps.values().filter(|a| a.running).count()));
    if cfg.paused {
        return "Paused \u{00b7} nothing is being trimmed".into();
    }
    if cfg.apps.is_empty() {
        return "No apps yet \u{00b7} open Trimly to add one".into();
    }
    format!("{} freed \u{00b7} {} of {} running", fmt_size(freed), running, cfg.apps.len())
}

unsafe fn show_menu(hwnd: HWND) {
    let Some(cfg) = APP.with(|a| a.borrow().as_ref().map(|app| app.cfg.clone())) else { return };
    let pal = theme::palette(&cfg.theme);
    theme::set_menu_dark(pal.dark);
    let dpi = GetDpiForWindow(hwnd).max(96);
    let elevated = is_elevated();
    let rows = vec![
        Row::Header { title: "Trimly".into(), sub: header_sub(&cfg) },
        Row::Separator,
        Row::Item { glyph: '\u{E80F}', label: "Open Trimly".into(), toggle: None, enabled: true },
        Row::Item { glyph: '\u{E945}', label: "Trim all now".into(), toggle: None, enabled: !cfg.apps.is_empty() },
        Row::Item { glyph: '\u{E769}', label: "Pause trimming".into(), toggle: Some(cfg.paused), enabled: true },
        Row::Separator,
        Row::Item { glyph: '\u{E7E8}', label: "Start with Windows".into(), toggle: Some(autostart::is_enabled()), enabled: true },
        Row::Item {
            glyph: '\u{E7EF}',
            label: if elevated { "Running as administrator" } else { "Restart as administrator" }.into(),
            toggle: None,
            enabled: !elevated,
        },
        Row::Separator,
        Row::Item { glyph: '\u{E8BB}', label: "Exit".into(), toggle: None, enabled: true },
    ];
    let ids = [0, 0, ID_OPEN, ID_TRIM_ALL, ID_PAUSE, 0, ID_AUTOSTART, ID_ADMIN, 0, ID_EXIT];
    let menu = CreatePopupMenu();
    for (i, row) in rows.iter().enumerate() {
        let selectable = matches!(row, Row::Item { enabled: true, .. });
        let flags = MF_OWNERDRAW | if selectable { MF_ENABLED } else { MF_DISABLED };
        AppendMenuW(menu, flags, ids[i], i as *const u16); // owner-drawn: the "string" is our row index
    }
    let info = MENUINFO {
        cbSize: size_of::<MENUINFO>() as u32,
        fMask: MIM_BACKGROUND,
        hbrBack: CreateSolidBrush(pal.surface),
        ..zeroed()
    };
    SetMenuInfo(menu, &info);
    MENU.with(|m| {
        *m.borrow_mut() = Some(Menu {
            rows,
            pal,
            dpi: dpi as i32,
            body: make_font(dpi, 95, 400, None),
            bold: make_font(dpi, 110, 600, None),
            small: make_font(dpi, 85, 400, None),
            icons: make_font(dpi, 110, 400, Some("Segoe MDL2 Assets")),
        })
    });

    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, null());
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);
    DeleteObject(info.hbrBack as HGDIOBJ);
    MENU.with(|m| m.borrow_mut().take());
    if cmd > 0 {
        command(hwnd, cmd as usize);
    }
}

/// Changes the config from the tray (outside the window) and applies it.
pub fn update_cfg(f: impl FnOnce(&mut config::Config)) {
    APP.with(|a| {
        if let Ok(mut a) = a.try_borrow_mut() {
            if let Some(app) = a.as_mut() {
                f(&mut app.cfg);
                app.commit();
            }
        }
    });
}

unsafe fn command(hwnd: HWND, id: usize) {
    match id {
        ID_OPEN => settings::open(),
        ID_TRIM_ALL => {
            let Some(cfg) = APP.with(|a| a.borrow().as_ref().map(|app| app.cfg.clone())) else { return };
            let freed = engine::trim_all(&cfg);
            notify(hwnd, &if freed >= 1024 * 1024 {
                format!("Freed {} across your apps.", fmt_size(freed))
            } else {
                "Everything is already lean.".to_string()
            });
        }
        ID_PAUSE => update_cfg(|c| c.paused = !c.paused),
        ID_AUTOSTART => autostart::set(!autostart::is_enabled()),
        ID_ADMIN => {
            let exe = wide(&std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default());
            let verb = wide("runas");
            let args = wide("--replace");
            let r = ShellExecuteW(null_mut(), verb.as_ptr(), exe.as_ptr(), args.as_ptr(), null(), SW_SHOWNORMAL);
            if r as isize > 32 {
                DestroyWindow(hwnd); // the elevated copy waits for us to exit
            }
        }
        ID_EXIT => {
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let ev = loword(lp as usize);
            if ev == WM_LBUTTONUP {
                settings::open();
            } else if ev == WM_RBUTTONUP || ev == WM_CONTEXTMENU {
                show_menu(hwnd);
            }
            0
        }
        WM_MEASUREITEM => {
            measure(&mut *(lp as *mut MEASUREITEMSTRUCT));
            1
        }
        WM_DRAWITEM => {
            draw(&*(lp as *const DRAWITEMSTRUCT));
            1
        }
        WM_OPEN_SETTINGS => {
            settings::open();
            0
        }
        WM_DESTROY => {
            settings::close();
            remove_icon(hwnd);
            engine::shutdown();
            PostQuitMessage(0);
            0
        }
        m if m != 0 && m == TASKBAR_CREATED.get() => {
            add_icon(hwnd); // Explorer restarted
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

pub fn run(app: App) {
    unsafe {
        let hinst = GetModuleHandleW(null());
        let class = wide(TRAY_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinst,
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        RegisterClassExW(&wc);
        let tc = wide("TaskbarCreated");
        TASKBAR_CREATED.set(RegisterWindowMessageW(tc.as_ptr()));
        // A hidden top-level window (not message-only) so it receives TaskbarCreated.
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), WS_OVERLAPPED,
            0, 0, 0, 0, null_mut(), null_mut(), hinst, null());

        engine::start(&app.cfg);
        APP.with(|a| *a.borrow_mut() = Some(app));
        add_icon(hwnd);
        update::check_in_background();

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
