//! DiscordTrim: a tiny tray app that keeps Discord's memory use down.
//! See README.md for how it works and how to add a feature.

#![windows_subsystem = "windows"]

mod autostart;
mod config;
mod procs;
mod settings;
mod tray;
mod tweaks;
mod update;
mod util;

use std::ptr::null;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, FindWindowW, PostMessageW, ASFW_ANY};

fn main() {
    let name = util::wide("Local\\DiscordTrim.SingleInstance");
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

        let mut cfg = config::load();
        if !cfg.first_run_done {
            // Start with Windows by default; the tray menu can turn it off.
            autostart::set(true);
            cfg.first_run_done = true;
            config::save(&cfg);
        } else if autostart::is_enabled() {
            autostart::set(true); // keep the startup entry pointing at this exe
        }
        let app = tray::App { cfg, tweaks: tweaks::all() };
        tray::run(app);
        CloseHandle(mutex);
    }
}
