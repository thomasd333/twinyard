use serde::Serialize;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const ATTR_DIR: u32 = 0x10;
pub const ATTR_HIDDEN: u32 = 0x02;
pub const ATTR_SYSTEM: u32 = 0x04;
pub const ATTR_REPARSE: u32 = 0x400;

/// Directory listing in columnar form.
///
/// Columns instead of an array of objects: for 100k entries this roughly
/// halves the JSON size and makes parsing on the JS side much cheaper.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub path: String,
    pub names: Vec<String>,
    pub sizes: Vec<u64>,
    /// Unix time in milliseconds (f64-safe for JS).
    pub mtimes: Vec<i64>,
    /// Raw Windows FILE_ATTRIBUTE_* bits.
    pub attrs: Vec<u32>,
    /// Position of each entry in natural name order (see `natural`).
    /// Lets the UI sort by any column with cheap integer compares.
    pub name_rank: Vec<u32>,
    /// Entries that could not be stat'ed (permission denied etc.).
    pub errors: u32,
    pub elapsed_ms: f64,
}

/// FILETIME (100ns since 1601) -> unix ms.
pub(crate) fn filetime_to_unix_ms(ft: u64) -> i64 {
    const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;
    (ft as i64 - EPOCH_DIFF_100NS) / 10_000
}

/// Lists a local directory. On Windows `DirEntry::metadata` comes from
/// FindNextFileW data, so no extra syscall per entry is made.
pub fn list_dir(path: &Path) -> io::Result<Listing> {
    if let Some(server) = crate::unc::server_only(&path.to_string_lossy()) {
        return crate::unc::list_shares(server);
    }
    let start = Instant::now();
    let rd = std::fs::read_dir(path)?;
    let mut out = Listing {
        path: path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    for entry in rd {
        let Ok(entry) = entry else {
            out.errors += 1;
            continue;
        };
        let Ok(md) = entry.metadata() else {
            out.errors += 1;
            continue;
        };
        out.names.push(entry.file_name().to_string_lossy().into_owned());
        out.sizes.push(if md.is_dir() { 0 } else { md.file_size() });
        out.mtimes.push(filetime_to_unix_ms(md.last_write_time()));
        out.attrs.push(md.file_attributes());
    }
    out.finish(start);
    Ok(out)
}

impl Listing {
    /// Computes derived columns and timing. Call after filling the columns.
    pub fn finish(&mut self, start: Instant) {
        self.name_rank = crate::natural::name_ranks(&self.names);
        self.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
}

/// Parent directory, or None at a drive root (`C:\`) / UNC share root.
pub fn parent_of(path: &Path) -> Option<PathBuf> {
    if crate::unc::server_only(&path.to_string_lossy()).is_some() {
        return None; // \server is the top
    }
    // \server\share has no parent for Windows; for us it is \server (share list).
    path.parent().map(Path::to_path_buf).or_else(|| crate::unc::share_parent(path).map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_temp_dir() {
        let dir = std::env::temp_dir().join(format!("twinyard-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        let l = list_dir(&dir).unwrap();
        assert_eq!(l.names.len(), 2);
        let i = l.names.iter().position(|n| n == "a.txt").unwrap();
        assert_eq!(l.sizes[i], 5);
        assert_eq!(l.attrs[i] & ATTR_DIR, 0);
        let j = l.names.iter().position(|n| n == "sub").unwrap();
        assert_ne!(l.attrs[j] & ATTR_DIR, 0);
        assert!(l.mtimes[i] > 1_600_000_000_000);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parent_of_root_is_none() {
        assert!(parent_of(Path::new(r"C:\")).is_none());
        assert_eq!(parent_of(Path::new(r"C:\Windows")).unwrap(), Path::new(r"C:\"));
    }
}
