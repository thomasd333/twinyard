//! RAR, read-only, through the UnRAR library (C++ source compiled by the
//! `unrar_sys` crate). Thin own wrapper instead of the `unrar` crate: data
//! is streamed through the UCM_PROCESSDATA callback, so big files show
//! progress and can be cancelled mid-file; passwords go through
//! UCM_NEEDPASSWORDW.
//!
//! UnRAR licence (required notice):
//! "UnRAR source code may be used in any software to handle RAR archives
//! without limitations free of charge, but cannot be used to develop RAR
//! (WinRAR) compatible archiver and to re-create RAR compression algorithm,
//! which is proprietary. Distribution of modified UnRAR source code in
//! separate form or as a part of other software is permitted, provided that
//! full text of this paragraph, starting from "UnRAR source code" words, is
//! included in license, or in documentation if license is not available, and
//! in source code comments of resulting package."
//! twinyard only reads RAR archives — never creates or modifies them.

use super::{Entry, Sink, dos_to_unix_ms, need_pw, norm_name};
use std::cell::Cell;
use std::collections::HashMap;
use std::io;
use std::os::raw::c_int;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use unrar_sys as native;

// UnRAR uses OpenProcessToken / CryptGenRandom; unrar_sys doesn't link them.
#[link(name = "advapi32")]
unsafe extern "system" {}

/// Receives decoded chunks; false = stop this file.
type Feed<'a> = &'a mut dyn FnMut(&[u8]) -> bool;

/// Callback state, boxed so its address is stable for the handle's lifetime.
struct Ctx {
    /// UTF-16, no terminator.
    pw: Option<Vec<u16>>,
    /// UnRAR asked for a password (so a failure may mean a wrong one).
    asked: Cell<bool>,
    /// `*mut Feed` while RARProcessFileW runs, else 0.
    feed: Cell<usize>,
}

/// Field order matters: the handle is closed before the context goes away.
struct Archive {
    h: *const native::Handle,
    ctx: Box<Ctx>,
}

impl Drop for Archive {
    fn drop(&mut self) {
        // SAFETY: handle from RAROpenArchiveEx, closed once.
        unsafe { native::RARCloseArchive(self.h) };
    }
}

extern "C" fn callback(msg: native::UINT, user: native::LPARAM, p1: native::LPARAM, p2: native::LPARAM) -> c_int {
    // SAFETY: `user` is the boxed Ctx of a live Archive (or 0).
    let ctx = (user != 0).then(|| unsafe { &*(user as *const Ctx) });
    match msg {
        // Next volume missing: give up instead of asking.
        native::UCM_CHANGEVOLUME | native::UCM_CHANGEVOLUMEW => {
            if p2 == native::RAR_VOL_ASK {
                -1
            } else {
                0
            }
        }
        native::UCM_NEEDPASSWORDW => {
            let Some(ctx) = ctx else { return -1 };
            ctx.asked.set(true);
            let Some(pw) = &ctx.pw else { return -1 };
            // SAFETY: UnRAR passes a wchar buffer of p2 elements.
            let buf = unsafe { std::slice::from_raw_parts_mut(p1 as *mut u16, p2 as usize) };
            if pw.len() >= buf.len() {
                return -1;
            }
            buf[..pw.len()].copy_from_slice(pw);
            buf[pw.len()] = 0;
            1
        }
        native::UCM_NEEDPASSWORD => -1, // the wide variant above is asked first
        native::UCM_PROCESSDATA => {
            let Some(f) = ctx.map(|c| c.feed.get()).filter(|&f| f != 0) else { return 0 };
            // SAFETY: set right before RARProcessFileW, cleared right after.
            let feed = unsafe { &mut *(f as *mut Feed) };
            let data = unsafe { std::slice::from_raw_parts(p1 as *const u8, p2 as usize) };
            if feed(data) { 1 } else { -1 }
        }
        _ => 0,
    }
}

/// `encrypted`: the failing entry (or headers) is password-protected.
fn rar_err(code: c_int, ctx: &Ctx, encrypted: bool) -> io::Error {
    let m = match code {
        native::ERAR_MISSING_PASSWORD => return need_pw(false),
        native::ERAR_BAD_PASSWORD => return need_pw(true),
        // RAR 4 has no password check: a wrong one shows as bad data.
        native::ERAR_BAD_DATA | native::ERAR_UNKNOWN if ctx.pw.is_some() && (encrypted || ctx.asked.get()) => return need_pw(true),
        native::ERAR_BAD_DATA => crate::i18n::tr("Uszkodzone dane (błąd sumy kontrolnej)", "Corrupted data (checksum error)"),
        native::ERAR_BAD_ARCHIVE => crate::i18n::tr("To nie jest poprawne archiwum RAR", "Not a valid RAR archive"),
        native::ERAR_UNKNOWN_FORMAT => crate::i18n::tr("Nieznany format archiwum", "Unknown archive format"),
        native::ERAR_EOPEN => crate::i18n::tr("Nie można otworzyć archiwum (albo brak kolejnej części)", "Cannot open the archive (or the next volume is missing)"),
        native::ERAR_ECREATE => crate::i18n::tr("Nie można utworzyć pliku", "Cannot create the file"),
        native::ERAR_EREAD => crate::i18n::tr("Błąd odczytu", "Read error"),
        native::ERAR_EWRITE => crate::i18n::tr("Błąd zapisu", "Write error"),
        native::ERAR_NO_MEMORY => crate::i18n::tr("Brak pamięci", "Out of memory"),
        _ => return io::Error::other(crate::trf!("RAR: błąd {code}", "RAR: error {code}")),
    };
    io::Error::other(m)
}

fn open(path: &Path, mode: u32, pw: Option<&str>) -> io::Result<Archive> {
    let ctx = Box::new(Ctx { pw: pw.map(|p| p.encode_utf16().collect()), asked: Cell::new(false), feed: Cell::new(0) });
    let user = &*ctx as *const Ctx as native::LPARAM;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut data = native::OpenArchiveDataEx::new(wide.as_ptr(), mode);
    data.callback = Some(callback);
    data.user_data = user;
    // SAFETY: `data`, `wide` and `ctx` outlive the call.
    let h = unsafe { native::RAROpenArchiveEx(&mut data as *mut _) };
    if h.is_null() || data.open_result != 0 {
        if !h.is_null() {
            unsafe { native::RARCloseArchive(h) };
        }
        return Err(rar_err(data.open_result as c_int, &ctx, true));
    }
    // SAFETY: valid handle; ctx lives in the returned Archive.
    unsafe { native::RARSetCallback(h, Some(callback), user) };
    Ok(Archive { h, ctx })
}

impl Archive {
    /// Next header into `hd`; false at the end.
    fn next(&self, hd: &mut native::HeaderDataEx) -> io::Result<bool> {
        // SAFETY: valid handle; header struct owned by the caller.
        match unsafe { native::RARReadHeaderEx(self.h, hd as *mut _) } {
            native::ERAR_SUCCESS => Ok(true),
            native::ERAR_END_ARCHIVE => Ok(false),
            c => Err(rar_err(c, &self.ctx, true)), // encrypted headers
        }
    }

    fn process(&self, op: c_int) -> c_int {
        // SAFETY: valid handle, right after a successful header read.
        unsafe { native::RARProcessFileW(self.h, op, std::ptr::null(), std::ptr::null()) }
    }

    fn skip(&self, hd: &native::HeaderDataEx) -> io::Result<()> {
        match self.process(native::RAR_SKIP) {
            native::ERAR_SUCCESS => Ok(()),
            c => Err(rar_err(c, &self.ctx, encrypted(hd))),
        }
    }
}

fn encrypted(hd: &native::HeaderDataEx) -> bool {
    hd.flags & native::RHDF_ENCRYPTED != 0
}

fn to_entry(i: usize, hd: &native::HeaderDataEx) -> Option<Entry> {
    let len = hd.filename_w.iter().position(|&c| c == 0).unwrap_or(hd.filename_w.len());
    let name = norm_name(&String::from_utf16_lossy(&hd.filename_w[..len]))?;
    let ft = ((hd.mtime_high as u64) << 32) | hd.mtime_low as u64;
    let mtime = if ft != 0 {
        crate::listing::filetime_to_unix_ms(ft)
    } else {
        zip::DateTime::try_from_msdos((hd.file_time >> 16) as u16, hd.file_time as u16).map(dos_to_unix_ms).unwrap_or(0)
    };
    Some(Entry {
        index: i,
        name,
        dir: hd.flags & native::RHDF_DIRECTORY != 0,
        size: ((hd.unp_size_high as u64) << 32) | hd.unp_size as u64,
        mtime,
    })
}

pub(super) fn entries(path: &Path, pw: Option<&str>) -> io::Result<Vec<Entry>> {
    let ar = open(path, native::RAR_OM_LIST, pw)?;
    let mut hd = Box::<native::HeaderDataEx>::default();
    let mut out = Vec::new();
    let mut i = 0;
    while ar.next(&mut hd)? {
        if let Some(e) = to_entry(i, &hd) {
            out.push(e);
        }
        i += 1;
        ar.skip(&hd)?;
    }
    Ok(out)
}

pub(super) fn decode(path: &Path, plan: &[(&Entry, PathBuf)], sink: &mut Sink, pw: Option<&str>) -> io::Result<()> {
    let ar = open(path, native::RAR_OM_EXTRACT, pw)?;
    let want: HashMap<&str, (&Entry, &PathBuf)> = plan.iter().map(|(e, t)| (e.name.as_str(), (*e, t))).collect();
    let mut left = want.len();
    let mut hd = Box::<native::HeaderDataEx>::default();
    while left > 0 && !sink.stop && ar.next(&mut hd)? {
        let hit = to_entry(0, &hd).filter(|e| !e.dir).and_then(|e| want.get(e.name.as_str()).copied());
        let Some((e, target)) = hit else {
            ar.skip(&hd)?;
            continue;
        };
        if encrypted(&hd) && pw.is_none() {
            return Err(need_pw(false)); // ask before creating the target
        }
        left -= 1;
        if !sink.open(e, target) {
            ar.skip(&hd)?;
            continue;
        }
        let code = {
            let mut f = |d: &[u8]| sink.write(d);
            let mut feed: Feed = &mut f;
            ar.ctx.feed.set(&mut feed as *mut Feed as usize);
            let c = ar.process(native::RAR_TEST);
            ar.ctx.feed.set(0);
            c
        };
        if code == native::ERAR_SUCCESS {
            sink.close();
            continue;
        }
        if sink.cur.is_none() {
            continue; // our write error / cancel stopped it: already handled by the sink
        }
        let err = rar_err(code, &ar.ctx, encrypted(&hd));
        if super::need_pw_of(&err).is_some() {
            sink.abort(None);
            return Err(err);
        }
        sink.abort(Some(err.to_string()));
    }
    Ok(())
}
