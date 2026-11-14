//! 7z, read-only (`sevenz-rust2`, pure Rust, Apache-2.0), AES-256 passwords.
//! Solid archives decode front to back: entries before a wanted one are
//! decompressed and dropped by the library.

use super::{Entry, Sink, need_pw, norm_name};
use sevenz_rust2::{Archive, ArchiveReader, EntryDecision, Error, Password};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

fn err(e: Error, pw: Option<&str>) -> io::Error {
    match e {
        Error::PasswordRequired => need_pw(pw.is_some()),
        Error::MaybeBadPassword(_) => need_pw(true),
        Error::Io(e, _) | Error::FileOpen(e, _) => e,
        e => io::Error::other(format!("7z: {e}")),
    }
}

fn password(pw: Option<&str>) -> Password {
    pw.map_or_else(Password::empty, Password::from)
}

pub(super) fn entries(path: &Path, pw: Option<&str>) -> io::Result<Vec<Entry>> {
    // Header encryption: even the names need the password.
    let ar = Archive::open_with_password(path, &password(pw)).map_err(|e| err(e, pw))?;
    Ok(ar
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.is_anti_item)
        .filter_map(|(i, f)| {
            Some(Entry {
                index: i,
                name: norm_name(&f.name)?,
                dir: f.is_directory,
                size: f.size,
                mtime: if f.has_last_modified_date { crate::listing::filetime_to_unix_ms(u64::from(f.last_modified_date)) } else { 0 },
            })
        })
        .collect())
}

pub(super) fn decode(path: &Path, plan: &[(&Entry, PathBuf)], sink: &mut Sink, pw: Option<&str>) -> io::Result<()> {
    let mut reader = ArchiveReader::open(path, password(pw)).map_err(|e| err(e, pw))?;
    let want: HashMap<&str, (&Entry, &PathBuf)> = plan.iter().map(|(e, t)| (e.name.as_str(), (*e, t))).collect();
    let mut left = want.len();
    let mut buf = vec![0u8; 256 * 1024];
    let mut bad_password = false;
    let res = reader.for_each_entries_with_decision(|ae, rd| {
        if sink.stop || left == 0 {
            return Ok(EntryDecision::Stop);
        }
        let Some((e, target)) = norm_name(&ae.name).and_then(|n| want.get(n.as_str()).copied()) else {
            return Ok(EntryDecision::Continue);
        };
        if ae.is_directory {
            return Ok(EntryDecision::Continue);
        }
        left -= 1;
        if !sink.open(e, target) {
            return Ok(if sink.stop { EntryDecision::Stop } else { EntryDecision::Continue });
        }
        loop {
            match rd.read(&mut buf) {
                Ok(0) => {
                    sink.close();
                    break;
                }
                Ok(n) => {
                    if !sink.write(&buf[..n]) {
                        break; // the library drains the rest of this entry
                    }
                }
                Err(e) => {
                    // With a password given, garbage out of the decoder = wrong password.
                    if pw.is_some() {
                        sink.abort(None);
                        bad_password = true;
                        return Ok(EntryDecision::Stop);
                    }
                    sink.abort(Some(format!("7z: {e}")));
                    // A broken stream can't be resumed: rest of this block is lost.
                    return Ok(if sink.stop { EntryDecision::Stop } else { EntryDecision::SkipBlock });
                }
            }
        }
        Ok(if sink.stop { EntryDecision::Stop } else { EntryDecision::Continue })
    });
    if bad_password {
        return Err(need_pw(true));
    }
    if let Err(e) = res {
        // Verification after the last read (CRC) fails the whole iteration.
        let checksum = matches!(e, Error::ChecksumVerificationFailed);
        let e = err(e, pw);
        if super::need_pw_of(&e).is_some() || (checksum && pw.is_some()) {
            sink.abort(None);
            sink.undo_last();
            return Err(if super::need_pw_of(&e).is_some() { e } else { need_pw(true) });
        }
        if sink.cur.is_some() {
            sink.abort(Some(e.to_string()));
        } else {
            return Err(e);
        }
    }
    Ok(())
}
