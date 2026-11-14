//! Directory compare / sync: local↔local, local↔FTP, FTP↔FTP — any two
//! panel paths. Returns the differences; the UI decides directions and runs
//! the copies through the normal job engine.

use crate::blocking;
use twinyard_core::Listing;
use twinyard_core::listing::{ATTR_DIR, ATTR_REPARSE};
use twinyard_remote::{Remote, is_remote};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use tauri::State;

/// FAT / some FTP servers keep 2 s resolution.
const MTIME_TOLERANCE_MS: i64 = 2000;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Meta {
    size: u64,
    mtime: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// '/'-separated, relative to both roots.
    rel: String,
    dir: bool,
    left: Option<Meta>,
    right: Option<Meta>,
    /// same | leftOnly | rightOnly | leftNewer | rightNewer | differ
    state: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    entries: Vec<Entry>,
    left_files: u64,
    right_files: u64,
    /// Subdirectories that could not be read (access denied…), skipped.
    skipped: Vec<String>,
    /// A side is an FTP server without exact dates (no MLSD): "newer" is a
    /// guess there, the UI proposes no direction for it.
    coarse_dates: bool,
}

fn list_any(r: &Remote, path: &str) -> Result<Listing, String> {
    if is_remote(path) {
        r.list(path)
    } else {
        crate::list_local(path)
    }
}

fn join(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        return root.to_string();
    }
    if is_remote(root) {
        format!("{}/{}", root.trim_end_matches('/'), rel)
    } else {
        format!("{}\\{}", root.trim_end_matches('\\'), rel.replace('/', "\\"))
    }
}

/// rel -> (is_dir, meta). Directories that exist only on one side are not
/// descended into by the caller (they are copied as a whole).
fn level(r: &Remote, root: &str, rel: &str) -> Result<BTreeMap<String, (bool, Meta)>, String> {
    let l = list_any(r, &join(root, rel))?;
    let mut out = BTreeMap::new();
    for i in 0..l.names.len() {
        let dir = l.attrs[i] & ATTR_DIR != 0;
        if dir && l.attrs[i] & ATTR_REPARSE != 0 {
            continue; // junctions / FTP symlinks: never follow
        }
        let name = &l.names[i];
        let key = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        out.insert(key, (dir, Meta { size: l.sizes[i], mtime: l.mtimes[i] }));
    }
    Ok(out)
}

/// Byte-for-byte equality of two local files of the same size.
fn same_content(a: &str, b: &str) -> std::io::Result<bool> {
    use std::io::Read;
    let (mut fa, mut fb) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let n = fa.read(&mut ba)?;
        if n == 0 {
            return Ok(fb.read(&mut bb[..1])? == 0);
        }
        fb.read_exact(&mut bb[..n])?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

/// Plain local files (not FTP, not inside an archive) can be compared by content.
fn content_capable(root: &str) -> bool {
    !is_remote(root) && twinyard_core::archive::split(root).is_none()
}

fn compare(r: &Remote, left: &str, right: &str, subdirs: bool, ignore_dates: bool, content: bool) -> Result<Report, String> {
    let content = content && content_capable(left) && content_capable(right);
    let mut rep = Report { entries: Vec::new(), left_files: 0, right_files: 0, skipped: Vec::new(), coarse_dates: false };
    let mut queue = vec![String::new()];
    while let Some(rel) = queue.pop() {
        // The roots must be readable; deeper unreadable dirs are skipped.
        let (a, b) = match (level(r, left, &rel), level(r, right, &rel)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) if rel.is_empty() => return Err(e),
            (Err(e), _) | (_, Err(e)) => {
                rep.skipped.push(e);
                continue;
            }
        };
        // Case-insensitive pairing (Windows vs FTP case differences are rare
        // but a "Index.php" vs "index.php" pair is the same file for the user).
        let mut b_ci: BTreeMap<String, (String, bool, Meta)> =
            b.iter().map(|(k, (d, m))| (k.to_lowercase(), (k.clone(), *d, *m))).collect();
        for (k, (d, m)) in &a {
            if !d {
                rep.left_files += 1;
            }
            match b_ci.remove(&k.to_lowercase()) {
                None => rep.entries.push(Entry { rel: k.clone(), dir: *d, left: Some(*m), right: None, state: "leftOnly" }),
                Some((_, bd, _)) if *d && bd => {
                    if subdirs {
                        queue.push(k.clone());
                    }
                }
                Some((_, bd, bm)) if *d != bd => {
                    // File vs directory with the same name: show, never auto-sync.
                    rep.entries.push(Entry { rel: k.clone(), dir: *d, left: Some(*m), right: Some(bm), state: "differ" });
                }
                Some((_, _, bm)) => {
                    rep.right_files += 1;
                    let dt = m.mtime - bm.mtime;
                    // By content: equal size -> the bytes decide (dates only pick the direction).
                    let equal = if content && m.size == bm.size {
                        same_content(&join(left, k), &join(right, k)).ok()
                    } else {
                        None
                    };
                    let state = if equal == Some(true) {
                        "same"
                    } else if equal == Some(false) && (ignore_dates || dt.abs() <= MTIME_TOLERANCE_MS) {
                        "differ" // same size and date, other bytes
                    } else if m.size == bm.size && equal.is_none() && (ignore_dates || dt.abs() <= MTIME_TOLERANCE_MS) {
                        "same"
                    } else if ignore_dates {
                        "differ"
                    } else if dt > MTIME_TOLERANCE_MS {
                        "leftNewer"
                    } else if dt < -MTIME_TOLERANCE_MS {
                        "rightNewer"
                    } else {
                        "differ" // same time, different size
                    };
                    rep.entries.push(Entry { rel: k.clone(), dir: false, left: Some(*m), right: Some(bm), state });
                }
            }
        }
        for (_, (orig, d, m)) in b_ci {
            if !d {
                rep.right_files += 1;
            }
            rep.entries.push(Entry { rel: orig, dir: d, left: None, right: Some(m), state: "rightOnly" });
        }
    }
    rep.entries.sort_by(|x, y| twinyard_core::natural_cmp(&x.rel, &y.rel));
    rep.coarse_dates = [left, right]
        .iter()
        .filter_map(|p| twinyard_remote::path::split(p))
        .any(|(id, _)| !r.exact_dates(&id));
    Ok(rep)
}

#[tauri::command]
pub async fn compare_dirs(
    remote: State<'_, Arc<Remote>>,
    left: String,
    right: String,
    subdirs: bool,
    ignore_dates: bool,
    content: Option<bool>,
) -> Result<Report, String> {
    let r = remote.inner().clone();
    blocking(move || compare(&r, &left, &right, subdirs, ignore_dates, content.unwrap_or(false))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn touch(p: &std::path::Path, body: &str, age_s: u64) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(age_s)).unwrap();
    }

    fn states(rep: &Report) -> Vec<(String, &'static str)> {
        rep.entries.iter().map(|e| (e.rel.clone(), e.state)).collect()
    }

    #[test]
    fn local_vs_local() {
        let base = std::env::temp_dir().join(format!("twinyard-dircmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (l, r) = (base.join("L"), base.join("R"));
        touch(&l.join("same.txt"), "x", 100);
        touch(&r.join("same.txt"), "x", 100);
        touch(&l.join("newer.txt"), "new", 10);
        touch(&r.join("newer.txt"), "old", 1000);
        touch(&l.join("sub/only-left.txt"), "a", 5);
        touch(&r.join("sub/only-right.txt"), "b", 5);
        touch(&r.join("Same.CSS"), "c", 5);
        touch(&l.join("same.css"), "c", 5);
        std::fs::create_dir_all(l.join("dir-only-left/deep")).unwrap();
        let rr = Remote::new();
        let rep = compare(&rr, &l.display().to_string(), &r.display().to_string(), true, false, false).unwrap();
        assert_eq!(
            states(&rep),
            [
                ("dir-only-left".to_string(), "leftOnly"),
                ("newer.txt".to_string(), "leftNewer"),
                ("same.css".to_string(), "same"),
                ("same.txt".to_string(), "same"),
                ("sub/only-left.txt".to_string(), "leftOnly"),
                ("sub/only-right.txt".to_string(), "rightOnly"),
            ]
        );
        // Size-only mode: same size + different date = same.
        touch(&r.join("newer.txt"), "new", 1000);
        let rep = compare(&rr, &l.display().to_string(), &r.display().to_string(), true, true, false).unwrap();
        assert!(rep.entries.iter().any(|e| e.rel == "newer.txt" && e.state == "same"));
        // By content: same size + date but other bytes = differ; same bytes, other date = same.
        touch(&l.join("same.txt"), "y", 100);
        touch(&r.join("same.txt"), "x", 100);
        touch(&r.join("newer.txt"), "new", 5000);
        let rep = compare(&rr, &l.display().to_string(), &r.display().to_string(), true, false, true).unwrap();
        let st = |n: &str| rep.entries.iter().find(|e| e.rel == n).map(|e| e.state);
        assert_eq!(st("same.txt"), Some("differ"));
        assert_eq!(st("newer.txt"), Some("same"));
        assert_eq!(st("same.css"), Some("same"));
        std::fs::remove_dir_all(&base).ok();
    }

    /// Needs the local pyftpdlib test server (port 2121), see README "Testy FTP".
    #[test]
    fn local_vs_ftp() {
        if std::env::var("TWINYARD_FTP_TEST").is_err() {
            return;
        }
        use twinyard_remote::conn::{self, Connection, Store, Tls};
        let base = std::env::temp_dir().join(format!("twinyard-dircmp-ftp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        // SAFETY: only test in this binary touching TWINYARD_DATA.
        unsafe { std::env::set_var("TWINYARD_DATA", base.join("data")) };
        conn::save(&Store {
            connections: vec![Connection {
                id: "t".into(),
                name: "t".into(),
                host: "127.0.0.1".into(),
                port: 2121,
                tls: Tls::None,
                user: "test".into(),
                initial_dir: String::new(),
                passive: true,
                mlsd: true,
                keepalive: 0,
                encoding: Default::default(),
            }],
            pins: vec![],
        })
        .unwrap();
        let rr = Remote::new();
        rr.connect("t", Some("pass".into()), false).unwrap();
        let l = base.join("L");
        touch(&l.join("a.txt"), "aaa", 5);
        touch(&l.join("d/b.txt"), "bb", 5);
        let remote_root = "ftp://t/dircmp";
        let _ = rr.mkdir(remote_root);
        let rep = compare(&rr, &l.display().to_string(), remote_root, true, true, false).unwrap();
        let st = states(&rep);
        assert!(st.contains(&("a.txt".into(), "leftOnly")) && st.contains(&("d".into(), "leftOnly")), "{st:?}");
        std::fs::remove_dir_all(&base).ok();
    }
}
