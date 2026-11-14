//! Live tests against a local FTP server (pyftpdlib, see README "Testy FTP").
//! Run: `set TWINYARD_FTP_TEST=1 && cargo test -p twinyard-remote --test live -- --test-threads=1 --nocapture`

use twinyard_core::ops::{
    Conflict, ConflictAction, ConflictAnswer, ErrorAction, Host, OpError, OpKind, OpSpec, Progress,
};
use twinyard_remote::conn::{self, Connection, Store, Tls};
use twinyard_remote::{ConnectResult, Remote, engine, path};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

struct TestHost {
    errors: Mutex<Vec<String>>,
    progress_calls: Mutex<u64>,
}

impl Host for TestHost {
    fn progress(&self, _p: &Progress) {
        *self.progress_calls.lock().unwrap() += 1;
    }
    fn conflict(&self, _c: &Conflict) -> ConflictAnswer {
        ConflictAnswer { action: ConflictAction::Overwrite, all: true }
    }
    fn error(&self, e: &OpError) -> ErrorAction {
        self.errors.lock().unwrap().push(format!("{}: {}", e.path, e.message));
        ErrorAction::Cancel
    }
    fn cancelled(&self) -> bool {
        false
    }
}

fn host() -> TestHost {
    TestHost { errors: Mutex::new(vec![]), progress_calls: Mutex::new(0) }
}

fn enabled() -> bool {
    std::env::var("TWINYARD_FTP_TEST").is_ok()
}

fn setup(port: u16, tls: Tls) -> (String, PathBuf) {
    let dir = std::env::temp_dir().join(format!("twinyard-live-{}-{port}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: tests run with --test-threads=1.
    unsafe { std::env::set_var("TWINYARD_DATA", &dir) };
    let id = format!("t{port}");
    let store = Store {
        connections: vec![Connection {
            id: id.clone(),
            name: id.clone(),
            host: "127.0.0.1".into(),
            port,
            tls,
            user: "test".into(),
            initial_dir: String::new(),
            passive: true,
            mlsd: true,
            keepalive: 0,
            encoding: Default::default(),
        }],
        pins: vec![],
    };
    conn::save(&store).unwrap();
    (id, dir)
}

fn connect(r: &Remote, id: &str) {
    let _ = env_logger::builder().is_test(true).try_init();
    match r.connect(id, Some("pass".into()), false).unwrap() {
        ConnectResult::Ok { .. } => {}
        ConnectResult::Cert { sha256, reason, .. } => {
            println!("cert rejected as expected ({reason}), pinning {sha256}");
            r.trust(id, &sha256).unwrap();
            assert!(matches!(r.connect(id, Some("pass".into()), false).unwrap(), ConnectResult::Ok { .. }));
        }
        other => panic!("{other:?}"),
    }
}

fn make_tree(root: &Path) {
    std::fs::create_dir_all(root.join("pod katalog")).unwrap();
    let big: Vec<u8> = (0..1_000_000u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(root.join("duży plik.bin"), &big).unwrap();
    std::fs::write(root.join("pod katalog").join("zażółć.txt"), "gęślą jaźń").unwrap();
    std::fs::write(root.join("pusty.txt"), "").unwrap();
}

fn same_tree(a: &Path, b: &Path) {
    for f in ["duży plik.bin", "pod katalog/zażółć.txt", "pusty.txt"] {
        assert_eq!(std::fs::read(a.join(f)).unwrap(), std::fs::read(b.join(f)).unwrap(), "{f}");
    }
}

fn roundtrip(port: u16, tls: Tls) {
    if !enabled() {
        return;
    }
    let (id, dir) = setup(port, tls);
    let r = Remote::new();
    connect(&r, &id);

    let src = dir.join("src");
    make_tree(&src);
    let remote_dir = path::make(&id, &format!("/up{port}"));
    let ph = host();
    let pre = engine::run(&r, &OpSpec { kind: OpKind::Delete, sources: vec![remote_dir.clone().into()], dest: None }, &ph);
    println!("pre-clean: {:?} {:?}", pre, ph.errors.lock().unwrap());
    r.mkdir(&remote_dir).unwrap();

    // Upload the folder into /upNNNN.
    let h = host();
    let out = engine::run(&r, &OpSpec { kind: OpKind::Copy, sources: vec![src.clone()], dest: Some(remote_dir.clone().into()) }, &h);
    assert!(out.errors.is_empty() && h.errors.lock().unwrap().is_empty(), "{:?} {:?}", out.errors, h.errors);
    assert_eq!(out.done_files, 3);
    let l = r.list(&path::make(&id, &format!("/up{port}/src"))).unwrap();
    let mut names = l.names.clone();
    names.sort();
    assert_eq!(names, ["duży plik.bin", "pod katalog", "pusty.txt"]);
    // MFMT: the uploaded file keeps its local modification time.
    let i = l.names.iter().position(|n| n == "duży plik.bin").unwrap();
    let local_ms = std::fs::metadata(src.join("duży plik.bin")).unwrap().modified().unwrap()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    assert!((l.mtimes[i] - local_ms).abs() < 2000, "remote {} vs local {}", l.mtimes[i], local_ms);

    // Download it back.
    let dl = dir.join("dl");
    std::fs::create_dir_all(&dl).unwrap();
    let h = host();
    let out = engine::run(
        &r,
        &OpSpec { kind: OpKind::Copy, sources: vec![path::make(&id, &format!("/up{port}/src")).into()], dest: Some(dl.clone()) },
        &h,
    );
    assert!(out.errors.is_empty() && h.errors.lock().unwrap().is_empty(), "{:?} {:?}", out.errors, h.errors);
    same_tree(&src, &dl.join("src"));

    // Upload again over existing files (conflicts -> overwrite all).
    let out = engine::run(&r, &OpSpec { kind: OpKind::Copy, sources: vec![src.clone()], dest: Some(remote_dir.clone().into()) }, &host());
    assert!(out.errors.is_empty(), "{:?}", out.errors);

    // Rename on the server, then delete everything.
    r.rename(&path::make(&id, &format!("/up{port}/src")), &path::make(&id, &format!("/up{port}/renamed"))).unwrap();
    let out = engine::run(&r, &OpSpec { kind: OpKind::Delete, sources: vec![remote_dir.clone().into()], dest: None }, &host());
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert!(r.list(&path::make(&id, "/")).unwrap().names.iter().all(|n| *n != format!("up{port}")));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn plain_ftp() {
    roundtrip(2121, Tls::None);
}

#[test]
fn explicit_tls() {
    roundtrip(2122, Tls::Explicit);
}

#[test]
fn implicit_tls() {
    roundtrip(2990, Tls::Implicit);
}

/// Server 2123 kills the data connection once per file after 300 kB:
/// the engine must reconnect and resume without asking.
#[test]
fn resumes_after_dropped_connection() {
    if !enabled() {
        return;
    }
    let (id, dir) = setup(2123, Tls::None);
    let r = Remote::new();
    connect(&r, &id);
    let src = dir.join("src");
    make_tree(&src);
    let remote_dir = path::make(&id, "/flaky");
    let _ = engine::run(&r, &OpSpec { kind: OpKind::Delete, sources: vec![remote_dir.clone().into()], dest: None }, &host());
    r.mkdir(&remote_dir).unwrap();
    let out = engine::run(&r, &OpSpec { kind: OpKind::Copy, sources: vec![src.clone()], dest: Some(remote_dir.clone().into()) }, &host());
    assert!(out.errors.is_empty(), "{:?}", out.errors);

    let dl = dir.join("dl");
    std::fs::create_dir_all(&dl).unwrap();
    let h = host();
    let out = engine::run(&r, &OpSpec { kind: OpKind::Copy, sources: vec![path::make(&id, "/flaky/src").into()], dest: Some(dl.clone()) }, &h);
    assert!(out.errors.is_empty() && h.errors.lock().unwrap().is_empty(), "{:?} {:?}", out.errors, h.errors);
    same_tree(&src, &dl.join("src"));
    let _ = engine::run(&r, &OpSpec { kind: OpKind::Delete, sources: vec![remote_dir.into()], dest: None }, &host());
    std::fs::remove_dir_all(&dir).ok();
}

/// Configured as implicit, server is explicit (greets "220" in plain text):
/// must fall back to AUTH TLS by itself.
#[test]
fn implicit_falls_back_to_explicit() {
    if !enabled() {
        return;
    }
    let (id, dir) = setup(2122, Tls::Implicit);
    let r = Remote::new();
    connect(&r, &id);
    assert!(r.list(&path::make(&id, "/")).is_ok());
    std::fs::remove_dir_all(&dir).ok();
}

/// Server with static-RSA-only TLS (like old industrial/hosting servers):
/// rustls can't, the session must switch to SChannel and do a full roundtrip.
#[test]
fn old_rsa_only_server_uses_schannel() {
    roundtrip(2124, Tls::Explicit);
}

/// The user's important client, reproduced: plain-text greeting after ~3 s
/// (longer than the greeting probe), AUTH TLS, static RSA only — and the
/// connection configured as *implicit*. Must just work: implicit -> explicit
/// fallback on the "corrupt message", then rustls -> SChannel.
#[test]
fn slow_old_server_configured_implicit() {
    roundtrip(2125, Tls::Implicit);
}

#[test]
fn detected_mode_is_saved() {
    if !enabled() {
        return;
    }
    let (id, dir) = setup(2125, Tls::Implicit);
    let r = Remote::new();
    connect(&r, &id);
    let saved = conn::load().unwrap();
    assert_eq!(saved.get(&id).unwrap().tls, Tls::Explicit);
    std::fs::remove_dir_all(&dir).ok();
}

/// Old server storing names in Windows-1250 (port 2126), connection set to
/// CP1250: Polish names must survive upload, listing and download.
#[test]
fn cp1250_names() {
    if !enabled() {
        return;
    }
    let (id, dir) = setup(2126, Tls::None);
    let mut store = conn::load().unwrap();
    store.connections[0].encoding = conn::NameEncoding::Cp1250;
    conn::save(&store).unwrap();
    let r = Remote::new();
    connect(&r, &id);
    let src = dir.join("zażółć gęślą.txt");
    std::fs::write(&src, "treść").unwrap();
    let remote_dir = path::make(&id, "/cp");
    let _ = engine::run(&r, &OpSpec { kind: OpKind::Delete, sources: vec![remote_dir.clone().into()], dest: None }, &host());
    r.mkdir(&remote_dir).unwrap();
    let out = engine::run(&r, &OpSpec { kind: OpKind::Copy, sources: vec![src.clone()], dest: Some(remote_dir.clone().into()) }, &host());
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    let l = r.list(&remote_dir).unwrap();
    assert_eq!(l.names, ["zażółć gęślą.txt"]);
    let dl = dir.join("dl");
    std::fs::create_dir_all(&dl).unwrap();
    let out = engine::run(
        &r,
        &OpSpec { kind: OpKind::Copy, sources: vec![path::make(&id, "/cp/zażółć gęślą.txt").into()], dest: Some(dl.clone()) },
        &host(),
    );
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(std::fs::read_to_string(dl.join("zażółć gęślą.txt")).unwrap(), "treść");
    std::fs::remove_dir_all(&dir).ok();
}

/// Server -> server copy (relayed through a temp dir), copy within one server,
/// and a cross-server move that deletes the source only after success.
#[test]
fn server_to_server() {
    if !enabled() {
        return;
    }
    let (a, dir) = setup(2121, Tls::None);
    let mut store = conn::load().unwrap();
    let mut second = store.connections[0].clone();
    second.id = "t2126".into();
    second.name = "t2126".into();
    second.port = 2126;
    store.connections.push(second);
    conn::save(&store).unwrap();
    let r = Remote::new();
    connect(&r, &a);
    connect(&r, "t2126");

    let src = dir.join("src");
    make_tree(&src);
    let clean = |u: &str| {
        let _ = engine::run(&r, &OpSpec { kind: OpKind::Delete, sources: vec![u.into()], dest: None }, &host());
    };
    for d in ["/relayA", "/relayB", "/relayC"] {
        clean(&path::make(&a, d));
    }
    r.mkdir(&path::make(&a, "/relayA")).unwrap();
    r.mkdir(&path::make("t2126", "/relayB")).unwrap();
    r.mkdir(&path::make(&a, "/relayC")).unwrap();
    let out = engine::run(&r, &OpSpec { kind: OpKind::Copy, sources: vec![src.clone()], dest: Some(path::make(&a, "/relayA").into()) }, &host());
    assert!(out.errors.is_empty(), "{:?}", out.errors);

    // Cross-server copy.
    let h = host();
    let out = engine::run(
        &r,
        &OpSpec { kind: OpKind::Copy, sources: vec![path::make(&a, "/relayA/src").into()], dest: Some(path::make("t2126", "/relayB").into()) },
        &h,
    );
    assert!(out.errors.is_empty() && h.errors.lock().unwrap().is_empty(), "{:?} {:?}", out.errors, h.errors);
    let mut names = r.list(&path::make("t2126", "/relayB/src")).unwrap().names;
    names.sort();
    assert_eq!(names, ["duży plik.bin", "pod katalog", "pusty.txt"]);

    // Copy within one server.
    let out = engine::run(
        &r,
        &OpSpec { kind: OpKind::Copy, sources: vec![path::make(&a, "/relayA/src/pusty.txt").into()], dest: Some(path::make(&a, "/relayC").into()) },
        &host(),
    );
    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(r.list(&path::make(&a, "/relayC")).unwrap().names, ["pusty.txt"]);

    // Cross-server move is refused up front: nothing copied, source stays.
    let out = engine::run(
        &r,
        &OpSpec { kind: OpKind::Move, sources: vec![path::make("t2126", "/relayB/src/pusty.txt").into()], dest: Some(path::make(&a, "/relayC").into()) },
        &host(),
    );
    assert_eq!(out.errors.len(), 1, "{:?}", out.errors);
    assert!(r.list(&path::make("t2126", "/relayB/src")).unwrap().names.iter().any(|n| n == "pusty.txt"));
    for d in ["/relayA", "/relayB", "/relayC"] {
        clean(&path::make(&a, d));
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// Alt+F7 on FTP: names by mask and text inside small text files.
#[test]
fn remote_search() {
    if !enabled() {
        return;
    }
    let (id, dir) = setup(2121, Tls::None);
    let r = Remote::new();
    connect(&r, &id);
    let src = dir.join("src");
    make_tree(&src);
    let root = path::make(&id, "/szukaj");
    let _ = engine::run(&r, &OpSpec { kind: OpKind::Delete, sources: vec![root.clone().into()], dest: None }, &host());
    r.mkdir(&root).unwrap();
    let out = engine::run(&r, &OpSpec { kind: OpKind::Copy, sources: vec![src.clone()], dest: Some(root.clone().into()) }, &host());
    assert!(out.errors.is_empty(), "{:?}", out.errors);

    let exts: Vec<String> = twinyard_core::search::DEFAULT_TEXT_EXT.iter().map(|s| s.to_string()).collect();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let q = twinyard_core::search::Query {
        root: root.clone(),
        mask: "*.txt".into(),
        text: "JAŹŃ".into(),
        subdirs: true,
        case_sensitive: false,
        skip_noise: true,
    };
    let mut hits = Vec::new();
    twinyard_remote::search::run(&r, &q, &exts, &cancel, &mut |h| hits.push(h), &mut |_| {}).unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(hits[0].path.ends_with("/pod katalog/zażółć.txt") && hits[0].line == Some(1));

    let q = twinyard_core::search::Query { text: String::new(), mask: "*.bin".into(), ..q };
    let mut names = Vec::new();
    twinyard_remote::search::run(&r, &q, &exts, &cancel, &mut |h| names.push(h.path), &mut |_| {}).unwrap();
    assert_eq!(names.len(), 1);
    let _ = engine::run(&r, &OpSpec { kind: OpKind::Delete, sources: vec![root.into()], dest: None }, &host());
    std::fs::remove_dir_all(&dir).ok();
}
