//! Process lookups: running exes, their paths, names and memory, and whether an app has a window
//! on screen. Used by the engine and by the "Add app" picker.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM};
use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SET_QUOTA,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};
use crate::util::wide;

/// (pid, exe name) of every running process.
pub fn snapshot() -> Vec<(u32, String)> {
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
                out.push((entry.th32ProcessID, String::from_utf16_lossy(&entry.szExeFile[..len])));
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    out
}

/// Process ids of every running process whose exe name is in `exes` (any case).
pub fn pids(exes: &[String]) -> Vec<u32> {
    snapshot().into_iter().filter(|(_, n)| exes.iter().any(|e| n.eq_ignore_ascii_case(e))).map(|(p, _)| p).collect()
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
    // Skip tool windows such as Discord's in-game overlay, which aren't the app's own window.
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

/// Opens a process just to read it. Works on processes `open` can't touch, such as elevated ones.
fn open_query(pid: u32) -> Option<HANDLE> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    (!h.is_null()).then_some(h)
}

pub fn close(h: HANDLE) {
    unsafe { CloseHandle(h) };
}

/// Working set (RAM in use) of an open process, in bytes.
pub fn working_set(h: HANDLE) -> u64 {
    let mut c: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    c.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let ok = unsafe { GetProcessMemoryInfo(h, &mut c, c.cb) };
    if ok == 0 { 0 } else { c.WorkingSetSize as u64 }
}

/// RAM in use by a process, or None if it can't be read.
pub fn memory(pid: u32) -> Option<u64> {
    let h = open_query(pid)?;
    let bytes = working_set(h);
    close(h);
    Some(bytes)
}

/// Full path of a process's exe.
pub fn exe_path(pid: u32) -> Option<PathBuf> {
    let h = open_query(pid)?;
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) };
    close(h);
    (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

/// The "File description" from an exe's version info, e.g. "Discord" or "Steam".
pub fn file_description(path: &std::path::Path) -> Option<String> {
    let file = wide(&path.display().to_string());
    unsafe {
        let size = GetFileVersionInfoSizeW(file.as_ptr(), null_mut());
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(file.as_ptr(), 0, size, data.as_mut_ptr() as *mut c_void) == 0 {
            return None;
        }
        // First language/code page pair, then that block's FileDescription.
        let mut ptr: *mut c_void = null_mut();
        let mut len = 0u32;
        let trans = wide("\\VarFileInfo\\Translation");
        let lang = if VerQueryValueW(data.as_ptr() as _, trans.as_ptr(), &mut ptr, &mut len) != 0 && len >= 4 {
            let pair = std::slice::from_raw_parts(ptr as *const u16, 2);
            format!("{:04x}{:04x}", pair[0], pair[1])
        } else {
            "040904b0".to_string() // US English, Unicode
        };
        let key = wide(&format!("\\StringFileInfo\\{lang}\\FileDescription"));
        if VerQueryValueW(data.as_ptr() as _, key.as_ptr(), &mut ptr, &mut len) == 0 || len == 0 {
            return None;
        }
        let text = std::slice::from_raw_parts(ptr as *const u16, len as usize);
        // Some exes pad the value after its NUL; anything past the first NUL is junk.
        let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        let s = String::from_utf16_lossy(&text[..end]).trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}
