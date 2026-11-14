//! Start-up chores: cleaning old copies in `%TEMP%\twinyard` (files fetched
//! from FTP for F3/F4/Enter, extracted from archives, relay/xfer leftovers)
//! and keeping a single instance per data folder.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// Copies older than this go. Age = when the copy was made (creation time):
/// downloads carry the server's modification time, which may be years old.
const KEEP: Duration = Duration::from_secs(3 * 24 * 3600);
const KEEP_EDIT: Duration = Duration::from_secs(30 * 24 * 3600);

/// Runs on its own thread: a big temp folder must not delay the window.
pub fn clean_temp_later() {
    std::thread::Builder::new()
        .name("temp-cleanup".into())
        .spawn(|| {
            let root = std::env::temp_dir().join("twinyard");
            let now = SystemTime::now();
            // F4 copies may hold edits never sent (upload failed): kept a month.
            let (mut files, mut bytes) = (0, 0);
            for sub in ["~edit", "~edit-zip"] {
                let (f, b) = clean(&root.join(sub), now - (KEEP_EDIT - KEEP));
                files += f;
                bytes += b;
            }
            let (f, b) = clean(&root, now);
            files += f;
            bytes += b;
            if files > 0 {
                log::info!("temp: usunięto {files} starych kopii ({} MB)", bytes / (1024 * 1024));
            }
        })
        .ok();
}

/// Deletes old files below `dir` and the folders that became empty.
/// Returns (files, bytes) removed. Never follows links.
fn clean(dir: &Path, now: SystemTime) -> (u64, u64) {
    let (mut files, mut bytes) = (0, 0);
    let Ok(rd) = std::fs::read_dir(dir) else { return (0, 0) };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with("~edit") {
            continue; // F4 copies: own, longer limit (see clean_temp_later)
        }
        let Ok(md) = e.path().symlink_metadata() else { continue };
        let p = e.path();
        if md.is_dir() {
            let (f, b) = clean(&p, now);
            files += f;
            bytes += b;
            let _ = std::fs::remove_dir(&p); // only succeeds when empty
        } else if md.is_file() {
            let made = md.created().ok().into_iter().chain(md.modified().ok()).max();
            if made.is_some_and(|t| now.duration_since(t).unwrap_or_default() > KEEP) && std::fs::remove_file(&p).is_ok() {
                files += 1;
                bytes += md.len();
            }
        }
    }
    (files, bytes)
}

/// One running twinyard per data folder (two would overwrite each other's
/// tabs and settings). Returns false when another one runs — it is brought
/// to the front then and this process should quit. A dev build and the
/// portable copy have different data folders, so they can run side by side.
pub fn single_instance() -> bool {
    use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    let dir = twinyard_core::config::data_dir().to_string_lossy().to_lowercase();
    // Mutex names can't hold '\'; a hash keeps the name short.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in dir.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let name: Vec<u16> = format!("Local\\twinyard-{h:016x}").encode_utf16().chain([0]).collect();
    // SAFETY: valid NUL-terminated name; the handle is kept for the process lifetime.
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() || unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return true;
    }
    let pid_file = twinyard_core::config::data_dir().join("instance.pid");
    let pid = std::fs::read_to_string(&pid_file).ok().and_then(|s| s.trim().parse::<u32>().ok());
    // A "running" instance without a visible window (hung while closing,
    // still starting up) must not lock the user out: start anyway.
    !pid.is_some_and(focus_window_of)
}

/// The first instance records its process id, for `single_instance` of a second one.
pub fn write_pid() {
    let _ = std::fs::write(twinyard_core::config::data_dir().join("instance.pid"), std::process::id().to_string());
}

/// Brings up a visible top-level window of `pid`; false = it has none.
fn focus_window_of(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> i32 {
        let mut owner = 0u32;
        // SAFETY: plain window queries on a handle EnumWindows gave us.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut owner);
            if owner == lp as u32 && IsWindowVisible(hwnd) != 0 {
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                SetForegroundWindow(hwnd);
                return 0; // stop
            }
        }
        1
    }
    // EnumWindows returns 0 when the callback stopped it, i.e. a window was found.
    // SAFETY: callback only reads its arguments.
    unsafe { EnumWindows(Some(each), pid as LPARAM) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_old_files_and_empty_dirs() {
        let base = std::env::temp_dir().join(format!("twinyard-hk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("a").join("b")).unwrap();
        std::fs::write(base.join("a").join("b").join("x.txt"), b"x").unwrap();
        std::fs::create_dir_all(base.join("keep")).unwrap();
        std::fs::write(base.join("keep").join("y.txt"), b"y").unwrap();
        // "Now" a week ahead: everything is old.
        let later = SystemTime::now() + Duration::from_secs(7 * 24 * 3600);
        assert_eq!(clean(&base, SystemTime::now()), (0, 0), "fresh files stay");
        assert!(base.join("keep").join("y.txt").exists());
        assert_eq!(clean(&base, later).0, 2);
        assert!(!base.join("a").exists(), "emptied folders go");
        let _ = std::fs::remove_dir_all(&base);
    }
}
