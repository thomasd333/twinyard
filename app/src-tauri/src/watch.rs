//! Auto-refresh: watches the directories shown in the panels (non-recursive,
//! ReadDirectoryChangesW via `notify`) and tells the UI which one changed.
//! Bursts (unzip, build) are coalesced: one event per dir per 300 ms.

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

const DEBOUNCE: Duration = Duration::from_millis(300);

pub struct DirWatch {
    watcher: Mutex<Option<RecommendedWatcher>>,
    watched: Mutex<HashSet<PathBuf>>,
    /// Wanted set -> worker thread. (Un)watching a dead network share can
    /// block for a long time; that must never happen on the UI thread.
    wanted: Mutex<mpsc::Sender<HashSet<PathBuf>>>,
}

fn key(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches('\\').to_lowercase()
}

impl DirWatch {
    pub fn new(app: AppHandle) -> Arc<DirWatch> {
        let (tx, rx) = mpsc::channel::<PathBuf>();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                for p in ev.paths {
                    // Event paths are the changed entries; their parent is the panel dir.
                    if let Some(parent) = p.parent() {
                        let _ = tx.send(parent.to_path_buf());
                    }
                    let _ = tx.send(p); // the dir itself (e.g. it was deleted/renamed)
                }
            }
        })
        .ok();
        let (wtx, wrx) = mpsc::channel::<HashSet<PathBuf>>();
        let dw = Arc::new(DirWatch { watcher: Mutex::new(watcher), watched: Mutex::new(HashSet::new()), wanted: Mutex::new(wtx) });

        let weak_w = Arc::downgrade(&dw);
        std::thread::Builder::new()
            .name("dir-watch-set".into())
            .spawn(move || {
                while let Ok(mut want) = wrx.recv() {
                    while let Ok(newer) = wrx.try_recv() {
                        want = newer; // only the latest set matters
                    }
                    let Some(dw) = weak_w.upgrade() else { return };
                    apply(&dw, want);
                }
            })
            .ok();

        let weak = Arc::downgrade(&dw);
        std::thread::Builder::new()
            .name("dir-watch".into())
            .spawn(move || {
                let mut pending: HashMap<String, (String, Instant)> = HashMap::new();
                loop {
                    let msg = rx.recv_timeout(Duration::from_millis(100));
                    let Some(dw) = weak.upgrade() else { return };
                    if let Ok(p) = msg {
                        let watched = dw.watched.lock().unwrap();
                        if let Some(w) = watched.iter().find(|w| key(w) == key(&p)) {
                            pending.entry(key(w)).or_insert_with(|| (w.display().to_string(), Instant::now()));
                        }
                        continue;
                    }
                    let due: Vec<String> = pending.iter().filter(|(_, (_, t))| t.elapsed() >= DEBOUNCE).map(|(k, _)| k.clone()).collect();
                    for k in due {
                        if let Some((path, _)) = pending.remove(&k) {
                            let _ = app.emit("dir-changed", path);
                        }
                    }
                }
            })
            .ok();
        dw
    }
}

/// The UI sends the full set of visible local dirs whenever it changes;
/// returns at once, the (possibly slow) work happens on the worker thread.
#[tauri::command]
pub fn watch_dirs(state: State<'_, Arc<DirWatch>>, paths: Vec<String>) {
    // No filesystem access here (not even is_dir): this runs on the UI thread.
    let want: HashSet<PathBuf> = paths
        .into_iter()
        .filter(|p| !twinyard_remote::is_remote(p) && !p.starts_with("::") && twinyard_core::unc::server_only(p).is_none())
        .map(PathBuf::from)
        .collect();
    let _ = state.wanted.lock().unwrap().send(want);
}

fn apply(state: &DirWatch, want: HashSet<PathBuf>) {
    // ZIP insides can't be watched (checking this touches the disk: worker only).
    let want: HashSet<PathBuf> = want.into_iter().filter(|p| twinyard_core::archive::split(&p.to_string_lossy()).is_none()).collect();
    let mut watched = state.watched.lock().unwrap();
    let mut guard = state.watcher.lock().unwrap();
    let Some(w) = guard.as_mut() else { return };
    for old in watched.difference(&want) {
        let _ = w.unwatch(old);
    }
    for new in want.difference(&watched) {
        // Fails for vanished dirs / some network shares — then no auto-refresh there.
        let _ = w.watch(new, RecursiveMode::NonRecursive);
    }
    *watched = want;
}
