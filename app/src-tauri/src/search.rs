//! Alt+F7: background search; results stream to the UI in batches.

use twinyard_core::search::{Hit, Query, Stats};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

const BATCH_EVERY: Duration = Duration::from_millis(150);

#[derive(Default)]
pub struct Searches {
    next: AtomicU64,
    active: Mutex<HashMap<u64, Arc<AtomicBool>>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Batch {
    id: u64,
    hits: Vec<Hit>,
    stats: Stats,
    done: bool,
}

#[tauri::command]
pub async fn search_start(
    app: AppHandle,
    state: State<'_, Arc<Searches>>,
    remote: State<'_, Arc<twinyard_remote::Remote>>,
    query: Query,
) -> Result<u64, String> {
    let is_ftp = twinyard_remote::is_remote(&query.root);
    let remote = remote.inner().clone();
    // Off the UI thread: a dead network path may take long to answer.
    let root = query.root.clone();
    if !is_ftp && !crate::blocking(move || Ok(std::path::Path::new(&root).is_dir())).await? {
        return Err(twinyard_core::trf!("Nie ma takiego katalogu: {}", "No such directory: {}", query.root));
    }
    let id = state.next.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    state.active.lock().unwrap().insert(id, cancel.clone());
    let searches = state.inner().clone();
    let text_ext = twinyard_core::config::load().map(|c| c.search.text_ext).unwrap_or_default();
    std::thread::Builder::new()
        .name(format!("search-{id}"))
        .spawn(move || {
            let mut pending: Vec<Hit> = Vec::new();
            let mut last = Instant::now();
            let mut stats = Stats::default();
            let emit = |hits: Vec<Hit>, stats: &Stats, done: bool| {
                let _ = app.emit("search-batch", Batch { id, hits, stats: stats.clone(), done });
            };
            let mut flush = |pending: &mut Vec<Hit>, stats: &Stats, force: bool| {
                if force || last.elapsed() >= BATCH_EVERY {
                    emit(std::mem::take(pending), stats, false);
                    last = Instant::now();
                }
            };
            // Hits and stats arrive through two callbacks; buffer hits, flush on stats.
            let hits_buf = std::cell::RefCell::new(Vec::new());
            let mut on_hit = |h| hits_buf.borrow_mut().push(h);
            let mut on_stats = |s: &Stats| {
                stats = s.clone();
                pending.append(&mut hits_buf.borrow_mut());
                flush(&mut pending, &stats, false);
            };
            let final_stats = if is_ftp {
                twinyard_remote::search::run(&remote, &query, &text_ext, &cancel, &mut on_hit, &mut on_stats).unwrap_or_else(|e| {
                    log::warn!("search: {e}");
                    Stats { current: twinyard_core::trf!("Błąd: {e}", "Error: {e}"), ..Default::default() }
                })
            } else {
                twinyard_core::search::run(&query, &text_ext, &cancel, &mut on_hit, &mut on_stats)
            };
            pending.append(&mut hits_buf.borrow_mut());
            emit(pending, &final_stats, true);
            searches.active.lock().unwrap().remove(&id);
        })
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn search_cancel(state: State<'_, Arc<Searches>>, id: u64) {
    if let Some(c) = state.active.lock().unwrap().get(&id) {
        c.store(true, Ordering::Relaxed);
    }
}
