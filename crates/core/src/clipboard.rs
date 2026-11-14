//! Windows clipboard for files (CF_HDROP + "Preferred DropEffect", the same
//! format Explorer uses: Ctrl+C here -> Ctrl+V in Explorer and back) and
//! plain text (copying paths / names).

use std::io;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{GlobalFree, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::UI::Shell::DragQueryFileW;

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;

/// shlobj.h DROPFILES (20 bytes), followed by the NUL-separated wide paths.
#[repr(C)]
struct DropFiles {
    p_files: u32,
    x: i32,
    y: i32,
    f_nc: i32,
    f_wide: i32,
}

fn drop_effect_format() -> u32 {
    let name: Vec<u16> = "Preferred DropEffect".encode_utf16().chain([0]).collect();
    // SAFETY: valid NUL-terminated name.
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

/// Opened clipboard; closed on drop. Retries a little: another program
/// (clipboard managers, RDP) may hold it for a moment.
struct Open;

impl Open {
    fn new(owner: HWND) -> io::Result<Open> {
        for _ in 0..10 {
            // SAFETY: plain call; the handle may be any window of ours.
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Open);
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        Err(io::Error::last_os_error())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: we opened it.
        unsafe { CloseClipboard() };
    }
}

/// Copies `bytes` into a movable global block (ownership passes to the
/// clipboard on success).
fn put(format: u32, bytes: &[u8]) -> io::Result<()> {
    // SAFETY: allocation + copy into the locked block of the right size;
    // freed by us only if SetClipboardData refuses it.
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if h.is_null() {
            return Err(io::Error::last_os_error());
        }
        let p = GlobalLock(h) as *mut u8;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(h);
        if SetClipboardData(format, h).is_null() {
            let e = io::Error::last_os_error();
            GlobalFree(h);
            return Err(e);
        }
    }
    Ok(())
}

fn wide_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// Files to the clipboard; `cut` = Explorer's Ctrl+X (paste moves them).
/// `owner` must be a window of this process (else Windows drops the data).
pub fn set_files(owner: isize, paths: &[String], cut: bool) -> io::Result<()> {
    let mut data = Vec::new();
    let head = DropFiles { p_files: std::mem::size_of::<DropFiles>() as u32, x: 0, y: 0, f_nc: 0, f_wide: 1 };
    // SAFETY: plain-old-data struct viewed as bytes.
    data.extend_from_slice(unsafe {
        std::slice::from_raw_parts(&head as *const DropFiles as *const u8, std::mem::size_of::<DropFiles>())
    });
    for p in paths {
        data.extend(wide_bytes(p));
    }
    data.extend([0, 0]); // list end
    let _open = Open::new(owner as HWND)?;
    // SAFETY: clipboard is open.
    unsafe { EmptyClipboard() };
    put(CF_HDROP, &data)?;
    let effect = if cut { DROPEFFECT_MOVE } else { DROPEFFECT_COPY };
    put(drop_effect_format(), &effect.to_le_bytes())?;
    // Paths as text too: pasting into an editor gives the paths.
    put(CF_UNICODETEXT, &wide_bytes(&paths.join("\r\n")))
}

pub fn set_text(owner: isize, text: &str) -> io::Result<()> {
    let _open = Open::new(owner as HWND)?;
    // SAFETY: clipboard is open.
    unsafe { EmptyClipboard() };
    put(CF_UNICODETEXT, &wide_bytes(text))
}

/// Files on the clipboard (copied in Explorer or here) and whether they
/// were cut. None = no files there.
pub fn get_files() -> io::Result<Option<(Vec<PathBuf>, bool)>> {
    // SAFETY: format query, no clipboard ownership needed.
    if unsafe { IsClipboardFormatAvailable(CF_HDROP) } == 0 {
        return Ok(None);
    }
    let _open = Open::new(std::ptr::null_mut())?;
    // SAFETY: clipboard open; the HDROP stays owned by the clipboard and we
    // only read it while it is open.
    unsafe {
        let h = GetClipboardData(CF_HDROP);
        if h.is_null() {
            return Ok(None);
        }
        let count = DragQueryFileW(h, u32::MAX, std::ptr::null_mut(), 0);
        let mut paths = Vec::with_capacity(count as usize);
        for i in 0..count {
            let len = DragQueryFileW(h, i, std::ptr::null_mut(), 0) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(h, i, buf.as_mut_ptr(), buf.len() as u32);
            paths.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
        }
        let mut cut = false;
        let fmt = drop_effect_format();
        if IsClipboardFormatAvailable(fmt) != 0 {
            let he = GetClipboardData(fmt);
            if !he.is_null() && GlobalSize(he) >= 4 {
                let p = GlobalLock(he) as *const u32;
                if !p.is_null() {
                    cut = p.read_unaligned() & DROPEFFECT_MOVE != 0;
                    GlobalUnlock(he);
                }
            }
        }
        Ok(Some((paths, cut)))
    }
}

/// After a pasted cut the clipboard must not offer the (moved) files again.
pub fn clear() {
    if let Ok(_open) = Open::new(std::ptr::null_mut()) {
        // SAFETY: clipboard open.
        unsafe { EmptyClipboard() };
    }
}
