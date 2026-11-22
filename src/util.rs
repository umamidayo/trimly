//! Small helpers shared across modules.

/// Converts a Rust string to a NUL-terminated UTF-16 buffer for Win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Win32 `LOWORD` style helper.
pub fn loword(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}
