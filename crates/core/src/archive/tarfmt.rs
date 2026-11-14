//! .tar, .tar.gz / .tgz and a lone .gz — read-only (`tar` + `flate2`).
//!
//! A compressed tar has no index: listing reads (decompresses) it from front
//! to back, so the entry list is cached per file (path + size + mtime) — going
//! into folders of the same archive doesn't decompress it again. Extracting
//! is one more pass that stops after the last wanted file.
//! Only files and folders are shown; links and devices are skipped.

use super::{Entry, Format, Sink, norm_name};
use flate2::read::MultiGzDecoder;
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;
use tar::EntryType;

/// (archive, size, mtime) -> entries; a handful of recent archives.
type Cached = (PathBuf, u64, Option<SystemTime>, Vec<Entry>);
static CACHE: Mutex<Vec<Cached>> = Mutex::new(Vec::new());
const CACHE_LEN: usize = 4;

fn stamp(path: &Path) -> io::Result<(u64, Option<SystemTime>)> {
    let md = fs::metadata(path)?;
    Ok((md.len(), md.modified().ok()))
}

pub(super) fn entries(path: &Path, fmt: Format) -> io::Result<Vec<Entry>> {
    let (len, mtime) = stamp(path)?;
    if let Some(c) = CACHE.lock().unwrap().iter().find(|c| c.0 == path && c.1 == len && c.2 == mtime) {
        return Ok(c.3.clone());
    }
    let list = match fmt {
        Format::Gz => gz_entries(path)?,
        _ => {
            let mut out = Vec::new();
            each(path, fmt, |index, e, _| {
                out.push(Entry { index, name: e.name, dir: e.dir, size: e.size, mtime: e.mtime });
                Ok(Step::Skip)
            })?;
            out
        }
    };
    let mut cache = CACHE.lock().unwrap();
    cache.retain(|c| c.0 != path);
    cache.insert(0, (path.to_path_buf(), len, mtime, list.clone()));
    cache.truncate(CACHE_LEN);
    Ok(list)
}

struct Meta {
    name: String,
    dir: bool,
    size: u64,
    mtime: i64,
}

enum Step {
    /// Not interested in the data.
    Skip,
    /// Data was consumed (or not needed); stop reading the archive.
    Stop,
}

/// Opens the (possibly gzipped) tar stream.
fn reader(path: &Path, fmt: Format) -> io::Result<Box<dyn Read>> {
    let f = BufReader::with_capacity(256 * 1024, File::open(path)?);
    Ok(match fmt {
        Format::TarGz => Box::new(MultiGzDecoder::new(f)),
        _ => Box::new(f),
    })
}

/// Walks the files and folders in archive order; `index` counts every tar
/// entry, so it is the same in listing and extraction. `f` may read the data.
fn each(path: &Path, fmt: Format, mut f: impl FnMut(usize, Meta, &mut dyn Read) -> io::Result<Step>) -> io::Result<()> {
    fn meta<R: Read>(e: &tar::Entry<'_, R>) -> Option<Meta> {
        let kind = e.header().entry_type();
        let dir = kind == EntryType::Directory;
        if !dir && !matches!(kind, EntryType::Regular | EntryType::Continuous) {
            return None; // links, devices, fifos
        }
        Some(Meta {
            name: norm_name(&String::from_utf8_lossy(&e.path_bytes()))?,
            dir,
            size: if dir { 0 } else { e.size() },
            mtime: e.header().mtime().map_or(0, |s| s as i64 * 1000),
        })
    }
    if fmt == Format::Tar {
        // Plain tar: data of skipped entries is seeked over, not read.
        let mut ar = tar::Archive::new(BufReader::new(File::open(path)?));
        for (index, e) in ar.entries_with_seek()?.enumerate() {
            let mut e = e?;
            let Some(m) = meta(&e) else { continue };
            if let Step::Stop = f(index, m, &mut e)? {
                return Ok(());
            }
        }
    } else {
        let mut ar = tar::Archive::new(reader(path, fmt)?);
        for (index, e) in ar.entries()?.enumerate() {
            let mut e = e?;
            let Some(m) = meta(&e) else { continue };
            if let Step::Stop = f(index, m, &mut e)? {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Copies one entry into the sink (write errors / cancel are recorded there).
fn pump(sink: &mut Sink, rd: &mut dyn Read, buf: &mut [u8]) -> io::Result<()> {
    loop {
        match rd.read(buf) {
            Ok(0) => {
                sink.close();
                return Ok(());
            }
            Ok(n) => {
                if !sink.write(&buf[..n]) {
                    return Ok(()); // write error (recorded) or cancel
                }
            }
            Err(e) => {
                sink.abort(Some(e.to_string()));
                return Err(e); // a broken stream can't go on
            }
        }
    }
}

pub(super) fn decode(path: &Path, fmt: Format, plan: &[(&Entry, PathBuf)], sink: &mut Sink) -> io::Result<()> {
    let mut buf = vec![0u8; 256 * 1024];
    if fmt == Format::Gz {
        let Some((e, target)) = plan.first() else { return Ok(()) };
        if sink.open(e, target) {
            let mut rd = MultiGzDecoder::new(BufReader::with_capacity(256 * 1024, File::open(path)?));
            pump(sink, &mut rd, &mut buf)?;
        }
        return Ok(());
    }
    let want: std::collections::HashMap<usize, (&Entry, &PathBuf)> = plan.iter().map(|(e, t)| (e.index, (*e, t))).collect();
    let mut left = want.len();
    if left == 0 {
        return Ok(());
    }
    each(path, fmt, |index, _, rd| {
        let Some((e, target)) = want.get(&index).copied() else { return Ok(Step::Skip) };
        if e.dir {
            return Ok(Step::Skip);
        }
        left -= 1;
        if sink.open(e, target) {
            pump(sink, rd, &mut buf)?;
        }
        Ok(if sink.stop || left == 0 { Step::Stop } else { Step::Skip })
    })
}

/// A lone `x.gz` holds one file, `x`. Its size is in the last 4 bytes
/// (modulo 4 GiB — good enough for the panel; extraction reads to the end).
fn gz_entries(path: &Path) -> io::Result<Vec<Entry>> {
    let mut f = File::open(path)?;
    let md = f.metadata()?;
    let mut size = 0u64;
    if md.len() >= 18 {
        let mut b = [0u8; 4];
        f.seek(SeekFrom::End(-4))?;
        f.read_exact(&mut b)?;
        size = u32::from_le_bytes(b) as u64;
    }
    f.seek(SeekFrom::Start(0))?;
    let gz = flate2::read::GzDecoder::new(BufReader::new(f));
    let header_mtime = gz.header().map_or(0, |h| h.mtime() as i64 * 1000);
    let file_mtime = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64);
    let fname = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let name = fname.get(..fname.len().saturating_sub(3)).filter(|s| !s.is_empty()).unwrap_or("plik").to_string();
    Ok(vec![Entry {
        index: 0,
        name: norm_name(&name).unwrap_or_else(|| "plik".into()),
        dir: false,
        size,
        mtime: if header_mtime > 0 { header_mtime } else { file_mtime },
    }])
}
