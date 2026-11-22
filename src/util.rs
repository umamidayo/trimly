//! Small helpers shared across modules.

/// Converts a Rust string to a NUL-terminated UTF-16 buffer for Win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Win32 `LOWORD` style helper.
pub fn loword(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}

/// "512 MB" or "1.25 GB".
pub fn fmt_size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 { format!("{:.2} GB", mb / 1024.0) } else { format!("{mb:.0} MB") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes_read_naturally() {
        assert_eq!(super::fmt_size(0), "0 MB");
        assert_eq!(super::fmt_size(512 * 1024 * 1024), "512 MB");
        assert_eq!(super::fmt_size(1280 * 1024 * 1024), "1.25 GB");
    }
}
