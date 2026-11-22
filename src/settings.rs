//! The Trimly window: the page in `ui.html`, shown in a WebView2 inside a frameless, resizable
//! Win32 window. The page draws its own title bar and resize edges and sends small JSON commands
//! (add an app, change its mode, trim now, pick a theme...). Rust answers by pushing the full
//! state to `dt.update(...)` every second and after each command.
//! The webview exists only while the window is open, so it costs no memory in the background.

use crate::config::{AppCfg, Config, Mode};
use crate::tray::{error_box, update_cfg, APP};
use crate::{autostart, config, engine, procs, theme, update, util::wide};
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::mem::{size_of, zeroed};
use std::num::NonZeroIsize;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

const HTML: &str = include_str!("ui.html");
/// Posted after a command so the page gets fresh state outside the webview's callback.
const WM_PUSH_STATE: u32 = WM_APP + 20;
const TIMER_ID: usize = 1;
const MB: f64 = 1024.0 * 1024.0;
// Default and minimum window size in 96-DPI pixels.
const DEFAULT_SIZE: (i32, i32) = (480, 800);
const MIN_SIZE: (i32, i32) = (400, 540);
const THEMES: [&str; 4] = ["system", "mint", "ember", "daylight"];
/// Processes never offered in the "Add app" list: core Windows parts and Trimly's own.
const HIDDEN_PROCESSES: [&str; 16] = [
    "system", "registry", "memory compression", "secure system", "smss.exe", "csrss.exe", "wininit.exe",
    "services.exe", "lsass.exe", "svchost.exe", "dwm.exe", "fontdrvhost.exe", "winlogon.exe", "trimly.exe",
    "msedgewebview2.exe", "lsaiso.exe",
];

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(null_mut()) };
    static WEBVIEW: RefCell<Option<WebView>> = const { RefCell::new(None) };
    static CLASS_READY: Cell<bool> = const { Cell::new(false) };
    /// exe path -> display name, so reopening the picker is fast.
    static NAMES: RefCell<HashMap<PathBuf, String>> = RefCell::new(HashMap::new());
}

/// Commands the page can send. Anything else is ignored.
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Cmd {
    Ready,
    Close,
    Minimize,
    ToggleMaximize,
    /// Mouse down on the page's title bar: hand the drag to Windows.
    Drag,
    /// Mouse down on a window edge: "l", "r", "t", "b", "tl", "tr", "bl" or "br".
    Resize { edge: String },
    TrimNow { app: Option<String> },
    ListProcesses,
    AddApp { exe: String },
    RemoveApp { app: String },
    SetMode { app: String, mode: Mode },
    SetEco { app: String, on: bool },
    Timing { which: String, secs: u64 },
    Pause { on: bool },
    Autostart { on: bool },
    Theme { name: String },
    Update,
    OpenFolder,
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

fn cfg() -> Config {
    APP.with(|a| a.try_borrow().ok().and_then(|a| a.as_ref().map(|app| app.cfg.clone()))).unwrap_or_default()
}

pub fn open() {
    unsafe {
        let existing = WINDOW.get();
        if !existing.is_null() {
            ShowWindow(existing, SW_RESTORE);
            SetForegroundWindow(existing);
            return;
        }
        let cfg = cfg();
        let pal = theme::palette(&cfg.theme);
        let hinst = GetModuleHandleW(null());
        let class = wide("TrimlySettings");
        if !CLASS_READY.get() {
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinst,
                lpszClassName: class.as_ptr(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hIcon: LoadIconW(hinst, 1 as _),
                ..zeroed()
            };
            CLASS_READY.set(RegisterClassExW(&wc) != 0);
        }
        let title = wide("Trimly");
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_THICKFRAME;
        let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), style, CW_USEDEFAULT, CW_USEDEFAULT,
            0, 0, null_mut(), null_mut(), hinst, null());
        if hwnd.is_null() {
            return;
        }
        WINDOW.set(hwnd);
        set_background(hwnd, pal.bg);
        // The page draws its own title bar and edges. WM_NCCALCSIZE removes the system frame; a
        // 1 px frame extension keeps the DWM drop shadow. Rounded corners apply on Windows 11 only.
        let margins = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 };
        DwmExtendFrameIntoClientArea(hwnd, &margins);
        let round: i32 = 2; // DWMWCP_ROUND
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &round as *const i32 as _, 4);
        place(hwnd, cfg.window_size.unwrap_or(DEFAULT_SIZE));

        let bg = pal.bg;
        // Open in the saved theme, so the page never flashes the default one first.
        let html = HTML.replacen(r#"<html lang="en" data-theme="mint">"#,
            &format!(r#"<html lang="en" data-theme="{}">"#, theme::resolve(&cfg.theme)), 1);
        let mut ctx = WebContext::new(Some(config::dir().join("WebView2")));
        let built = WebViewBuilder::new_with_web_context(&mut ctx)
            .with_html(html)
            .with_background_color(((bg & 0xff) as u8, (bg >> 8 & 0xff) as u8, (bg >> 16 & 0xff) as u8, 255))
            .with_devtools(cfg!(debug_assertions))
            .with_ipc_handler(|req| on_command(req.body()))
            // Lock the webview to our one inline page (it loads as about:blank). No web pages,
            // no file:// pages, no popups, no downloads, no dropped files, no browser shortcuts,
            // so nothing outside ui.html can ever run here or talk to the app.
            .with_navigation_handler(|url| url.starts_with("about:") || url.starts_with("data:"))
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_drag_drop_handler(|_| true)
            .with_browser_accelerator_keys(false)
            .with_default_context_menus(false)
            .with_hotkeys_zoom(false)
            .build(&Host(hwnd));
        match built {
            Ok(webview) => WEBVIEW.with(|w| *w.borrow_mut() = Some(webview)),
            Err(e) => {
                DestroyWindow(hwnd);
                error_box(&format!(
                    "The Trimly window needs the Microsoft Edge WebView2 Runtime.\n\
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

/// Closes the window if it is open (used when Trimly exits).
pub fn close() {
    let hwnd = WINDOW.get();
    if !hwnd.is_null() {
        unsafe { DestroyWindow(hwnd) };
    }
}

/// Paints the area behind the page in the theme color, so resizing never flashes white.
unsafe fn set_background(hwnd: HWND, color: u32) {
    let old = SetClassLongPtrW(hwnd, GCLP_HBRBACKGROUND, CreateSolidBrush(color) as isize);
    if old != 0 {
        DeleteObject(old as HGDIOBJ);
    }
    InvalidateRect(hwnd, null(), 1);
}

fn dpi(hwnd: HWND) -> i32 {
    unsafe { GetDpiForWindow(hwnd) }.max(96) as i32
}

/// Sizes the window (96-DPI size, scaled to its monitor) and centers it in the work area.
/// SWP_FRAMECHANGED applies the WM_NCCALCSIZE change.
unsafe fn place(hwnd: HWND, (w96, h96): (i32, i32)) {
    let d = dpi(hwnd);
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY), &mut mi);
    let work = mi.rcWork;
    let w = (w96.max(MIN_SIZE.0) * d / 96).min(work.right - work.left);
    let h = (h96.max(MIN_SIZE.1) * d / 96).min(work.bottom - work.top);
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

fn mb(bytes: u64) -> f64 {
    bytes as f64 / MB
}

/// Everything the page shows, as JSON.
fn state_json() -> String {
    let cfg = cfg();
    let hwnd = WINDOW.get();
    let (total, avail) = engine::system_memory();
    let update = update::AVAILABLE.lock().ok().and_then(|g| g.as_ref().map(|u| u.version.clone()));
    let s = engine::STATS.lock();
    let Ok(s) = s else { return "null".into() };
    let apps: Vec<Value> = cfg.apps.iter().map(|app| {
        let a = s.apps.get(app.key()).cloned().unwrap_or_default();
        json!({
            "key": app.key(),
            "name": app.name,
            "exes": app.exes,
            "mode": app.mode,
            "eco": app.eco,
            "running": a.running,
            "ramMb": mb(a.ram),
            "peakMb": mb(a.peak),
            "procs": a.procs,
            "freedMb": mb(a.freed),
            "allFreedMb": mb(s.all.per_app.get(&app.name).copied().unwrap_or(0)),
            "trims": a.trims,
            "lastTrim": a.last_trim,
        })
    }).collect();
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "now": engine::now(),
        "active": unsafe { GetForegroundWindow() } == hwnd,
        "maximized": unsafe { IsZoomed(hwnd) } != 0,
        "theme": theme::resolve(&cfg.theme),
        "themeSetting": cfg.theme,
        "paused": cfg.paused,
        "autostart": autostart::is_enabled(),
        "hiddenEvery": cfg.hidden_every_secs,
        "alwaysEvery": cfg.always_every_secs,
        "update": update,
        "system": { "totalMb": mb(total), "usedMb": mb(total.saturating_sub(avail)) },
        "session": { "freedMb": mb(s.freed), "trims": s.trims, "started": s.started },
        "allTime": {
            "freedMb": mb(s.all.freed), "trims": s.all.trims, "since": s.all.since,
            "biggestMb": mb(s.all.biggest), "biggestApp": s.all.biggest_app,
        },
        "history": s.history.iter().map(|&b| (mb(b) * 10.0).round() / 10.0).collect::<Vec<_>>(),
        "apps": apps,
        "events": s.events.iter().map(|e| json!({ "at": e.at, "app": e.app, "freedMb": mb(e.freed) })).collect::<Vec<_>>(),
    })
    .to_string()
}

fn push_state() {
    eval(&format!("window.dt && dt.update({})", state_json()));
}

/// Display name for an exe: its file description, else its file name without ".exe".
fn display_name(path: Option<&Path>, exe: &str) -> String {
    let fallback = || {
        let stem = exe.strip_suffix(".exe").or(exe.strip_suffix(".EXE")).unwrap_or(exe);
        let mut c = stem.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
    };
    let Some(path) = path else { return fallback() };
    NAMES.with(|n| {
        n.borrow_mut()
            .entry(path.to_path_buf())
            .or_insert_with(|| procs::file_description(path).unwrap_or_else(fallback))
            .clone()
    })
}

/// True when a folder directly holds an .exe (a product folder, not a vendor folder).
fn has_exe(dir: &Path) -> bool {
    std::fs::read_dir(dir).map_or(false, |entries| {
        entries.flatten().any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")))
    })
}

/// The folder an app is installed in (lowercase), used to group its processes: Steam's
/// steam.exe and its steamwebhelper.exe (deep in Steam\bin\cef\...) both belong to "...\steam".
///   - Under Program Files, AppData and similar: the program's folder there. A vendor folder such
///     as "Program Files\Google", which holds no exe itself, goes one level deeper ("google\chrome").
///   - Elsewhere (e.g. E:\Steam): the highest folder above the exe that directly holds an exe.
/// None for Windows' own programs, which are never grouped.
fn install_root(path: &Path) -> Option<PathBuf> {
    // Windows paths are case-insensitive, and SystemRoot is often "C:\WINDOWS" while process
    // paths say "C:\Windows", so compare in lowercase.
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    let path = lower(path);
    let env = |k: &str| std::env::var_os(k).map(|v| lower(Path::new(&v)));
    if env("SystemRoot").is_some_and(|w| path.starts_with(w)) {
        return None;
    }
    let local = env("LOCALAPPDATA");
    let bases = [local.as_ref().map(|l| l.join("programs")), local, env("ProgramFiles"), env("ProgramFiles(x86)"), env("APPDATA")];
    for base in bases.into_iter().flatten() {
        let Ok(rest) = path.strip_prefix(&base) else { continue };
        let mut parts = rest.components();
        let (Some(first), Some(second)) = (parts.next(), parts.next()) else { break };
        let top = base.join(first);
        if parts.next().is_none() || has_exe(&top) {
            return Some(top);
        }
        return Some(top.join(second));
    }
    let dir = path.parent()?;
    let mut root = dir.to_path_buf();
    for folder in dir.ancestors().skip(1) {
        if folder.parent().is_none() {
            break; // never group a whole drive
        }
        if has_exe(folder) {
            root = folder.to_path_buf();
        }
    }
    Some(root)
}

/// One installed program and its running processes.
struct Group {
    /// Lowercase exe names; the main one first.
    exes: Vec<String>,
    /// File name (original case) and path of each exe, in the same order.
    files: Vec<(String, Option<PathBuf>)>,
    pids: Vec<u32>,
}

/// Running processes grouped by the program they belong to.
fn running_groups() -> Vec<Group> {
    // Path and install folder per exe name, looked up once rather than per process.
    let mut seen: HashMap<String, (Option<PathBuf>, Option<PathBuf>)> = HashMap::new();
    let mut groups: BTreeMap<String, (Option<PathBuf>, Group)> = BTreeMap::new();
    for (pid, file) in procs::snapshot() {
        let exe = file.to_ascii_lowercase();
        if pid <= 4 || HIDDEN_PROCESSES.contains(&exe.as_str()) {
            continue;
        }
        let (path, root) = seen.entry(exe.clone()).or_insert_with(|| {
            let path = procs::exe_path(pid);
            let root = path.as_deref().and_then(install_root);
            (path, root)
        }).clone();
        let key = root.as_ref().map_or_else(|| exe.clone(), |r| r.to_string_lossy().to_lowercase());
        let (_, g) = groups.entry(key).or_insert_with(|| (root, Group { exes: vec![], files: vec![], pids: vec![] }));
        if !g.exes.contains(&exe) {
            g.exes.push(exe);
            g.files.push((file, path));
        }
        g.pids.push(pid);
    }
    groups.into_values().map(|(root, mut g)| {
        // Main exe: the one named like its folder (Steam\steam.exe), else the one with a description.
        let folder = root.as_ref().and_then(|r| r.file_name()).map(|f| f.to_string_lossy().to_lowercase());
        let main = g.exes.iter().position(|e| folder.as_deref().is_some_and(|f| e.trim_end_matches(".exe") == f))
            .or_else(|| g.files.iter().position(|(_, p)| p.as_deref().and_then(procs::file_description).is_some()))
            .unwrap_or(0);
        g.exes.swap(0, main);
        g.files.swap(0, main);
        g
    }).collect()
}

/// Running programs for the "Add app" picker, biggest first.
fn process_list(cfg: &Config) -> Vec<Value> {
    let mut list: Vec<(u64, Value)> = running_groups().into_iter().filter_map(|g| {
        let ram: u64 = g.pids.iter().filter_map(|&p| procs::memory(p)).sum();
        if ram < 15 * 1024 * 1024 {
            return None; // small background helpers just clutter the list
        }
        let (file, path) = &g.files[0];
        let added = cfg.apps.iter().any(|a| a.exes.iter().any(|e| g.exes.contains(e)));
        let more = if g.exes.len() > 1 { format!(" + {} more", g.exes.len() - 1) } else { String::new() };
        Some((ram, json!({
            "exe": g.exes[0],
            "file": format!("{file}{more}"),
            "name": display_name(path.as_deref(), file),
            "ramMb": mb(ram),
            "procs": g.pids.len(),
            "added": added,
        })))
    }).collect();
    list.sort_by(|a, b| b.0.cmp(&a.0));
    list.into_iter().take(80).map(|(_, v)| v).collect()
}

/// Builds an app from a running exe, together with every other exe of the same program.
fn new_app(exe: &str) -> Option<AppCfg> {
    let exe = exe.to_ascii_lowercase();
    let g = running_groups().into_iter().find(|g| g.exes.contains(&exe))?;
    let (file, path) = &g.files[0];
    let name: String = display_name(path.as_deref(), file).chars().take(60).collect();
    Some(AppCfg { name, exes: g.exes, mode: Mode::Hidden, eco: false })
}

/// Changes one app (found by key) in the config.
fn with_app(key: &str, f: impl FnOnce(&mut AppCfg)) {
    update_cfg(|c| {
        if let Some(app) = c.apps.iter_mut().find(|a| a.key() == key) {
            f(app);
        }
    });
}

/// Handles one message from the page. The page is ours, but input is still checked here.
fn on_command(body: &str) {
    let Ok(cmd) = serde_json::from_str::<Cmd>(body) else { return };
    let hwnd = WINDOW.get();
    match cmd {
        Cmd::Ready => {}
        Cmd::Close => {
            unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
            return;
        }
        Cmd::Minimize => {
            unsafe { PostMessageW(hwnd, WM_SYSCOMMAND, SC_MINIMIZE as usize, 0) };
            return;
        }
        Cmd::ToggleMaximize => {
            let sc = if unsafe { IsZoomed(hwnd) } != 0 { SC_RESTORE } else { SC_MAXIMIZE };
            unsafe { PostMessageW(hwnd, WM_SYSCOMMAND, sc as usize, 0) };
            return;
        }
        Cmd::Drag | Cmd::Resize { .. } => {
            let hit = match &cmd {
                Cmd::Resize { edge } => match edge.as_str() {
                    "l" => HTLEFT, "r" => HTRIGHT, "t" => HTTOP, "b" => HTBOTTOM,
                    "tl" => HTTOPLEFT, "tr" => HTTOPRIGHT, "bl" => HTBOTTOMLEFT, "br" => HTBOTTOMRIGHT,
                    _ => return,
                },
                _ => HTCAPTION,
            };
            if hit != HTCAPTION && unsafe { IsZoomed(hwnd) } != 0 {
                return; // a maximized window has no edges to drag
            }
            unsafe {
                let mut pt: POINT = zeroed();
                GetCursorPos(&mut pt);
                ReleaseCapture();
                let at = ((pt.y as u16 as u32) << 16 | pt.x as u16 as u32) as LPARAM;
                PostMessageW(hwnd, WM_NCLBUTTONDOWN, hit as usize, at);
            }
            return;
        }
        Cmd::TrimNow { app } => {
            let cfg = cfg();
            let freed = match &app {
                Some(key) => match cfg.apps.iter().find(|a| a.key() == key) {
                    Some(a) => engine::trim_now(a),
                    None => return,
                },
                None => engine::trim_all(&cfg),
            };
            let label = serde_json::to_string(&app).unwrap_or_else(|_| "null".into());
            eval(&format!("window.dt && dt.trimmed({}, {label})", mb(freed)));
        }
        Cmd::ListProcesses => {
            let list = Value::Array(process_list(&cfg()));
            eval(&format!("window.dt && dt.processes({list})"));
            return;
        }
        Cmd::AddApp { exe } => {
            let taken = cfg().apps.iter().any(|a| a.exes.iter().any(|e| e.eq_ignore_ascii_case(&exe)));
            match new_app(&exe) {
                Some(app) if !taken => {
                    let name = serde_json::to_string(&app.name).unwrap_or_default();
                    update_cfg(|c| c.apps.push(app));
                    eval(&format!("window.dt && dt.added({name})"));
                }
                _ => return,
            }
        }
        Cmd::RemoveApp { app } => update_cfg(|c| c.apps.retain(|a| a.key() != app)),
        Cmd::SetMode { app, mode } => with_app(&app, |a| a.mode = mode),
        Cmd::SetEco { app, on } => with_app(&app, |a| a.eco = on),
        Cmd::Timing { which, secs } => {
            let secs = secs.clamp(10, 86_400);
            match which.as_str() {
                "hidden" => update_cfg(|c| c.hidden_every_secs = secs),
                "always" => update_cfg(|c| c.always_every_secs = secs),
                _ => return,
            }
        }
        Cmd::Pause { on } => update_cfg(|c| c.paused = on),
        Cmd::Autostart { on } => autostart::set(on),
        Cmd::Theme { name } => {
            if !THEMES.contains(&name.as_str()) {
                return;
            }
            update_cfg(|c| c.theme = name.clone());
            unsafe { set_background(hwnd, theme::palette(&name).bg) };
        }
        Cmd::Update => start_update(),
        Cmd::OpenFolder => unsafe {
            let dir = wide(&config::dir().display().to_string());
            let op = wide("open");
            ShellExecuteW(null_mut(), op.as_ptr(), dir.as_ptr(), null(), null(), SW_SHOWNORMAL);
        },
    }
    unsafe { PostMessageW(hwnd, WM_PUSH_STATE, 0, 0) };
}

/// Opens the new installer's download link in the browser. The user runs it from there; the
/// installer closes the running app itself (Inno Setup's CloseApplications).
fn start_update() {
    let Some(u) = update::AVAILABLE.lock().ok().and_then(|g| g.clone()) else { return };
    let (url, open) = (wide(&u.url), wide("open"));
    unsafe { ShellExecuteW(null_mut(), open.as_ptr(), url.as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER | WM_PUSH_STATE => {
            push_state();
            0
        }
        // No system title bar or frame: the whole window is client area. When maximized, Windows
        // places the window's invisible frame off-screen, so pull the content back in by that much.
        WM_NCCALCSIZE if wp != 0 => {
            if IsZoomed(hwnd) != 0 {
                let p = &mut *(lp as *mut NCCALCSIZE_PARAMS);
                let d = dpi(hwnd) as u32;
                let fx = GetSystemMetricsForDpi(SM_CXFRAME, d) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, d);
                let fy = GetSystemMetricsForDpi(SM_CYFRAME, d) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, d);
                let r = &mut p.rgrc[0];
                r.left += fx;
                r.right -= fx;
                r.top += fy;
                r.bottom -= fy;
            }
            0
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lp as *mut MINMAXINFO);
            let d = dpi(hwnd);
            info.ptMinTrackSize = POINT { x: MIN_SIZE.0 * d / 96, y: MIN_SIZE.1 * d / 96 };
            0
        }
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
            // Remember the size (unless maximized or minimized) for next time.
            if IsZoomed(hwnd) == 0 && IsIconic(hwnd) == 0 {
                let mut r: RECT = zeroed();
                GetWindowRect(hwnd, &mut r);
                let d = dpi(hwnd);
                let size = ((r.right - r.left) * 96 / d, (r.bottom - r.top) * 96 / d);
                update_cfg(|c| c.window_size = Some(size));
            }
            // Dropping the webview shuts down its browser processes.
            WEBVIEW.with(|w| w.borrow_mut().take());
            WINDOW.set(null_mut());
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

#[cfg(test)]
mod tests {
    use super::install_root;
    use std::path::PathBuf;

    #[test]
    fn groups_by_install_folder() {
        // A fake %LOCALAPPDATA% with a product folder (exe at its top) and a vendor folder (no exe).
        let base = std::env::temp_dir().join(format!("trimly-test-{}", std::process::id()));
        let steam = base.join("Steam");
        std::fs::create_dir_all(steam.join("bin").join("cef")).unwrap();
        std::fs::write(steam.join("steam.exe"), b"").unwrap();
        std::fs::create_dir_all(base.join("Vendor").join("Product")).unwrap();
        std::env::set_var("LOCALAPPDATA", &base);

        // Roots come back lowercase.
        let lc = |p: PathBuf| Some(PathBuf::from(p.to_string_lossy().to_lowercase()));
        let helper = steam.join("bin").join("cef").join("steamwebhelper.exe");
        assert_eq!(install_root(&helper), lc(steam.clone()));
        assert_eq!(install_root(&steam.join("steam.exe")), lc(steam));
        let product = base.join("Vendor").join("Product").join("app").join("product.exe");
        assert_eq!(install_root(&product), lc(base.join("Vendor").join("Product")));

        // Process paths often differ in case from %SystemRoot% ("C:\Windows" vs "C:\WINDOWS").
        let windows = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap();
        let other_case = PathBuf::from(windows.to_string_lossy().to_lowercase());
        assert_eq!(install_root(&other_case.join("system32").join("audiodg.exe")), None);
        let _ = std::fs::remove_dir_all(&base);
    }
}
