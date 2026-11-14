//! ZIP: list, decode, and the one write primitive — `rewrite`.

use super::{Entry, NewItem, Run, Sink, dos_to_unix_ms, need_pw, norm_name, now_ms, unix_ms_to_dos};
use zip::result::ZipError;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

fn open(zip: &Path) -> io::Result<ZipArchive<fs::File>> {
    ZipArchive::new(fs::File::open(zip)?).map_err(io::Error::other)
}

pub(super) fn entries(zip: &Path) -> io::Result<Vec<Entry>> {
    let mut ar = open(zip)?;
    let mut out = Vec::with_capacity(ar.len());
    for i in 0..ar.len() {
        let f = ar.by_index_raw(i).map_err(io::Error::other)?;
        let Some(name) = f.name().ok().and_then(|n| norm_name(&n)) else { continue };
        out.push(Entry {
            index: i,
            dir: f.is_dir(),
            size: f.size(),
            mtime: f.last_modified().map(dos_to_unix_ms).unwrap_or(0),
            name,
        });
    }
    Ok(out)
}

/// Encrypted entries (ZipCrypto / AES) need `pw`; missing or wrong ->
/// `need_pw`, and the caller asks and calls again with the rest.
pub(super) fn decode(zip: &Path, plan: &[(&Entry, PathBuf)], sink: &mut Sink, pw: Option<&str>) -> io::Result<()> {
    let mut ar = open(zip)?;
    let mut buf = vec![0u8; 256 * 1024];
    for (e, target) in plan {
        if sink.stop {
            break;
        }
        let encrypted = ar.by_index_raw(e.index).map(|f| f.encrypted()).unwrap_or(false);
        let opened = match (encrypted, pw) {
            (true, None) => return Err(need_pw(false)),
            (true, Some(p)) => ar.by_index_decrypt(e.index, p.as_bytes()),
            (false, _) => ar.by_index(e.index),
        };
        let mut f = match opened {
            Ok(f) => f,
            Err(ZipError::InvalidPassword) => return Err(need_pw(true)),
            Err(err) => {
                sink.fail(target, err);
                continue;
            }
        };
        if !sink.open(e, target) {
            continue;
        }
        loop {
            match f.read(&mut buf) {
                Ok(0) => {
                    sink.close();
                    break;
                }
                Ok(n) => {
                    if !sink.write(&buf[..n]) {
                        break;
                    }
                }
                // ZipCrypto accepts ~1 in 256 wrong passwords; the CRC/HMAC
                // check at the end catches them.
                Err(_) if encrypted => {
                    sink.abort(None);
                    return Err(need_pw(true));
                }
                Err(err) => {
                    sink.abort(Some(err.to_string()));
                    break;
                }
            }
        }
    }
    Ok(())
}

/// What a rewrite changes. Entries not dropped are copied raw (no recompression).
#[derive(Default)]
pub(super) struct Plan {
    /// ZIP entry indices to leave out.
    pub drop: HashSet<usize>,
    /// Entry index -> new name (no trailing slash; folders get it back).
    pub rename: HashMap<usize, String>,
    /// Appended at the end, in order.
    pub add: Vec<NewItem>,
    /// Folder that would vanish otherwise: written as an explicit entry.
    pub keep_dir: Option<String>,
    /// Positions in `add` that replace a dropped entry: if such a file can't
    /// be read, the whole rewrite stops (the old version must not vanish).
    pub replacing: HashSet<usize>,
}

/// `<dest>.<pid>.part`: never shared with another twinyard window (within
/// this process the archive lock keeps writers apart).
fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(format!(".{}.part", std::process::id()));
    s.into()
}

/// Writes `src`'s entries (per `plan`) plus the new items into `<dest>.part`
/// and swaps it in. Ok(false) = cancelled; `dest` untouched unless Ok(true).
/// `run.ok` gets positions in `plan.add` of the files written.
pub(super) fn rewrite(src: Option<&Path>, dest: &Path, plan: Plan, run: &mut Run) -> io::Result<bool> {
    let tmp = part_path(dest);
    let res = write_part(src, &tmp, &plan, run);
    if !matches!(res, Ok(true)) {
        let _ = fs::remove_file(&tmp);
        return res;
    }
    if let Err(e) = crate::win::move_path(&tmp, dest, true) {
        let _ = fs::remove_file(&tmp);
        return Err(io::Error::new(e.kind(), crate::trf!("Nie można podmienić archiwum (otwarte w innym programie?): {e}", "Cannot replace the archive (open in another program?): {e}")));
    }
    Ok(true)
}

fn write_part(src: Option<&Path>, tmp: &Path, plan: &Plan, run: &mut Run) -> io::Result<bool> {
    let mut old = src.map(open).transpose()?;
    let mut zw = ZipWriter::new(fs::File::create(tmp)?);
    if let Some(ar) = old.as_mut() {
        if !ar.comment().is_empty() {
            zw.set_raw_comment(ar.comment().to_vec().into_boxed_slice()).map_err(io::Error::other)?;
        }
        for i in 0..ar.len() {
            if plan.drop.contains(&i) {
                continue;
            }
            let f = ar.by_index_raw(i).map_err(io::Error::other)?;
            let packed = f.compressed_size();
            run.p.current = f.name().map(|n| n.into_owned()).unwrap_or_default();
            match plan.rename.get(&i) {
                Some(n) => {
                    let n = if f.is_dir() { format!("{n}/") } else { n.clone() };
                    zw.raw_copy_file_rename(f, n)
                }
                None => zw.raw_copy_file(f),
            }
            .map_err(io::Error::other)?;
            run.p.done_bytes += packed;
            run.host.progress(&run.p);
            if run.host.cancelled() {
                return Ok(false);
            }
        }
    }
    if let Some(d) = &plan.keep_dir {
        zw.add_directory(format!("{d}/"), SimpleFileOptions::default().last_modified_time(unix_ms_to_dos(now_ms())))
            .map_err(io::Error::other)?;
    }
    let mut buf = vec![0u8; 256 * 1024];
    for (pos, it) in plan.add.iter().enumerate() {
        let opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .last_modified_time(unix_ms_to_dos(it.mtime))
            .large_file(it.size >= 0xFFFF_FFFF);
        if it.dir {
            zw.add_directory(format!("{}/", it.name), opts).map_err(io::Error::other)?;
            continue;
        }
        let Some(path) = &it.src else { continue };
        run.p.current = path.display().to_string();
        let mut f = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) if plan.replacing.contains(&pos) => {
                return Err(io::Error::new(e.kind(), format!("{}: {e}", path.display())));
            }
            Err(e) => {
                if !run.fail(&path.display().to_string(), e) {
                    return Ok(false);
                }
                continue;
            }
        };
        zw.start_file(it.name.as_str(), opts).map_err(io::Error::other)?;
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            io::Write::write_all(&mut zw, &buf[..n])?;
            run.p.done_bytes += n as u64;
            run.host.progress(&run.p);
            if run.host.cancelled() {
                return Ok(false);
            }
        }
        run.ok.push(pos);
        run.out.done_files += 1;
        run.p.done_files += 1;
    }
    if run.host.cancelled() {
        return Ok(false); // last chance: nothing has been swapped in yet
    }
    zw.finish().map_err(io::Error::other)?;
    Ok(true)
}
