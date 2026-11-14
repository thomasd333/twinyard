//! Detailed FTP log for diagnosing servers: `%APPDATA%\twinyard\logs\ftp.log`.
//! Every control-channel command/response (suppaftp trace), TLS decisions,
//! our reconnects and errors. Passwords are masked. Rotates at 5 MB.

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_BYTES: u64 = 5 * 1024 * 1024;

struct FileLog {
    path: PathBuf,
    file: Mutex<Option<File>>,
    /// Last (thread, message): suppaftp logs every reply twice.
    last: Mutex<(String, String)>,
}

pub fn log_path() -> PathBuf {
    twinyard_core::config::config_path().with_file_name("logs").join("ftp.log")
}

fn wanted(target: &str, level: Level) -> bool {
    if target.starts_with("suppaftp") {
        true // trace = every command and reply
    } else if target.starts_with("twinyard") || target.starts_with("rustls") || target.starts_with("native_tls") {
        level <= Level::Debug
    } else {
        level <= Level::Warn
    }
}

/// suppaftp logs replies as byte arrays ("CC IN: [50, 50, 48, …]"); make them text.
fn readable(msg: &str) -> String {
    if let Some(rest) = msg.strip_prefix("CC IN: [")
        && let Some(list) = rest.strip_suffix(']')
    {
        let bytes: Vec<u8> = list.split(',').filter_map(|b| b.trim().parse().ok()).collect();
        return format!("<- {}", String::from_utf8_lossy(&bytes).trim_end());
    }
    if let Some(cmd) = msg.strip_prefix("CC OUT: ") {
        let cmd = cmd.trim_end();
        if cmd.len() >= 5 && cmd[..5].eq_ignore_ascii_case("PASS ") {
            return "-> PASS ****".into();
        }
        return format!("-> {cmd}");
    }
    msg.to_string()
}

impl Log for FileLog {
    fn enabled(&self, m: &Metadata) -> bool {
        wanted(m.target(), m.level())
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let msg = readable(&r.args().to_string());
        // suppaftp prints each reply twice (parse + raw) — skip the parse note.
        if msg.starts_with("Code parsed from response") {
            return;
        }
        let thread = std::thread::current();
        {
            let key = (thread.name().unwrap_or("?").to_string(), msg.clone());
            let mut last = self.last.lock().unwrap();
            if *last == key {
                return;
            }
            *last = key;
        }
        let now = chrono_like_now();
        let line = format!("{now} {:5} [{}] {}: {msg}\n", r.level(), thread.name().unwrap_or("?"), r.target());
        let mut g = self.file.lock().unwrap();
        if let Some(f) = g.as_mut() {
            if f.metadata().map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
                let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
                *g = OpenOptions::new().create(true).append(true).open(&self.path).ok();
            }
        }
        if let Some(f) = g.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = f.flush();
        }
    }
}

/// Local time "2026-11-12 13:40:05.123" without a date crate.
fn chrono_like_now() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    // SAFETY: GetLocalTime only writes the struct.
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

pub fn init() {
    let path = log_path();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    let logger = Box::new(FileLog { path, file: Mutex::new(file), last: Mutex::new(Default::default()) });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(LevelFilter::Trace);
        log::info!("--- twinyard {} start ---", env!("CARGO_PKG_VERSION"));
    }
}
