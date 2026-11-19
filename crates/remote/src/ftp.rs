//! One FTP / FTPS control connection with hard timeouts everywhere.

use crate::conn::{Connection, NameEncoding, Tls};
use crate::tls::{PinVerifier, Rejected};
use twinyard_core::Listing;
use twinyard_core::listing::{ATTR_DIR, ATTR_REPARSE};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use crate::tlsio::{Connector, Tls as TlsIo};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use suppaftp::list::ListParser;
use suppaftp::types::FileType;
use suppaftp::{FtpError, ImplFtpStream, Mode, Status};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// No byte on the control or data socket for this long = the connection is dead.
pub const IO_TIMEOUT: Duration = Duration::from_secs(30);
/// Whole connect + TLS + login sequence (watchdog thread).
pub const CONNECT_DEADLINE: Duration = Duration::from_secs(45);

pub type Stream = ImplFtpStream<TlsIo>;

#[derive(Debug)]
pub enum ConnectError {
    /// Certificate not trusted — the user may pin it.
    Cert(Rejected),
    Login(String),
    Other(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Cert(r) => write!(f, "{}: {}", twinyard_core::i18n::tr("Niezaufany certyfikat", "Untrusted certificate"), r.reason),
            ConnectError::Login(m) | ConnectError::Other(m) => f.write_str(m),
        }
    }
}

/// Human readable (Polish where it is ours, server text otherwise).
pub fn describe(e: &FtpError) -> String {
    match e {
        FtpError::ConnectionError(io) if is_timeout(io) => twinyard_core::i18n::tr("Brak odpowiedzi serwera (przekroczony czas)", "No response from the server (timed out)").into(),
        FtpError::ConnectionError(io) => twinyard_core::trf!("Błąd połączenia: {io}", "Connection error: {io}"),
        FtpError::SecureError(m) => twinyard_core::trf!("Błąd TLS: {m}", "TLS error: {m}"),
        FtpError::UnexpectedResponse(r) if r.status == Status::NotAvailable => {
            let body = String::from_utf8_lossy(&r.body);
            twinyard_core::trf!(
                "Serwer odmawia połączenia: {} (często czasowa blokada po wielu nieudanych próbach lub limit połączeń — spróbuj za kilka minut)", "The server refuses the connection: {} (often a temporary block after many failed attempts, or a connection limit — try again in a few minutes)",
                body.trim()
            )
        }
        FtpError::UnexpectedResponse(r) => {
            let body = String::from_utf8_lossy(&r.body);
            twinyard_core::trf!("Serwer: {}", "Server: {}", body.trim())
        }
        other => other.to_string(),
    }
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
}

/// Errors after which the connection should be thrown away and re-opened
/// (as opposed to "file not found", "permission denied"…).
pub fn is_connection_error(e: &FtpError) -> bool {
    match e {
        FtpError::ConnectionError(_) | FtpError::SecureError(_) | FtpError::BadResponse => true,
        FtpError::DataConnectionAlreadyOpen => true,
        FtpError::UnexpectedResponse(r) => matches!(
            r.status,
            Status::NotAvailable | Status::CannotOpenDataConnection | Status::TransferAborted
        ),
        _ => false,
    }
}

pub fn is_not_found(e: &FtpError) -> bool {
    matches!(e, FtpError::UnexpectedResponse(r) if r.status == Status::FileUnavailable)
}

fn set_timeouts(tcp: &TcpStream) {
    let _ = tcp.set_read_timeout(Some(IO_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(IO_TIMEOUT));
    let _ = tcp.set_nodelay(true);
}

fn data_stream(addr: SocketAddr) -> suppaftp::FtpResult<TcpStream> {
    let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(FtpError::ConnectionError)?;
    set_timeouts(&tcp);
    Ok(tcp)
}

pub struct Session {
    pub s: Stream,
    pub mlsd: bool,
    pub last_used: Instant,
    /// Last keepalive NOOP (does not count as user activity).
    pub last_ping: Instant,
    /// TLS mode that actually worked (may differ from the saved one).
    pub tls_used: Tls,
    /// Server can set modification times (RFC draft MFMT).
    pub mfmt: bool,
}

impl Session {
    /// Connects and logs in, on a watchdog thread: never blocks the caller
    /// longer than `CONNECT_DEADLINE`, whatever the network does.
    pub fn open(conn: &Connection, password: &str, pins: Vec<String>) -> Result<Session, ConnectError> {
        let (tx, rx) = mpsc::channel();
        let (c, p) = (conn.clone(), password.to_string());
        std::thread::Builder::new()
            .name("ftp-connect".into())
            .spawn(move || {
                let _ = tx.send(open_inner(&c, &p, pins));
            })
            .map_err(|e| ConnectError::Other(e.to_string()))?;
        rx.recv_timeout(CONNECT_DEADLINE).unwrap_or_else(|_| {
            Err(ConnectError::Other(twinyard_core::trf!("{}: serwer nie odpowiada (przekroczony czas łączenia)", "{}: the server is not responding (connection timed out)", conn.host)))
        })
    }

    /// Sets the remote file's modification time (UTC, MFMT). Best effort.
    pub fn set_mtime(&mut self, path: &str, unix_ms: i64) -> bool {
        if !self.mfmt || unix_ms <= 0 {
            return false;
        }
        let ts = utc_stamp(unix_ms / 1000);
        self.s.custom_command(format!("MFMT {ts} {path}"), &[Status::File, Status::RequestedFileActionOk]).is_ok()
    }

    pub fn noop(&mut self) -> suppaftp::FtpResult<()> {
        self.s.noop()?;
        self.last_used = Instant::now();
        Ok(())
    }

    /// Lists `path` (absolute). Uses CWD first: clear errors and no problems
    /// with spaces in LIST arguments on picky servers.
    pub fn list(&mut self, path: &str) -> suppaftp::FtpResult<Vec<Entry>> {
        self.last_used = Instant::now();
        self.s.cwd(path)?;
        let lines = if self.mlsd { self.s.mlsd(None)? } else { self.s.list(None)? };
        self.last_used = Instant::now();
        Ok(lines.iter().filter_map(|l| Entry::parse(l, self.mlsd)).filter(|e| e.name != "." && e.name != "..").collect())
    }
}

type TlsSetup = Option<(Connector, Arc<PinVerifier>)>;

fn open_inner(conn: &Connection, password: &str, pins: Vec<String>) -> Result<Session, ConnectError> {
    // "Implicit" servers that actually greet in plain text ("220 …") and expect
    // AUTH TLS — e.g. explicit TLS on port 990 (what TC's ftps:// does). A real
    // implicit server never speaks first. Must be checked *before* trying:
    // suppaftp's implicit connect waits for the handshake without any timeout.
    let mut c = conn.clone();
    if c.tls == Tls::Implicit && greets_in_plain_text(&c.host, c.port) {
        log::info!("{}: serwer wita się otwartym tekstem — AUTH TLS zamiast niejawnego TLS", c.host_port());
        c.tls = Tls::Explicit;
    }
    let r = open_any_tls(&c, password, &pins);
    // The greeting probe can miss slow greeters (some servers do a reverse DNS
    // lookup first): rustls then reads "220 …" as a TLS record. Same remedy.
    if c.tls == Tls::Implicit
        && let Err(ConnectError::Other(m)) = &r
        && is_plain_text_reply(m)
    {
        log::warn!("{}: na niejawne TLS przyszedł tekst ({m}) — ponawiam z AUTH TLS", c.host_port());
        mark_plain_text(&c.host, c.port);
        c.tls = Tls::Explicit;
        return open_any_tls(&c, password, &pins);
    }
    r
}

fn is_plain_text_reply(m: &str) -> bool {
    let m = m.to_ascii_lowercase();
    m.contains("invalidcontenttype") || m.contains("corrupt message")
}

/// rustls first; SChannel for servers rustls can't handshake with.
fn open_any_tls(c: &Connection, password: &str, pins: &[String]) -> Result<Session, ConnectError> {
    let pins = pins.to_vec();
    // Old servers rustls cannot talk to (e.g. static RSA key exchange only):
    // retry with Windows' SChannel and remember that for this run.
    let key = c.host_port();
    let native = compat_mode(&key, false);
    let r = open_mode(&c, password, pins.clone(), native);
    if let Err(ConnectError::Other(m)) = &r
        && !native
        && c.tls != Tls::None
        && is_tls_incompatibility(m)
    {
        log::warn!("{key}: rustls nie dogadał się z serwerem ({m}) — tryb zgodności SChannel");
        compat_mode(&key, true);
        return open_mode(&c, password, pins, true);
    }
    r
}

fn is_tls_incompatibility(m: &str) -> bool {
    let m = m.to_ascii_lowercase();
    ["handshakefailure", "handshake failure", "peerincompatible", "protocolversion", "insufficientsecurity", "nosharedcipher"]
        .iter()
        .any(|k| m.contains(k))
}

/// Hosts that need SChannel (set = true marks one). Kept for this run.
fn compat_mode(key: &str, set: bool) -> bool {
    static HOSTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut h = HOSTS.lock().unwrap();
    if set && !h.iter().any(|k| k == key) {
        h.push(key.to_string());
    }
    h.iter().any(|k| k == key)
}

static SEEN: Mutex<Option<std::collections::HashMap<String, bool>>> = Mutex::new(None);

fn mark_plain_text(host: &str, port: u16) {
    SEEN.lock().unwrap().get_or_insert_with(Default::default).insert(format!("{}:{port}", host.to_ascii_lowercase()), true);
}

/// Does the server send an FTP greeting before any TLS handshake? Cached per
/// host:port for this run (costs up to 6 s once for real implicit servers; a
/// successful connect also fixes the saved mode, see `Remote::connect`).
fn greets_in_plain_text(host: &str, port: u16) -> bool {
    use std::collections::HashMap;
    use std::io::Read;
    let key = format!("{}:{port}", host.to_ascii_lowercase());
    if let Some(&v) = SEEN.lock().unwrap().as_ref().and_then(|m| m.get(&key)) {
        return v;
    }
    let Some(addr) = (host, port).to_socket_addrs().ok().and_then(|mut a| a.next()) else { return false };
    let Ok(mut tcp) = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) else { return false };
    // Some servers greet only after a reverse DNS lookup (seen: ~3 s).
    let _ = tcp.set_read_timeout(Some(Duration::from_secs(6)));
    let mut buf = [0u8; 4];
    // Any FTP reply code ("220 ready", "421 too many connections"…) = plain text.
    let plain = matches!(tcp.read(&mut buf), Ok(n) if n >= 3 && buf[..3].iter().all(u8::is_ascii_digit));
    SEEN.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, plain);
    plain
}

fn open_mode(conn: &Connection, password: &str, pins: Vec<String>, native: bool) -> Result<Session, ConnectError> {
    let tls: TlsSetup = if conn.tls == Tls::None {
        None
    } else {
        let (cfg, v) = PinVerifier::config(pins).map_err(ConnectError::Other)?;
        let c = if native { Connector::native(v.clone()).map_err(ConnectError::Other)? } else { Connector::Rustls(cfg) };
        Some((c, v))
    };
    log::info!(
        "łączenie z {} ({:?}{}, {})",
        conn.host_port(),
        conn.tls,
        if native { ", SChannel" } else { "" },
        if conn.passive { "PASV" } else { "aktywny" }
    );
    // rustls handshakes lazily (on the first command after AUTH TLS), so a
    // rejected certificate can surface as any I/O error — check the verifier.
    open_steps(conn, password, &tls).map_err(|e| match tls.as_ref().and_then(|(_, v)| v.rejected.lock().unwrap().take()) {
        Some(r) => ConnectError::Cert(r),
        None => e,
    })
}

fn open_steps(conn: &Connection, password: &str, tls: &TlsSetup) -> Result<Session, ConnectError> {
    let other = |m: String| ConnectError::Other(m);
    let tls_err = |e: FtpError| other(describe(&e));
    let addr = (conn.host.as_str(), conn.port)
        .to_socket_addrs()
        .map_err(|e| other(format!("{}: {e}", conn.host)))?
        .next()
        .ok_or_else(|| other(format!("{}: nie znaleziono adresu", conn.host)))?;

    let s = match (&conn.tls, tls) {
        (Tls::Implicit, Some((cfg, _))) => {
            let mut s = Stream::connect_secure_implicit(addr, cfg.clone(), &conn.host).map_err(tls_err)?;
            set_timeouts(&s.get_ref());
            // suppaftp only does this for AUTH TLS; without PROT P the data
            // channel stays plain and servers refuse it (550 TLS required).
            s.custom_command("PBSZ 0", &[Status::CommandOk]).map_err(tls_err)?;
            s.custom_command("PROT P", &[Status::CommandOk]).map_err(tls_err)?;
            s
        }
        _ => {
            let tcp = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
                .map_err(|e| other(format!("{}:{}: {}", conn.host, conn.port, describe(&FtpError::ConnectionError(e)))))?;
            set_timeouts(&tcp);
            let s = Stream::connect_with_stream(tcp).map_err(|e| other(describe(&e)))?;
            match tls {
                Some((cfg, _)) => s.into_secure(cfg.clone(), &conn.host).map_err(tls_err)?,
                None => s,
            }
        }
    };

    let mut s = s.passive_stream_builder(data_stream);
    if conn.passive {
        s.set_mode(Mode::Passive);
        // Servers behind NAT announce a private IP in PASV — use the control IP.
        s.set_passive_nat_workaround(true);
    } else {
        s = s.active_mode(IO_TIMEOUT);
    }

    let user = if conn.user.is_empty() { "anonymous" } else { conn.user.as_str() };
    s.login(user, password).map_err(|e| match &e {
        FtpError::UnexpectedResponse(r) if r.status == Status::NotLoggedIn => {
            ConnectError::Login(twinyard_core::trf!("Logowanie nieudane ({user}): {}", "Login failed ({user}): {}", String::from_utf8_lossy(&r.body).trim()))
        }
        _ => other(describe(&e)),
    })?;
    s.transfer_type(FileType::Binary).map_err(|e| other(describe(&e)))?;

    let feats = s.feat().unwrap_or_default();
    let has = |k: &str| feats.keys().any(|f| f.eq_ignore_ascii_case(k));
    match conn.encoding {
        NameEncoding::Utf8 => {
            let _ = utf8_on(&mut s);
        }
        NameEncoding::Cp1250 => s.set_encoding(crate::names::Cp1250),
        // Announcing UTF8 is not enough: some servers (Serv-U: "501 Please
        // CLNT first") only switch after OPTS succeeds. If it doesn't, the
        // server still stores names in its ANSI code page.
        NameEncoding::Auto if has("UTF8") && utf8_on(&mut s) => {}
        NameEncoding::Auto => {
            log::info!("{}: serwer nie przełączył się na UTF-8 — nazwy wykrywane (domyślnie Windows-1250)", conn.host_port());
            s.set_encoding(crate::names::Sniff::default())
        }
    }
    let mlsd = conn.mlsd && has("MLST");
    let mfmt = has("MFMT");
    Ok(Session { s, mlsd, last_used: Instant::now(), last_ping: Instant::now(), tls_used: conn.tls, mfmt })
}

/// `OPTS UTF8 ON`, introducing ourselves with CLNT first if the server wants that.
fn utf8_on(s: &mut Stream) -> bool {
    if s.opts("UTF8", Some("ON")).is_ok() {
        return true;
    }
    let _ = s.custom_command("CLNT twinyard", &[Status::CommandOk]);
    s.opts("UTF8", Some("ON")).is_ok()
}

/// One remote directory entry.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub link: bool,
    pub size: u64,
    pub mtime: i64,
}

impl Entry {
    fn parse(line: &str, mlsd: bool) -> Option<Entry> {
        // MLSD "type=cdir"/"type=pdir" = the listed dir and its parent; some
        // servers name them with a full path instead of "."/"..", so they
        // showed up as fake folders like "/WWW/site/css".
        if mlsd && is_cdir_pdir(line) {
            return None;
        }
        let f = if mlsd {
            // "type=OS.unix=slink:/target" / "OS.unix=symlink" (ProFTPD, Pure-FTPd):
            // suppaftp refuses them, so such links vanished from the panel and
            // deleting their folder failed with "not empty".
            ListParser::parse_mlsd(line).ok().or_else(|| ListParser::parse_mlsd(&mlsd_unix_link(line)?).ok())?
        } else {
            ListParser::parse_posix(line).or_else(|_| ListParser::parse_dos(line)).ok()?
        };
        // A real entry name never contains a slash.
        if f.name().contains('/') {
            return None;
        }
        let mtime = f.modified().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        // Symlinks on hostings are mostly dirs (www -> public_html); entering a
        // file link just fails, transfers try both.
        Some(Entry {
            name: f.name().to_string(),
            dir: f.is_directory() || f.is_symlink(),
            link: f.is_symlink(),
            size: f.size() as u64,
            mtime,
        })
    }
}

fn is_cdir_pdir(line: &str) -> bool {
    let facts = line.split_once(' ').map_or(line, |(f, _)| f);
    facts.split(';').any(|f| {
        let l = f.trim().to_ascii_lowercase();
        l == "type=cdir" || l == "type=pdir"
    })
}

/// An MLSD line with a Unix link type rewritten to the standard `type=link`.
fn mlsd_unix_link(line: &str) -> Option<String> {
    let (facts, name) = line.split_once(' ')?;
    let mut hit = false;
    let fixed: Vec<&str> = facts
        .split(';')
        .map(|f| {
            let l = f.to_ascii_lowercase();
            if l.starts_with("type=os.unix=slink") || l.starts_with("type=os.unix=symlink") {
                hit = true;
                "type=link"
            } else {
                f
            }
        })
        .collect();
    hit.then(|| format!("{} {name}", fixed.join(";")))
}

pub fn to_listing(url: &str, entries: &[Entry], start: Instant) -> Listing {
    let mut l = Listing { path: url.to_string(), ..Default::default() };
    for e in entries {
        l.names.push(e.name.clone());
        l.sizes.push(if e.dir { 0 } else { e.size });
        l.mtimes.push(e.mtime);
        l.attrs.push(if e.dir { ATTR_DIR } else { 0 } | if e.link { ATTR_REPARSE } else { 0 });
    }
    l.finish(start);
    l
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mlsd_and_list() {
        let e = Entry::parse("type=file;size=1234;modify=20240102030405; plik ze spacją.txt", true).unwrap();
        assert_eq!((e.name.as_str(), e.dir, e.size), ("plik ze spacją.txt", false, 1234));
        assert_eq!(e.mtime, 1704164645000);
        let d = Entry::parse("type=dir;modify=20240102030405; public_html", true).unwrap();
        assert!(d.dir);
        let p = Entry::parse("drwxr-xr-x    2 user  group     4096 Jan  2  2024 logs", false).unwrap();
        assert_eq!((p.name.as_str(), p.dir), ("logs", true));
        let f = Entry::parse("-rw-r--r--    1 user  group      42 Jan  2  2024 a b.txt", false).unwrap();
        assert_eq!((f.name.as_str(), f.size), ("a b.txt", 42));
        let l = Entry::parse("lrwxrwxrwx    1 user  group       11 Jan  2  2024 www -> public_html", false).unwrap();
        assert!(l.dir && l.link);
        assert_eq!(l.name, "www");
        let m = Entry::parse("type=OS.unix=slink:/home/u/public_html;modify=20240102030405;unix.mode=0777; www", true).unwrap();
        assert!(m.dir && m.link && m.name == "www");
        let m = Entry::parse("type=OS.unix=symlink;modify=20240102030405; www2", true).unwrap();
        assert!(m.link && m.name == "www2");
        assert!(Entry::parse("type=cdir;modify=20240102030405; /WWW/site/css", true).is_none());
        assert!(Entry::parse("Type=pdir;modify=20240102030405; /WWW/site", true).is_none());
        assert!(Entry::parse("type=dir;modify=20240102030405; /WWW/site", true).is_none());
    }
}

/// Unix seconds -> "YYYYMMDDHHMMSS" (UTC), civil-from-days without a date crate.
pub fn utc_stamp(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (h, mi, se) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}{m:02}{d:02}{h:02}{mi:02}{se:02}")
}

#[cfg(test)]
mod stamp_tests {
    #[test]
    fn utc() {
        assert_eq!(super::utc_stamp(0), "19700101000000");
        assert_eq!(super::utc_stamp(1704164645), "20240102030405");
        assert_eq!(super::utc_stamp(951782400), "20000229000000"); // leap day
    }
}
