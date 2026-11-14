//! F4 on a remote file (or one inside a ZIP): copy to %TEMP%, watch the local
//! copy, and when the editor saves it tell the UI (which asks and puts it back:
//! upload to the server / update the archive).

use crate::blocking;
use twinyard_core::ops::{Conflict, ConflictAction, ConflictAnswer, ErrorAction, Host, OpError, OpKind, OpSpec, Progress};
use twinyard_remote::Remote;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, State};

struct Watch {
    remote: String,
    stamp: (Option<SystemTime>, u64),
    /// A change seen once; reported when the next poll sees the same stamp
    /// (the editor finished writing — never upload half a save).
    settling: Option<(Option<SystemTime>, u64)>,
    /// Upload on every save without asking.
    auto: bool,
}

#[derive(Default)]
pub struct Edits {
    files: Mutex<HashMap<PathBuf, Watch>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Changed {
    local: String,
    remote: String,
    auto: bool,
}

fn stamp(p: &PathBuf) -> (Option<SystemTime>, u64) {
    std::fs::metadata(p).map(|m| (m.modified().ok(), m.len())).unwrap_or((None, 0))
}

/// Polls the watched copies (a handful of files; 1 s is plenty and needs no
/// directory handles that would block deleting the temp dir).
pub fn start(app: AppHandle, edits: Arc<Edits>) {
    std::thread::Builder::new()
        .name("edit-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let mut changed = Vec::new();
                {
                    let mut files = edits.files.lock().unwrap();
                    for (local, w) in files.iter_mut() {
                        let now = stamp(local);
                        if now.0.is_none() || now == w.stamp {
                            w.settling = None;
                        } else if w.settling == Some(now) {
                            w.stamp = now;
                            w.settling = None;
                            changed.push(Changed { local: local.display().to_string(), remote: w.remote.clone(), auto: w.auto });
                        } else {
                            w.settling = Some(now);
                        }
                    }
                }
                for c in changed {
                    let _ = app.emit("remote-edit-changed", c);
                }
            }
        })
        .ok();
}

/// Downloads `remote` and starts watching the copy. Returns the local path.
#[tauri::command]
/// `local`: the copy was already downloaded (a big file, fetched as a job
/// with progress — see `remote_temp_path`); just start watching it.
pub async fn edit_remote(
    remote: State<'_, Arc<Remote>>,
    edits: State<'_, Arc<Edits>>,
    path: String,
    local: Option<String>,
) -> Result<String, String> {
    let (r, e) = (remote.inner().clone(), edits.inner().clone());
    blocking(move || {
        // Already being edited: the same copy again. Downloading over it would
        // throw away changes not sent yet (and look like a save to the watcher).
        if let Some((l, _)) = e.files.lock().unwrap().iter().find(|(l, w)| w.remote == path && l.exists()) {
            if let Some(big) = local {
                let _ = std::fs::remove_file(big); // fetched for nothing
            }
            return Ok(l.display().to_string());
        }
        let local = if twinyard_core::archive::is_inside(&path) {
            twinyard_core::archive::extract_to_temp_in(&path, "~edit-zip").map_err(|e| format!("{path}: {e}"))?
        } else {
            // F4 copies live apart from F3/Enter copies (%TEMP%\twinyard\~edit).
            let dest = twinyard_remote::manager::temp_path_in(&path, Some("~edit"))?;
            // `Index.php` and `index.php` (or `a:b` and `a_b`) map to one local
            // copy: a save must never go to the other file.
            if let Some(w) = e.files.lock().unwrap().get(&dest)
                && w.remote != path
                && dest.exists()
            {
                if let Some(big) = local {
                    let _ = std::fs::remove_file(big);
                }
                return Err(twinyard_core::trf!(
                    "Ta sama kopia lokalna jest już edytowana dla {} (nazwy różnią się tylko wielkością liter lub znakami niedozwolonymi w Windows). Skopiuj ten plik na dysk (F5) i edytuj tam.",
                    "The same local copy is already being edited for {} (the names differ only in letter case or in characters Windows forbids). Copy this file to a local disk (F5) and edit it there.",
                    w.remote
                ));
            }
            match local {
                Some(big) => {
                    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
                    twinyard_core::win::move_path(std::path::Path::new(&big), &dest, true).map_err(|e| format!("{}: {e}", dest.display()))?;
                }
                None => r.fetch_to(&path, &dest)?,
            }
            dest
        };
        watch(&e, local.clone(), path);
        Ok(local.display().to_string())
    })
    .await
}

fn watch(e: &Edits, local: PathBuf, remote: String) {
    let st = stamp(&local);
    let mut files = e.files.lock().unwrap();
    let auto = files.get(&local).is_some_and(|w| w.auto);
    files.insert(local, Watch { remote, stamp: st, settling: None, auto });
}

#[tauri::command]
pub fn edit_set_auto(edits: State<'_, Arc<Edits>>, local: String, auto: bool) {
    if let Some(w) = edits.files.lock().unwrap().get_mut(&PathBuf::from(local)) {
        w.auto = auto;
    }
}

/// Overwrites (or, for a new file, skips) without asking; errors are
/// collected (after the automatic retries).
struct Silent {
    errors: Mutex<Vec<String>>,
    overwrite: bool,
}

impl Host for Silent {
    fn progress(&self, _: &Progress) {}
    fn conflict(&self, _: &Conflict) -> ConflictAnswer {
        let action = if self.overwrite { ConflictAction::Overwrite } else { ConflictAction::Skip };
        ConflictAnswer { action, all: true }
    }
    fn error(&self, e: &OpError) -> ErrorAction {
        self.errors.lock().unwrap().push(e.message.clone());
        ErrorAction::Cancel
    }
    fn cancelled(&self) -> bool {
        false
    }
}

/// Uploads the edited copy over the remote file.
#[tauri::command]
pub async fn edit_upload(remote: State<'_, Arc<Remote>>, local: String, remote_path: String) -> Result<(), String> {
    let r = remote.inner().clone();
    blocking(move || {
        let host = Silent { errors: Mutex::new(vec![]), overwrite: true };
        let spec = OpSpec { kind: OpKind::Copy, sources: vec![PathBuf::from(&local)], dest: Some(PathBuf::from(&remote_path)) };
        let out = if twinyard_core::archive::is_inside(&remote_path) {
            twinyard_core::ops::run(&spec, &host) // rewrites the ZIP with the new copy
        } else {
            twinyard_remote::engine::run(&r, &spec, &host)
        };
        let mut errs: Vec<String> = out.errors.into_iter().map(|e| e.message).collect();
        errs.extend(host.errors.into_inner().unwrap());
        if errs.is_empty() && out.done_files == 1 { Ok(()) } else { Err(errs.join("; ")) }
    })
    .await
}

/// Local copy path for a big remote file the UI downloads as a normal job
/// (progress, cancel, retries, resume). Any older copy is removed first, so
/// the job never asks about overwriting it.
#[tauri::command]
pub async fn remote_temp_path(path: String) -> Result<String, String> {
    blocking(move || {
        let local = twinyard_remote::manager::temp_path(&path)?;
        std::fs::create_dir_all(local.parent().unwrap()).map_err(|e| e.to_string())?;
        match std::fs::remove_file(&local) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("{}: {e}", local.display())),
            _ => {}
        }
        Ok(local.display().to_string())
    })
    .await
}

/// Shift+F4 on FTP: create an empty file on the server, then edit it like F4.
#[tauri::command]
pub async fn edit_new_remote(remote: State<'_, Arc<Remote>>, edits: State<'_, Arc<Edits>>, path: String) -> Result<String, String> {
    let (r, e) = (remote.inner().clone(), edits.inner().clone());
    blocking(move || {
        let name = twinyard_remote::path::file_name(&path).to_string();
        let empty = std::env::temp_dir().join("twinyard").join("new").join(&name);
        std::fs::create_dir_all(empty.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&empty, b"").map_err(|e| e.to_string())?;
        // The panel's list may be old: the engine checks the server itself,
        // and a file that turns out to exist is skipped (never emptied) —
        // then it is simply edited like F4.
        let host = Silent { errors: Mutex::new(vec![]), overwrite: false };
        let spec = OpSpec { kind: OpKind::Copy, sources: vec![empty.clone()], dest: Some(PathBuf::from(&path)) };
        let out = twinyard_remote::engine::run(&r, &spec, &host);
        let _ = std::fs::remove_file(&empty);
        let mut errs: Vec<String> = out.errors.into_iter().map(|e| e.message).collect();
        errs.extend(host.errors.into_inner().unwrap());
        if !errs.is_empty() {
            return Err(errs.join("; "));
        }
        let local = twinyard_remote::manager::temp_path_in(&path, Some("~edit"))?;
        r.fetch_to(&path, &local)?;
        watch(&e, local.clone(), path);
        Ok(local.display().to_string())
    })
    .await
}
