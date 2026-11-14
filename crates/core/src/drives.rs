use serde::Serialize;
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    /// e.g. `C:\`
    pub root: String,
    pub letter: char,
    /// fixed | removable | network | cdrom | ramdisk | unknown
    pub kind: &'static str,
}

/// (free for the user, total) bytes of the volume holding `path` (also UNC).
/// May block on a dead network share: call off the UI thread.
pub fn disk_space(path: &str) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut p = path.to_string();
    if !p.ends_with('\\') {
        p.push('\\');
    }
    let wide: Vec<u16> = p.encode_utf16().chain(Some(0)).collect();
    let (mut free, mut total, mut all_free) = (0u64, 0u64, 0u64);
    // SAFETY: valid NUL-terminated path, out pointers to locals.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut all_free) };
    (ok != 0).then_some((free, total))
}

/// Lists drive letters without touching the drives (no spin-up, no network wait).
pub fn list_drives() -> Vec<Drive> {
    // SAFETY: no arguments, returns a bitmask.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| {
            let letter = (b'A' + i) as char;
            let root = format!("{letter}:\\");
            let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
            // SAFETY: `wide` is a valid NUL-terminated UTF-16 string.
            let kind = match unsafe { GetDriveTypeW(wide.as_ptr()) } {
                2 => "removable",
                3 => "fixed",
                4 => "network",
                5 => "cdrom",
                6 => "ramdisk",
                _ => "unknown",
            };
            Drive { root, letter, kind }
        })
        .collect()
}
