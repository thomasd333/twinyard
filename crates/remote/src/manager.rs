//! Connection manager: one cached *listing* session per server (transfers
//! open their own, so browsing never waits for a running copy), transparent
//! reconnects, keepalive and password lookup.

use crate::conn::{self, Connection};
use crate::creds;
use crate::ftp::{ConnectError, Session, describe, is_connection_error, to_listing};
use crate::path;
use crate::tls::pretty;
use twinyard_core::Listing;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use suppaftp::FtpResult;

/// Probe a cached session with NOOP before use when idle longer than this.
const STALE_AFTER: Duration = Duration::from_secs(20);
/// Keepalive NOOP interval for idle listing sessions…
const KEEPALIVE: Duration = Duration::from_secs(45);
/// …but only for this long; after that the session is closed politely.
const IDLE_CLOSE: Duration = Duration::from_secs(20 * 60);

type Slot = Arc<Mutex<Option<Session>>>;

#[derive(Default)]
pub struct Remote {
    sessions: Mutex<HashMap<String, Slot>>,
    /// Passwords for this run (typed in or read from the Credential Manager).
    passwords: Mutex<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ConnectResult {
    #[serde(rename_all = "camelCase")]
    Ok { path: String },
    #[serde(rename_all = "camelCase")]
    NeedPassword { name: String, user: String, error: Option<String> },
    #[serde(rename_all = "camelCase")]
    Cert { name: String, host: String, sha256: String, sha1: String, reason: String },
}

fn anonymous(c: &Connection) -> bool {
    c.user.is_empty() || c.user.eq_ignore_ascii_case("anonymous") || c.user.eq_ignore_ascii_case("ftp")
}

impl Remote {
    pub fn new() -> Arc<Remote> {
        let r = Arc::new(Remote::default());
        let weak = Arc::downgrade(&r);
        std::thread::Builder::new().name("ftp-keepalive".into()).spawn(move || keepalive(weak)).ok();
        r
    }

    fn conn(&self, id: &str) -> Result<(Connection, Vec<String>), String> {
        let store = conn::load().map_err(|e| format!("connections.json: {e}"))?;
        let c = store.get(id).cloned().ok_or_else(|| twinyard_core::trf!("Nie ma połączenia „{id}” (Ctrl+F — menedżer połączeń)", "No connection “{id}” (Ctrl+F — connection manager)"))?;
        let pins = store.pins_for(&c.host_port());
        Ok((c, pins))
    }

    fn password(&self, id: &str, c: &Connection) -> Option<String> {
        if let Some(p) = self.passwords.lock().unwrap().get(id) {
            return Some(p.clone());
        }
        creds::get(id).or_else(|| anonymous(c).then(|| "twinyard@".to_string()))
    }

    /// Explicit connect from the UI. `password: None` = use the saved one.
    pub fn connect(&self, id: &str, password: Option<String>, remember: bool) -> Result<ConnectResult, String> {
        let (c, pins) = self.conn(id)?;
        let typed = password.is_some();
        let Some(pw) = password.or_else(|| self.password(id, &c)) else {
            return Ok(ConnectResult::NeedPassword { name: c.name, user: c.user, error: None });
        };
        match Session::open(&c, &pw, pins) {
            Ok(mut s) => {
                if s.tls_used != c.tls {
                    // e.g. "implicit" that is really AUTH TLS: remember what works.
                    log::info!("{id}: zapisuję wykryty tryb TLS {:?} (było {:?})", s.tls_used, c.tls);
                    if let Ok(mut store) = conn::load()
                        && let Some(x) = store.connections.iter_mut().find(|x| x.id == c.id)
                    {
                        x.tls = s.tls_used;
                        let _ = conn::save(&store);
                    }
                }
                self.passwords.lock().unwrap().insert(id.to_string(), pw.clone());
                if typed && remember {
                    creds::set(id, &c.user, &pw).map_err(|e| twinyard_core::trf!("Zapis hasła w Menedżerze poświadczeń: {e}", "Saving the password: {e}"))?;
                }
                let start = if c.initial_dir.trim().is_empty() {
                    s.s.pwd().unwrap_or_else(|_| "/".into())
                } else {
                    c.initial_dir.clone()
                };
                *self.slot(id).lock().unwrap() = Some(s);
                Ok(ConnectResult::Ok { path: path::make(id, &start) })
            }
            Err(ConnectError::Cert(r)) => Ok(ConnectResult::Cert {
                name: c.name.clone(),
                host: c.host_port(),
                sha256: pretty(&r.sha256),
                sha1: pretty(&r.sha1),
                reason: r.reason,
            }),
            Err(ConnectError::Login(m)) => {
                self.passwords.lock().unwrap().remove(id);
                Ok(ConnectResult::NeedPassword { name: c.name, user: c.user, error: Some(m) })
            }
            Err(ConnectError::Other(m)) => {
                log::warn!("{id}: połączenie nieudane: {m}");
                Err(m)
            }
        }
    }

    /// The user trusts this certificate for the connection's host:port.
    pub fn trust(&self, id: &str, sha256: &str) -> Result<(), String> {
        let mut store = conn::load().map_err(|e| e.to_string())?;
        let hp = store.get(id).ok_or(twinyard_core::i18n::tr("Nie ma takiego połączenia", "No such connection"))?.host_port();
        store.add_pin(&hp, sha256);
        conn::save(&store).map_err(|e| e.to_string())
    }

    pub fn forget_password(&self, id: &str) -> Result<(), String> {
        self.passwords.lock().unwrap().remove(id);
        creds::delete(id).map_err(|e| e.to_string())
    }

    pub fn has_saved_password(&self, id: &str) -> bool {
        creds::get(id).is_some()
    }

    pub fn disconnect(&self, id: &str) {
        let slot = self.sessions.lock().unwrap().remove(id);
        if let Some(slot) = slot
            && let Some(mut s) = slot.lock().unwrap().take()
        {
            let _ = s.s.quit();
        }
    }

    fn slot(&self, id: &str) -> Slot {
        self.sessions.lock().unwrap().entry(id.to_string()).or_default().clone()
    }

    /// A fresh session (for a transfer job), never the shared listing one.
    pub fn open_session(&self, id: &str) -> Result<Session, String> {
        let (c, pins) = self.conn(id)?;
        let pw = self.password(id, &c).ok_or_else(|| twinyard_core::trf!("Brak hasła do „{}” — połącz się przez Ctrl+F", "No password for “{}” — connect with Ctrl+F", c.name))?;
        Session::open(&c, &pw, pins).map_err(|e| e.to_string())
    }

    /// Runs `f` on the listing session; reconnects once if the connection died.
    pub fn with<T>(&self, id: &str, mut f: impl FnMut(&mut Session) -> FtpResult<T>) -> Result<T, String> {
        let slot = self.slot(id);
        let mut g = slot.lock().unwrap();
        let mut last = String::new();
        for _ in 0..2 {
            if g.is_none() {
                *g = Some(self.open_session(id)?);
            }
            let s = g.as_mut().unwrap();
            if s.last_used.max(s.last_ping).elapsed() > STALE_AFTER && s.noop().is_err() {
                *g = None;
                continue;
            }
            match f(s) {
                Ok(v) => return Ok(v),
                Err(e) if is_connection_error(&e) => {
                    last = describe(&e);
                    *g = None;
                }
                Err(e) => return Err(describe(&e)),
            }
        }
        Err(last)
    }

    /// Listing dates of this server can be trusted for "which one is newer":
    /// MLSD gives UTC with seconds; LIST gives the server's local time,
    /// without seconds (and without the year for older files).
    pub fn exact_dates(&self, id: &str) -> bool {
        self.slot(id).lock().unwrap().as_ref().is_some_and(|s| s.mlsd)
    }

    pub fn list(&self, url: &str) -> Result<Listing, String> {
        let start = Instant::now();
        let (id, p) = path::split(url).ok_or(twinyard_core::i18n::tr("Zły adres FTP", "Bad FTP address"))?;
        let entries = self.with(&id, |s| s.list(&p)).map_err(|e| format!("{url}: {e}"))?;
        Ok(to_listing(&path::make(&id, &p), &entries, start))
    }

    /// mkdir -p: missing parents are created too.
    pub fn mkdir(&self, url: &str) -> Result<(), String> {
        let (id, p) = path::split(url).ok_or(twinyard_core::i18n::tr("Zły adres FTP", "Bad FTP address"))?;
        self.with(&id, |s| {
            if s.s.cwd(&p).is_ok() {
                return Err(suppaftp::FtpError::UnexpectedResponse(suppaftp::types::Response::new(
                    suppaftp::Status::FileUnavailable,
                    twinyard_core::trf!("Już istnieje: {p}", "Already exists: {p}").into_bytes(),
                )));
            }
            let mut cur = String::new();
            for seg in p.split('/').filter(|s| !s.is_empty()) {
                cur = format!("{cur}/{seg}");
                if s.s.cwd(&cur).is_err() {
                    s.s.mkdir(&cur)?;
                }
            }
            Ok(())
        })
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        let (id, a) = path::split(from).ok_or(twinyard_core::i18n::tr("Zły adres FTP", "Bad FTP address"))?;
        let (id2, b) = path::split(to).ok_or(twinyard_core::i18n::tr("Zły adres FTP", "Bad FTP address"))?;
        if !id.eq_ignore_ascii_case(&id2) {
            return Err(twinyard_core::i18n::tr("Zmiana nazwy tylko w obrębie jednego serwera", "Renaming works only within one server").into());
        }
        // RNTO replaces an existing target on most servers without a word:
        // refuse like a local rename does (a case-only change is fine).
        let (al, bl) = (a.to_lowercase(), b.to_lowercase());
        self.with(&id, |s| {
            if al != bl {
                let parent = path::parent(&b).unwrap_or_else(|| "/".into());
                let name = path::file_name(&bl).to_string();
                if s.list(&parent)?.iter().any(|e| e.name.to_lowercase() == name) {
                    return Err(suppaftp::FtpError::UnexpectedResponse(suppaftp::types::Response::new(
                        suppaftp::Status::FileUnavailable,
                        twinyard_core::trf!("Już istnieje: {b}", "Already exists: {b}").into_bytes(),
                    )));
                }
            }
            s.s.rename(&a, &b)
        })
    }

    /// Downloads a remote file to `%TEMP%\twinyard\<id>\…` (for F3/compare/open).
    pub fn fetch_temp(&self, url: &str) -> Result<PathBuf, String> {
        let local = temp_path(url)?;
        self.fetch_to(url, &local)?;
        Ok(local)
    }

    /// Downloads `url` to `local`: into `<local>.part` first, swapped in only
    /// when complete — a reader (or the F4 watcher) never sees half a file.
    pub fn fetch_to(&self, url: &str, local: &std::path::Path) -> Result<(), String> {
        let (id, p) = path::split(url).ok_or(twinyard_core::i18n::tr("Zły adres FTP", "Bad FTP address"))?;
        std::fs::create_dir_all(local.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut part = local.as_os_str().to_owned();
        part.push(".part");
        let part = PathBuf::from(part);
        let r = (|| {
            let mut sess = self.open_session(&id)?; // own session: may be big
            // A cut-short copy edited with F4 would go back cut short.
            let expect = sess.s.size(&p).ok().map(|n| n as u64);
            let mut file = std::fs::File::create(&part).map_err(|e| format!("{}: {e}", part.display()))?;
            let mut stream = sess.s.retr_as_stream(&p).map_err(|e| format!("{url}: {}", describe(&e)))?;
            let mut buf = vec![0u8; 256 * 1024];
            let mut got = 0u64;
            loop {
                let n = stream.read(&mut buf).map_err(|e| format!("{url}: {e}"))?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", part.display()))?;
                got += n as u64;
            }
            stream.finish().map_err(|e| describe(&e))?;
            if let Some(n) = expect
                && got < n
            {
                return Err(twinyard_core::trf!("{url}: pobrano {got} z {n} bajtów", "{url}: downloaded {got} of {n} bytes"));
            }
            let _ = sess.s.quit();
            drop(file);
            twinyard_core::win::move_path(&part, local, true).map_err(|e| format!("{}: {e}", local.display()))
        })();
        if r.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        r
    }
}

/// Where a remote file's local copy lives: `%TEMP%\twinyard\<id>\<path>`.
pub fn temp_path(url: &str) -> Result<PathBuf, String> {
    temp_path_in(url, None)
}

/// Like `temp_path`, under a separate folder `sub` (`~edit`: F4 copies, so
/// viewing a file never overwrites the copy being edited). `~` never
/// appears in a connection id.
pub fn temp_path_in(url: &str, sub: Option<&str>) -> Result<PathBuf, String> {
    let (id, p) = path::split(url).ok_or(twinyard_core::i18n::tr("Zły adres FTP", "Bad FTP address"))?;
    let mut local = std::env::temp_dir().join("twinyard");
    if let Some(s) = sub {
        local.push(s);
    }
    local.push(&id);
    for seg in p.split('/').filter(|s| !s.is_empty()) {
        local.push(seg.replace(['<', '>', ':', '"', '|', '?', '*'], "_"));
    }
    Ok(local)
}

fn keepalive(weak: Weak<Remote>) {
    loop {
        std::thread::sleep(Duration::from_secs(10));
        let Some(r) = weak.upgrade() else { return };
        let slots: Vec<(String, Slot)> = r.sessions.lock().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (_, slot) in slots {
            // Busy (listing in progress) = alive anyway.
            let Ok(mut g) = slot.try_lock() else { continue };
            let Some(s) = g.as_mut() else { continue };
            let idle = s.last_used.elapsed();
            if idle > IDLE_CLOSE {
                let _ = s.s.quit();
                *g = None;
            } else if idle > KEEPALIVE && s.last_ping.elapsed() > KEEPALIVE {
                // Raw NOOP: `last_used` keeps counting the user's idle time.
                if s.s.noop().is_err() {
                    *g = None;
                } else {
                    s.last_ping = Instant::now();
                }
            }
        }
    }
}
