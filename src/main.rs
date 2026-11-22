//! Trimly: a tiny tray app that keeps the apps you choose lean on memory.
//! See README.md for how it works.

#![windows_subsystem = "windows"]

mod autostart;
mod config;
mod engine;
mod procs;
mod settings;
mod theme;
mod tray;
mod update;
mod util;

use std::ptr::null;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, FindWindowW, PostMessageW, ASFW_ANY};

fn main() {
    let name = util::wide("Local\\Trimly.SingleInstance");
    let replace = std::env::args().any(|a| a == "--replace");
    unsafe {
        let mutex = CreateMutexW(null(), 1, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if !replace {
                // Already running: open its Settings window, and let it take the foreground.
                let class = util::wide(tray::TRAY_CLASS);
                let hwnd = FindWindowW(class.as_ptr(), null());
                if !hwnd.is_null() {
                    AllowSetForegroundWindow(ASFW_ANY);
                    PostMessageW(hwnd, tray::WM_OPEN_SETTINGS, 0, 0);
                }
                return;
            }
            // "Restart as administrator": wait (max 5 s) for the old instance to exit.
            let r = WaitForSingleObject(mutex, 5000);
            if r != WAIT_OBJECT_0 && r != WAIT_ABANDONED {
                return;
            }
        }

        config::migrate_old_dir();
        autostart::migrate_old_value();
        // Start with Windows is only ever turned on by the user: a checkbox in the installer, the
        // tray menu or the window. Adding it silently on first launch looks like malware
        // persistence to antivirus heuristics.
        let app = tray::App { cfg: config::load() };
        tray::run(app);
        CloseHandle(mutex);
    }
}
