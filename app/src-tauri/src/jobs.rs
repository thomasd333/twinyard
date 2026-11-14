//! Background file jobs: one OS thread per job, events to the UI,
//! blocking questions answered through `job_answer`.

use twinyard_core::ops::{self, Conflict, ConflictAction, ConflictAnswer, ErrorAction, Host, OpError, OpSpec, Progress};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

const PROGRESS_EVERY: Duration = Duration::from_millis(80);

#[derive(Default)]
pub struct Jobs {
    next: AtomicU64,
    active: Mutex<HashMap<u64, Arc<Ctl>>>,
}

#[derive(Default)]
struct Ctl {
    cancel: AtomicBool,
    /// Present while the worker waits for an answer.
    pending: Mutex<Option<Sender<Value>>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    id: u64,
    #[serde(flatten)]
    progress: &'a Progress,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum Ask<'a> {
    Conflict { id: u64, conflict: &'a Conflict },
    Error { id: u64, error: &'a OpError },
    Password { id: u64, archive: &'a str, wrong: bool },
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DoneEvent {
    id: u64,
    outcome: ops::Outcome,
}

struct TauriHost {
    app: AppHandle,
    id: u64,
    ctl: Arc<Ctl>,
    last: Mutex<Instant>,
}

impl TauriHost {
    /// Emits the question and blocks until answered (None = cancelled).
    fn ask(&self, ask: Ask) -> Option<Value> {
        let (tx, rx) = channel();
        *self.ctl.pending.lock().unwrap() = Some(tx);
        if self.ctl.cancel.load(Ordering::Relaxed) {
            return None;
        }
        let _ = self.app.emit("job-ask", ask);
        let answer = rx.recv().ok();
        *self.ctl.pending.lock().unwrap() = None;
        answer
    }
}

impl Host for TauriHost {
    fn progress(&self, p: &Progress) {
        let mut last = self.last.lock().unwrap();
        if last.elapsed() >= PROGRESS_EVERY || p.done_files == p.total_files {
            *last = Instant::now();
            let _ = self.app.emit("job-progress", ProgressEvent { id: self.id, progress: p });
        }
    }

    fn conflict(&self, c: &Conflict) -> ConflictAnswer {
        self.ask(Ask::Conflict { id: self.id, conflict: c })
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or(ConflictAnswer { action: ConflictAction::Cancel, all: false })
    }

    fn error(&self, e: &OpError) -> ErrorAction {
        self.ask(Ask::Error { id: self.id, error: e })
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or(ErrorAction::Cancel)
    }

    fn cancelled(&self) -> bool {
        self.ctl.cancel.load(Ordering::Relaxed)
    }

    fn password(&self, archive: &str, wrong: bool) -> Option<String> {
        self.ask(Ask::Password { id: self.id, archive, wrong }).and_then(|v| v.as_str().map(str::to_string))
    }
}

#[tauri::command]
pub fn start_job(
    app: AppHandle,
    jobs: State<'_, Arc<Jobs>>,
    remote: State<'_, Arc<twinyard_remote::Remote>>,
    spec: OpSpec,
) -> Result<u64, String> {
    let id = jobs.next.fetch_add(1, Ordering::Relaxed) + 1;
    let ctl = Arc::new(Ctl::default());
    jobs.active.lock().unwrap().insert(id, ctl.clone());
    let jobs = jobs.inner().clone();
    let remote = remote.inner().clone();
    std::thread::Builder::new()
        .name(format!("job-{id}"))
        .stack_size(8 * 1024 * 1024) // deep trees are scanned recursively
        .spawn(move || {
            let host = TauriHost { app: app.clone(), id, ctl, last: Mutex::new(Instant::now() - PROGRESS_EVERY) };
            let remote_job = spec.sources.iter().chain(spec.dest.iter()).any(|p| twinyard_remote::is_remote(&p.to_string_lossy()));
            let in_archive = spec.sources.iter().chain(spec.dest.iter()).any(|p| twinyard_core::archive::is_inside(&p.to_string_lossy()));
            // A bug (panic) ends this job with an error — never the whole
            // program in the middle of other jobs.
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if remote_job && in_archive {
                    ops::Outcome {
                        errors: vec![OpError { path: String::new(), message: twinyard_core::i18n::tr("Archiwum ↔ FTP: najpierw skopiuj na dysk lokalny", "Archive ↔ FTP: copy to a local disk first").into() }],
                        ..Default::default()
                    }
                } else if remote_job {
                    twinyard_remote::engine::run(&remote, &spec, &host)
                } else {
                    ops::run(&spec, &host)
                }
            }));
            let outcome = run.unwrap_or_else(|p| {
                let what = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
                ops::Outcome {
                    errors: vec![OpError {
                        path: String::new(),
                        message: twinyard_core::trf!("Błąd wewnętrzny programu — zadanie przerwane: {what}", "Internal error — the job was stopped: {what}"),
                    }],
                    cancelled: true,
                    ..Default::default()
                }
            });
            jobs.active.lock().unwrap().remove(&id);
            let _ = app.emit("job-done", DoneEvent { id, outcome });
        })
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn job_answer(jobs: State<'_, Arc<Jobs>>, id: u64, answer: Value) {
    if let Some(ctl) = jobs.active.lock().unwrap().get(&id) {
        if let Some(tx) = ctl.pending.lock().unwrap().take() {
            let _ = tx.send(answer);
        }
    }
}

#[tauri::command]
pub fn job_cancel(jobs: State<'_, Arc<Jobs>>, id: u64) {
    if let Some(ctl) = jobs.active.lock().unwrap().get(&id) {
        ctl.cancel.store(true, Ordering::Relaxed);
        // Dropping the sender wakes a worker blocked on a question.
        ctl.pending.lock().unwrap().take();
    }
}
