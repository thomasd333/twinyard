//! Alt+F7 on FTP: walk the tree with a dedicated connection; names by mask,
//! text only in small text-type files (each one has to be downloaded).

use crate::ftp::{Entry, describe, is_connection_error};
use crate::manager::Remote;
use crate::path;
use twinyard_core::search::{Hit, Mask, Query, Stats};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

/// Text search reads at most this much per remote file.
pub const MAX_REMOTE_TEXT: u64 = 2 * 1024 * 1024;

const NOISE_DIRS: &[&str] = &[".git", ".svn", ".hg", "node_modules", "__pycache__", ".venv", ".cache"];

pub fn run(
    remote: &Remote,
    q: &Query,
    text_ext: &[String],
    cancel: &AtomicBool,
    on_hit: &mut dyn FnMut(Hit),
    on_stats: &mut dyn FnMut(&Stats),
) -> Result<Stats, String> {
    let (id, root) = path::split(&q.root).ok_or(twinyard_core::i18n::tr("Zły adres FTP", "Bad FTP address"))?;
    let mut sess = remote.open_session(&id)?;
    let mask = Mask::new(&q.mask);
    let needle = if q.case_sensitive { q.text.clone() } else { q.text.to_lowercase() };
    let mut st = Stats::default();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        st.dirs += 1;
        st.current = path::make(&id, &dir);
        on_stats(&st);
        let entries: Vec<Entry> = match sess.list(&dir) {
            Ok(e) => e,
            Err(e) if is_connection_error(&e) => {
                // One reconnect, then give up on this dir.
                sess = remote.open_session(&id)?;
                match sess.list(&dir) {
                    Ok(e) => e,
                    Err(e) => {
                        log::warn!("search {dir}: {}", describe(&e));
                        continue;
                    }
                }
            }
            Err(_) => continue, // no access etc.
        };
        let mut subdirs = Vec::new();
        for e in entries {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let full = path::join(&dir, &e.name);
            if e.dir {
                let noise = q.skip_noise && NOISE_DIRS.iter().any(|n| n.eq_ignore_ascii_case(&e.name));
                if q.subdirs && !noise && !e.link {
                    subdirs.push(full.clone());
                }
            } else {
                st.files += 1;
            }
            if !mask.matches(&e.name) {
                continue;
            }
            let mut hit = Hit { path: path::make(&id, &full), dir: e.dir, size: e.size, mtime: e.mtime, line: None, snippet: None };
            if !needle.is_empty() {
                let ext = e.name.rsplit_once('.').map(|(_, x)| x).unwrap_or("");
                if e.dir || e.size > MAX_REMOTE_TEXT || !text_ext.iter().any(|t| t.eq_ignore_ascii_case(ext)) {
                    continue;
                }
                st.read += 1;
                let mut bytes = Vec::new();
                let ok = sess.s.retr_as_stream(&full).and_then(|mut s| {
                    let _ = (&mut s).take(MAX_REMOTE_TEXT).read_to_end(&mut bytes);
                    s.finish()
                });
                if ok.is_err() || twinyard_core::text::looks_binary(&bytes[..bytes.len().min(8192)]) {
                    continue;
                }
                let (text, _) = twinyard_core::text::decode(&bytes);
                let found = text.lines().enumerate().find(|(_, l)| {
                    if q.case_sensitive { l.contains(&needle) } else { l.to_lowercase().contains(&needle) }
                });
                match found {
                    Some((i, l)) => {
                        hit.line = Some(i as u32 + 1);
                        hit.snippet = Some(l.trim().chars().take(200).collect());
                    }
                    None => continue,
                }
            }
            st.hits += 1;
            on_hit(hit);
        }
        subdirs.sort();
        stack.extend(subdirs.into_iter().rev());
    }
    let _ = sess.s.quit();
    st.current.clear();
    on_stats(&st);
    Ok(st)
}
