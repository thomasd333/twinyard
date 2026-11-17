mod ctxmenu;
mod dircmp;
mod dnd;
mod edits;
mod extras;
mod filelog;
mod ftp;
pub mod housekeeping;
mod jobs;
mod search;
mod shellicon;
mod watch;
mod winstate;

use twinyard_core::{Drive, Listing};
use twinyard_remote::{Remote, is_remote};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;
use tauri::async_runtime::spawn_blocking;

/// Every filesystem call goes through spawn_blocking: a slow network drive
/// must never freeze the UI or starve the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    spawn_blocking(f).await.map_err(|e| e.to_string())?
}

type RemoteState<'a> = State<'a, Arc<Remote>>;

#[tauri::command]
async fn list_dir(remote: RemoteState<'_>, path: String) -> Result<Listing, String> {
    if is_remote(&path) {
        let r = remote.inner().clone();
        return blocking(move || r.list(&path)).await;
    }
    blocking(move || list_local(&path)).await
}

/// Local dir, or the inside of a ZIP (`C:\x\a.zip\inner`).
pub(crate) fn list_local(path: &str) -> Result<Listing, String> {
    if twinyard_core::archive::split(path).is_some() {
        return twinyard_core::archive::list(path).map_err(|e| format!("{path}: {e}"));
    }
    twinyard_core::list_dir(&PathBuf::from(path)).map_err(|e| format!("{path}: {e}"))
}

#[tauri::command]
fn parent_dir(path: String) -> Option<String> {
    if is_remote(&path) {
        return twinyard_remote::path::parent_url(&path);
    }
    twinyard_core::parent_of(&PathBuf::from(path)).map(|p| p.to_string_lossy().into_owned())
}

/// Remote files are downloaded to %TEMP% first (for tools / opening).
fn local_copy(remote: &Remote, path: &str) -> Result<String, String> {
    if is_remote(path) {
        return remote.fetch_temp(path).map(|p| p.display().to_string());
    }
    if twinyard_core::archive::is_inside(path) {
        return twinyard_core::archive::extract_to_temp(path).map(|p| p.display().to_string()).map_err(|e| format!("{path}: {e}"));
    }
    Ok(path.to_string())
}

#[tauri::command]
async fn list_drives() -> Result<Vec<Drive>, String> {
    blocking(|| Ok(twinyard_core::list_drives())).await
}

/// Interface language ("pl" / "en") of backend messages; the UI owns the setting.
#[tauri::command]
fn set_lang(lang: String) {
    twinyard_core::i18n::set_lang(&lang);
}

#[tauri::command]
fn home_dir() -> String {
    std::env::var("USERPROFILE").unwrap_or_else(|_| r"C:\".into())
}

#[tauri::command]
async fn open_path(remote: RemoteState<'_>, path: String) -> Result<(), String> {
    let r = remote.inner().clone();
    blocking(move || {
        let local = local_copy(&r, &path)?;
        let p = PathBuf::from(&local);
        if twinyard_core::win::shell_open(&p).is_ok() {
            return Ok(());
        }
        // No (working) association. Text-like files -> the configured editor;
        // anything else -> Windows' "Open with…" chooser. Never Windows' error box.
        let cfg = twinyard_core::config::load().map_err(|e| e.to_string())?;
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if cfg.search.text_ext.iter().any(|e| e.eq_ignore_ascii_case(&ext))
            && twinyard_core::config::launch(&cfg.tools.editor, &[("%f", &local)]).is_ok()
        {
            return Ok(());
        }
        twinyard_core::win::open_with_dialog(&p).map_err(|e| e.to_string())
    })
    .await
}

/// F7: creates a directory; nested `a\b\c` is allowed.
#[tauri::command]
async fn create_dir(remote: RemoteState<'_>, path: String) -> Result<(), String> {
    let r = remote.inner().clone();
    blocking(move || {
        if is_remote(&path) {
            return r.mkdir(&path);
        }
        if twinyard_core::archive::is_inside(&path) {
            return twinyard_core::archive::mkdir(&path).map_err(|e| e.to_string());
        }
        if Path::new(&path).exists() {
            return Err(twinyard_core::trf!("Już istnieje: {path}", "Already exists: {path}"));
        }
        std::fs::create_dir_all(&path).map_err(|e| format!("{path}: {e}"))
    })
    .await
}

/// Password for an encrypted archive, typed after a "Wymagane/Złe hasło do
/// archiwum: …" error. Kept in memory only, until the program closes.
#[tauri::command]
fn archive_password(archive: String, password: String) {
    twinyard_core::archive::set_password(&archive, &password);
}

/// Creates an empty file (and missing parent dirs). Returns false when the
/// file already existed — the caller just opens it then (like TC's Shift+F4).
#[tauri::command]
async fn create_file(path: String) -> Result<bool, String> {
    blocking(move || {
        if is_remote(&path) {
            return Err(twinyard_core::i18n::tr("Nowy plik na FTP — jeszcze nie (utwórz lokalnie i skopiuj F5)", "A new file on FTP — not yet (create it locally and copy it with F5)").into());
        }
        if twinyard_core::archive::is_inside(&path) {
            return Err(twinyard_core::i18n::tr("Nowy plik w archiwum — utwórz go obok i skopiuj F5", "A new file in an archive — create it outside and copy it in with F5").into());
        }
        let p = Path::new(&path);
        if p.is_dir() {
            return Err(twinyard_core::trf!("To jest katalog: {path}", "This is a directory: {path}"));
        }
        if p.exists() {
            return Ok(false);
        }
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(p)
            .map(|_| true)
            .map_err(|e| format!("{path}: {e}"))
    })
    .await
}

#[derive(serde::Serialize)]
struct Resolved {
    dir: String,
    focus: Option<String>,
}

/// Turns what the user typed into the path bar into a directory to open:
/// quotes stripped, %VARS% expanded, relative to `base`, `..` resolved.
/// A file path opens its directory with the cursor on the file.
#[tauri::command]
async fn resolve_path(input: String, base: String) -> Result<Resolved, String> {
    blocking(move || {
        let raw = expand_env(input.trim().trim_matches('"').trim());
        if raw.is_empty() {
            return Err(twinyard_core::i18n::tr("Pusta ścieżka", "Empty path").into());
        }
        // ftp://… typed, or a relative path while in a remote dir (existence is
        // checked by the listing itself).
        let local_abs = raw.len() >= 2 && raw.as_bytes()[1] == b':' || raw.starts_with("\\\\");
        if is_remote(&raw) || (is_remote(&base) && !local_abs) {
            let dir = twinyard_remote::path::resolve(&base, &raw).ok_or(twinyard_core::i18n::tr("Zły adres FTP", "Bad FTP address"))?;
            return Ok(Resolved { dir, focus: None });
        }
        let mut s = raw.replace('/', "\\");
        if s.len() == 2 && s.ends_with(':') {
            s.push('\\'); // "D:" means the drive root, not D:'s current dir
        }
        // "\\server": not a directory for Windows — the listing shows its shares.
        if let Some(server) = twinyard_core::unc::server_only(&s) {
            return Ok(Resolved { dir: format!(r"\\{server}"), focus: None });
        }
        let joined = if Path::new(&s).is_absolute() || s.starts_with("\\\\") {
            PathBuf::from(&s)
        } else {
            Path::new(&base).join(&s) // also handles "\foo" (root of base's drive)
        };
        let full = std::path::absolute(&joined).map_err(|e| format!("{s}: {e}"))?;
        // Inside an archive (a.zip\dir, b.tgz\x\file.txt): the disk doesn't know
        // these paths — a folder there is listed, a file focused in its folder.
        let full_s = full.display().to_string();
        if twinyard_core::archive::is_inside(&full_s) {
            if twinyard_core::archive::list(&full_s).is_ok() {
                return Ok(Resolved { dir: full_s, focus: None });
            }
            let parent = full.parent().unwrap_or(&full).display().to_string();
            let focus = full.file_name().map(|n| n.to_string_lossy().into_owned());
            return match twinyard_core::archive::list(&parent) {
                Ok(l) if focus.as_ref().is_some_and(|f| l.names.iter().any(|n| n.eq_ignore_ascii_case(f))) => Ok(Resolved { dir: parent, focus }),
                Ok(_) => Err(twinyard_core::trf!("{full_s}: nie ma tego w archiwum", "{full_s}: not in the archive")),
                Err(e) => Err(format!("{full_s}: {e}")),
            };
        }
        match std::fs::metadata(&full) {
            Ok(m) if m.is_dir() => Ok(Resolved { dir: full.display().to_string(), focus: None }),
            Ok(_) => Ok(Resolved {
                dir: full.parent().unwrap_or(&full).display().to_string(),
                focus: full.file_name().map(|n| n.to_string_lossy().into_owned()),
            }),
            Err(e) => Err(format!("{}: {e}", full.display())),
        }
    })
    .await
}

/// Expands %NAME% (case-insensitive, like cmd). Unknown names stay as typed.
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) if b > 0 => {
                let name = &after[..b];
                out.push_str(&rest[..a]);
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => out.push_str(&rest[a..a + b + 2]),
                }
                rest = &after[b + 1..];
            }
            _ => {
                out.push_str(&rest[..=a]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Shift+F6 / F2: rename within the same directory.
#[tauri::command]
async fn rename_path(remote: RemoteState<'_>, from: String, to: String) -> Result<(), String> {
    let r = remote.inner().clone();
    blocking(move || {
        if is_remote(&from) {
            return r.rename(&from, &to);
        }
        if twinyard_core::archive::is_inside(&from) {
            return twinyard_core::archive::rename(&from, &to).map_err(|e| e.to_string());
        }
        let (f, t) = (Path::new(&from), Path::new(&to));
        // Case-only rename (a.txt -> A.txt) is fine on Windows; otherwise refuse to clobber.
        if t.exists() && from.to_lowercase() != to.to_lowercase() {
            return Err(twinyard_core::trf!("Już istnieje: {to}", "Already exists: {to}"));
        }
        twinyard_core::win::move_path(f, t, false).map_err(|e| format!("{from}: {e}"))
    })
    .await
}

/// F3 / F4 / compare: runs the tool configured in config.toml.
/// The config is re-read each time, so edits apply without restart.
#[tauri::command]
/// Returns false (and launches nothing) when the tool is set to 'builtin' —
/// the UI then shows its own viewer / diff.
async fn run_tool(remote: RemoteState<'_>, tool: String, files: Vec<String>) -> Result<bool, String> {
    let r = remote.inner().clone();
    blocking(move || {
        let cfg = twinyard_core::config::load().map_err(|e| e.to_string())?;
        let builtin = |t: &str| t.trim().eq_ignore_ascii_case(twinyard_core::config::BUILTIN) || t.trim().is_empty() && tool != "editor";
        if (tool == "viewer" && builtin(&cfg.tools.viewer)) || (tool == "diff" && builtin(&cfg.tools.diff)) {
            return Ok(false);
        }
        let files = files.iter().map(|f| local_copy(&r, f)).collect::<Result<Vec<_>, _>>()?;
        let (template, vars): (&str, Vec<(&str, &str)>) = match (tool.as_str(), files.as_slice()) {
            ("editor", [f]) => (&cfg.tools.editor, vec![("%f", f)]),
            ("viewer", [f]) => (&cfg.tools.viewer, vec![("%f", f)]),
            ("diff", [l, r]) => (&cfg.tools.diff, vec![("%l", l), ("%r", r)]),
            _ => return Err(twinyard_core::trf!("Nieprawidłowe wywołanie narzędzia: {tool}", "Invalid tool call: {tool}")),
        };
        twinyard_core::config::launch(template, &vars).map_err(|e| e.to_string())?;
        Ok(true)
    })
    .await
}

/// Hex view: first `limit` bytes (remote files via %TEMP%).
#[tauri::command]
async fn read_bytes(remote: RemoteState<'_>, path: String, limit: usize) -> Result<tauri::ipc::Response, String> {
    let r = remote.inner().clone();
    blocking(move || {
        use std::io::Read;
        let local = local_copy(&r, &path)?;
        let mut buf = Vec::new();
        std::fs::File::open(&local)
            .and_then(|f| f.take(limit as u64).read_to_end(&mut buf))
            .map_err(|e| format!("{path}: {e}"))?;
        Ok(tauri::ipc::Response::new(buf)) // raw bytes -> ArrayBuffer, no JSON
    })
    .await
}

/// Local path to show (remote files are downloaded first) — for images.
#[tauri::command]
async fn local_path(remote: RemoteState<'_>, path: String) -> Result<String, String> {
    let r = remote.inner().clone();
    blocking(move || local_copy(&r, &path)).await
}

/// Built-in viewer / diff: first `limit` bytes as text (remote files are
/// downloaded to %TEMP% first).
#[tauri::command]
async fn read_text(remote: RemoteState<'_>, path: String, limit: usize) -> Result<twinyard_core::text::TextFile, String> {
    let r = remote.inner().clone();
    blocking(move || {
        let local = local_copy(&r, &path)?;
        twinyard_core::text::read(Path::new(&local), limit).map_err(|e| format!("{path}: {e}"))
    })
    .await
}

/// Terminal in `path`: Windows Terminal if installed, else cmd.exe.
#[tauri::command]
fn open_terminal(path: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x10;
    if is_remote(&path) {
        return Err(twinyard_core::i18n::tr("Terminal tylko dla katalogów lokalnych", "The terminal works only in local directories").into());
    }
    if std::process::Command::new("wt.exe").args(["-d", &path]).spawn().is_ok() {
        return Ok(());
    }
    std::process::Command::new("cmd.exe")
        .current_dir(&path)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Opens config.toml in the configured editor (created on first use).
/// `colors_template`: the commented colour section, appended if missing.
#[tauri::command]
async fn open_config(colors_template: Option<String>) -> Result<(), String> {
    blocking(move || {
        let cfg = twinyard_core::config::load().map_err(|e| e.to_string())?;
        if let Some(t) = colors_template {
            twinyard_core::config::ensure_colors_section(&t).map_err(|e| e.to_string())?;
        }
        let path = twinyard_core::config::config_path().to_string_lossy().into_owned();
        twinyard_core::config::launch(&cfg.tools.editor, &[("%f", &path)]).map_err(|e| e.to_string())
    })
    .await
}

/// Interface colour overrides from config.toml (re-read on every call).
#[tauri::command]
async fn get_colors() -> Result<twinyard_core::config::Colors, String> {
    blocking(|| twinyard_core::config::load().map(|c| c.colors).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn get_favorites() -> Result<Vec<twinyard_core::config::Favorite>, String> {
    blocking(|| twinyard_core::config::load_favorites().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn set_favorites(list: Vec<twinyard_core::config::Favorite>) -> Result<(), String> {
    blocking(move || twinyard_core::config::save_favorites(&list).map_err(|e| e.to_string())).await
}

/// Synthetic listing for UI performance tests (no disk involved).
#[tauri::command]
async fn debug_fake_listing(count: usize) -> Result<Listing, String> {
    blocking(move || {
        let start = std::time::Instant::now();
        let exts = ["txt", "rs", "jpg", "pdf", "docx", "zip", "", "json"];
        let mut l = Listing {
            path: format!("::fake::{count}"),
            ..Default::default()
        };
        for i in 0..count {
            let is_dir = i % 50 == 0;
            let ext = exts[i % exts.len()];
            l.names.push(if is_dir || ext.is_empty() {
                format!("item_{i:06}")
            } else {
                format!("file_{i:06}.{ext}")
            });
            l.sizes.push(if is_dir { 0 } else { (i as u64 * 7919) % 50_000_000 });
            l.mtimes.push(1_700_000_000_000 + (i as i64 * 61_000));
            l.attrs.push(if is_dir { 0x10 } else { 0x20 });
        }
        l.finish(start);
        Ok(l)
    })
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    filelog::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(jobs::Jobs::default()))
        .manage(Remote::new())
        .manage(Arc::new(edits::Edits::default()))
        .manage(Arc::new(search::Searches::default()))
        .manage(Arc::new(extras::SizeJobs::default()))
        .setup(|app| {
            use tauri::Manager;
            edits::start(app.handle().clone(), app.state::<Arc<edits::Edits>>().inner().clone());
            app.manage(watch::DirWatch::new(app.handle().clone()));
            if let Some(win) = app.get_webview_window("main") {
                winstate::attach(&win);
            }
            housekeeping::write_pid();
            housekeeping::clean_temp_later();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_dir,
            parent_dir,
            list_drives,
            home_dir,
            set_lang,
            open_path,
            create_dir,
            create_file,
            archive_password,
            resolve_path,
            rename_path,
            run_tool,
            read_text,
            load_state,
            dnd::drag_out,
            ctxmenu::shell_context_menu,
            save_state,
            open_terminal,
            dircmp::compare_dirs,
            search::search_start,
            search::search_cancel,
            read_bytes,
            local_path,
            open_config,
            get_colors,
            get_favorites,
            set_favorites,
            debug_fake_listing,
            ftp::ftp_connections,
            ftp::ftp_save_connection,
            ftp::ftp_delete_connection,
            ftp::ftp_tc_default_path,
            ftp::ftp_connect,
            ftp::ftp_trust,
            ftp::ftp_forget_password,
            ftp::ftp_disconnect,
            ftp::ftp_import_tc,
            ftp::ftp_open_log,
            edits::edit_remote,
            edits::edit_set_auto,
            edits::edit_upload,
            edits::edit_new_remote,
            edits::remote_temp_path,
            extras::disk_space,
            extras::dir_size,
            extras::dir_size_cancel,
            extras::dir_size_progress,
            shellicon::file_icons,
            extras::clipboard_set_files,
            extras::clipboard_set_text,
            extras::clipboard_get_files,
            extras::clipboard_clear,
            watch::watch_dirs,
            jobs::start_job,
            jobs::job_answer,
            jobs::job_cancel
        ])
        .run(tauri::generate_context!())
        .expect("error while running twinyard");
}

fn state_path() -> PathBuf {
    twinyard_core::config::data_dir().join("state.json")
}

/// UI state (tabs, theme, toolbar) — see app/src/lib/uiState.ts.
#[tauri::command]
fn load_state() -> std::collections::HashMap<String, String> {
    // A BOM (file saved by Notepad / PowerShell) must not wipe tabs and settings.
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|s| serde_json::from_str(s.trim_start_matches('\u{feff}')).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn save_state(state: std::collections::HashMap<String, String>) -> Result<(), String> {
    let path = state_path();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    twinyard_core::win::move_path(&tmp, &path, true).map_err(|e| e.to_string())
}
