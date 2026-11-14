//! Transfers between local disk and FTP: download, upload, delete, rename-move.
//!
//! Stability rules (the reason this exists):
//! - every job has its *own* connection (browsing stays responsive);
//! - a dropped connection is re-opened automatically (5× with backoff, ~1 min)
//!   and the file continues from where it stopped (REST / APPE);
//! - sizes are verified after each file; a short file counts as a dropped
//!   connection (→ resume);
//! - only after the automatic attempts fail is the user asked.

use crate::ftp::{Entry, Session, describe, is_connection_error, is_not_found};
use crate::manager::Remote;
use crate::path::{self, is_remote};
use twinyard_core::ops::{
    Conflict, ConflictAction, ErrorAction, Host, OpError, OpKind, OpSpec, Outcome, Phase, Progress, unique_name,
};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};
use suppaftp::FtpError;

const AUTO_RETRIES: u32 = 5;
/// Seconds before each automatic retry (a router restart takes ~a minute).
const BACKOFF: [u64; AUTO_RETRIES as usize] = [1, 3, 6, 15, 30];
const BUF: usize = 256 * 1024;

enum E {
    Ftp(FtpError),
    /// Could not (re)connect: the network may come back — retried like a drop.
    Connect(String),
    /// Local disk error — retrying automatically won't help.
    Local(io::Error),
    /// Size check failed: treated like a dropped connection.
    Incomplete(String),
    Cancelled,
}

impl E {
    fn reconnect(&self) -> bool {
        match self {
            E::Ftp(e) => is_connection_error(e),
            E::Incomplete(_) | E::Connect(_) => true,
            _ => false,
        }
    }
    fn text(&self) -> String {
        match self {
            E::Ftp(e) => describe(e),
            E::Local(e) => e.to_string(),
            E::Incomplete(m) | E::Connect(m) => m.clone(),
            E::Cancelled => "Przerwano".into(),
        }
    }
}

impl From<FtpError> for E {
    fn from(e: FtpError) -> Self {
        E::Ftp(e)
    }
}

/// Network side of a stream read/write.
fn net(e: io::Error) -> E {
    E::Ftp(FtpError::ConnectionError(e))
}

struct Stop;
type Step<T> = Result<T, Stop>;

/// A server name as a Windows file name: characters Windows forbids become
/// '_' ("a:b" would otherwise land in an alternate data stream of "a").
fn part_path(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".part");
    s.into()
}

fn win_name(s: &str) -> String {
    s.replace(['<', '>', ':', '"', '|', '?', '*', '\\'], "_")
}

struct Item {
    /// Remote path or local path (as string).
    src: String,
    /// Relative to the source root, '/'-separated ("" = the root itself).
    rel: String,
    dir: bool,
    size: u64,
    mtime: i64,
}

pub fn run(remote: &Remote, spec: &OpSpec, host: &dyn Host) -> Outcome {
    run_inner(remote, spec, host, false)
}

/// `strict_names`: a server name Windows can't hold fails instead of being
/// saved as `a_b` (relaying must send every file back under its own name).
fn run_inner(remote: &Remote, spec: &OpSpec, host: &dyn Host, strict_names: bool) -> Outcome {
    // FTP has no server-side copy: a copy between servers (or on one server)
    // is relayed through a local temp dir. Moving between servers is not
    // offered at all — too many ways to lose the source halfway.
    let all_remote = spec.sources.iter().all(|s| is_remote(&s.to_string_lossy()));
    if all_remote && matches!(spec.kind, OpKind::Copy | OpKind::Move) && !spec.sources.is_empty() {
        let dest = spec.dest.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let sid = path::split(&spec.sources[0].to_string_lossy()).map(|(id, _)| id);
        let did = path::split(&dest).map(|(id, _)| id);
        if is_remote(&dest) && spec.kind == OpKind::Move && sid != did {
            let mut out = Outcome::default();
            out.errors.push(OpError {
                path: dest,
                message: twinyard_core::i18n::tr(
                    "Przenoszenie między serwerami jest wyłączone — skopiuj (F5), sprawdź, a potem usuń źródło (F8)",
                    "Moving between servers is disabled — copy (F5), check, then delete the source (F8)",
                )
                .into(),
            });
            return out;
        }
        if is_remote(&dest) && spec.kind == OpKind::Copy {
            return relay(remote, spec, &dest, host);
        }
    }
    let src_remote = spec.sources.iter().any(|s| is_remote(&s.to_string_lossy()));
    let dst_remote = spec.dest.as_ref().is_some_and(|d| is_remote(&d.to_string_lossy()));
    let first = spec.sources.first().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dest = spec.dest.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();

    let id_of = |p: &str| path::split(p).map(|(id, _)| id);
    let id = if src_remote { id_of(&first) } else { id_of(&dest) }.unwrap_or_default();
    let mut e = Eng {
        remote,
        host,
        id: id.clone(),
        sess: None,
        p: Progress::default(),
        out: Outcome::default(),
        conflict_all: None,
        skip_errors: false,
        dir_cache: HashMap::new(),
        strict_names,
    };

    let mixed = spec.sources.iter().any(|s| is_remote(&s.to_string_lossy()) != src_remote)
        || (src_remote && spec.sources.iter().any(|s| id_of(&s.to_string_lossy()).as_deref() != Some(id.as_str())));
    let res = if mixed {
        e.fail(&first, twinyard_core::i18n::tr("Źródła muszą być z jednego miejsca", "All sources must come from one place").into());
        Ok(())
    } else {
        match spec.kind {
            OpKind::Trash | OpKind::Delete => e.delete(spec),
            OpKind::Pack => {
                e.fail(&first, twinyard_core::i18n::tr("Pakowanie na FTP — jeszcze nie (spakuj lokalnie Alt+F5 i wyślij F5)", "Packing on FTP is not supported yet (pack locally with Alt+F5, then upload with F5)").into());
                Ok(())
            }
            OpKind::Copy | OpKind::Move if src_remote && dst_remote => {
                // (copies and cross-server moves were relayed above)
                e.server_move(spec, &dest)
            }
            OpKind::Copy | OpKind::Move if src_remote => e.download(spec, Path::new(&dest)),
            OpKind::Copy | OpKind::Move => e.upload(spec, &dest),
        }
    };
    if res.is_err() {
        e.out.cancelled = true;
    }
    if let Some(mut s) = e.sess.take() {
        let _ = s.s.quit();
    }
    e.out
}

struct Eng<'a> {
    remote: &'a Remote,
    host: &'a dyn Host,
    id: String,
    sess: Option<Session>,
    p: Progress,
    out: Outcome,
    conflict_all: Option<ConflictAction>,
    skip_errors: bool,
    /// Remote dir listings for conflict checks on upload.
    dir_cache: HashMap<String, Vec<Entry>>,
    strict_names: bool,
}

impl Eng<'_> {
    // ---------- infrastructure ----------

    fn check(&self) -> Step<()> {
        if self.host.cancelled() { Err(Stop) } else { Ok(()) }
    }

    fn sess(&mut self) -> Result<&mut Session, E> {
        if self.sess.is_none() {
            self.sess = Some(self.remote.open_session(&self.id).map_err(E::Connect)?);
        }
        Ok(self.sess.as_mut().unwrap())
    }

    fn fail(&mut self, path: &str, message: String) {
        self.out.errors.push(OpError { path: path.to_string(), message });
    }

    /// Runs `f`; on a dropped connection reconnects and retries silently
    /// (backoff 1/3/6 s), then asks the user. None = skipped.
    fn attempt<T>(&mut self, what: &str, mut f: impl FnMut(&mut Self) -> Result<T, E>) -> Step<Option<T>> {
        let mut auto = 0;
        loop {
            self.check()?;
            let err = match f(self) {
                Ok(v) => return Ok(Some(v)),
                Err(E::Cancelled) => return Err(Stop),
                Err(e) => e,
            };
            if err.reconnect() {
                if let Some(mut s) = self.sess.take() {
                    let _ = s.s.quit();
                }
                if auto < AUTO_RETRIES {
                    auto += 1;
                    self.p.current = twinyard_core::trf!("Ponowne łączenie ({auto}/{AUTO_RETRIES})… {what}", "Reconnecting ({auto}/{AUTO_RETRIES})… {what}");
                    self.host.progress(&self.p);
                    self.sleep(Duration::from_secs(BACKOFF[auto as usize - 1]))?;
                    continue;
                }
            }
            let message = err.text();
            if self.skip_errors {
                self.fail(what, message);
                return Ok(None);
            }
            match self.host.error(&OpError { path: what.to_string(), message: message.clone() }) {
                ErrorAction::Retry => auto = 0,
                ErrorAction::Skip => {
                    self.fail(what, message);
                    return Ok(None);
                }
                ErrorAction::SkipAll => {
                    self.skip_errors = true;
                    self.fail(what, message);
                    return Ok(None);
                }
                ErrorAction::Cancel => return Err(Stop),
            }
        }
    }

    fn sleep(&self, d: Duration) -> Step<()> {
        let steps = d.as_millis() / 100;
        for _ in 0..steps {
            self.check()?;
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }

    fn conflict(&mut self, c: Conflict) -> Step<ConflictAction> {
        let action = match self.conflict_all {
            Some(a) => a,
            None => {
                let ans = self.host.conflict(&c);
                if ans.all {
                    self.conflict_all = Some(ans.action);
                }
                ans.action
            }
        };
        Ok(match action {
            ConflictAction::Cancel => return Err(Stop),
            ConflictAction::OverwriteOlder if c.target_mtime < c.source_mtime => ConflictAction::Overwrite,
            ConflictAction::OverwriteOlder => ConflictAction::Skip,
            // "Resume all" on a target that is not a shorter copy = overwrite.
            ConflictAction::Resume if !c.can_resume => ConflictAction::Overwrite,
            a => a,
        })
    }

    fn file_done(&mut self, size: u64) {
        self.out.done_files += 1;
        self.p.done_files += 1;
        self.p.done_bytes += size;
        self.host.progress(&self.p);
    }

    fn file_skipped(&mut self, size: u64) {
        self.out.skipped += 1;
        self.p.done_files += 1;
        self.p.done_bytes += size;
        self.host.progress(&self.p);
    }

    fn start_run(&mut self, items: &[Item]) {
        self.p.phase = Phase::Run;
        self.p.total_files = items.iter().filter(|i| !i.dir).count() as u64;
        self.p.total_bytes = items.iter().filter(|i| !i.dir).map(|i| i.size).sum();
        self.host.progress(&self.p);
    }

    // ---------- scanning ----------

    fn list(&mut self, dir: &str) -> Step<Option<Vec<Entry>>> {
        let d = dir.to_string();
        self.attempt(dir, move |e| Ok(e.sess()?.list(&d)?))
    }

    /// Remote sources -> flat pre-order item list. `del`: for deleting —
    /// links are then items of their own (DELE removes the link, never
    /// what it points to).
    fn scan_remote(&mut self, sources: &[PathBuf], del: bool) -> Step<Vec<Item>> {
        let mut items = Vec::new();
        let mut parents: HashMap<String, Vec<Entry>> = HashMap::new();
        for s in sources {
            let Some((_, p)) = path::split(&s.to_string_lossy()) else { continue };
            let parent = path::parent(&p).unwrap_or_else(|| "/".into());
            if !parents.contains_key(&parent) {
                let l = self.list(&parent)?.unwrap_or_default();
                parents.insert(parent.clone(), l);
            }
            let name = path::file_name(&p).to_string();
            let Some(ent) = parents[&parent].iter().find(|e| e.name == name).cloned() else {
                self.fail(&p, twinyard_core::i18n::tr("Nie znaleziono na serwerze", "Not found on the server").into());
                continue;
            };
            self.walk_remote(&p, String::new(), &ent, &mut items, del)?;
        }
        Ok(items)
    }

    fn walk_remote(&mut self, src: &str, rel: String, ent: &Entry, items: &mut Vec<Item>, del: bool) -> Step<()> {
        self.check()?;
        if ent.dir && !(del && ent.link) {
            // A symlink to a file fails CWD with 550 -> treat as a file.
            let d = src.to_string();
            let listed = self.attempt(src, move |e| match e.sess()?.list(&d) {
                Err(err) if ent.link && is_not_found(&err) => Ok(None),
                r => Ok(Some(r?)),
            })?;
            match listed {
                Some(Some(_)) if ent.link => {
                    // A link to a directory (www -> public_html): not followed,
                    // like junctions on disk (and no endless loops on cyclic links).
                    self.fail(src, twinyard_core::i18n::tr("Pominięto dowiązanie do katalogu (symlink)", "Skipped a link to a directory (symlink)").into());
                    return Ok(());
                }
                Some(Some(children)) => {
                    items.push(Item { src: src.into(), rel: rel.clone(), dir: true, size: 0, mtime: ent.mtime });
                    self.p.current = src.to_string();
                    self.host.progress(&self.p);
                    for c in children {
                        let r = if rel.is_empty() { c.name.clone() } else { format!("{rel}/{}", c.name) };
                        self.walk_remote(&path::join(src, &c.name), r, &c, items, del)?;
                    }
                    return Ok(());
                }
                Some(None) => {}       // file symlink
                None => return Ok(()), // skipped
            }
        }
        self.p.total_files += 1;
        self.p.total_bytes += ent.size;
        items.push(Item { src: src.into(), rel, dir: false, size: ent.size, mtime: ent.mtime });
        Ok(())
    }

    fn scan_local(&mut self, sources: &[PathBuf]) -> Step<Vec<Item>> {
        let mut items = Vec::new();
        for s in sources {
            self.walk_local(s, String::new(), &mut items)?;
        }
        Ok(items)
    }

    fn walk_local(&mut self, src: &Path, rel: String, items: &mut Vec<Item>) -> Step<()> {
        self.check()?;
        let md = match fs::symlink_metadata(src) {
            Ok(m) => m,
            Err(e) => {
                self.fail(&src.display().to_string(), e.to_string());
                return Ok(());
            }
        };
        let mtime = md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64);
        if md.is_dir() {
            if twinyard_core::win::is_link(&md) {
                self.fail(&src.display().to_string(), twinyard_core::i18n::tr("Pominięto dowiązanie (junction/symlink)", "Skipped a link (junction/symlink)").into());
                return Ok(());
            }
            items.push(Item { src: src.display().to_string(), rel: rel.clone(), dir: true, size: 0, mtime });
            let rd = match fs::read_dir(src) {
                Ok(r) => r,
                Err(e) => {
                    self.fail(&src.display().to_string(), e.to_string());
                    return Ok(());
                }
            };
            for ent in rd {
                let ent = match ent {
                    Ok(e) => e,
                    Err(e) => {
                        self.fail(&src.display().to_string(), e.to_string()); // reported, not silently left out
                        continue;
                    }
                };
                let name = ent.file_name().to_string_lossy().into_owned();
                let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                self.walk_local(&ent.path(), r, items)?;
            }
        } else {
            self.p.total_files += 1;
            self.p.total_bytes += md.len();
            items.push(Item { src: src.display().to_string(), rel, dir: false, size: md.len(), mtime });
        }
        Ok(())
    }

    // ---------- download ----------

    fn download(&mut self, spec: &OpSpec, dest: &Path) -> Step<()> {
        let items = self.scan_remote(&spec.sources, false)?;
        self.start_run(&items);
        let need: u64 = items.iter().filter(|i| !i.dir).map(|i| i.size).sum();
        if let Some(free) = free_space(dest)
            && free < need
        {
            let (n, f) = (need.div_ceil(1 << 20), free >> 20);
            self.fail(
                &dest.display().to_string(),
                twinyard_core::trf!("Za mało miejsca na dysku: potrzeba {n} MB, wolne {f} MB", "Not enough disk space: {n} MB needed, {f} MB free"),
            );
            return Ok(());
        }
        let roots = items.iter().filter(|i| i.rel.is_empty()).count();
        // Same rule as local copy: existing dir = put inside; single source = new name.
        let into_dir = dest.is_dir() || roots > 1;
        if roots > 1 && !dest.exists() && fs::create_dir_all(dest).is_err() {
            self.fail(&dest.display().to_string(), twinyard_core::i18n::tr("Nie można utworzyć katalogu docelowego", "Cannot create the destination directory").into());
            return Ok(());
        }
        let is_move = spec.kind == OpKind::Move;
        let mut root_target = PathBuf::new();
        let mut done_dirs = Vec::new();
        for it in &items {
            if it.rel.is_empty() {
                root_target = if into_dir { dest.join(win_name(path::file_name(&it.src))) } else { dest.to_path_buf() };
            }
            let target = it.rel.split('/').filter(|s| !s.is_empty()).fold(root_target.clone(), |p, s| p.join(win_name(s)));
            if self.strict_names {
                let root = path::file_name(&it.src);
                let rel_ok = it.rel.split('/').all(|s| win_name(s) == s);
                if !rel_ok || (it.rel.is_empty() && win_name(root) != root) {
                    self.fail(&it.src, twinyard_core::i18n::tr(
                        "Nazwa ze znakami niedozwolonymi w Windows (: * ? \" < > | \\) — nie da się jej przekazać przez dysk lokalny",
                        "The name has characters Windows forbids (: * ? \" < > | \\) — it can't be relayed through the local disk",
                    ).into());
                    if !it.dir {
                        self.file_skipped(it.size);
                    }
                    continue;
                }
            }
            if it.dir {
                let t = target.clone();
                self.attempt(&target.display().to_string(), move |_| fs::create_dir_all(&t).map_err(E::Local))?;
                done_dirs.push(it.src.clone());
            } else {
                self.download_file(it, target, is_move)?;
            }
        }
        if is_move {
            for d in done_dirs.iter().rev() {
                let d = d.clone();
                let _ = self.sess().map(|s| s.s.rmdir(&d)); // non-empty (skipped files) stays
            }
        }
        Ok(())
    }

    fn download_file(&mut self, it: &Item, mut target: PathBuf, is_move: bool) -> Step<()> {
        self.check()?;
        self.p.current = it.src.clone();
        let mut resume = false;
        if let Ok(md) = fs::metadata(&target) {
            if md.is_dir() {
                self.fail(&target.display().to_string(), "W miejscu docelowym istnieje katalog o tej nazwie".into());
                self.file_skipped(it.size);
                return Ok(());
            }
            let tmtime = md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64);
            match self.conflict(Conflict {
                source: it.src.clone(),
                target: target.display().to_string(),
                source_size: it.size,
                source_mtime: it.mtime,
                target_size: md.len(),
                target_mtime: tmtime,
                can_resume: md.len() < it.size,
            })? {
                ConflictAction::Skip => {
                    self.file_skipped(it.size);
                    return Ok(());
                }
                ConflictAction::Rename => target = unique_name(&target),
                ConflictAction::Resume => resume = true,
                _ => {
                    let _ = twinyard_core::win::clear_readonly(&target);
                }
            }
        }

        let base = self.p.done_bytes;
        let (src, size, mtime) = (it.src.clone(), it.size, it.mtime);
        // Written into `<name>.part` and swapped in when complete: the old
        // file stays until then, and a cancel / skip never leaves half a file
        // under the real name. A user-chosen Resume continues the target itself.
        let t = if resume { target.clone() } else { part_path(&target) };
        let part = (!resume).then(|| t.clone());
        let fin = target.clone();
        let mut expect = size;
        let mut retry = false;
        let r = self.attempt(&src.clone(), move |e| {
            // After a failure: what the server holds now. The listing may be
            // stale — a file shorter than listed would otherwise never finish.
            if retry && let Ok(n) = e.sess()?.s.size(&src) {
                expect = n as u64;
            }
            retry = true;
            // After any failure continue from what is already on disk.
            let off = if resume { fs::metadata(&t).map(|m| m.len().min(expect)).unwrap_or(0) } else { 0 };
            resume = true;
            if !(off > 0 && off == expect) {
                e.get_file(&src, &t, off, expect, base)?;
            } else if let Ok(f) = fs::OpenOptions::new().write(true).open(&t) {
                let _ = f.set_len(off); // a longer earlier attempt: cut back
            }
            let got = fs::metadata(&t).map_err(E::Local)?.len();
            // More than the listing said (a link, a growing log): the whole
            // file came down — that is not a dropped connection.
            if got < expect {
                return Err(E::Incomplete(twinyard_core::trf!("Pobrano {got} z {expect} bajtów", "Downloaded {got} of {expect} bytes")));
            }
            if mtime > 0
                && let Ok(f) = fs::OpenOptions::new().write(true).open(&t)
            {
                let _ = f.set_modified(UNIX_EPOCH + Duration::from_millis(mtime as u64));
            }
            if t != fin {
                twinyard_core::win::move_path(&t, &fin, true).map_err(E::Local)?;
            }
            Ok(())
        });
        if let Some(p) = &part
            && !matches!(r, Ok(Some(())))
        {
            let _ = fs::remove_file(p); // cancelled / skipped: no leftovers
        }
        let ok = r?;
        if ok.is_none() {
            self.file_skipped(it.size);
            return Ok(());
        }
        if is_move {
            let s = it.src.clone();
            self.attempt(&it.src, move |e| Ok(e.sess()?.s.rm(&s)?))?;
        }
        self.file_done(it.size);
        Ok(())
    }

    fn get_file(&mut self, src: &str, dst: &Path, off: u64, size: u64, base: u64) -> Result<(), E> {
        // Resume: cut the file back to exactly `off` first — anything past it
        // (an earlier, longer attempt) would otherwise stay glued to the end.
        let mut file = if off > 0 {
            (|| {
                let mut f = fs::OpenOptions::new().write(true).open(dst)?;
                f.set_len(off)?;
                f.seek(SeekFrom::End(0))?;
                Ok(f)
            })()
        } else {
            fs::File::create(dst)
        }
        .map_err(E::Local)?;
        let host = self.host;
        let mut snap = self.p.clone();
        let s = self.sess()?;
        let mut off = off;
        if off > 0
            && let Err(err) = s.s.resume_transfer(off as usize)
        {
            if is_connection_error(&err) {
                return Err(err.into());
            }
            // The server refuses REST: download the whole file again.
            file.set_len(0).map_err(E::Local)?;
            file.seek(SeekFrom::Start(0)).map_err(E::Local)?;
            off = 0;
        }
        let mut stream = s.s.retr_as_stream(src)?;
        let mut buf = vec![0u8; BUF];
        let mut done = off;
        loop {
            let n = stream.read(&mut buf).map_err(net)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(E::Local)?;
            done += n as u64;
            snap.done_bytes = base + done.min(size);
            host.progress(&snap);
            if host.cancelled() {
                let _ = s.s.abort(stream);
                return Err(E::Cancelled);
            }
        }
        if done < size {
            // Data connection ended early. A dying server often never sends the
            // completion reply — don't wait IO_TIMEOUT for it: kill the control
            // connection (finish then fails at once) and let attempt() resume.
            let _ = s.s.get_ref().shutdown(std::net::Shutdown::Both);
            drop(stream);
            return Err(E::Incomplete(twinyard_core::trf!("Połączenie przerwane po {done} z {size} bajtów", "Connection lost after {done} of {size} bytes")));
        }
        stream.finish()?;
        Ok(())
    }

    // ---------- upload ----------

    fn remote_is_dir(&mut self, p: &str) -> Step<bool> {
        let d = p.to_string();
        Ok(self
            .attempt(p, move |e| match e.sess()?.s.cwd(&d) {
                Ok(()) => Ok(true),
                Err(err) if is_not_found(&err) => Ok(false),
                Err(err) => Err(err.into()),
            })?
            .unwrap_or(false))
    }

    fn upload(&mut self, spec: &OpSpec, dest_url: &str) -> Step<()> {
        let Some((_, dest)) = path::split(dest_url) else { return Ok(()) };
        let items = self.scan_local(&spec.sources)?;
        self.start_run(&items);
        let roots = items.iter().filter(|i| i.rel.is_empty()).count();
        let into_dir = roots > 1 || self.remote_is_dir(&dest)?;
        if roots > 1 && !self.remote_is_dir(&dest)? {
            self.mkdir_p(&dest)?;
        }
        let is_move = spec.kind == OpKind::Move;
        let mut root_target = String::new();
        let mut done_dirs = Vec::new();
        for it in &items {
            if it.rel.is_empty() {
                let name = Path::new(&it.src).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                root_target = if into_dir { path::join(&dest, &name) } else { dest.clone() };
            }
            let target = if it.rel.is_empty() { root_target.clone() } else { path::join(&root_target, &it.rel) };
            if it.dir {
                self.mkdir_p(&target)?;
                done_dirs.push(it.src.clone());
            } else {
                self.upload_file(it, target, is_move)?;
            }
        }
        if is_move {
            for d in done_dirs.iter().rev() {
                let _ = fs::remove_dir(d);
            }
        }
        Ok(())
    }

    fn mkdir_p(&mut self, dir: &str) -> Step<()> {
        if self.remote_is_dir(dir)? {
            return Ok(());
        }
        let d = dir.to_string();
        let created = self.attempt(dir, move |e| {
            let s = e.sess()?;
            let mut cur = String::new();
            let mut made = false;
            for seg in d.split('/').filter(|s| !s.is_empty()) {
                cur = format!("{cur}/{seg}");
                made = false;
                if s.s.cwd(&cur).is_err() {
                    s.s.mkdir(&cur)?;
                    made = true;
                }
            }
            Ok(made)
        })?;
        // Known empty only when we really created it (an existing folder the
        // check above failed on would otherwise look empty: no conflicts).
        if created == Some(true) {
            self.dir_cache.insert(path::normalize(dir), Vec::new());
        }
        Ok(())
    }

    /// What is at `target` on the server. Outer None = the parent could not be
    /// listed (error already recorded): conflicts can't be checked, so the
    /// caller must not write there blindly. Names match case-insensitively
    /// (Windows servers): a needless question beats a silent overwrite.
    fn remote_entry(&mut self, target: &str) -> Step<Option<Option<Entry>>> {
        let parent = path::parent(target).unwrap_or_else(|| "/".into());
        if !self.dir_cache.contains_key(&parent) {
            let Some(l) = self.list(&parent)? else { return Ok(None) };
            self.dir_cache.insert(parent.clone(), l);
        }
        let name = path::file_name(target).to_lowercase();
        Ok(Some(self.dir_cache[&parent].iter().find(|e| e.name.to_lowercase() == name).cloned()))
    }

    fn remote_unique(&mut self, target: &str) -> Step<String> {
        let parent = path::parent(target).unwrap_or_else(|| "/".into());
        let name = path::file_name(target).to_string();
        let taken: Vec<String> = self.dir_cache.get(&parent).map(|v| v.iter().map(|e| e.name.clone()).collect()).unwrap_or_default();
        // Like core's unique_name, but counted against the remote listing.
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name.as_str(), ""),
        };
        let mut n = 2;
        loop {
            let c = format!("{stem} ({n}){ext}");
            if !taken.iter().any(|t| t.eq_ignore_ascii_case(&c)) {
                return Ok(path::join(&parent, &c));
            }
            n += 1;
        }
    }

    fn upload_file(&mut self, it: &Item, mut target: String, is_move: bool) -> Step<()> {
        self.check()?;
        self.p.current = it.src.clone();
        let mut resume = false;
        let Some(mut existing) = self.remote_entry(&target)? else {
            self.file_skipped(it.size);
            return Ok(());
        };
        // Some servers hide dot files (.htaccess) from LIST: ask directly
        // before treating one as new (no question, removed on failure).
        let name = path::file_name(&target).to_string();
        if existing.is_none() && name.starts_with('.') {
            let t = target.clone();
            match self.attempt(&target, move |e| match e.sess()?.s.size(&t) {
                Ok(n) => Ok(Some(n as u64)),
                Err(err) if is_connection_error(&err) => Err(err.into()),
                Err(_) => Ok(None),
            })? {
                None => {
                    self.file_skipped(it.size);
                    return Ok(());
                }
                Some(Some(n)) => existing = Some(Entry { name, dir: false, link: false, size: n, mtime: 0 }),
                Some(None) => {}
            }
        }
        // Nothing of value under this name on the server before us.
        let mut fresh = existing.is_none();
        if let Some(ent) = existing {
            if ent.dir {
                self.fail(&target, "W miejscu docelowym istnieje katalog o tej nazwie".into());
                self.file_skipped(it.size);
                return Ok(());
            }
            match self.conflict(Conflict {
                source: it.src.clone(),
                target: path::make(&self.id, &target),
                source_size: it.size,
                source_mtime: it.mtime,
                target_size: ent.size,
                target_mtime: ent.mtime,
                can_resume: ent.size < it.size,
            })? {
                ConflictAction::Skip => {
                    self.file_skipped(it.size);
                    return Ok(());
                }
                ConflictAction::Rename => {
                    target = self.remote_unique(&target)?;
                    fresh = true;
                }
                ConflictAction::Resume => resume = true,
                _ => {}
            }
        }

        let base = self.p.done_bytes;
        let (src, mtime) = (it.src.clone(), it.mtime);
        let t = target.clone();
        // A retry may continue (APPE) only from what *this* job has sent: if
        // the first STOR never reached the server, the remote file is still
        // the old one being overwritten — appending to it would corrupt it.
        let mut started = false;
        let r = self.attempt(&src.clone(), move |e| {
            // The size that counts is the file's as opened now (it may have
            // changed since the scan); exactly that many bytes are sent.
            let file = open_for_upload(Path::new(&src)).map_err(E::Local)?;
            let len = file.metadata().map_err(E::Local)?.len();
            let off = if resume || started {
                match e.sess()?.s.size(&t) {
                    Ok(n) if n as u64 <= len => n as u64,
                    // More on the server than this file has (or unknown):
                    // never append — send it whole.
                    _ => 0,
                }
            } else {
                0
            };
            e.put_file(file, &t, off, len, base, &mut started)?;
            e.verify_upload(&t, len)?;
            // Keep the original date (MFMT) so later compares/syncs see "same".
            e.sess()?.set_mtime(&t, mtime);
            Ok(())
        });
        if fresh && !matches!(r, Ok(Some(()))) {
            // Cancelled / skipped new file: don't leave a truncated copy that
            // looks like the real thing (best effort, the link may be down).
            let t = target.clone();
            if let Ok(s) = self.sess() {
                let _ = s.s.rm(&t);
            }
        }
        let ok = r?;
        if ok.is_none() {
            self.file_skipped(it.size);
            return Ok(());
        }
        // Keep the conflict cache right for later files with the same name.
        let parent = path::parent(&target).unwrap_or_else(|| "/".into());
        if let Some(v) = self.dir_cache.get_mut(&parent) {
            let name = path::file_name(&target).to_string();
            v.retain(|e| e.name != name);
            v.push(Entry { name, dir: false, link: false, size: it.size, mtime: it.mtime });
        }
        if is_move {
            let p = PathBuf::from(&it.src);
            self.attempt(&it.src, move |_| {
                let _ = twinyard_core::win::clear_readonly(&p);
                fs::remove_file(&p).map_err(E::Local)
            })?;
        }
        self.file_done(it.size);
        Ok(())
    }

    /// After a STOR: the server must hold exactly `len` bytes. Without SIZE
    /// the listing tells (a move deletes the local file after this).
    fn verify_upload(&mut self, t: &str, len: u64) -> Result<(), E> {
        let n = match self.sess()?.s.size(t) {
            Ok(n) => Some(n as u64),
            Err(err) if is_connection_error(&err) => return Err(err.into()),
            Err(_) => {
                let parent = path::parent(t).unwrap_or_else(|| "/".into());
                let name = path::file_name(t).to_string();
                self.sess()?.list(&parent)?.into_iter().find(|e| e.name == name).map(|e| e.size)
            }
        };
        match n {
            Some(n) if n != len => Err(E::Incomplete(twinyard_core::trf!("Na serwerze {n} z {len} bajtów", "{n} of {len} bytes on the server"))),
            _ => Ok(()), // equal, or the server shows no way to check
        }
    }

    /// Sends bytes `off..len` of `file`. `started` is set once the server
    /// accepted STOR/APPE (from then on the remote file holds our bytes, so
    /// a retry may resume).
    fn put_file(&mut self, file: fs::File, dst: &str, off: u64, len: u64, base: u64, started: &mut bool) -> Result<(), E> {
        let size = len;
        let mut file = file;
        if off > 0 {
            file.seek(SeekFrom::Start(off)).map_err(E::Local)?;
        }
        let mut file = file.take(len - off);
        let host = self.host;
        let mut snap = self.p.clone();
        let s = self.sess()?;
        // APPE is more widely supported than REST+STOR.
        let mut stream = if off > 0 { s.s.append_with_stream(dst)? } else { s.s.put_with_stream(dst)? };
        *started = true;
        // Empty files over TLS: rustls handshakes lazily, and suppaftp only
        // drains the TLS socket on close (unread TLS 1.3 tickets -> RST -> the
        // server hangs) when something was written. An empty write marks it.
        stream.write(&[]).map_err(net)?;
        stream.flush().map_err(net)?;
        let mut buf = vec![0u8; BUF];
        let mut done = off;
        loop {
            let n = file.read(&mut buf).map_err(E::Local)?;
            if n == 0 {
                break;
            }
            stream.write_all(&buf[..n]).map_err(net)?;
            done += n as u64;
            snap.done_bytes = base + done.min(size);
            host.progress(&snap);
            if host.cancelled() {
                let _ = s.s.abort(stream);
                return Err(E::Cancelled);
            }
        }
        stream.finish()?;
        Ok(())
    }

    // ---------- delete / server-side move ----------

    fn delete(&mut self, spec: &OpSpec) -> Step<()> {
        let items = self.scan_remote(&spec.sources, true)?;
        self.p.phase = Phase::Run;
        self.p.total_bytes = 0;
        self.p.total_files = items.len() as u64;
        self.host.progress(&self.p);
        for it in items.iter().filter(|i| !i.dir) {
            self.p.current = it.src.clone();
            let s = it.src.clone();
            match self.attempt(&it.src, move |e| Ok(e.sess()?.s.rm(&s)?))? {
                Some(()) => self.file_done(0),
                None => self.file_skipped(0),
            }
        }
        // Pre-order list reversed = children before parents.
        for it in items.iter().rev().filter(|i| i.dir) {
            self.p.current = it.src.clone();
            let s = it.src.clone();
            match self.attempt(&it.src, move |e| Ok(e.sess()?.s.rmdir(&s)?))? {
                Some(()) => self.file_done(0),
                None => self.file_skipped(0),
            }
        }
        Ok(())
    }

    /// Move within one server = RNFR/RNTO.
    fn server_move(&mut self, spec: &OpSpec, dest_url: &str) -> Step<()> {
        let Some((_, dest)) = path::split(dest_url) else { return Ok(()) };
        let into_dir = spec.sources.len() > 1 || self.remote_is_dir(&dest)?;
        self.p.phase = Phase::Run;
        self.p.total_files = spec.sources.len() as u64;
        for s in &spec.sources {
            let Some((_, from)) = path::split(&s.to_string_lossy()) else { continue };
            let to = if into_dir { path::join(&dest, path::file_name(&from)) } else { dest.clone() };
            if path::normalize(&to) == path::normalize(&from) {
                self.file_skipped(0);
                continue;
            }
            match self.remote_entry(&to)? {
                None => {
                    self.file_skipped(0); // destination unreadable: don't risk it
                    continue;
                }
                Some(Some(_)) => {
                    self.fail(&to, twinyard_core::i18n::tr("Już istnieje w miejscu docelowym", "Already exists at the destination").into());
                    self.file_skipped(0);
                    continue;
                }
                Some(None) => {}
            }
            self.p.current = from.clone();
            let (a, b) = (from.clone(), to.clone());
            match self.attempt(&from, move |e| Ok(e.sess()?.s.rename(&a, &b)?))? {
                Some(()) => self.file_done(0),
                None => self.file_skipped(0),
            }
        }
        Ok(())
    }
}

/// Opens a file for sending: others may read it but not write it meanwhile
/// (a save halfway through would mix two versions). A file someone already
/// holds open for writing (a live log) is opened shared instead.
fn open_for_upload(p: &Path) -> io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const SHARE_READ: u32 = 1;
    const SHARE_WRITE: u32 = 2;
    const SHARING_VIOLATION: i32 = 32;
    match fs::OpenOptions::new().read(true).share_mode(SHARE_READ).open(p) {
        Err(e) if e.raw_os_error() == Some(SHARING_VIOLATION) => fs::OpenOptions::new().read(true).share_mode(SHARE_READ | SHARE_WRITE).open(p),
        r => r,
    }
}

/// Free bytes on the volume that holds `p` (or its nearest existing parent).
fn free_space(p: &Path) -> Option<u64> {
    let mut cur = Some(p);
    while let Some(x) = cur {
        if x.is_dir() {
            return twinyard_core::drives::disk_space(&x.to_string_lossy()).map(|(free, _)| free);
        }
        cur = x.parent();
    }
    None
}

/// Server -> local temp -> server (copy only; moving between servers is
/// refused in `run_inner`).
fn relay(remote: &Remote, spec: &OpSpec, dest: &str, host: &dyn Host) -> Outcome {
    let tmp = std::env::temp_dir().join("twinyard").join("relay").join(format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
    ));
    if let Err(e) = fs::create_dir_all(&tmp) {
        let mut out = Outcome::default();
        out.errors.push(OpError { path: tmp.display().to_string(), message: e.to_string() });
        return out;
    }
    let down = run_inner(remote, &OpSpec { kind: OpKind::Copy, sources: spec.sources.clone(), dest: Some(tmp.clone()) }, host, true);
    let mut out = down.clone();
    if !down.cancelled {
        // strict_names: every name that arrived is the server's own.
        let local: Vec<PathBuf> = spec
            .sources
            .iter()
            .map(|s| tmp.join(path::file_name(&s.to_string_lossy())))
            .filter(|p| p.exists())
            .collect();
        let up = run_inner(remote, &OpSpec { kind: OpKind::Copy, sources: local, dest: Some(PathBuf::from(dest)) }, host, false);
        out.errors.extend(up.errors.clone());
        out.cancelled = up.cancelled;
        out.done_files = up.done_files;
        out.skipped += up.skipped;
    }
    let _ = fs::remove_dir_all(&tmp);
    out
}
