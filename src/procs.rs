//! Finds an app's processes by exe name and tells whether any of them has a window on screen.
//! Shared by the trim and throttle features.

use std::mem::size_of;
use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SET_QUOTA,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

/// Process ids of every running process whose exe name is in `exes`
/// (for Discord: the main process, renderers, GPU and utility processes).
pub fn pids(exes: &[&str]) -> Vec<u32> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if exes.iter().any(|e| name.eq_ignore_ascii_case(e)) {
                    out.push(entry.th32ProcessID);
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

struct Scan<'a> {
    pids: &'a [u32],
    shown: bool,
}

unsafe extern "system" fn visit(hwnd: HWND, lp: LPARAM) -> BOOL {
    let scan = &mut *(lp as *mut Scan);
    if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
        return 1;
    }
    // Skip tool windows such as the in-game overlay, which is not the Discord window the user sees.
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    if ex & WS_EX_TOOLWINDOW != 0 {
        return 1;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if scan.pids.contains(&pid) && GetWindowTextLengthW(hwnd) > 0 {
        scan.shown = true;
        return 0; // stop enumerating
    }
    1
}

/// True when a visible, restored, titled window belongs to one of `pids`.
pub fn any_window_shown(pids: &[u32]) -> bool {
    if pids.is_empty() {
        return false;
    }
    let mut scan = Scan { pids, shown: false };
    unsafe { EnumWindows(Some(visit), &mut scan as *mut Scan as LPARAM) };
    scan.shown
}

/// Opens a process with the rights needed for trimming and power throttling.
pub fn open(pid: u32) -> Option<HANDLE> {
    let rights = PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION | PROCESS_SET_INFORMATION;
    let h = unsafe { OpenProcess(rights, 0, pid) };
    (!h.is_null()).then_some(h)
}

pub fn close(h: HANDLE) {
    unsafe { CloseHandle(h) };
}
