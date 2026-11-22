//! Tray icon, right-click menu and the app's message loop.

use crate::{autostart, config, settings, tweaks, update, util::{loword, wide}};
use std::cell::RefCell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 1;
/// Posted by a second launch (e.g. the Start menu shortcut) to open Settings in this instance.
pub const WM_OPEN_SETTINGS: u32 = WM_APP + 2;
pub const TRAY_CLASS: &str = "DiscordTrimTray";
const ID_TWEAK_BASE: usize = 1000;
const ID_AUTOSTART: usize = 2003;
const ID_OPEN_CONFIG: usize = 2004;
const ID_ADMIN: usize = 2005;
const ID_EXIT: usize = 2006;
const ID_SETTINGS: usize = 2007;

pub struct App {
    pub cfg: config::Config,
    pub tweaks: Vec<Box<dyn tweaks::Tweak>>,
}

impl App {
    fn wanted(&self, t: &dyn tweaks::Tweak) -> bool {
        self.cfg.tweaks.get(t.id()).copied().unwrap_or_else(|| t.default_enabled())
    }

    /// Brings every feature in line with the config.
    pub fn sync(&mut self) {
        for i in 0..self.tweaks.len() {
            let want = self.wanted(self.tweaks[i].as_ref());
            let t = &mut self.tweaks[i];
            if want && !t.is_enabled() {
                if let Err(e) = t.enable(&self.cfg) {
                    error_box(&format!("{} could not start:\n{e}", t.name()));
                }
            } else if !want && t.is_enabled() {
                t.disable();
            } else if want {
                t.apply_config(&self.cfg);
            }
        }
    }
}

thread_local! {
    pub static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub fn error_box(text: &str) {
    let t = wide(text);
    let c = wide("DiscordTrim");
    unsafe { MessageBoxW(null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONWARNING) };
}

unsafe fn add_icon(hwnd: HWND) {
    let hinst = GetModuleHandleW(null());
    let mut icon = LoadImageW(hinst, 1 as _, IMAGE_ICON, GetSystemMetrics(SM_CXSMICON),
        GetSystemMetrics(SM_CYSMICON), LR_DEFAULTCOLOR) as HICON;
    if icon.is_null() {
        icon = LoadIconW(null_mut(), IDI_APPLICATION);
    }
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = icon;
    let tip = wide("DiscordTrim");
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    Shell_NotifyIconW(NIM_ADD, &nid);
}

unsafe fn remove_icon(hwnd: HWND) {
    let mut nid: NOTIFYICONDATAW = zeroed();
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    Shell_NotifyIconW(NIM_DELETE, &nid);
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

unsafe fn append(menu: HMENU, id: usize, label: &str, checked: bool) {
    let text = wide(label);
    let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
    AppendMenuW(menu, flags, id, text.as_ptr());
}

unsafe fn show_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    // Build the menu while borrowing, but release the borrow before the
    // modal TrackPopupMenu loop runs.
    APP.with(|a| {
        let a = a.borrow();
        let Some(app) = a.as_ref() else { return };
        for (i, t) in app.tweaks.iter().enumerate() {
            append(menu, ID_TWEAK_BASE + i, t.name(), t.is_enabled());
        }
    });
    if !APP.with(|a| a.borrow().as_ref().is_some_and(|app| app.tweaks.is_empty())) {
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
    }
    append(menu, ID_SETTINGS, "Settings...", false);
    append(menu, ID_AUTOSTART, "Start with Windows", autostart::is_enabled());
    append(menu, ID_OPEN_CONFIG, "Open config folder", false);
    // Discord's main process can only be trimmed from an elevated instance.
    if is_elevated() {
        append(menu, ID_ADMIN, "Running as administrator", false);
    } else {
        append(menu, ID_ADMIN, "Restart as administrator", false);
    }
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
    append(menu, ID_EXIT, "Exit", false);

    let mut pt: POINT = zeroed();
    GetCursorPos(&mut pt);
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, null());
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);
    if cmd > 0 {
        command(hwnd, cmd as usize);
    }
}

unsafe fn command(hwnd: HWND, id: usize) {
    match id {
        ID_SETTINGS => settings::open(),
        ID_AUTOSTART => autostart::set(!autostart::is_enabled()),
        ID_OPEN_CONFIG => {
            let dir = wide(&config::dir().display().to_string());
            let op = wide("open");
            ShellExecuteW(null_mut(), op.as_ptr(), dir.as_ptr(), null(), null(), SW_SHOWNORMAL);
        }
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
        id if (ID_TWEAK_BASE..ID_TWEAK_BASE + 100).contains(&id) => {
            APP.with(|a| {
                if let Some(app) = a.borrow_mut().as_mut() {
                    if let Some(t) = app.tweaks.get(id - ID_TWEAK_BASE) {
                        let on = !t.is_enabled();
                        let key = t.id().to_string();
                        app.cfg.tweaks.insert(key, on);
                    }
                    config::save(&app.cfg);
                    app.sync();
                }
            });
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
        WM_OPEN_SETTINGS => {
            settings::open();
            0
        }
        WM_DESTROY => {
            remove_icon(hwnd);
            APP.with(|a| {
                if let Some(app) = a.borrow_mut().as_mut() {
                    for t in app.tweaks.iter_mut() {
                        t.disable();
                    }
                }
            });
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

        APP.with(|a| *a.borrow_mut() = Some(app));
        APP.with(|a| {
            if let Some(app) = a.borrow_mut().as_mut() {
                app.sync();
            }
        });
        add_icon(hwnd);
        update::check_in_background();

        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            // Lets Tab, Enter (Save) and Esc (Close) work inside the settings window.
            let sh = settings::hwnd();
            if !sh.is_null() && IsDialogMessageW(sh, &msg) != 0 {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
