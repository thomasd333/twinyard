//! Archives as directories: `C:\x\pack.zip\inner\dir` lists the inside,
//! copying out of it extracts.
//!
//! - ZIP: read and write. Adding (F5/F6/drag into the archive), deleting (F8),
//!   renaming, F7 and moving out rewrite the whole archive into `<name>.part`
//!   — entries that stay are copied raw, without recompression — and swap it
//!   in only when everything went well: an error or cancel leaves the
//!   original untouched.
//! - 7z (`sevenz-rust2`), RAR (UnRAR library, licence in rar.rs), .tar / .tar.gz /
//!   .tgz / .gz (`tar` + `flate2`): read-only.

mod rar;
mod sevenz;
mod tarfmt;
mod zipfmt;

use crate::listing::{ATTR_DIR, Listing};
use crate::ops::{Conflict, ConflictAction, ConflictAnswer, ErrorAction, Host, OpError, OpKind, OpSpec, Outcome, Phase, Progress, unique_name};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Condvar, Mutex};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::System::Time::{
    FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime, TzSpecificLocalTimeToSystemTime,
};
use zip::DateTime;

fn read_only() -> &'static str {
    crate::i18n::tr(
        "Archiwa 7z, RAR, tar i gz są tylko do odczytu — można z nich kopiować (F5)",
        "7z, RAR, tar and gz archives are read-only — you can copy out of them (F5)",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    SevenZ,
    Rar,
    Tar,
    /// .tar.gz / .tgz
    TarGz,
    /// A single gzipped file.
    Gz,
}

pub fn format_of(name: &str) -> Option<Format> {
    let l = name.to_ascii_lowercase();
    if l.ends_with(".zip") {
        Some(Format::Zip)
    } else if l.ends_with(".7z") {
        Some(Format::SevenZ)
    } else if l.ends_with(".rar") {
        Some(Format::Rar)
    } else if l.ends_with(".tar") {
        Some(Format::Tar)
    } else if l.ends_with(".tar.gz") || l.ends_with(".tgz") {
        Some(Format::TarGz)
    } else if l.ends_with(".gz") {
        Some(Format::Gz)
    } else {
        None
    }
}

pub fn is_archive_name(name: &str) -> bool {
    format_of(name).is_some()
}

/// `C:\a\b.zip\x\y` -> (`C:\a\b.zip`, "x/y"); `C:\a\b.zip` -> (…, "").
/// None for ordinary paths (also for a directory named `x.zip`).
pub fn split(path: &str) -> Option<(PathBuf, String)> {
    if Path::new(path).is_dir() {
        return None;
    }
    let p = path.replace('/', "\\");
    let mut end = 0;
    for seg in p.split('\\') {
        end += seg.len();
        if is_archive_name(seg) && Path::new(&p[..end]).is_file() {
            let inner = p[end..].trim_matches('\\').replace('\\', "/");
            return Some((PathBuf::from(&p[..end]), inner));
        }
        end += 1;
    }
    None
}

/// Something *inside* an archive: `a.zip\x`, or `a.zip\` (its root as a
/// destination). The bare `a.zip` is the archive file itself.
pub fn is_inside(path: &str) -> bool {
    split(path).is_some_and(|(_, inner)| !inner.is_empty() || path.ends_with(['\\', '/']))
}

// ---------- DOS (local) time <-> unix ms ----------

fn dos_to_unix_ms(dt: DateTime) -> i64 {
    let local = SYSTEMTIME {
        wYear: dt.year(),
        wMonth: dt.month() as u16,
        wDayOfWeek: 0,
        wDay: dt.day() as u16,
        wHour: dt.hour() as u16,
        wMinute: dt.minute() as u16,
        wSecond: dt.second() as u16,
        wMilliseconds: 0,
    };
    // SAFETY: plain value conversions.
    unsafe {
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut ft: FILETIME = std::mem::zeroed();
        if TzSpecificLocalTimeToSystemTime(std::ptr::null(), &local, &mut utc) == 0 || SystemTimeToFileTime(&utc, &mut ft) == 0 {
            return 0;
        }
        crate::listing::filetime_to_unix_ms(((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64)
    }
}

fn unix_ms_to_dos(ms: i64) -> DateTime {
    let ticks = (ms.max(0) as u64) * 10_000 + 116_444_736_000_000_000;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    // SAFETY: plain value conversions.
    unsafe {
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        let mut local: SYSTEMTIME = std::mem::zeroed();
        if FileTimeToSystemTime(&ft, &mut utc) == 0 || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) == 0 {
            return DateTime::default();
        }
        DateTime::from_date_and_time(local.wYear, local.wMonth as u8, local.wDay as u8, local.wHour as u8, local.wMinute as u8, local.wSecond as u8)
            .unwrap_or_default()
    }
}

fn mtime_ms(md: &fs::Metadata) -> i64 {
    md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

// ---------- entries ----------

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    /// The format's own entry number (ZIP: central directory index).
    index: usize,
    /// '/'-separated, no leading/trailing slash.
    name: String,
    dir: bool,
    size: u64,
    mtime: i64,
}

/// Normalized entry name; None for unsafe ones ("../evil", "C:x") — those
/// are never listed nor extracted.
fn norm_name(raw: &str) -> Option<String> {
    let segs: Vec<&str> = raw.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".").collect();
    if segs.is_empty() || segs.iter().any(|s| *s == ".." || s.contains(':')) {
        return None;
    }
    Some(segs.join("/"))
}

fn format_of_path(ar: &Path) -> Format {
    format_of(&ar.to_string_lossy()).unwrap_or(Format::Zip)
}

// ---------- passwords ----------
// Kept in memory only, until the program closes (never written to disk).

static PASSWORDS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn pw_key(ar: &Path) -> String {
    ar.to_string_lossy().to_lowercase()
}

fn cached_pw(ar: &Path) -> Option<String> {
    PASSWORDS.lock().unwrap().as_ref()?.get(&pw_key(ar)).cloned()
}

/// Remembers the password for `archive` (from the UI's password prompt).
pub fn set_password(archive: &str, password: &str) {
    PASSWORDS.lock().unwrap().get_or_insert_default().insert(pw_key(Path::new(archive)), password.to_string());
}

/// Formats signal "password missing / wrong" with this error.
#[derive(Debug)]
struct NeedPassword {
    wrong: bool,
}

impl std::fmt::Display for NeedPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(if self.wrong { "Złe hasło" } else { "Wymagane hasło" })
    }
}

impl std::error::Error for NeedPassword {}

fn need_pw(wrong: bool) -> io::Error {
    io::Error::other(NeedPassword { wrong })
}

fn need_pw_of(e: &io::Error) -> Option<bool> {
    e.get_ref()?.downcast_ref::<NeedPassword>().map(|n| n.wrong)
}

/// The UI recognizes this text and asks for the password (then retries).
pub fn password_message(archive: &Path, wrong: bool) -> String {
    format!("{} hasło do archiwum: {}", if wrong { "Złe" } else { "Wymagane" }, archive.display())
}

/// Runs `f` with the cached password; on NeedPassword asks `host` and retries.
fn with_password<T>(ar: &Path, host: &dyn Host, mut f: impl FnMut(Option<&str>) -> io::Result<T>) -> io::Result<T> {
    loop {
        let pw = cached_pw(ar);
        let r = f(pw.as_deref());
        let Some(wrong) = r.as_ref().err().and_then(need_pw_of) else { return r };
        let wrong = wrong || pw.is_some();
        match host.password(&ar.display().to_string(), wrong) {
            Some(p) => set_password(&ar.to_string_lossy(), &p),
            None => return Err(io::Error::other(password_message(ar, wrong))),
        }
    }
}

fn entries(ar: &Path, host: &dyn Host) -> io::Result<Vec<Entry>> {
    with_password(ar, host, |pw| match format_of_path(ar) {
        Format::Zip => zipfmt::entries(ar),
        Format::SevenZ => sevenz::entries(ar, pw),
        Format::Rar => rar::entries(ar, pw),
        f @ (Format::Tar | Format::TarGz | Format::Gz) => tarfmt::entries(ar, f),
    })
}

/// Decodes the planned files (archive order is up to the format). A missing
/// or wrong password: ask, then continue with what isn't done yet.
fn decode(ar: &Path, plan: &[(&Entry, PathBuf)], sink: &mut Sink) -> io::Result<()> {
    loop {
        let done: HashSet<usize> = sink.run.ok.iter().copied().collect();
        let todo: Vec<(&Entry, PathBuf)> = plan.iter().filter(|(e, _)| !done.contains(&e.index)).cloned().collect();
        if todo.is_empty() || sink.stop {
            return Ok(());
        }
        let pw = cached_pw(ar);
        let r = match format_of_path(ar) {
            Format::Zip => zipfmt::decode(ar, &todo, sink, pw.as_deref()),
            Format::SevenZ => sevenz::decode(ar, &todo, sink, pw.as_deref()),
            Format::Rar => rar::decode(ar, &todo, sink, pw.as_deref()),
            f @ (Format::Tar | Format::TarGz | Format::Gz) => tarfmt::decode(ar, f, &todo, sink),
        };
        let Some(wrong) = r.as_ref().err().and_then(need_pw_of) else { return r };
        let wrong = wrong || pw.is_some();
        match sink.run.host.password(&ar.display().to_string(), wrong) {
            Some(p) => set_password(&ar.to_string_lossy(), &p),
            None => return Err(io::Error::other(password_message(ar, wrong))),
        }
    }
}

/// Entries at or below `inner` ("" = everything).
fn under<'e>(all: &'e [Entry], inner: &str) -> impl Iterator<Item = &'e Entry> {
    let prefix = format!("{inner}/");
    all.iter().filter(move |e| inner.is_empty() || e.name == inner || e.name.starts_with(&prefix))
}

/// Folder in the archive: explicit entry or implied by children.
fn is_dir_in(all: &[Entry], inner: &str) -> bool {
    let prefix = format!("{inner}/");
    inner.is_empty() || all.iter().any(|e| (e.dir && e.name == inner) || e.name.starts_with(&prefix))
}

pub fn list(path: &str) -> io::Result<Listing> {
    let start = Instant::now();
    let (ar, inner) = split(path).ok_or_else(|| io::Error::other(crate::i18n::tr("to nie jest archiwum", "not an archive")))?;
    let all = entries(&ar, &Quiet)?;
    let prefix = if inner.is_empty() { String::new() } else { format!("{inner}/") };
    if !inner.is_empty() && !all.iter().any(|e| e.name.starts_with(&prefix)) && !is_dir_in(&all, &inner) {
        return Err(io::Error::new(io::ErrorKind::NotFound, crate::trf!("Nie ma „{inner}” w archiwum", "No “{inner}” in the archive")));
    }
    // child name -> (dir, size, mtime); folders may exist only implicitly.
    let mut kids: BTreeMap<String, (bool, u64, i64)> = BTreeMap::new();
    for e in &all {
        let Some(rest) = e.name.strip_prefix(&prefix) else { continue };
        if rest.is_empty() {
            continue;
        }
        let (first, deeper) = match rest.split_once('/') {
            Some((f, _)) => (f, true),
            None => (rest, false),
        };
        let is_dir = deeper || e.dir;
        let k = kids.entry(first.to_string()).or_insert((is_dir, 0, 0));
        k.0 |= is_dir;
        if !is_dir {
            k.1 = e.size;
        }
        if !deeper {
            k.2 = e.mtime;
        }
    }
    let shown = if inner.is_empty() {
        ar.display().to_string()
    } else {
        format!("{}\\{}", ar.display(), inner.replace('/', "\\"))
    };
    let mut l = Listing { path: shown, ..Default::default() };
    for (name, (dir, size, mtime)) in kids {
        l.names.push(name);
        l.sizes.push(size);
        l.mtimes.push(mtime);
        l.attrs.push(if dir { ATTR_DIR } else { 0 });
    }
    l.finish(start);
    Ok(l)
}

/// Host for synchronous one-shot operations (temp extraction, rename, F7).
struct Quiet;
impl Host for Quiet {
    fn progress(&self, _: &Progress) {}
    fn conflict(&self, _: &Conflict) -> ConflictAnswer {
        ConflictAnswer { action: ConflictAction::Overwrite, all: true }
    }
    fn error(&self, _: &OpError) -> ErrorAction {
        ErrorAction::Skip
    }
    fn cancelled(&self) -> bool {
        false
    }
}

fn first_error(out: Outcome) -> io::Error {
    io::Error::other(out.errors.into_iter().next().map_or_else(|| crate::i18n::tr("nie udało się", "failed").to_string(), |e| e.message))
}

/// Single file out of an archive into %TEMP%\twinyard\zip\… (viewer, open).
pub fn extract_to_temp(path: &str) -> io::Result<PathBuf> {
    extract_to_temp_in(path, "zip")
}

/// Like `extract_to_temp`, under %TEMP%\twinyard\<sub> (F4 keeps its copies apart).
pub fn extract_to_temp_in(path: &str, sub: &str) -> io::Result<PathBuf> {
    let (ar, inner) = split(path).ok_or_else(|| io::Error::other(crate::i18n::tr("to nie jest archiwum", "not an archive")))?;
    let all = entries(&ar, &Quiet)?;
    let e = all.iter().find(|e| e.name == inner && !e.dir).ok_or_else(|| io::Error::other(crate::trf!("Nie ma pliku „{inner}”", "No file “{inner}”")))?;
    // Name + hash of the full archive path: two "dane.zip" in different
    // folders must never share copies (an edit would go back to the wrong one).
    let mut h: u32 = 0x811c9dc5;
    for b in ar.to_string_lossy().to_lowercase().bytes() {
        h = (h ^ b as u32).wrapping_mul(0x01000193);
    }
    let stem = ar.file_name().map(|s| format!("{}-{h:08x}", s.to_string_lossy().replace('.', "_"))).unwrap_or_default();
    let mut out = std::env::temp_dir().join("twinyard").join(sub).join(stem);
    for seg in inner.split('/') {
        out.push(seg);
    }
    let mut run = Run::new(&Quiet);
    let plan = [(e, out.clone())];
    let mut sink = Sink::new(&mut run);
    let res = decode(&ar, &plan, &mut sink);
    drop(sink);
    res?;
    if run.ok.contains(&e.index) { Ok(out) } else { Err(first_error(run.out)) }
}

// ---------- one writer per archive ----------
// Every ZIP change reads the entry list, then rewrites the whole file: two
// at once would lose one change (or mix two `.part` files). Jobs wait for
// their turn; quick UI commands (F2, F7) refuse instead of freezing.

static BUSY: Mutex<Vec<String>> = Mutex::new(Vec::new());
static FREED: Condvar = Condvar::new();

fn lock_key(ar: &Path) -> String {
    fs::canonicalize(ar).unwrap_or_else(|_| ar.to_path_buf()).to_string_lossy().to_lowercase()
}

pub(crate) struct ArchiveLock(String);

impl Drop for ArchiveLock {
    fn drop(&mut self) {
        BUSY.lock().unwrap().retain(|k| *k != self.0);
        FREED.notify_all();
    }
}

fn busy_msg() -> &'static str {
    crate::i18n::tr("Archiwum jest właśnie zmieniane przez inne zadanie — spróbuj za chwilę", "The archive is being changed by another job — try again in a moment")
}

fn try_lock(ar: &Path) -> io::Result<ArchiveLock> {
    let key = lock_key(ar);
    let mut busy = BUSY.lock().unwrap();
    if busy.contains(&key) {
        return Err(io::Error::other(busy_msg()));
    }
    busy.push(key.clone());
    Ok(ArchiveLock(key))
}

/// Waits for the archive; None = cancelled while waiting.
fn lock_wait(ar: &Path, run: &mut Run) -> Option<ArchiveLock> {
    let key = lock_key(ar);
    let mut busy = BUSY.lock().unwrap();
    let mut told = false;
    while busy.contains(&key) {
        if run.host.cancelled() {
            return None;
        }
        if !told {
            told = true;
            run.p.current = crate::trf!("Czekam, aż inne zadanie skończy zmieniać {}", "Waiting for another job to finish changing {}", ar.display());
            run.host.progress(&run.p);
        }
        busy = FREED.wait_timeout(busy, Duration::from_millis(200)).unwrap().0;
    }
    busy.push(key.clone());
    Some(ArchiveLock(key))
}

// ---------- jobs ----------

pub(crate) struct Run<'a> {
    host: &'a dyn Host,
    p: Progress,
    out: Outcome,
    conflict_all: Option<ConflictAction>,
    skip_errors: bool,
    /// Done items: entry indices (extract) or positions in `Plan::add` (ZIP writes).
    ok: Vec<usize>,
}

impl<'a> Run<'a> {
    fn new(host: &'a dyn Host) -> Self {
        Run { host, p: Progress::default(), out: Outcome::default(), conflict_all: None, skip_errors: false, ok: Vec::new() }
    }

    fn error_out(mut self, path: impl ToString, message: impl ToString) -> Outcome {
        self.out.errors.push(OpError { path: path.to_string(), message: message.to_string() });
        self.out
    }

    fn cancelled_out(mut self) -> Outcome {
        self.out.cancelled = true;
        self.out
    }

    /// Records the error; false = the user chose Cancel.
    fn fail(&mut self, path: &str, e: impl ToString) -> bool {
        let message = e.to_string();
        if !self.skip_errors {
            match self.host.error(&OpError { path: path.into(), message: message.clone() }) {
                ErrorAction::Cancel => return false,
                ErrorAction::SkipAll => self.skip_errors = true,
                _ => {} // Retry isn't meaningful for archive I/O: treat as skip
            }
        }
        self.out.errors.push(OpError { path: path.into(), message });
        true
    }

    fn ask(&mut self, c: &Conflict) -> ConflictAction {
        if let Some(a) = self.conflict_all {
            return a;
        }
        let a = self.host.conflict(c);
        if a.all {
            self.conflict_all = Some(a.action);
        }
        a.action
    }

    /// Local target conflict. None = skip, Some(path) = write there; Err(()) = cancelled.
    fn resolve(&mut self, src: &str, size: u64, mtime: i64, target: PathBuf) -> Result<Option<PathBuf>, ()> {
        let Ok(md) = fs::metadata(&target) else { return Ok(Some(target)) };
        let c = Conflict {
            source: src.into(),
            target: target.display().to_string(),
            source_size: size,
            source_mtime: mtime,
            target_size: md.len(),
            target_mtime: mtime_ms(&md),
            can_resume: false,
        };
        Ok(match self.ask(&c) {
            ConflictAction::Cancel => return Err(()),
            ConflictAction::Skip => None,
            ConflictAction::Rename => Some(unique_name(&target)),
            ConflictAction::OverwriteOlder if c.target_mtime >= c.source_mtime => None,
            _ => {
                let _ = crate::win::clear_readonly(&target);
                Some(target)
            }
        })
    }

    fn start(&mut self, files: u64, bytes: u64) {
        self.p.total_files = files;
        self.p.total_bytes = bytes;
        self.p.phase = Phase::Run;
        self.host.progress(&self.p);
    }
}

/// Where decoded bytes go, one target file at a time. Formats call
/// `open` → `write`… → `close` (or `abort`); `stop` = cancelled.
pub(crate) struct Sink<'r, 'h> {
    run: &'r mut Run<'h>,
    cur: Option<(usize, PathBuf, fs::File, i64)>,
    /// Last closed file (a check after it may still fail it).
    last: Option<(usize, PathBuf)>,
    stop: bool,
}

impl<'r, 'h> Sink<'r, 'h> {
    fn new(run: &'r mut Run<'h>) -> Self {
        Sink { run, cur: None, last: None, stop: false }
    }

    /// The last closed file turned out bad (checksum after the end): remove it.
    fn undo_last(&mut self) {
        if let Some((index, path)) = self.last.take()
            && self.run.ok.last() == Some(&index)
        {
            self.run.ok.pop();
            self.run.out.done_files -= 1;
            self.run.p.done_files -= 1;
            let _ = fs::remove_file(path);
        }
    }

    /// Creates the target; false = don't feed this entry.
    fn open(&mut self, e: &Entry, target: &Path) -> bool {
        self.run.p.current = e.name.clone();
        let r = (|| {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::File::create(target)
        })();
        match r {
            Ok(f) => {
                self.cur = Some((e.index, target.to_path_buf(), f, e.mtime));
                true
            }
            Err(err) => {
                self.fail(target, err);
                false
            }
        }
    }

    /// false = stop feeding the current entry (write error or cancel).
    fn write(&mut self, data: &[u8]) -> bool {
        let Some(cur) = self.cur.as_mut() else { return false };
        if let Err(e) = cur.2.write_all(data) {
            self.abort(Some(e.to_string()));
            return false;
        }
        self.run.p.done_bytes += data.len() as u64;
        self.run.host.progress(&self.run.p);
        if self.run.host.cancelled() {
            self.stop = true;
            self.abort(None);
            return false;
        }
        true
    }

    fn close(&mut self) {
        let Some((index, path, f, mtime)) = self.cur.take() else { return };
        if mtime > 0 {
            let _ = f.set_modified(UNIX_EPOCH + Duration::from_millis(mtime as u64));
        }
        self.last = Some((index, path));
        self.run.ok.push(index);
        self.run.out.done_files += 1;
        self.run.p.done_files += 1;
        self.run.host.progress(&self.run.p);
    }

    /// The current entry failed or was cut short: no half files are left.
    fn abort(&mut self, err: Option<String>) {
        if let Some((_, path, f, _)) = self.cur.take() {
            drop(f);
            let _ = fs::remove_file(&path);
            if let Some(e) = err {
                self.fail(&path, e);
            }
        }
    }

    fn fail(&mut self, path: &Path, e: impl ToString) {
        if !self.run.fail(&path.display().to_string(), e) {
            self.stop = true;
        }
    }
}

/// F5 / F6 out of an archive into a local directory.
pub fn extract(spec: &OpSpec, host: &dyn Host) -> Outcome {
    let mut run = Run::new(host);
    let first = spec.sources.first().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let Some((ar, _)) = split(&first) else { return run.out };
    let moving = spec.kind == OpKind::Move;
    if moving && format_of_path(&ar) != Format::Zip {
        return run.error_out(first, read_only());
    }
    let Some(dest) = spec.dest.clone() else { return run.out };
    let _lock = if moving {
        match lock_wait(&ar, &mut run) {
            Some(l) => Some(l),
            None => return run.cancelled_out(),
        }
    } else {
        None
    };
    let all = match entries(&ar, host) {
        Ok(x) => x,
        Err(e) => return run.error_out(ar.display(), e),
    };
    // (entry, path relative to the target dir)
    let mut work: Vec<(&Entry, String)> = Vec::new();
    let mut roots = 0;
    for s in &spec.sources {
        let Some((_, inner)) = split(&s.to_string_lossy()) else { continue };
        let base = inner.rsplit_once('/').map_or("", |(p, _)| p);
        let strip = if base.is_empty() { 0 } else { base.len() + 1 };
        if inner.is_empty() {
            continue;
        }
        let before = work.len();
        work.extend(under(&all, &inner).map(|e| (e, e.name[strip..].to_string())));
        if work.len() > before {
            roots += 1;
        }
    }
    run.start(work.iter().filter(|(e, _)| !e.dir).count() as u64, work.iter().filter(|(e, _)| !e.dir).map(|(e, _)| e.size).sum());

    // Existing dir = extract into it; single source + new name = rename root.
    let rename_root = roots == 1 && !dest.is_dir();
    let root_name = first.rsplit(['\\', '/']).next().unwrap_or("").to_string();
    let new_root = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let base = if rename_root { dest.parent().map(Path::to_path_buf).unwrap_or_default() } else { dest.clone() };
    let mut plan: Vec<(&Entry, PathBuf)> = Vec::new();
    for (e, rel) in &work {
        let rel = if rename_root { rel.replacen(root_name.as_str(), &new_root, 1) } else { rel.clone() };
        let target = rel.split('/').fold(base.clone(), |p, s| p.join(s));
        if e.dir {
            match fs::create_dir_all(&target) {
                Ok(()) => run.ok.push(e.index),
                Err(err) => {
                    if !run.fail(&target.display().to_string(), err) {
                        return run.cancelled_out();
                    }
                }
            }
            continue;
        }
        match run.resolve(&e.name, e.size, e.mtime, target) {
            Err(()) => return run.cancelled_out(),
            Ok(None) => {
                run.out.skipped += 1;
                run.p.done_files += 1;
                run.p.done_bytes += e.size;
            }
            Ok(Some(t)) => plan.push((e, t)),
        }
    }
    let mut sink = Sink::new(&mut run);
    let res = decode(&ar, &plan, &mut sink);
    let stopped = sink.stop;
    drop(sink);
    if let Err(e) = res {
        run.out.errors.push(OpError { path: ar.display().to_string(), message: e.to_string() });
    }
    if stopped {
        return run.cancelled_out();
    }
    if moving {
        // Drop what was extracted; a folder only when nothing of it stayed behind.
        let done: HashSet<usize> = run.ok.iter().copied().collect();
        let drop: HashSet<usize> = work
            .iter()
            .filter(|(e, _)| done.contains(&e.index) && (!e.dir || under(&all, &e.name).all(|k| done.contains(&k.index))))
            .map(|(e, _)| e.index)
            .collect();
        if !drop.is_empty() {
            run.p.current = crate::i18n::tr("Aktualizacja archiwum", "Updating the archive").into();
            run.start(run.p.total_files, run.p.total_bytes + fs::metadata(&ar).map_or(0, |m| m.len()));
            let keep_dir = keep_parent(&all, &first, &drop);
            let plan = zipfmt::Plan { drop, keep_dir, ..Default::default() };
            match zipfmt::rewrite(Some(&ar), &ar, plan, &mut run) {
                Ok(true) => {}
                Ok(false) => return run.cancelled_out(),
                Err(e) => return run.error_out(ar.display(), e),
            }
        }
    }
    run.out
}

/// Deleting/moving out the last items of a folder must not make the folder
/// (the one shown in the panel) vanish: it gets an explicit entry then.
fn keep_parent(all: &[Entry], first: &str, drop: &HashSet<usize>) -> Option<String> {
    let (_, inner) = split(first)?;
    let parent = inner.rsplit_once('/')?.0;
    let explicit = all.iter().any(|e| e.dir && e.name == parent && !drop.contains(&e.index));
    let left = under(all, parent).any(|e| e.name != parent && !drop.contains(&e.index));
    (!explicit && !left).then(|| parent.to_string())
}

/// Files and folders to put into an archive; `name` relative to its root.
pub(crate) struct NewItem {
    src: Option<PathBuf>,
    name: String,
    dir: bool,
    size: u64,
    mtime: i64,
}

/// Pre-order walk; links and junctions are skipped (as in local copying).
/// Whatever can't be read or is skipped goes to `errs` — an archive must
/// never look complete when it isn't.
fn walk(p: &Path, rel: String, out: &mut Vec<NewItem>, errs: &mut Vec<OpError>) {
    let fail = |errs: &mut Vec<OpError>, m: String| errs.push(OpError { path: p.display().to_string(), message: m });
    let md = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) => return fail(errs, e.to_string()),
    };
    // Only real links (junction / symlink); OneDrive placeholders are
    // reparse points too, but ordinary files and folders.
    if crate::win::is_link(&md) {
        return fail(errs, crate::i18n::tr("Pominięto dowiązanie (junction/symlink)", "Skipped a link (junction/symlink)").into());
    }
    if md.is_dir() {
        out.push(NewItem { src: Some(p.to_path_buf()), name: rel.clone(), dir: true, size: 0, mtime: mtime_ms(&md) });
        let rd = match fs::read_dir(p) {
            Ok(rd) => rd,
            Err(e) => return fail(errs, e.to_string()),
        };
        let mut kids = Vec::new();
        for k in rd {
            match k {
                Ok(k) => kids.push(k),
                Err(e) => fail(errs, e.to_string()),
            }
        }
        kids.sort_by_key(|e| e.file_name());
        for k in kids {
            let name = k.file_name().to_string_lossy().into_owned();
            walk(&k.path(), format!("{rel}/{name}"), out, errs);
        }
    } else {
        out.push(NewItem { src: Some(p.to_path_buf()), name: rel, dir: false, size: md.len(), mtime: mtime_ms(&md) });
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn zip_only(ar: &Path) -> Result<(), &'static str> {
    if format_of_path(ar) == Format::Zip { Ok(()) } else { Err(read_only()) }
}

/// `spec.dest` = the .zip to create; sources = local files / dirs.
pub fn pack(spec: &OpSpec, host: &dyn Host) -> Outcome {
    let mut run = Run::new(host);
    let Some(dest) = spec.dest.clone() else { return run.out };
    let Some(_lock) = lock_wait(&dest, &mut run) else { return run.cancelled_out() };
    let mut items = Vec::new();
    for s in &spec.sources {
        let name = s.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        walk(s, name, &mut items, &mut run.out.errors);
    }
    items.retain(|i| !i.src.as_deref().is_some_and(|s| same_file(s, &dest)));
    let bytes = items.iter().filter(|i| !i.dir).map(|i| i.size).sum();
    run.start(items.iter().filter(|i| !i.dir).count() as u64, bytes);
    let dest = match run.resolve(&dest.display().to_string(), bytes, 0, dest.clone()) {
        Err(()) | Ok(None) => return run.cancelled_out(),
        Ok(Some(d)) => d,
    };
    match zipfmt::rewrite(None, &dest, zipfmt::Plan { add: items, ..Default::default() }, &mut run) {
        Ok(true) => run.out,
        Ok(false) => run.cancelled_out(),
        Err(e) => run.error_out(dest.display(), e),
    }
}

/// F5 / F6 / drag from local disk into a ZIP (`spec.dest` inside it).
pub fn add(spec: &OpSpec, host: &dyn Host) -> Outcome {
    let mut run = Run::new(host);
    let Some(dest) = spec.dest.as_ref() else { return run.out };
    let dest_s = dest.to_string_lossy().into_owned();
    let Some((zip, inner)) = split(&dest_s) else { return run.out };
    if let Err(m) = zip_only(&zip) {
        return run.error_out(dest_s, m);
    }
    let Some(_lock) = lock_wait(&zip, &mut run) else { return run.cancelled_out() };
    let all = match entries(&zip, host) {
        Ok(x) => x,
        Err(e) => return run.error_out(zip.display(), e),
    };
    // Into a folder of the archive, or a single source under a new name.
    let (base, new_name) = if dest_s.ends_with(['\\', '/']) || spec.sources.len() != 1 || is_dir_in(&all, &inner) {
        (inner.clone(), None)
    } else {
        match inner.rsplit_once('/') {
            Some((b, n)) => (b.to_string(), Some(n.to_string())),
            None => (String::new(), Some(inner.clone())),
        }
    };
    let mut items = Vec::new();
    for s in &spec.sources {
        let name = new_name.clone().unwrap_or_else(|| s.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let full = if base.is_empty() { name } else { format!("{base}/{name}") };
        // "../x" or "C:x" would be written but never shown (nor extracted).
        if norm_name(&full).as_deref() != Some(full.as_str()) {
            return run.error_out(full, crate::i18n::tr("Niepoprawna nazwa w archiwum", "Invalid name inside the archive"));
        }
        walk(s, full, &mut items, &mut run.out.errors);
    }
    items.retain(|i| !i.src.as_deref().is_some_and(|s| same_file(s, &zip)));
    let dirs: Vec<PathBuf> = items.iter().filter(|i| i.dir).filter_map(|i| i.src.clone()).collect();

    // Conflicts with what is already inside (case-insensitive, as on Windows).
    let existing: HashMap<String, &Entry> = all.iter().map(|e| (e.name.to_lowercase(), e)).collect();
    let mut taken: HashSet<String> = existing.keys().cloned().collect();
    let mut plan = zipfmt::Plan::default();
    for mut it in items {
        let key = it.name.to_lowercase();
        let Some(e) = existing.get(&key) else {
            taken.insert(key);
            plan.add.push(it);
            continue;
        };
        if it.dir {
            if !e.dir {
                run.out.errors.push(OpError { path: it.name.clone(), message: "W archiwum jest plik o tej nazwie".into() });
            }
            continue; // the folder is already there
        }
        if e.dir {
            run.out.errors.push(OpError { path: it.name.clone(), message: "W archiwum jest katalog o tej nazwie".into() });
            continue;
        }
        let c = Conflict {
            source: it.src.as_ref().map(|s| s.display().to_string()).unwrap_or_default(),
            target: format!("{}\\{}", zip.display(), e.name.replace('/', "\\")),
            source_size: it.size,
            source_mtime: it.mtime,
            target_size: e.size,
            target_mtime: e.mtime,
            can_resume: false,
        };
        match run.ask(&c) {
            ConflictAction::Cancel => return run.cancelled_out(),
            ConflictAction::Skip => run.out.skipped += 1,
            ConflictAction::OverwriteOlder if e.mtime >= it.mtime => run.out.skipped += 1,
            ConflictAction::Rename => {
                it.name = unique_in(&taken, &it.name);
                taken.insert(it.name.to_lowercase());
                plan.add.push(it);
            }
            _ => {
                // The old entry goes only if the new file can be read: a file
                // locked by another program must not cost the old version.
                if let Some(src) = &it.src
                    && let Err(err) = fs::File::open(src)
                {
                    run.out.errors.push(OpError { path: src.display().to_string(), message: err.to_string() });
                    continue;
                }
                plan.drop.insert(e.index);
                plan.replacing.insert(plan.add.len());
                plan.add.push(it);
            }
        }
    }
    let new_bytes: u64 = plan.add.iter().filter(|i| !i.dir).map(|i| i.size).sum();
    run.start(plan.add.iter().filter(|i| !i.dir).count() as u64, new_bytes + fs::metadata(&zip).map_or(0, |m| m.len()));
    if plan.add.is_empty() {
        return run.out;
    }
    let added: Vec<Option<PathBuf>> = plan.add.iter().map(|i| if i.dir { None } else { i.src.clone() }).collect();
    match zipfmt::rewrite(Some(&zip), &zip, plan, &mut run) {
        Ok(true) => {}
        Ok(false) => return run.cancelled_out(),
        Err(e) => return run.error_out(zip.display(), e),
    }
    if spec.kind == OpKind::Move {
        // Only what really landed in the archive; folders when left empty.
        for i in std::mem::take(&mut run.ok) {
            if let Some(Some(src)) = added.get(i) {
                let _ = crate::win::clear_readonly(src);
                if let Err(e) = fs::remove_file(src) {
                    run.out.errors.push(OpError { path: src.display().to_string(), message: e.to_string() });
                }
            }
        }
        for d in dirs.iter().rev() {
            let _ = fs::remove_dir(d);
        }
    }
    run.out
}

/// `name.ext` -> `name (2).ext`, first one not in `taken` (lowercase names).
fn unique_in(taken: &HashSet<String>, name: &str) -> String {
    let (dir, file) = name.rsplit_once('/').map_or(("", name), |(d, f)| (d, f));
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (file, String::new()),
    };
    (2..)
        .map(|n| if dir.is_empty() { format!("{stem} ({n}){ext}") } else { format!("{dir}/{stem} ({n}){ext}") })
        .find(|n| !taken.contains(&n.to_lowercase()))
        .unwrap()
}

/// F8 inside a ZIP (no recycle bin there: always permanent).
pub fn delete(spec: &OpSpec, host: &dyn Host) -> Outcome {
    let mut run = Run::new(host);
    let first = spec.sources.first().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let Some((zip, _)) = split(&first) else { return run.out };
    if let Err(m) = zip_only(&zip) {
        return run.error_out(first, m);
    }
    let Some(_lock) = lock_wait(&zip, &mut run) else { return run.cancelled_out() };
    let all = match entries(&zip, host) {
        Ok(x) => x,
        Err(e) => return run.error_out(zip.display(), e),
    };
    let mut drop = HashSet::new();
    let mut files = 0;
    for s in &spec.sources {
        let s = s.to_string_lossy();
        let Some((z, inner)) = split(&s) else { continue };
        if !z.as_os_str().eq_ignore_ascii_case(zip.as_os_str()) || inner.is_empty() {
            run.out.errors.push(OpError { path: s.to_string(), message: crate::i18n::tr("Nie z tego archiwum", "Not from this archive").into() });
            continue;
        }
        for e in under(&all, &inner) {
            if drop.insert(e.index) && !e.dir {
                files += 1;
            }
        }
    }
    if drop.is_empty() {
        return run.out;
    }
    run.p.current = zip.display().to_string();
    run.start(files, fs::metadata(&zip).map_or(0, |m| m.len()));
    let keep_dir = keep_parent(&all, &first, &drop);
    match zipfmt::rewrite(Some(&zip), &zip, zipfmt::Plan { drop, keep_dir, ..Default::default() }, &mut run) {
        Ok(true) => {
            run.out.done_files = files;
            run.out
        }
        Ok(false) => run.cancelled_out(),
        Err(e) => run.error_out(zip.display(), e),
    }
}

/// Archive -> archive (also within one): extract to a temp dir, add from there.
pub fn between(spec: &OpSpec, host: &dyn Host) -> Outcome {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let tmp = std::env::temp_dir().join("twinyard").join(format!("xfer-{}-{nanos}", std::process::id()));
    if let Err(e) = fs::create_dir_all(&tmp) {
        return Run::new(host).error_out(tmp.display(), e);
    }
    let out = (|| {
        if let Some(d) = &spec.dest
            && let Some((z, _)) = split(&d.to_string_lossy())
            && let Err(m) = zip_only(&z)
        {
            return Run::new(host).error_out(d.display(), m);
        }
        let first = extract(&OpSpec { kind: OpKind::Copy, sources: spec.sources.clone(), dest: Some(tmp.clone()) }, host);
        if first.cancelled || !first.errors.is_empty() {
            return first;
        }
        let mut sources: Vec<PathBuf> = fs::read_dir(&tmp).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        sources.sort();
        let mut out = add(&OpSpec { kind: OpKind::Copy, sources, dest: spec.dest.clone() }, host);
        if spec.kind == OpKind::Move && !out.cancelled && out.errors.is_empty() && out.skipped == 0 {
            let del = delete(&OpSpec { kind: OpKind::Delete, sources: spec.sources.clone(), dest: None }, host);
            out.errors.extend(del.errors);
        }
        out
    })();
    let _ = fs::remove_dir_all(&tmp);
    out
}

/// F2 / Shift+F6 inside one ZIP (files and folders).
pub fn rename(from: &str, to: &str) -> io::Result<()> {
    let (Some((zip, a)), Some((zip2, b))) = (split(from), split(to)) else { return Err(io::Error::other(crate::i18n::tr("to nie jest archiwum", "not an archive"))) };
    if !zip.as_os_str().eq_ignore_ascii_case(zip2.as_os_str()) {
        return Err(io::Error::other(crate::i18n::tr("Przenoszenie między archiwami: użyj F6", "Moving between archives: use F6")));
    }
    zip_only(&zip).map_err(io::Error::other)?;
    if a.is_empty() || b.is_empty() {
        return Err(io::Error::other(crate::i18n::tr("Podaj nazwę wewnątrz archiwum", "Enter a name inside the archive")));
    }
    if norm_name(&b).as_deref() != Some(b.as_str()) {
        return Err(io::Error::other(crate::i18n::tr("Niepoprawna nazwa w archiwum", "Invalid name inside the archive")));
    }
    let _lock = try_lock(&zip)?;
    let all = entries(&zip, &Quiet)?;
    // Case-insensitive, as everywhere on Windows; a case-only rename of
    // `a` itself is fine.
    let (al, bl) = (a.to_lowercase(), b.to_lowercase());
    let within = |n: &str, root: &str| n == root || n.starts_with(&format!("{root}/"));
    if all.iter().any(|e| {
        let n = e.name.to_lowercase();
        within(&n, &bl) && !within(&n, &al)
    }) {
        return Err(io::Error::other(crate::trf!("Już istnieje: {to}", "Already exists: {to}")));
    }
    let mut plan = zipfmt::Plan::default();
    for e in under(&all, &a) {
        plan.rename.insert(e.index, format!("{b}{}", &e.name[a.len()..]));
    }
    if plan.rename.is_empty() {
        return Err(io::Error::other(crate::trf!("Nie ma „{a}” w archiwum", "No “{a}” in the archive")));
    }
    let mut run = Run::new(&Quiet);
    zipfmt::rewrite(Some(&zip), &zip, plan, &mut run)?;
    if run.out.errors.is_empty() { Ok(()) } else { Err(first_error(run.out)) }
}

/// F7 inside a ZIP: an explicit folder entry (nested `a/b` is fine).
pub fn mkdir(path: &str) -> io::Result<()> {
    let (zip, inner) = split(path).ok_or_else(|| io::Error::other(crate::i18n::tr("to nie jest archiwum", "not an archive")))?;
    zip_only(&zip).map_err(io::Error::other)?;
    let inner = norm_name(&inner).ok_or_else(|| io::Error::other(crate::i18n::tr("Niepoprawna nazwa", "Invalid name")))?;
    let _lock = try_lock(&zip)?;
    let all = entries(&zip, &Quiet)?;
    if under(&all, &inner).next().is_some() {
        return Err(io::Error::other(crate::trf!("Już istnieje: {path}", "Already exists: {path}")));
    }
    let item = NewItem { src: None, name: inner, dir: true, size: 0, mtime: now_ms() };
    let mut run = Run::new(&Quiet);
    zipfmt::rewrite(Some(&zip), &zip, zipfmt::Plan { add: vec![item], ..Default::default() }, &mut run)?;
    Ok(())
}

#[cfg(test)]
mod tests;
