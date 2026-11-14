//! `\\server` (no share): Windows has no directory there — list its shares
//! (NetShareEnum), like Explorer and TC do.

use crate::listing::{ATTR_DIR, Listing};
use std::io;
use std::path::Path;
use std::time::Instant;
use windows_sys::Win32::NetworkManagement::NetManagement::{MAX_PREFERRED_LENGTH, NetApiBufferFree};
use windows_sys::Win32::Storage::FileSystem::{NetShareEnum, SHARE_INFO_1, STYPE_DISKTREE, STYPE_SPECIAL};

const ERROR_MORE_DATA: u32 = 234;

/// `\\server` or `\\server\` -> Some("server").
pub fn server_only(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(r"\\")?.trim_end_matches('\\');
    (!rest.is_empty() && !rest.contains('\\') && !rest.starts_with('?') && !rest.starts_with('.')).then_some(rest)
}

/// `\\server\share` (root of a share) -> `\\server`, for "..".
pub fn share_parent(path: &Path) -> Option<String> {
    let s = path.to_string_lossy();
    let rest = s.strip_prefix(r"\\")?.trim_end_matches('\\');
    let (server, share) = rest.split_once('\\')?;
    (!share.contains('\\') && !server.is_empty() && !server.starts_with('?')).then(|| format!(r"\\{server}"))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Disk shares of a server (hidden `$` and admin shares skipped).
pub fn list_shares(server: &str) -> io::Result<Listing> {
    let start = Instant::now();
    let name = wide(&format!(r"\\{server}"));
    let mut names: Vec<String> = Vec::new();
    let mut resume: u32 = 0;
    loop {
        let mut buf: *mut u8 = std::ptr::null_mut();
        let (mut read, mut total) = (0u32, 0u32);
        // SAFETY: out-pointers are valid; the buffer is freed below.
        let rc = unsafe { NetShareEnum(name.as_ptr(), 1, &mut buf, MAX_PREFERRED_LENGTH, &mut read, &mut total, &mut resume) };
        if rc != 0 && rc != ERROR_MORE_DATA {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        if !buf.is_null() {
            // SAFETY: NetShareEnum returned `read` SHARE_INFO_1 records.
            let items = unsafe { std::slice::from_raw_parts(buf as *const SHARE_INFO_1, read as usize) };
            for it in items {
                if it.shi1_type & !STYPE_SPECIAL != STYPE_DISKTREE || it.shi1_type & STYPE_SPECIAL != 0 {
                    continue; // printers, IPC$, ADMIN$, C$…
                }
                // SAFETY: NUL-terminated wide string from the API.
                let n = unsafe {
                    let p = it.shi1_netname;
                    let len = (0..).take_while(|&i| *p.add(i) != 0).count();
                    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
                };
                if !n.ends_with('$') {
                    names.push(n);
                }
            }
            unsafe { NetApiBufferFree(buf as *const _) };
        }
        if rc != ERROR_MORE_DATA {
            break;
        }
    }
    let mut l = Listing { path: format!(r"\\{server}"), ..Default::default() };
    for n in names {
        l.names.push(n);
        l.sizes.push(0);
        l.mtimes.push(0);
        l.attrs.push(ATTR_DIR);
    }
    l.finish(start);
    Ok(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(server_only(r"\\10.83.1.172"), Some("10.83.1.172"));
        assert_eq!(server_only(r"\\nas\"), Some("nas"));
        assert_eq!(server_only(r"\\nas\share"), None);
        assert_eq!(server_only(r"\\?\C:\x"), None);
        assert_eq!(server_only(r"C:\x"), None);
        assert_eq!(share_parent(Path::new(r"\\nas\share")).as_deref(), Some(r"\\nas"));
        assert_eq!(share_parent(Path::new(r"\\nas\share\")).as_deref(), Some(r"\\nas"));
        assert_eq!(share_parent(Path::new(r"\\nas\share\dir")), None);
    }

    /// `set TWINYARD_UNC_TEST=server` — prints only the number of shares.
    #[test]
    fn real_server() {
        let Ok(server) = std::env::var("TWINYARD_UNC_TEST") else { return };
        let l = list_shares(&server).unwrap();
        println!("shares: {}", l.names.len());
        assert!(!l.names.is_empty());
    }

    /// The local machine always has shares to enumerate (or none — still Ok).
    #[test]
    fn enumerates_localhost() {
        assert!(list_shares("localhost").is_ok());
    }
}
