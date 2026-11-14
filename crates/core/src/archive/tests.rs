use super::*;
use std::sync::Mutex;

/// Answers every conflict with `action`, records errors (and skips them).
struct H {
    action: ConflictAction,
    errors: Mutex<Vec<String>>,
    /// Answers to password questions, in order; then None.
    passwords: Mutex<Vec<String>>,
    /// `wrong` flag of each password question.
    asked: Mutex<Vec<bool>>,
}

impl H {
    fn new(action: ConflictAction) -> Self {
        H { action, errors: Mutex::new(vec![]), passwords: Mutex::new(vec![]), asked: Mutex::new(vec![]) }
    }

    fn with_passwords(pw: &[&str]) -> Self {
        let h = H::new(ConflictAction::Overwrite);
        *h.passwords.lock().unwrap() = pw.iter().rev().map(|s| s.to_string()).collect();
        h
    }
}

impl Host for H {
    fn progress(&self, _: &Progress) {}
    fn conflict(&self, _: &Conflict) -> ConflictAnswer {
        ConflictAnswer { action: self.action, all: true }
    }
    fn error(&self, e: &OpError) -> ErrorAction {
        self.errors.lock().unwrap().push(e.message.clone());
        ErrorAction::Skip
    }
    fn cancelled(&self) -> bool {
        false
    }
    fn password(&self, _: &str, wrong: bool) -> Option<String> {
        self.asked.lock().unwrap().push(wrong);
        self.passwords.lock().unwrap().pop()
    }
}

fn base(name: &str) -> PathBuf {
    let b = std::env::temp_dir().join(format!("twinyard-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&b);
    fs::create_dir_all(&b).unwrap();
    b
}

fn spec(kind: OpKind, sources: &[String], dest: Option<String>) -> OpSpec {
    OpSpec { kind, sources: sources.iter().map(PathBuf::from).collect(), dest: dest.map(PathBuf::from) }
}

fn names(path: &str) -> Vec<String> {
    let mut n = list(path).unwrap().names;
    n.sort();
    n
}

fn read_inside(path: &str) -> String {
    fs::read_to_string(extract_to_temp(path).unwrap()).unwrap()
}

fn ok(out: &Outcome) {
    assert!(out.errors.is_empty() && !out.cancelled, "{out:?}");
}

#[test]
fn pack_list_extract_roundtrip() {
    let base = base("zip");
    let src = base.join("projekt");
    fs::create_dir_all(src.join("css").join("pusty")).unwrap();
    fs::write(src.join("index.php"), "<?php echo 'zażółć';").unwrap();
    fs::write(src.join("css").join("styl.css"), "a{}").unwrap();
    let zip = base.join("paczka.zip");
    let h = H::new(ConflictAction::Overwrite);

    let out = pack(&OpSpec { kind: OpKind::Pack, sources: vec![src.clone()], dest: Some(zip.clone()) }, &h);
    ok(&out);
    assert_eq!(out.done_files, 2);

    let root = list(&zip.display().to_string()).unwrap();
    assert_eq!(root.names, ["projekt"]);
    let inner = list(&format!("{}\\projekt", zip.display())).unwrap();
    let mut n = inner.names.clone();
    n.sort();
    assert_eq!(n, ["css", "index.php"]);
    let i = inner.names.iter().position(|n| n == "index.php").unwrap();
    let local_ms = mtime_ms(&fs::metadata(src.join("index.php")).unwrap());
    assert!((inner.mtimes[i] - local_ms).abs() <= 2000, "DOS time roundtrip {} vs {}", inner.mtimes[i], local_ms);
    assert!(list(&format!("{}\\nie-ma", zip.display())).is_err());

    // Extract one folder into another dir.
    let dst = base.join("out");
    fs::create_dir_all(&dst).unwrap();
    let out = extract(&spec(OpKind::Copy, &[format!("{}\\projekt\\css", zip.display())], Some(dst.display().to_string())), &h);
    ok(&out);
    assert_eq!(fs::read_to_string(dst.join("css").join("styl.css")).unwrap(), "a{}");
    assert!(dst.join("css").join("pusty").is_dir());

    // Single file under a new name.
    let out = extract(
        &spec(OpKind::Copy, &[format!("{}\\projekt\\index.php", zip.display())], Some(dst.join("kopia.php").display().to_string())),
        &h,
    );
    ok(&out);
    assert_eq!(fs::read_to_string(dst.join("kopia.php")).unwrap(), "<?php echo 'zażółć';");

    let t = extract_to_temp(&format!("{}\\projekt\\index.php", zip.display())).unwrap();
    assert!(t.exists());
    assert!(split("C:\\Windows").is_none());
    // The archive file itself is not "inside"; its root as a destination is.
    let z = zip.display().to_string();
    assert!(!is_inside(&z) && is_inside(&format!("{z}\\")) && is_inside(&format!("{z}\\projekt")));
    fs::remove_dir_all(&base).ok();
}

#[test]
fn zip_write_operations() {
    let base = base("zipw");
    let src = base.join("src");
    fs::create_dir_all(src.join("img")).unwrap();
    fs::write(src.join("a.txt"), "A").unwrap();
    fs::write(src.join("img").join("logo.svg"), "<svg/>").unwrap();
    let zip = base.join("z.zip");
    let z = zip.display().to_string();
    let h = H::new(ConflictAction::Overwrite);
    ok(&pack(&spec(OpKind::Pack, &[src.join("a.txt").display().to_string()], Some(z.clone())), &h));

    // Add into the root (dest = "z.zip\"), a folder too.
    let more = base.join("b.txt");
    fs::write(&more, "B").unwrap();
    ok(&add(&spec(OpKind::Copy, &[more.display().to_string(), src.join("img").display().to_string()], Some(format!("{z}\\"))), &h));
    assert_eq!(names(&z), ["a.txt", "b.txt", "img"]);
    assert_eq!(read_inside(&format!("{z}\\img\\logo.svg")), "<svg/>");
    assert_eq!(read_inside(&format!("{z}\\a.txt")), "A", "raw-copied entry still reads");

    // Conflict: overwrite replaces, rename keeps both.
    fs::write(&more, "B2").unwrap();
    ok(&add(&spec(OpKind::Copy, &[more.display().to_string()], Some(format!("{z}\\"))), &h));
    assert_eq!(read_inside(&format!("{z}\\b.txt")), "B2");
    assert_eq!(names(&z), ["a.txt", "b.txt", "img"]);
    let hr = H::new(ConflictAction::Rename);
    ok(&add(&spec(OpKind::Copy, &[more.display().to_string()], Some(format!("{z}\\"))), &hr));
    assert_eq!(names(&z), ["a.txt", "b (2).txt", "b.txt", "img"]);
    let hs = H::new(ConflictAction::Skip);
    let out = add(&spec(OpKind::Copy, &[more.display().to_string()], Some(format!("{z}\\"))), &hs);
    assert_eq!(out.skipped, 1);

    // Single file under a new name, into a subfolder.
    ok(&add(&spec(OpKind::Copy, &[more.display().to_string()], Some(format!("{z}\\img\\nowy.txt"))), &h));
    assert_eq!(names(&format!("{z}\\img")), ["logo.svg", "nowy.txt"]);

    // F8: delete; the shown folder survives losing its last files.
    ok(&delete(&spec(OpKind::Delete, &[format!("{z}\\b (2).txt")], None), &h));
    assert_eq!(names(&z), ["a.txt", "b.txt", "img"]);
    ok(&delete(&spec(OpKind::Delete, &[format!("{z}\\img\\logo.svg"), format!("{z}\\img\\nowy.txt")], None), &h));
    assert!(names(&format!("{z}\\img")).is_empty());

    // F7 / rename inside.
    mkdir(&format!("{z}\\nowy\\głęboki")).unwrap();
    assert!(mkdir(&format!("{z}\\nowy")).is_err());
    assert_eq!(names(&format!("{z}\\nowy")), ["głęboki"]);
    rename(&format!("{z}\\nowy"), &format!("{z}\\stary")).unwrap();
    assert_eq!(names(&z), ["a.txt", "b.txt", "img", "stary"]);
    rename(&format!("{z}\\a.txt"), &format!("{z}\\ą.txt")).unwrap();
    assert_eq!(read_inside(&format!("{z}\\ą.txt")), "A");
    assert!(rename(&format!("{z}\\b.txt"), &format!("{z}\\ą.txt")).is_err());

    // F6 into the archive removes the source; F6 out removes the entry.
    let mv = base.join("move-me.txt");
    fs::write(&mv, "M").unwrap();
    ok(&add(&spec(OpKind::Move, &[mv.display().to_string()], Some(format!("{z}\\stary\\"))), &h));
    assert!(!mv.exists());
    assert_eq!(names(&format!("{z}\\stary")), ["głęboki", "move-me.txt"]);
    let out_dir = base.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    ok(&extract(&spec(OpKind::Move, &[format!("{z}\\stary\\move-me.txt")], Some(out_dir.display().to_string())), &h));
    assert_eq!(fs::read_to_string(out_dir.join("move-me.txt")).unwrap(), "M");
    assert_eq!(names(&format!("{z}\\stary")), ["głęboki"]);

    // Archive -> archive.
    let z2 = base.join("z2.zip").display().to_string();
    ok(&pack(&spec(OpKind::Pack, &[more.display().to_string()], Some(z2.clone())), &h));
    ok(&between(&spec(OpKind::Copy, &[format!("{z}\\ą.txt"), format!("{z}\\stary")], Some(format!("{z2}\\"))), &h));
    assert_eq!(names(&z2), ["b.txt", "stary", "ą.txt"]);
    assert_eq!(read_inside(&format!("{z2}\\ą.txt")), "A");

    // Through the generic entry point (as the UI calls it).
    ok(&crate::ops::run(&spec(OpKind::Trash, &[format!("{z2}\\b.txt")], None), &h));
    assert_eq!(names(&z2), ["stary", "ą.txt"]);
    assert!(!base.join("z.zip.part").exists() && !base.join("z2.zip.part").exists());
    assert!(h.errors.lock().unwrap().is_empty());
    fs::remove_dir_all(&base).ok();
}

#[test]
fn sevenz_list_extract() {
    let base = base("7z");
    let src = base.join("dane");
    fs::create_dir_all(src.join("pod")).unwrap();
    fs::write(src.join("x.txt"), "iks").unwrap();
    fs::write(src.join("pod").join("żółw.txt"), "zółw".repeat(1000)).unwrap();
    let ar = base.join("a.7z");
    sevenz_rust2::compress_to_path(&src, &ar).unwrap();
    let a = ar.display().to_string();
    let root = names(&a);
    // compress_to_path stores the folder's contents.
    assert_eq!(root, ["pod", "x.txt"]);
    assert_eq!(read_inside(&format!("{a}\\pod\\żółw.txt")), "zółw".repeat(1000));
    let out = base.join("out");
    fs::create_dir_all(&out).unwrap();
    let h = H::new(ConflictAction::Overwrite);
    ok(&crate::ops::run(&spec(OpKind::Copy, &[format!("{a}\\pod"), format!("{a}\\x.txt")], Some(out.display().to_string())), &h));
    assert_eq!(fs::read_to_string(out.join("x.txt")).unwrap(), "iks");
    assert!(out.join("pod").join("żółw.txt").is_file());
    // Read-only.
    assert!(!crate::ops::run(&spec(OpKind::Delete, &[format!("{a}\\x.txt")], None), &h).errors.is_empty());
    assert!(mkdir(&format!("{a}\\nowy")).is_err());
    fs::remove_dir_all(&base).ok();
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name).display().to_string()
}

#[test]
fn rar_list_extract() {
    for f in ["solid.rar", "unicode.rar", "version.rar"] {
        let a = fixture(f);
        let l = list(&a).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert!(!l.names.is_empty(), "{f}");
        let base = base(&format!("rar-{f}"));
        let h = H::new(ConflictAction::Overwrite);
        let sources: Vec<String> = l.names.iter().map(|n| format!("{a}\\{n}")).collect();
        let out = crate::ops::run(&spec(OpKind::Copy, &sources, Some(base.display().to_string())), &h);
        ok(&out);
        for (i, n) in l.names.iter().enumerate() {
            if l.attrs[i] & ATTR_DIR == 0 {
                assert_eq!(fs::metadata(base.join(n)).unwrap().len(), l.sizes[i], "{f}/{n}");
            }
        }
        eprintln!("{f}: {:?} {:?}", l.names, l.sizes);
        fs::remove_dir_all(&base).ok();
    }
}

#[test]
fn rar_passwords() {
    // Encrypted data (password "unrar"): names list freely.
    let a = fixture("crypted.rar");
    let l = list(&a).unwrap();
    let base = base("rar-crypted");
    let src = format!("{a}\\{}", l.names[0]);
    let to = Some(base.display().to_string());
    // No password given: a clear error, no half file.
    let h = H::new(ConflictAction::Overwrite);
    let out = crate::ops::run(&spec(OpKind::Copy, std::slice::from_ref(&src), to.clone()), &h);
    assert!(out.errors.iter().any(|e| e.message.starts_with("Wymagane hasło do archiwum")), "{out:?}");
    assert!(!base.join(&l.names[0]).exists());
    // Wrong, then right.
    let h = H::with_passwords(&["zle", "unrar"]);
    let out = crate::ops::run(&spec(OpKind::Copy, std::slice::from_ref(&src), to.clone()), &h);
    ok(&out);
    assert_eq!(*h.asked.lock().unwrap(), [false, true]);
    assert_eq!(fs::metadata(base.join(&l.names[0])).unwrap().len(), l.sizes[0]);
    // Remembered for the session: F3 (no host to ask) works now.
    assert!(extract_to_temp(&src).is_ok());

    // Encrypted headers (password "password"): even listing needs it.
    let a = fixture("comment-hpw-password.rar");
    let e = list(&a).unwrap_err().to_string();
    assert!(e.starts_with("Wymagane hasło do archiwum"), "{e}");
    set_password(&a, "zle");
    assert!(list(&a).unwrap_err().to_string().starts_with("Złe hasło"));
    set_password(&a, "password");
    let l = list(&a).unwrap();
    assert!(!l.names.is_empty());
    fs::remove_dir_all(&base).ok();
}

#[test]
fn zip_aes_password() {
    use zip::write::SimpleFileOptions;
    let base = base("zip-aes");
    let z = base.join("tajne.zip");
    {
        let mut w = zip::ZipWriter::new(fs::File::create(&z).unwrap());
        let o = SimpleFileOptions::default().with_aes_encryption(zip::AesMode::Aes256, "sezam");
        w.start_file("a.txt", o).unwrap();
        w.write_all("zażółć".as_bytes()).unwrap();
        w.start_file("b.txt", o).unwrap();
        w.write_all(b"B").unwrap();
        w.finish().unwrap();
    }
    let zs = z.display().to_string();
    assert_eq!(names(&zs), ["a.txt", "b.txt"], "names are not encrypted in ZIP");
    let e = extract_to_temp(&format!("{zs}\\a.txt")).unwrap_err().to_string();
    assert!(e.starts_with("Wymagane hasło do archiwum"), "{e}");

    // Adding to it keeps the old entries encrypted and readable.
    let extra = base.join("nowy.txt");
    fs::write(&extra, "N").unwrap();
    ok(&add(&spec(OpKind::Copy, &[extra.display().to_string()], Some(format!("{zs}\\"))), &H::new(ConflictAction::Overwrite)));

    let out = base.join("out");
    fs::create_dir_all(&out).unwrap();
    let h = H::with_passwords(&["zle", "sezam"]);
    let srcs = [format!("{zs}\\a.txt"), format!("{zs}\\b.txt"), format!("{zs}\\nowy.txt")];
    ok(&crate::ops::run(&spec(OpKind::Copy, &srcs, Some(out.display().to_string())), &h));
    assert_eq!(*h.asked.lock().unwrap(), [false, true], "asked once more after the wrong one, not per file");
    assert_eq!(fs::read_to_string(out.join("a.txt")).unwrap(), "zażółć");
    assert_eq!(fs::read_to_string(out.join("b.txt")).unwrap(), "B");
    assert_eq!(fs::read_to_string(out.join("nowy.txt")).unwrap(), "N");
    assert_eq!(read_inside(&format!("{zs}\\a.txt")), "zażółć", "remembered for F3");
    fs::remove_dir_all(&base).ok();
}

#[test]
fn sevenz_password() {
    let base = base("7z-pw");
    let src = base.join("dane");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("x.txt"), "iks".repeat(1000)).unwrap();
    let ar = base.join("tajne.7z");
    sevenz_rust2::compress_to_path_encrypted(&src, &ar, sevenz_rust2::Password::from("sezam")).unwrap();
    let a = ar.display().to_string();
    let out = base.join("out");
    fs::create_dir_all(&out).unwrap();
    let h = H::with_passwords(&["zle", "sezam"]);
    // Listing may need it too (header encryption) — the job asks in both cases.
    ok(&crate::ops::run(&spec(OpKind::Copy, &[format!("{a}\\x.txt")], Some(out.display().to_string())), &h));
    assert_eq!(fs::read_to_string(out.join("x.txt")).unwrap(), "iks".repeat(1000));
    assert_eq!(h.asked.lock().unwrap().last(), Some(&true));
    fs::remove_dir_all(&base).ok();
}

struct Cancelling;
impl Host for Cancelling {
    fn progress(&self, _: &Progress) {}
    fn conflict(&self, _: &Conflict) -> ConflictAnswer {
        ConflictAnswer { action: ConflictAction::Overwrite, all: true }
    }
    fn error(&self, _: &OpError) -> ErrorAction {
        ErrorAction::Cancel
    }
    fn cancelled(&self) -> bool {
        true
    }
}

#[test]
fn zip_cancel_leaves_original() {
    let base = base("zipc");
    let f = base.join("a.txt");
    fs::write(&f, "A").unwrap();
    let z = base.join("z.zip").display().to_string();
    ok(&pack(&spec(OpKind::Pack, &[f.display().to_string()], Some(z.clone())), &H::new(ConflictAction::Overwrite)));
    let before = fs::read(&z).unwrap();
    let big = base.join("big.bin");
    fs::write(&big, vec![7u8; 3 << 20]).unwrap();
    let out = add(&spec(OpKind::Move, &[big.display().to_string()], Some(format!("{z}\\"))), &Cancelling);
    assert!(out.cancelled);
    assert_eq!(fs::read(&z).unwrap(), before);
    assert!(big.exists(), "move cancelled: source kept");
    assert!(!base.join("z.zip.part").exists());
    let out = delete(&spec(OpKind::Delete, &[format!("{z}\\a.txt")], None), &Cancelling);
    assert!(out.cancelled);
    assert_eq!(names(&z), ["a.txt"]);
    fs::remove_dir_all(&base).ok();
}

/// Interop: a ZIP made by .NET (Compress-Archive), edited by us, must still
/// open in .NET. `cargo test interop -- --ignored` (spawns PowerShell).
#[test]
#[ignore]
fn zip_interop_with_dotnet() {
    let base = base("zipi");
    let src = base.join("src");
    fs::create_dir_all(src.join("pod")).unwrap();
    fs::write(src.join("pod").join("zażółć.txt"), "gęślą jaźń".repeat(500)).unwrap();
    fs::write(src.join("b.txt"), "B").unwrap();
    let z = base.join("net.zip");
    let ps = |cmd: String| {
        let o = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &cmd]).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    ps(format!("Compress-Archive -Path '{}\\*' -DestinationPath '{}'", src.display(), z.display()));
    let zs = z.display().to_string();
    assert_eq!(names(&zs), ["b.txt", "pod"]);
    let h = H::new(ConflictAction::Overwrite);
    let extra = base.join("nowy.txt");
    fs::write(&extra, "N").unwrap();
    ok(&add(&spec(OpKind::Copy, &[extra.display().to_string()], Some(format!("{zs}\\pod\\"))), &h));
    ok(&delete(&spec(OpKind::Delete, &[format!("{zs}\\b.txt")], None), &h));
    rename(&format!("{zs}\\pod"), &format!("{zs}\\katalog")).unwrap();
    let out = base.join("out");
    ps(format!("Expand-Archive -Path '{}' -DestinationPath '{}'", z.display(), out.display()));
    assert_eq!(fs::read_to_string(out.join("katalog").join("zażółć.txt")).unwrap(), "gęślą jaźń".repeat(500));
    assert_eq!(fs::read_to_string(out.join("katalog").join("nowy.txt")).unwrap(), "N");
    assert!(!out.join("b.txt").exists());
    fs::remove_dir_all(&base).ok();
}

/// Builds a tar with a nested folder, a Polish name, a >100-char path (GNU long
/// name) and a symlink (must be skipped).
fn make_tar(w: impl Write) -> io::Result<()> {
    let mut b = tar::Builder::new(w);
    let mut add = |name: &str, data: &[u8]| {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(1_700_000_000);
        h.set_entry_type(tar::EntryType::Regular);
        b.append_data(&mut h, name, data)
    };
    add("projekt/index.php", b"<?php echo 1;")?;
    add("projekt/css/żółw.css", b"a{}")?;
    let long = format!("projekt/{}/plik.txt", "d".repeat(120));
    add(&long, b"long")?;
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Directory);
    h.set_size(0);
    h.set_mode(0o755);
    b.append_data(&mut h, "projekt/pusty/", io::empty())?;
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Symlink);
    h.set_size(0);
    b.append_link(&mut h, "projekt/link", "index.php")?;
    b.into_inner()?.flush()
}

#[test]
fn tar_and_tgz_list_extract() {
    let base = base("tar");
    let tar_path = base.join("a.tar");
    make_tar(fs::File::create(&tar_path).unwrap()).unwrap();
    for name in ["b.tar.gz", "c.tgz"] {
        let gz = flate2::write::GzEncoder::new(fs::File::create(base.join(name)).unwrap(), flate2::Compression::default());
        make_tar(gz).unwrap();
    }
    let h = H::new(ConflictAction::Overwrite);
    for name in ["a.tar", "b.tar.gz", "c.tgz"] {
        let ar = base.join(name).display().to_string();
        assert_eq!(names(&ar), ["projekt"], "{name}");
        assert_eq!(names(&format!("{ar}\\projekt")), ["css", "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd", "index.php", "pusty"], "{name}: link skipped");
        let l = list(&format!("{ar}\\projekt\\css")).unwrap();
        assert_eq!(l.names, ["żółw.css"]);
        assert_eq!(l.sizes, [3]);
        assert_eq!(l.mtimes, [1_700_000_000_000]);
        assert_eq!(read_inside(&format!("{ar}\\projekt\\index.php")), "<?php echo 1;");

        // F5 of the whole folder, then a single file under a new name.
        let out_dir = base.join(format!("out-{name}"));
        fs::create_dir_all(&out_dir).unwrap();
        let out = extract(&spec(OpKind::Copy, &[format!("{ar}\\projekt")], Some(out_dir.display().to_string())), &h);
        ok(&out);
        assert_eq!(out.done_files, 3);
        let p = out_dir.join("projekt");
        assert_eq!(fs::read_to_string(p.join("css").join("żółw.css")).unwrap(), "a{}");
        assert_eq!(fs::read_to_string(p.join("d".repeat(120)).join("plik.txt")).unwrap(), "long");
        assert!(p.join("pusty").is_dir());
        assert!(!p.join("link").exists());
        let one = base.join(format!("one-{name}.php"));
        ok(&extract(&spec(OpKind::Copy, &[format!("{ar}\\projekt\\index.php")], Some(one.display().to_string())), &h));
        assert_eq!(fs::read_to_string(&one).unwrap(), "<?php echo 1;");

        // Read-only: no moving out, no adding, no deleting.
        let mv = extract(&spec(OpKind::Move, &[format!("{ar}\\projekt\\index.php")], Some(out_dir.display().to_string())), &h);
        assert!(!mv.errors.is_empty());
        let del = delete(&spec(OpKind::Delete, &[format!("{ar}\\projekt\\index.php")], None), &h);
        assert!(!del.errors.is_empty());
        assert!(mkdir(&format!("{ar}\\projekt\\nowy")).is_err());
    }
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn tar_cache_follows_changes() {
    let base = base("tarcache");
    let ar = base.join("x.tar");
    let write = |files: &[&str]| {
        let mut b = tar::Builder::new(fs::File::create(&ar).unwrap());
        for f in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(1);
            h.set_mode(0o644);
            b.append_data(&mut h, f, &b"x"[..]).unwrap();
        }
        b.into_inner().unwrap().flush().unwrap();
    };
    write(&["a.txt"]);
    assert_eq!(names(&ar.display().to_string()), ["a.txt"]);
    write(&["a.txt", "b.txt"]); // other size -> cache miss
    assert_eq!(names(&ar.display().to_string()), ["a.txt", "b.txt"]);
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn lone_gz() {
    let base = base("gz");
    let gz = base.join("access.log.gz");
    let body = "linia\n".repeat(10_000);
    let mut e = flate2::write::GzEncoder::new(fs::File::create(&gz).unwrap(), flate2::Compression::default());
    e.write_all(body.as_bytes()).unwrap();
    e.finish().unwrap();
    let l = list(&gz.display().to_string()).unwrap();
    assert_eq!(l.names, ["access.log"]);
    assert_eq!(l.sizes, [body.len() as u64]);
    assert_eq!(read_inside(&format!("{}\\access.log", gz.display())), body);
    let h = H::new(ConflictAction::Overwrite);
    ok(&extract(&spec(OpKind::Copy, &[format!("{}\\access.log", gz.display())], Some(base.display().to_string())), &h));
    assert_eq!(fs::read_to_string(base.join("access.log")).unwrap(), body);
    // Broken data: an error, no half file left behind.
    let bad = base.join("bad.txt.gz");
    let mut bytes = fs::read(&gz).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid..mid + 20].fill(0xAA);
    fs::write(&bad, bytes).unwrap();
    let out_dir = base.join("bad-out");
    fs::create_dir_all(&out_dir).unwrap();
    let out = extract(&spec(OpKind::Copy, &[format!("{}\\bad.txt", bad.display())], Some(out_dir.display().to_string())), &h);
    assert!(!out.errors.is_empty(), "{out:?}");
    assert!(!out_dir.join("bad.txt").exists());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn zip_overwrite_with_locked_source_keeps_old_entry() {
    use std::os::windows::fs::OpenOptionsExt;
    let base = base("ziplock");
    let src = base.join("a.txt");
    fs::write(&src, "stara").unwrap();
    let zip = base.join("p.zip");
    let h = H::new(ConflictAction::Overwrite);
    ok(&pack(&spec(OpKind::Pack, &[src.display().to_string()], Some(zip.display().to_string())), &h));
    fs::write(&src, "nowa").unwrap();
    // Another program holds the file without sharing it.
    let held = fs::OpenOptions::new().read(true).share_mode(0).open(&src).unwrap();
    let inside = format!("{}\\", zip.display());
    let out = add(&spec(OpKind::Copy, &[src.display().to_string()], Some(inside)), &h);
    assert_eq!(out.errors.len(), 1, "{out:?}");
    drop(held);
    assert_eq!(read_inside(&format!("{}/a.txt", zip.display())), "stara");
}

#[test]
fn zip_parallel_changes_all_land() {
    let base = base("zippar");
    let zip = base.join("p.zip");
    let seed = base.join("seed.txt");
    fs::write(&seed, "x").unwrap();
    let h = H::new(ConflictAction::Overwrite);
    ok(&pack(&spec(OpKind::Pack, &[seed.display().to_string()], Some(zip.display().to_string())), &h));
    let files: Vec<PathBuf> = (0..6)
        .map(|i| {
            let f = base.join(format!("f{i}.txt"));
            fs::write(&f, vec![b'a' + i as u8; 200_000]).unwrap();
            f
        })
        .collect();
    std::thread::scope(|s| {
        for f in &files {
            let (zip, h) = (&zip, &h);
            s.spawn(move || ok(&add(&spec(OpKind::Copy, &[f.display().to_string()], Some(format!("{}\\", zip.display()))), h)));
        }
    });
    let n = names(&zip.display().to_string());
    assert_eq!(n.len(), 7, "{n:?}");
}

#[test]
fn zip_bad_target_names_refused() {
    let base = base("zipname");
    let src = base.join("a.txt");
    fs::write(&src, "x").unwrap();
    let zip = base.join("p.zip");
    let h = H::new(ConflictAction::Overwrite);
    ok(&pack(&spec(OpKind::Pack, &[src.display().to_string()], Some(zip.display().to_string())), &h));
    let z = zip.display().to_string();
    assert!(rename(&format!("{z}/a.txt"), &format!("{z}/../x.txt")).is_err());
    assert_eq!(names(&z), vec!["a.txt".to_string()]);
    fs::write(base.join("B.txt"), "y").unwrap();
    ok(&add(&spec(OpKind::Copy, &[base.join("B.txt").display().to_string()], Some(format!("{z}\\"))), &h));
    // Case-insensitive clash, as on Windows.
    assert!(rename(&format!("{z}/a.txt"), &format!("{z}/b.TXT")).is_err());
}
