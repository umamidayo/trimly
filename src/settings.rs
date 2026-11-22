//! Settings window: the page in `ui.html`, shown in a WebView2 inside a plain Win32 window.
//! The page sends small JSON commands (toggle a feature, change a timing, trim now); Rust
//! answers by pushing the full state to `dt.update(...)` every second and after each command.
//! The webview exists only while the window is open, so it costs no memory in the background.

use crate::{autostart, config::{self, Config}, tray::{error_box, APP}, tweaks::trim, update, util::wide};
use serde::Deserialize;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::mem::{size_of, zeroed};
use std::num::NonZeroIsize;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
};
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use wry::{WebContext, WebView, WebViewBuilder};

const HTML: &str = include_str!("ui.html");
/// Posted after a command so the page gets fresh state outside the webview's callback.
const WM_PUSH_STATE: u32 = WM_APP + 20;
const TIMER_ID: usize = 1;
const MB: f64 = 1024.0 * 1024.0;
// Window client size in 96-DPI pixels.
const WIDTH: i32 = 460;
const HEIGHT: i32 = 790;

/// True while the update installer is downloading, so a second click can't start another.
static DOWNLOADING: AtomicBool = AtomicBool::new(false);

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
    static WEBVIEW: RefCell<Option<WebView>> = const { RefCell::new(None) };
    static CLASS_READY: Cell<bool> = const { Cell::new(false) };
}

/// Commands the page can send. Anything else is ignored.
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Cmd {
    Ready,
    Feature { id: String, on: bool },
    Autostart { on: bool },
    Timing { index: usize, value: u64 },
    TrimNow { app: Option<String> },
    Update,
    Close,
    Minimize,
    /// Mouse down on the page's title bar: hand the drag to Windows.
    Drag,
}

/// Config fields behind the page's timing rows, by index.
fn timing_mut(cfg: &mut Config, i: usize) -> Option<&mut u64> {
    Some(match i {
        0 => &mut cfg.trim_delay_secs,
        1 => &mut cfg.trim_interval_secs,
        2 => &mut cfg.active_trim_interval_secs,
        3 => &mut cfg.explorer_idle_secs,
        4 => &mut cfg.explorer_interval_secs,
        _ => return None,
    })
}

/// Lets wry attach the webview to our Win32 window.
struct Host(HWND);

impl HasWindowHandle for Host {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?;
        // Safe: the window outlives the borrow; it is destroyed only after the webview is dropped.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(hwnd))) })
    }
}

/// True when Windows is set to dark mode for apps.
fn system_dark() -> bool {
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let name = wide("AppsUseLightTheme");
    let (mut v, mut size) = (1u32, 4u32);
    let rc = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, null_mut(),
            &mut v as *mut u32 as _, &mut size)
    };
    rc == ERROR_SUCCESS && v == 0
}

pub fn open() {
    unsafe {
        let existing = WINDOW.get();
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            return;
        }
        let dark = system_dark();
        // Matches the page background so there is no white flash before it loads.
        let bg: (u8, u8, u8) = if dark { (14, 15, 19) } else { (243, 244, 248) };
        let hinst = GetModuleHandleW(null());
        let class = wide("TrimlySettings");
        if !CLASS_READY.get() {
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinst,
                lpszClassName: class.as_ptr(),
                hbrBackground: CreateSolidBrush(bg.0 as u32 | (bg.1 as u32) << 8 | (bg.2 as u32) << 16),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hIcon: LoadIconW(hinst, 1 as _),
                ..zeroed()
            };
            CLASS_READY.set(RegisterClassExW(&wc) != 0);
        }
        let title = wide("Trimly");
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), style, CW_USEDEFAULT, CW_USEDEFAULT,
            0, 0, null_mut(), null_mut(), hinst, null());
        if hwnd.is_null() {
            return;
        }
        WINDOW.set(hwnd);
        if dark {
            let on: i32 = 1;
            DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &on as *const i32 as _, 4);
        }
        // The page draws its own title bar. WM_NCCALCSIZE removes the system one; a 1 px frame
        // extension keeps the DWM drop shadow. Rounded corners apply on Windows 11 only.
        let margins = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 };
        DwmExtendFrameIntoClientArea(hwnd, &margins);
        let round: i32 = 2; // DWMWCP_ROUND
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &round as *const i32 as _, 4);
        place(hwnd);

        let mut ctx = WebContext::new(Some(config::dir().join("WebView2")));
        let built = WebViewBuilder::new_with_web_context(&mut ctx)
            .with_html(HTML)
            .with_background_color((bg.0, bg.1, bg.2, 255))
            .with_devtools(cfg!(debug_assertions))
            .with_ipc_handler(|req| on_command(req.body()))
            .build(&Host(hwnd));
        match built {
            Ok(webview) => WEBVIEW.with(|w| *w.borrow_mut() = Some(webview)),
            Err(e) => {
                DestroyWindow(hwnd);
                error_box(&format!(
                    "The settings window needs the Microsoft Edge WebView2 Runtime.\n\
                     Get it from https://developer.microsoft.com/microsoft-edge/webview2/\n\n({e})"
                ));
                return;
            }
        }
        SetTimer(hwnd, TIMER_ID, 1000, None);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
}

/// Sizes the window for its monitor's DPI and centers it in the work area. The window has no
/// system frame, so its size is the page size. SWP_FRAMECHANGED applies the WM_NCCALCSIZE change.
unsafe fn place(hwnd: HWND) {
    let dpi = GetDpiForWindow(hwnd).max(96) as i32;
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY), &mut mi);
    let work = mi.rcWork;
    let (w, h) = (WIDTH * dpi / 96, (HEIGHT * dpi / 96).min(work.bottom - work.top));
    let (x, y) = (work.left + (work.right - work.left - w) / 2, work.top + (work.bottom - work.top - h) / 2);
    SetWindowPos(hwnd, null_mut(), x, y, w, h, SWP_NOZORDER | SWP_FRAMECHANGED);
}

/// Runs a script in the page, if it is open and not busy in another call.
fn eval(js: &str) {
    WEBVIEW.with(|w| {
        if let Ok(w) = w.try_borrow() {
            if let Some(webview) = w.as_ref() {
                let _ = webview.evaluate_script(js);
            }
        }
    });
}

/// Everything the page shows, as JSON.
fn state_json() -> String {
    let (features, timings) = APP.with(|a| {
        let a = a.try_borrow().ok()?;
        let app = a.as_ref()?;
        let features: Vec<_> = app.tweaks.iter()
            .map(|t| json!({ "id": t.id(), "name": t.name(), "on": t.is_enabled() }))
            .collect();
        let mut cfg = app.cfg.clone();
        let timings: Vec<u64> = (0..5).filter_map(|i| timing_mut(&mut cfg, i).map(|v| *v)).collect();
        Some((features, timings))
    }).unwrap_or_default();
    let freed_by_app = trim::FREED_BY_APP.lock().map(|m| m.clone()).unwrap_or_default();
    let feature_on = |id: &str| features.iter().any(|f| f["id"] == id && f["on"] == true);
    let apps: Vec<_> = trim::APPS.iter().map(|app| {
        let ram = trim::app_ram(app);
        json!({
            "name": app.name,
            "running": ram.is_some(),
            "ramMb": ram.unwrap_or(0) as f64 / MB,
            "auto": app.features.iter().any(|id| feature_on(id)),
            "freedMb": freed_by_app.get(app.name).copied().unwrap_or(0) as f64 / MB,
        })
    }).collect();
    let update = update::AVAILABLE.lock().ok().and_then(|g| g.as_ref().map(|u| u.version.clone()));
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "apps": apps,
        "trims": trim::TRIMS.load(Ordering::Relaxed),
        "freedMb": trim::FREED_BYTES.load(Ordering::Relaxed) as f64 / MB,
        "features": features,
        "autostart": autostart::is_enabled(),
        "timings": timings,
        "update": update,
        "downloading": DOWNLOADING.load(Ordering::Relaxed),
        "active": unsafe { GetForegroundWindow() } == WINDOW.get(),
    })
    .to_string()
}

fn push_state() {
    eval(&format!("window.dt && dt.update({})", state_json()));
}

/// Applies a change to the running app and saves it. Skipped if the app is busy (it never
/// is for long; the next state push shows the real value either way).
fn with_app(f: impl FnOnce(&mut crate::tray::App) -> bool) {
    APP.with(|a| {
        let Ok(mut a) = a.try_borrow_mut() else { return };
        let Some(app) = a.as_mut() else { return };
        if f(app) {
            config::save(&app.cfg);
            app.sync();
        }
    });
}

/// Handles one message from the page. The page is ours, but input is still checked here.
fn on_command(body: &str) {
    let Ok(cmd) = serde_json::from_str::<Cmd>(body) else { return };
    match cmd {
        Cmd::Ready => {}
        Cmd::Feature { id, on } => with_app(|app| {
            let known = app.tweaks.iter().any(|t| t.id() == id);
            if known {
                app.cfg.tweaks.insert(id, on);
            }
            known
        }),
        Cmd::Autostart { on } => autostart::set(on),
        Cmd::Timing { index, value } => with_app(|app| {
            let Some(slot) = timing_mut(&mut app.cfg, index) else { return false };
            *slot = value.clamp(1, 86_400);
            true
        }),
        Cmd::TrimNow { app } => {
            // A named app trims just that app; no name trims every tracked app.
            let picked: Vec<_> = trim::APPS.iter().filter(|a| app.as_deref().map_or(true, |n| n == a.name)).collect();
            if picked.is_empty() {
                return;
            }
            let freed: u64 = picked.iter().map(|a| trim::trim_app(a)).sum();
            let label = serde_json::to_string(&app).unwrap_or_else(|_| "null".into());
            eval(&format!("window.dt && dt.trimmed({}, {label})", freed as f64 / MB));
        }
        Cmd::Update => start_update(),
        Cmd::Close => {
            unsafe { PostMessageW(WINDOW.get(), WM_CLOSE, 0, 0) };
            return;
        }
        Cmd::Minimize => {
            unsafe { PostMessageW(WINDOW.get(), WM_SYSCOMMAND, SC_MINIMIZE as usize, 0) };
            return;
        }
        Cmd::Drag => {
            unsafe {
                let mut pt: POINT = zeroed();
                GetCursorPos(&mut pt);
                ReleaseCapture();
                let at = ((pt.y as u16 as u32) << 16 | pt.x as u16 as u32) as LPARAM;
                PostMessageW(WINDOW.get(), WM_NCLBUTTONDOWN, HTCAPTION as usize, at);
            }
            return;
        }
    }
    unsafe { PostMessageW(WINDOW.get(), WM_PUSH_STATE, 0, 0) };
}

/// Downloads the newest installer on a worker thread, then runs it. The installer closes the
/// running app itself (Inno Setup's CloseApplications).
fn start_update() {
    let Some(u) = update::AVAILABLE.lock().ok().and_then(|g| g.clone()) else { return };
    if DOWNLOADING.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(move || {
        match update::download(&u) {
            Some(path) => unsafe {
                let file = wide(&path.display().to_string());
                let open = wide("open");
                ShellExecuteW(null_mut(), open.as_ptr(), file.as_ptr(), null(), null(), SW_SHOWNORMAL);
                if let Ok(mut slot) = update::AVAILABLE.lock() {
                    *slot = None;
                }
            },
            None => error_box("Could not download the update. Check your internet connection, then try again."),
        }
        DOWNLOADING.store(false, Ordering::Relaxed);
    });
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER | WM_PUSH_STATE => {
            push_state();
            0
        }
        // No system title bar or frame: the whole window is client area.
        WM_NCCALCSIZE if wp != 0 => 0,
        // Focus changed: refresh so the title bar dims or brightens right away.
        WM_ACTIVATE => {
            PostMessageW(hwnd, WM_PUSH_STATE, 0, 0);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        // Moved to a monitor with a different scale: take the size Windows suggests.
        WM_DPICHANGED => {
            let r = &*(lp as *const RECT);
            SetWindowPos(hwnd, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            0
        }
        WM_DESTROY => {
            KillTimer(hwnd, TIMER_ID);
            // Dropping the webview shuts down its browser processes.
            WEBVIEW.with(|w| w.borrow_mut().take());
            WINDOW.set(null_mut());
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
