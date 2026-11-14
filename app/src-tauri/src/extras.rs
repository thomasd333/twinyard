//! Smaller panel features: free space, directory sizes (Space), the
//! Windows clipboard for files and text.

use crate::blocking;
use twinyard_core::listing::{ATTR_DIR, ATTR_REPARSE};
use twinyard_remote::{Remote, is_remote};
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::State;

#[derive(serde::Serialize)]
pub struct Space {
    free: u64,
    total: u64,
}

/// Free / total space of the volume of a local (or UNC) path. None for FTP,
/// archives and paths Windows can't answer for.
#[tauri::command]
pub async fn disk_space(path: String) -> Result<Option<Space>, String> {
    if is_remote(&path) || path.starts_with("::") {
        return Ok(None);
    }
    blocking(move || {
        // Inside an archive: the volume of the archive file.
        let dir = twinyard_core::archive::split(&path).map_or(path.clone(), |(ar, _)| ar.parent().map(|p| p.display().to_string()).unwrap_or_default());
        Ok(twinyard_core::drives::disk_space(&dir).map(|(free, total)| Space { free, total }))
    })
    .await
}

/// Generation of the running size calculation: a newer request (or a
/// cancel) makes older walks stop early.
#[derive(Default)]
pub struct SizeJobs(AtomicU64);

#[derive(serde::Serialize)]
pub struct DirSize {
    bytes: u64,
    files: u64,
    dirs: u64,
    /// Stopped early (cancelled) — the numbers are a lower bound.
    partial: bool,
}

/// Space on a folder: total size of its files (local and FTP; links are not
/// followed; unreadable subfolders are skipped).
#[tauri::command]
pub async fn dir_size(remote: State<'_, Arc<Remote>>, jobs: State<'_, Arc<SizeJobs>>, path: String) -> Result<DirSize, String> {
    let (r, j) = (remote.inner().clone(), jobs.inner().clone());
    let gen_ = j.0.fetch_add(1, Ordering::Relaxed) + 1;
    blocking(move || {
        let alive = || j.0.load(Ordering::Relaxed) == gen_;
        let mut s = DirSize { bytes: 0, files: 0, dirs: 0, partial: false };
        if is_remote(&path) {
            remote_walk(&r, &path, &mut s, &alive);
        } else if twinyard_core::archive::split(&path).is_some() {
            archive_walk(&path, &mut s)?;
        } else {
            local_walk(Path::new(&path), &mut s, &alive);
        }
        Ok(s)
    })
    .await
}

/// Stops a running `dir_size` (Esc, leaving the folder).
#[tauri::command]
pub fn dir_size_cancel(jobs: State<'_, Arc<SizeJobs>>) {
    jobs.0.fetch_add(1, Ordering::Relaxed);
}

fn local_walk(dir: &Path, s: &mut DirSize, alive: &dyn Fn() -> bool) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if !alive() {
            s.partial = true;
            return;
        }
        let Ok(md) = e.path().symlink_metadata() else { continue };
        let a = md.file_attributes();
        if a & ATTR_DIR != 0 {
            s.dirs += 1;
            if a & ATTR_REPARSE == 0 {
                local_walk(&e.path(), s, alive);
            }
        } else {
            s.files += 1;
            s.bytes += md.file_size();
        }
    }
}

fn remote_walk(r: &Remote, url: &str, s: &mut DirSize, alive: &dyn Fn() -> bool) {
    let Ok(l) = r.list(url) else { return };
    for i in 0..l.names.len() {
        if !alive() {
            s.partial = true;
            return;
        }
        if l.attrs[i] & ATTR_DIR != 0 {
            s.dirs += 1;
            if l.attrs[i] & ATTR_REPARSE != 0 {
                continue; // symlink: not followed (cyclic links would never end)
            }
            remote_walk(r, &format!("{}/{}", url.trim_end_matches('/'), l.names[i]), s, alive);
        } else {
            s.files += 1;
            s.bytes += l.sizes[i];
        }
    }
}

/// Inside an archive the whole entry list is in memory anyway: one pass.
fn archive_walk(path: &str, s: &mut DirSize) -> Result<(), String> {
    fn walk(path: &str, s: &mut DirSize) -> Result<(), String> {
        let l = twinyard_core::archive::list(path).map_err(|e| e.to_string())?;
        for i in 0..l.names.len() {
            if l.attrs[i] & ATTR_DIR != 0 {
                s.dirs += 1;
                walk(&format!("{}\\{}", path.trim_end_matches('\\'), l.names[i]), s)?;
            } else {
                s.files += 1;
                s.bytes += l.sizes[i];
            }
        }
        Ok(())
    }
    walk(path, s)
}

fn hwnd(window: &tauri::WebviewWindow) -> Result<isize, String> {
    Ok(window.hwnd().map_err(|e| e.to_string())?.0 as isize)
}

/// Ctrl+C / Ctrl+X: files to the Windows clipboard (local files only —
/// other programs can't read our FTP / archive paths).
#[tauri::command]
pub fn clipboard_set_files(window: tauri::WebviewWindow, paths: Vec<String>, cut: bool) -> Result<(), String> {
    if paths.iter().any(|p| is_remote(p) || twinyard_core::archive::is_inside(p)) {
        return Err(twinyard_core::i18n::tr(
            "Do schowka można skopiować tylko pliki z dysku (z FTP / archiwum: skopiuj F5)",
            "Only files on a disk can go to the clipboard (from FTP / an archive: copy with F5)",
        )
        .into());
    }
    twinyard_core::clipboard::set_files(hwnd(&window)?, &paths, cut).map_err(|e| e.to_string())
}

/// Paths / names as text (Ctrl+Shift+C, Ctrl+Alt+C).
#[tauri::command]
pub fn clipboard_set_text(window: tauri::WebviewWindow, text: String) -> Result<(), String> {
    twinyard_core::clipboard::set_text(hwnd(&window)?, &text).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct ClipFiles {
    paths: Vec<String>,
    cut: bool,
}

/// Ctrl+V: what is on the clipboard (the UI starts a normal copy/move job).
#[tauri::command]
pub fn clipboard_get_files() -> Result<Option<ClipFiles>, String> {
    let got = twinyard_core::clipboard::get_files().map_err(|e| e.to_string())?;
    Ok(got.map(|(p, cut)| ClipFiles { paths: p.into_iter().map(|p| p.display().to_string()).collect(), cut }))
}

/// After pasting cut files (they are moved now).
#[tauri::command]
pub fn clipboard_clear() {
    twinyard_core::clipboard::clear();
}
