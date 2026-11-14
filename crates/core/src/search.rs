//! File search (Alt+F7): name masks + optional text inside files.
//! Text is searched only in files with a "text" extension (configurable) and
//! only if they don't look binary — videos, archives, databases are skipped.

use crate::listing::ATTR_REPARSE;
use crate::text;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

/// Extensions searched for text by default (`[search] text_ext` in config).
pub const DEFAULT_TEXT_EXT: &[&str] = &[
    "txt", "md", "log", "csv", "tsv", "json", "xml", "html", "htm", "xhtml", "css", "scss", "sass", "less", "js", "mjs",
    "cjs", "ts", "tsx", "jsx", "vue", "svelte", "php", "phtml", "inc", "tpl", "twig", "py", "rb", "pl", "java", "kt",
    "cs", "c", "h", "cpp", "hpp", "rs", "go", "swift", "sql", "ini", "cfg", "conf", "config", "toml", "yaml", "yml",
    "properties", "env", "htaccess", "sh", "bat", "cmd", "ps1", "psm1", "vbs", "reg", "svg", "srt", "tex", "rtf",
];

/// Text is not searched in files bigger than this.
pub const MAX_TEXT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    pub root: String,
    /// "*.php;*.js", "faktura", "" = everything.
    #[serde(default)]
    pub mask: String,
    /// Empty = names only.
    #[serde(default)]
    pub text: String,
    #[serde(default = "yes")]
    pub subdirs: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    /// Skip .git, node_modules, vendor caches….
    #[serde(default = "yes")]
    pub skip_noise: bool,
}

fn yes() -> bool {
    true
}

const NOISE_DIRS: &[&str] = &[".git", ".svn", ".hg", "node_modules", "__pycache__", ".venv", "target", ".cache"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub mtime: i64,
    /// Text search: first matching line (1-based) and its content.
    pub line: Option<u32>,
    pub snippet: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub dirs: u64,
    pub files: u64,
    /// Files whose content was read.
    pub read: u64,
    pub hits: u64,
    pub current: String,
}

/// Wildcard list matcher: `*.php;*.js`, `a?c`, plain text = "contains".
pub struct Mask(Vec<(bool, String)>);

impl Mask {
    pub fn new(mask: &str) -> Mask {
        Mask(
            mask.split([';', ' ', ','])
                .map(|p| p.trim().to_lowercase())
                .filter(|p| !p.is_empty() && p != "*" && p != "*.*")
                .map(|p| (p.contains(['*', '?']), p))
                .collect(),
        )
    }

    pub fn matches(&self, name: &str) -> bool {
        if self.0.is_empty() {
            return true;
        }
        let n = name.to_lowercase();
        self.0.iter().any(|(wild, p)| if *wild { wildcard(p.as_bytes(), n.as_bytes()) } else { n.contains(p.as_str()) })
    }
}

fn wildcard(p: &[u8], s: &[u8]) -> bool {
    // Iterative glob with backtracking on the last '*'.
    let (mut pi, mut si, mut star, mut mark) = (0, 0, None, 0);
    while si < s.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

pub fn run(
    q: &Query,
    text_ext: &[String],
    cancel: &AtomicBool,
    on_hit: &mut dyn FnMut(Hit),
    on_stats: &mut dyn FnMut(&Stats),
) -> Stats {
    let mask = Mask::new(&q.mask);
    let needle = if q.case_sensitive { q.text.clone() } else { q.text.to_lowercase() };
    let mut st = Stats::default();
    let mut stack = vec![std::path::PathBuf::from(&q.root)];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        st.dirs += 1;
        st.current = dir.display().to_string();
        on_stats(&st);
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut subdirs = Vec::new();
        for ent in rd.flatten() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let Ok(md) = ent.metadata() else { continue };
            let name = ent.file_name().to_string_lossy().into_owned();
            let is_dir = md.is_dir();
            if is_dir {
                let reparse = md.file_attributes() & ATTR_REPARSE != 0;
                let noise = q.skip_noise && NOISE_DIRS.iter().any(|n| n.eq_ignore_ascii_case(&name));
                if q.subdirs && !reparse && !noise {
                    subdirs.push(ent.path());
                }
            } else {
                st.files += 1;
            }
            if !mask.matches(&name) {
                continue;
            }
            let mtime = md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64);
            let mut hit = Hit { path: ent.path().display().to_string(), dir: is_dir, size: md.len(), mtime, line: None, snippet: None };
            if !needle.is_empty() {
                if is_dir || md.len() > MAX_TEXT_BYTES || !is_text_ext(&name, text_ext) {
                    continue;
                }
                st.read += 1;
                match find_text(&ent.path(), &needle, q.case_sensitive) {
                    Some((line, snippet)) => {
                        hit.line = Some(line);
                        hit.snippet = Some(snippet);
                    }
                    None => continue,
                }
            }
            st.hits += 1;
            on_hit(hit);
        }
        // Depth-first, alphabetical-ish order of visiting.
        subdirs.sort();
        stack.extend(subdirs.into_iter().rev());
    }
    st.current.clear();
    on_stats(&st);
    st
}

fn is_text_ext(name: &str, exts: &[String]) -> bool {
    let ext = match name.rfind('.') {
        Some(i) => &name[i + 1..],
        None => return false,
    };
    exts.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

fn find_text(path: &Path, needle: &str, case: bool) -> Option<(u32, String)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_TEXT_BYTES).read_to_end(&mut bytes).ok()?;
    if text::looks_binary(&bytes[..bytes.len().min(8192)]) {
        return None;
    }
    let (content, _) = text::decode(&bytes);
    for (i, line) in content.lines().enumerate() {
        let found = if case { line.contains(needle) } else { line.to_lowercase().contains(needle) };
        if found {
            let snippet: String = line.trim().chars().take(200).collect();
            return Some((i as u32 + 1, snippet));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks() {
        let m = Mask::new("*.php;*.JS");
        assert!(m.matches("index.php") && m.matches("a.js") && !m.matches("a.css"));
        assert!(Mask::new("").matches("x") && Mask::new("*.*").matches("noext"));
        assert!(Mask::new("fakt").matches("Faktura_2024.pdf"));
        assert!(Mask::new("a?c*").matches("abcdef") && !Mask::new("a?c").matches("abbc"));
        assert!(Mask::new("*.min.js").matches("x.min.js") && !Mask::new("*.min.js").matches("x.js"));
    }

    #[test]
    fn searches_names_and_text() {
        let root = std::env::temp_dir().join(format!("twinyard-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub").join("node_modules")).unwrap();
        std::fs::write(root.join("a.php"), "<?php\necho 'Zażółć';\n").unwrap();
        std::fs::write(root.join("sub").join("b.txt"), "nic\nZAŻÓŁĆ tutaj\n").unwrap();
        std::fs::write(root.join("sub").join("c.bin"), b"\0\0zazolc").unwrap();
        std::fs::write(root.join("sub").join("film.mp4"), "zażółć").unwrap(); // not a text ext
        std::fs::write(root.join("sub").join("node_modules").join("d.txt"), "zażółć").unwrap();
        let exts: Vec<String> = DEFAULT_TEXT_EXT.iter().map(|s| s.to_string()).collect();
        let cancel = AtomicBool::new(false);
        let mut hits = Vec::new();
        let q = Query { root: root.display().to_string(), mask: String::new(), text: "zażółć".into(), subdirs: true, case_sensitive: false, skip_noise: true };
        run(&q, &exts, &cancel, &mut |h| hits.push(h), &mut |_| {});
        let mut found: Vec<(String, u32)> =
            hits.iter().map(|h| (Path::new(&h.path).file_name().unwrap().to_string_lossy().into_owned(), h.line.unwrap())).collect();
        found.sort();
        assert_eq!(found, [("a.php".to_string(), 2), ("b.txt".to_string(), 2)]);

        let mut names = Vec::new();
        let q = Query { text: String::new(), mask: "*.txt".into(), ..q };
        run(&q, &exts, &cancel, &mut |h| names.push(h.path), &mut |_| {});
        assert_eq!(names.len(), 1, "node_modules skipped: {names:?}");
        std::fs::remove_dir_all(&root).ok();
    }
}
