//! File operations engine: copy / move / trash / delete.
//!
//! UI-agnostic and synchronous: run it on a worker thread. The `Host`
//! receives progress and answers questions (conflicts, errors); its
//! methods may block while the user decides.

use crate::listing::{ATTR_DIR, filetime_to_unix_ms};
use crate::win;
use serde::{Deserialize, Serialize};
use std::fs::{self, Metadata};
use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpKind {
    Copy,
    Move,
    /// Recycle bin.
    Trash,
    /// Permanent delete.
    Delete,
    /// Create the ZIP archive `dest` from `sources`.
    Pack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpSpec {
    pub kind: OpKind,
    pub sources: Vec<PathBuf>,
    /// Copy/Move: target directory, or the new name when a single source
    /// is given and `dest` is not an existing directory.
    #[serde(default)]
    pub dest: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    #[default]
    Scan,
    Run,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub phase: Phase,
    /// 0 for delete/trash: progress is by item count then.
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub total_files: u64,
    pub done_files: u64,
    pub current: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub source: String,
    pub target: String,
    pub source_size: u64,
    pub source_mtime: i64,
    pub target_size: u64,
    pub target_mtime: i64,
    /// Target is a partial copy that can be continued (remote transfers).
    pub can_resume: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictAction {
    Overwrite,
    /// Overwrite only if the target is older than the source.
    OverwriteOlder,
    Skip,
    /// Keep both: `name (2).ext`.
    Rename,
    /// Continue a partial transfer (only offered when `can_resume`).
    Resume,
    Cancel,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ConflictAnswer {
    pub action: ConflictAction,
    /// Apply to all further conflicts in this job.
    #[serde(default)]
    pub all: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpError {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorAction {
    Retry,
    Skip,
    SkipAll,
    Cancel,
}

pub trait Host: Sync {
    /// Called often (every chunk of a big file) — throttle on your side.
    fn progress(&self, p: &Progress);
    fn conflict(&self, c: &Conflict) -> ConflictAnswer;
    fn error(&self, e: &OpError) -> ErrorAction;
    fn cancelled(&self) -> bool;
    /// Password for an encrypted archive (`wrong` = the previous one failed).
    /// None = no password / cancelled.
    fn password(&self, _archive: &str, _wrong: bool) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub cancelled: bool,
    pub done_files: u64,
    pub skipped: u64,
    pub errors: Vec<OpError>,
}

/// Job was cancelled (by the user or a Cancel answer).
struct Stop;
type Step<T> = Result<T, Stop>;

struct Item {
    src: PathBuf,
    /// Path relative to the source root ("" for the root itself).
    rel: PathBuf,
    dir: bool,
    reparse: bool,
    size: u64,
    mtime: i64,
}

pub fn run(spec: &OpSpec, host: &dyn Host) -> Outcome {
    if spec.kind == OpKind::Pack {
        return crate::archive::pack(spec, host);
    }
    let src_in = spec.sources.iter().any(|s| crate::archive::is_inside(&s.to_string_lossy()));
    let dst_in = spec.dest.as_ref().is_some_and(|d| crate::archive::is_inside(&d.to_string_lossy()));
    match (src_in, dst_in, spec.kind) {
        (true, true, _) => return crate::archive::between(spec, host),
        (true, false, OpKind::Copy | OpKind::Move) => return crate::archive::extract(spec, host),
        (true, false, _) => return crate::archive::delete(spec, host), // no recycle bin inside archives
        (false, true, _) => return crate::archive::add(spec, host),
        _ => {}
    }
    let mut e = Engine {
        host,
        p: Progress::default(),
        conflict_all: None,
        skip_errors: false,
        out: Outcome::default(),
    };
    let r = match spec.kind {
        OpKind::Copy | OpKind::Move => e.transfer(spec),
        OpKind::Trash => e.trash(&spec.sources),
        OpKind::Delete => e.delete(&spec.sources),
        OpKind::Pack => unreachable!("handled above"),
    };
    e.out.cancelled = r.is_err();
    e.host.progress(&e.p);
    e.out
}

struct Engine<'a> {
    host: &'a dyn Host,
    p: Progress,
    conflict_all: Option<ConflictAction>,
    skip_errors: bool,
    out: Outcome,
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// `name.ext` -> `name (2).ext`, first free number.
pub fn unique_name(target: &Path) -> PathBuf {
    let parent = target.parent().unwrap_or(Path::new(""));
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = target.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..)
        .map(|n| parent.join(format!("{stem} ({n}){ext}")))
        .find(|p| fs::symlink_metadata(p).is_err())
        .unwrap()
}

fn remove_file_force(p: &Path) -> io::Result<()> {
    match fs::remove_file(p) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            win::clear_readonly(p)?;
            fs::remove_file(p)
        }
        r => r,
    }
}

impl Engine<'_> {
    fn check(&self) -> Step<()> {
        if self.host.cancelled() { Err(Stop) } else { Ok(()) }
    }

    fn fail(&mut self, path: &Path, message: String) {
        self.out.errors.push(OpError { path: path.display().to_string(), message });
        self.out.skipped += 1;
    }

    /// Runs `f`, asking the host what to do on errors.
    /// Ok(None) = skipped (already recorded), Err(Stop) = cancel the job.
    fn attempt<T>(&mut self, path: &Path, mut f: impl FnMut() -> io::Result<T>) -> Step<Option<T>> {
        loop {
            let err = match f() {
                Ok(v) => return Ok(Some(v)),
                Err(e) => e,
            };
            if err.raw_os_error() == Some(win::ERROR_REQUEST_ABORTED) || self.host.cancelled() {
                return Err(Stop);
            }
            let info = OpError { path: path.display().to_string(), message: err.to_string() };
            let action = if self.skip_errors { ErrorAction::Skip } else { self.host.error(&info) };
            match action {
                ErrorAction::Retry => continue,
                ErrorAction::Cancel => return Err(Stop),
                ErrorAction::SkipAll | ErrorAction::Skip => {
                    self.skip_errors |= action == ErrorAction::SkipAll;
                    self.out.errors.push(info);
                    self.out.skipped += 1;
                    return Ok(None);
                }
            }
        }
    }

    /// Pre-order walk (dir before its children). Does not follow junctions/symlinks.
    fn scan(&mut self, src: &Path, rel: PathBuf, items: &mut Vec<Item>) -> Step<()> {
        self.check()?;
        let Some(md) = self.attempt(src, || fs::symlink_metadata(src))? else {
            return Ok(());
        };
        let attrs = md.file_attributes();
        let dir = attrs & ATTR_DIR != 0;
        // Only real links; cloud placeholders (OneDrive) are plain items.
        let reparse = win::is_link(&md);
        let size = if dir { 0 } else { md.file_size() };
        items.push(Item {
            src: src.to_path_buf(),
            rel: rel.clone(),
            dir,
            reparse,
            size,
            mtime: filetime_to_unix_ms(md.last_write_time()),
        });
        if !dir {
            self.p.total_files += 1;
            self.p.total_bytes += size;
            return Ok(());
        }
        if reparse {
            return Ok(());
        }
        self.p.current = src.display().to_string();
        self.host.progress(&self.p);
        let Some(rd) = self.attempt(src, || fs::read_dir(src))? else {
            return Ok(());
        };
        // An unreadable entry is reported, never silently left out of a copy.
        let mut names = Vec::new();
        for e in rd {
            match e {
                Ok(e) => names.push(e.file_name()),
                Err(err) => self.fail(src, err.to_string()),
            }
        }
        for name in names {
            self.scan(&src.join(&name), rel.join(&name), items)?;
        }
        Ok(())
    }

    fn transfer(&mut self, spec: &OpSpec) -> Step<()> {
        let is_move = spec.kind == OpKind::Move;
        let Some(dest) = spec.dest.as_deref() else {
            self.fail(Path::new(""), crate::i18n::tr("Brak katalogu docelowego", "No destination directory").into());
            return Ok(());
        };
        let mut dest_is_dir = dest.is_dir();
        // Several sources into a folder that doesn't exist yet: create it
        // (otherwise every file would fail on its own).
        if !dest_is_dir && spec.sources.len() > 1 && fs::symlink_metadata(dest).is_err() {
            if let Err(e) = fs::create_dir_all(dest) {
                self.fail(dest, e.to_string());
                return Ok(());
            }
            dest_is_dir = true;
        }

        // 1. Plan: source -> target root.
        let mut plan = Vec::new();
        for s in &spec.sources {
            let target = if spec.sources.len() == 1 && !dest_is_dir {
                dest.to_path_buf()
            } else if let Some(name) = s.file_name() {
                dest.join(name)
            } else {
                self.fail(s, crate::i18n::tr("Nie można skopiować całego dysku", "Cannot copy a whole drive").into());
                continue;
            };
            let (ns, nt) = (norm(s), norm(&target));
            if ns == nt {
                if !is_move {
                    self.fail(s, crate::i18n::tr("Nie można skopiować pliku na samego siebie", "Cannot copy a file onto itself").into());
                }
                continue;
            }
            if nt.starts_with(&(ns.clone() + "\\")) {
                self.fail(s, crate::i18n::tr("Nie można skopiować katalogu do jego własnego podkatalogu", "Cannot copy a directory into its own subdirectory").into());
                continue;
            }
            plan.push((s.clone(), target));
        }

        // 2. Move fast path: whole item renamed in one call (same volume, free target).
        let mut rest = Vec::new();
        for (s, t) in plan {
            if is_move && fs::symlink_metadata(&t).is_err() && win::move_path(&s, &t, false).is_ok() {
                self.out.done_files += 1;
                continue;
            }
            rest.push((s, t));
        }
        if rest.is_empty() {
            return Ok(());
        }

        // 3. Scan for totals.
        let mut groups = Vec::new();
        for (s, t) in rest {
            let mut items = Vec::new();
            self.scan(&s, PathBuf::new(), &mut items)?;
            groups.push((t, items));
        }

        // 4. Run.
        self.p.phase = Phase::Run;
        self.host.progress(&self.p);
        for (root, items) in &groups {
            for it in items {
                let target = if it.rel.as_os_str().is_empty() { root.clone() } else { root.join(&it.rel) };
                if it.dir {
                    self.transfer_dir(it, &target, is_move)?;
                } else {
                    self.transfer_file(it, target, is_move)?;
                }
            }
            if is_move {
                // Children first; dirs that still hold skipped files stay.
                for it in items.iter().rev().filter(|i| i.dir && !i.reparse) {
                    let _ = fs::remove_dir(&it.src);
                }
            }
        }
        Ok(())
    }

    fn transfer_dir(&mut self, it: &Item, target: &Path, is_move: bool) -> Step<()> {
        self.check()?;
        if it.reparse {
            if is_move {
                self.attempt(&it.src, || win::move_path(&it.src, target, false))?;
            } else {
                self.fail(&it.src, crate::i18n::tr("Pominięto dowiązanie (junction/symlink)", "Skipped a link (junction/symlink)").into());
            }
            return Ok(());
        }
        self.attempt(target, || fs::create_dir_all(target))?;
        Ok(())
    }

    fn transfer_file(&mut self, it: &Item, mut target: PathBuf, is_move: bool) -> Step<()> {
        self.check()?;
        self.p.current = it.src.display().to_string();
        let mut overwrite = false;

        if let Ok(tmd) = fs::symlink_metadata(&target) {
            if tmd.file_attributes() & ATTR_DIR != 0 {
                self.fail(&target, crate::i18n::tr("W miejscu docelowym istnieje katalog o tej nazwie", "A directory with this name exists at the destination").into());
                return self.skip_file(it);
            }
            match self.resolve_conflict(it, &target, &tmd)? {
                ConflictAction::Skip => return self.skip_file(it),
                ConflictAction::Rename => target = unique_name(&target),
                _ => {
                    overwrite = true;
                    let _ = win::clear_readonly(&target);
                }
            }
        }

        if is_move {
            let moved = self.attempt(&it.src, || match win::move_path(&it.src, &target, overwrite) {
                Ok(()) => Ok(true),
                Err(e) if e.raw_os_error() == Some(win::ERROR_NOT_SAME_DEVICE) => Ok(false),
                Err(e) => Err(e),
            })?;
            match moved {
                Some(true) => return self.file_done(it.size),
                None => return self.skip_bytes(it),
                Some(false) => {} // other volume: copy + delete below
            }
        }

        let host = self.host;
        let base = self.p.done_bytes;
        let mut snap = self.p.clone();
        // Overwriting: copy next to the target and swap it in at the end —
        // a cancel or error halfway leaves the old version intact (CopyFileExW
        // straight onto it would delete it). Also works for hidden / system
        // targets, which CopyFileExW refuses to overwrite.
        let part = overwrite.then(|| win::part_path(&target));
        let copied = self.attempt(&it.src, || {
            let to = part.as_deref().unwrap_or(&target);
            win::copy_file(&it.src, to, overwrite, &mut |b| {
                snap.done_bytes = base + b;
                host.progress(&snap);
                !host.cancelled()
            })?;
            if let Some(p) = &part
                && let Err(e) = win::move_path(p, &target, true)
            {
                let _ = fs::remove_file(p);
                return Err(e);
            }
            Ok(())
        });
        if let Some(p) = &part
            && !matches!(copied, Ok(Some(())))
        {
            let _ = fs::remove_file(p);
        }
        if copied?.is_none() {
            return self.skip_bytes(it);
        }
        if is_move {
            // Source goes away only after a successful copy.
            self.attempt(&it.src, || remove_file_force(&it.src))?;
        }
        self.file_done(it.size)
    }

    fn resolve_conflict(&mut self, it: &Item, target: &Path, tmd: &Metadata) -> Step<ConflictAction> {
        let target_mtime = filetime_to_unix_ms(tmd.last_write_time());
        let action = match self.conflict_all {
            Some(a) => a,
            None => {
                let ans = self.host.conflict(&Conflict {
                    source: it.src.display().to_string(),
                    target: target.display().to_string(),
                    source_size: it.size,
                    source_mtime: it.mtime,
                    target_size: tmd.file_size(),
                    target_mtime,
                    can_resume: false,
                });
                if ans.all {
                    self.conflict_all = Some(ans.action);
                }
                ans.action
            }
        };
        Ok(match action {
            ConflictAction::Cancel => return Err(Stop),
            ConflictAction::OverwriteOlder if target_mtime < it.mtime => ConflictAction::Overwrite,
            ConflictAction::OverwriteOlder => ConflictAction::Skip,
            ConflictAction::Resume => ConflictAction::Overwrite,
            a => a,
        })
    }

    fn file_done(&mut self, size: u64) -> Step<()> {
        self.out.done_files += 1;
        self.p.done_files += 1;
        self.p.done_bytes += size;
        self.host.progress(&self.p);
        Ok(())
    }

    /// File not transferred (error already recorded): advance progress only.
    fn skip_bytes(&mut self, it: &Item) -> Step<()> {
        self.p.done_files += 1;
        self.p.done_bytes += it.size;
        self.host.progress(&self.p);
        Ok(())
    }

    /// Skipped by a conflict decision.
    fn skip_file(&mut self, it: &Item) -> Step<()> {
        self.out.skipped += 1;
        self.skip_bytes(it)
    }

    fn trash(&mut self, sources: &[PathBuf]) -> Step<()> {
        self.p.phase = Phase::Run;
        self.p.total_files = sources.len() as u64;
        for s in sources {
            self.check()?;
            self.p.current = s.display().to_string();
            self.host.progress(&self.p);
            let r = self.attempt(s, || trash::delete(s).map_err(|e| io::Error::other(e.to_string())))?;
            if r.is_some() {
                self.out.done_files += 1;
            }
            self.p.done_files += 1;
        }
        Ok(())
    }

    fn delete(&mut self, sources: &[PathBuf]) -> Step<()> {
        let mut items = Vec::new();
        for s in sources {
            self.scan(s, PathBuf::new(), &mut items)?;
        }
        self.p.phase = Phase::Run;
        self.p.total_bytes = 0;
        self.p.total_files = items.len() as u64;
        for it in items.iter().filter(|i| !i.dir) {
            self.check()?;
            self.p.current = it.src.display().to_string();
            if self.attempt(&it.src, || remove_file_force(&it.src))?.is_some() {
                self.out.done_files += 1;
            }
            self.p.done_files += 1;
            self.host.progress(&self.p);
        }
        // Deepest first. remove_dir on a junction removes the link, not its target.
        for it in items.iter().rev().filter(|i| i.dir) {
            self.check()?;
            self.p.current = it.src.display().to_string();
            if self.attempt(&it.src, || fs::remove_dir(&it.src))?.is_some() {
                self.out.done_files += 1;
            }
            self.p.done_files += 1;
            self.host.progress(&self.p);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct TestHost {
        conflicts: Mutex<Vec<ConflictAnswer>>,
        asked: Mutex<Vec<Conflict>>,
        on_error: ErrorAction,
        cancel: AtomicBool,
    }

    impl TestHost {
        fn new(answers: Vec<ConflictAnswer>) -> Self {
            Self {
                conflicts: Mutex::new(answers),
                asked: Mutex::new(vec![]),
                on_error: ErrorAction::Skip,
                cancel: AtomicBool::new(false),
            }
        }
    }

    impl Host for TestHost {
        fn progress(&self, _: &Progress) {}
        fn conflict(&self, c: &Conflict) -> ConflictAnswer {
            self.asked.lock().unwrap().push(c.clone());
            self.conflicts.lock().unwrap().remove(0)
        }
        fn error(&self, _: &OpError) -> ErrorAction {
            self.on_error
        }
        fn cancelled(&self) -> bool {
            self.cancel.load(Ordering::Relaxed)
        }
    }

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("twinyard-ops-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn write(&self, rel: &str, data: &str) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, data).unwrap();
            p
        }
        fn read(&self, rel: &str) -> String {
            fs::read_to_string(self.0.join(rel)).unwrap()
        }
        fn exists(&self, rel: &str) -> bool {
            self.0.join(rel).exists()
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn spec(kind: OpKind, sources: Vec<PathBuf>, dest: Option<PathBuf>) -> OpSpec {
        OpSpec { kind, sources, dest }
    }

    #[test]
    fn copies_tree() {
        let t = Tmp::new("copy");
        t.write("src/a.txt", "A");
        t.write("src/sub/b.txt", "BB");
        fs::create_dir_all(t.0.join("dst")).unwrap();
        let host = TestHost::new(vec![]);
        let out = run(&spec(OpKind::Copy, vec![t.0.join("src")], Some(t.0.join("dst"))), &host);
        assert!(!out.cancelled && out.errors.is_empty(), "{out:?}");
        assert_eq!(t.read("dst/src/a.txt"), "A");
        assert_eq!(t.read("dst/src/sub/b.txt"), "BB");
        assert!(t.exists("src/a.txt"));
        assert_eq!(out.done_files, 2);
    }

    #[test]
    fn single_file_to_new_name() {
        let t = Tmp::new("rename");
        let a = t.write("a.txt", "A");
        run(&spec(OpKind::Copy, vec![a], Some(t.0.join("b.txt"))), &TestHost::new(vec![]));
        assert_eq!(t.read("b.txt"), "A");
    }

    #[test]
    fn conflicts_skip_overwrite_rename_and_all() {
        let t = Tmp::new("conflict");
        for n in ["1", "2", "3", "4"] {
            t.write(&format!("src/{n}.txt"), "new");
            t.write(&format!("dst/{n}.txt"), "old");
        }
        let host = TestHost::new(vec![
            ConflictAnswer { action: ConflictAction::Skip, all: false },
            ConflictAnswer { action: ConflictAction::Rename, all: false },
            ConflictAnswer { action: ConflictAction::Overwrite, all: true },
        ]);
        let sources = ["1", "2", "3", "4"].map(|n| t.0.join(format!("src/{n}.txt"))).to_vec();
        let out = run(&spec(OpKind::Copy, sources, Some(t.0.join("dst"))), &host);
        assert_eq!(host.asked.lock().unwrap().len(), 3, "4th must use the 'all' answer");
        assert_eq!(t.read("dst/1.txt"), "old");
        assert_eq!(t.read("dst/2.txt"), "old");
        assert_eq!(t.read("dst/2 (2).txt"), "new");
        assert_eq!(t.read("dst/3.txt"), "new");
        assert_eq!(t.read("dst/4.txt"), "new");
        assert_eq!(out.skipped, 1);
    }

    #[test]
    fn overwrite_readonly_target() {
        let t = Tmp::new("ro");
        let a = t.write("src/a.txt", "new");
        let b = t.write("dst/a.txt", "old");
        let mut perm = fs::metadata(&b).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&b, perm).unwrap();
        let host = TestHost::new(vec![ConflictAnswer { action: ConflictAction::Overwrite, all: false }]);
        let out = run(&spec(OpKind::Copy, vec![a], Some(t.0.join("dst"))), &host);
        assert!(out.errors.is_empty(), "{out:?}");
        assert_eq!(t.read("dst/a.txt"), "new");
    }

    #[test]
    fn overwrite_hidden_target_leaves_no_part_file() {
        let t = Tmp::new("hidden");
        let a = t.write("src/a.txt", "new");
        let b = t.write("dst/a.txt", "old");
        let st = std::process::Command::new("attrib").arg("+h").arg(&b).status().unwrap();
        assert!(st.success());
        let host = TestHost::new(vec![ConflictAnswer { action: ConflictAction::Overwrite, all: false }]);
        let out = run(&spec(OpKind::Copy, vec![a], Some(t.0.join("dst"))), &host);
        assert!(out.errors.is_empty(), "{out:?}");
        assert_eq!(t.read("dst/a.txt"), "new");
        assert!(!t.exists("dst/a.txt.twinyard-part"));
    }

    #[test]
    fn refuses_copy_into_itself() {
        let t = Tmp::new("self");
        t.write("src/a.txt", "A");
        let out = run(&spec(OpKind::Copy, vec![t.0.join("src")], Some(t.0.join("src/inner"))), &TestHost::new(vec![]));
        assert_eq!(out.errors.len(), 1);
        assert!(!t.exists("src/inner"));
    }

    #[test]
    fn moves_and_merges_into_existing_dir() {
        let t = Tmp::new("move");
        t.write("src/d/a.txt", "A");
        t.write("dst/d/keep.txt", "K");
        let out = run(&spec(OpKind::Move, vec![t.0.join("src/d")], Some(t.0.join("dst"))), &TestHost::new(vec![]));
        assert!(out.errors.is_empty(), "{out:?}");
        assert_eq!(t.read("dst/d/a.txt"), "A");
        assert_eq!(t.read("dst/d/keep.txt"), "K");
        assert!(!t.exists("src/d"), "emptied source dir is removed");
    }

    #[test]
    fn move_fast_path_renames_whole_dir() {
        let t = Tmp::new("movefast");
        t.write("src/d/x/y.txt", "Y");
        fs::create_dir_all(t.0.join("dst")).unwrap();
        let out = run(&spec(OpKind::Move, vec![t.0.join("src/d")], Some(t.0.join("dst"))), &TestHost::new(vec![]));
        assert_eq!(out.done_files, 1);
        assert_eq!(t.read("dst/d/x/y.txt"), "Y");
    }

    #[test]
    fn deletes_permanently_including_readonly() {
        let t = Tmp::new("del");
        let f = t.write("d/sub/ro.txt", "R");
        let mut perm = fs::metadata(&f).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&f, perm).unwrap();
        t.write("d/b.txt", "B");
        let out = run(&spec(OpKind::Delete, vec![t.0.join("d")], None), &TestHost::new(vec![]));
        assert!(out.errors.is_empty(), "{out:?}");
        assert!(!t.exists("d"));
    }

    #[test]
    fn cancel_stops_job() {
        let t = Tmp::new("cancel");
        t.write("src/a.txt", "A");
        let host = TestHost::new(vec![]);
        host.cancel.store(true, Ordering::Relaxed);
        let out = run(&spec(OpKind::Copy, vec![t.0.join("src")], Some(t.0.join("dst"))), &host);
        assert!(out.cancelled);
        assert!(!t.exists("dst/src/a.txt"));
    }

    #[test]
    fn copies_and_moves_beyond_max_path() {
        let t = Tmp::new("long");
        let deep = (0..6).map(|i| format!("{i}{}", "x".repeat(45))).collect::<Vec<_>>().join("\\");
        let src = t.write(&format!("{deep}\\file.txt"), "L");
        assert!(src.as_os_str().len() > 280);
        let out = run(&spec(OpKind::Copy, vec![src.clone()], Some(t.0.join("copy.txt"))), &TestHost::new(vec![]));
        assert!(out.errors.is_empty(), "{out:?}");
        assert_eq!(t.read("copy.txt"), "L");
        let back = src.with_file_name("moved.txt");
        let out = run(&spec(OpKind::Move, vec![t.0.join("copy.txt")], Some(back.clone())), &TestHost::new(vec![]));
        assert!(out.errors.is_empty(), "{out:?}");
        assert_eq!(fs::read_to_string(&back).unwrap(), "L");
    }

    #[test]
    fn unique_name_counts_up() {
        let t = Tmp::new("uniq");
        t.write("a.txt", "");
        t.write("a (2).txt", "");
        assert_eq!(unique_name(&t.0.join("a.txt")), t.0.join("a (3).txt"));
    }
}
