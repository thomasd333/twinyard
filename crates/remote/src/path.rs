//! `ftp://<id>/abs/path` — remote paths as the panels see them.

pub const PREFIX: &str = "ftp://";

pub fn is_remote(p: &str) -> bool {
    p.len() >= PREFIX.len() && p[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
}

/// Splits into (connection id, normalized absolute path).
pub fn split(p: &str) -> Option<(String, String)> {
    if !is_remote(p) {
        return None;
    }
    let rest = p[PREFIX.len()..].replace('\\', "/");
    let (id, path) = match rest.find('/') {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => (rest.clone(), "/".to_string()),
    };
    if id.is_empty() {
        return None;
    }
    Some((id, normalize(&path)))
}

/// Absolute, no `.`/`..`/empty segments, no trailing slash (except root).
pub fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

pub fn make(id: &str, path: &str) -> String {
    format!("{PREFIX}{id}{}", normalize(path))
}

/// Remote path join (`dir` is absolute).
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

pub fn parent(path: &str) -> Option<String> {
    let p = normalize(path);
    if p == "/" {
        return None;
    }
    let i = p.rfind('/').unwrap_or(0);
    Some(if i == 0 { "/".into() } else { p[..i].to_string() })
}

pub fn file_name(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    &p[p.rfind('/').map_or(0, |i| i + 1)..]
}

/// Full panel URL of the parent dir, None at the server root.
pub fn parent_url(url: &str) -> Option<String> {
    let (id, path) = split(url)?;
    parent(&path).map(|p| make(&id, &p))
}

/// Resolves what the user typed while in a remote dir (relative or absolute).
pub fn resolve(base_url: &str, input: &str) -> Option<String> {
    if is_remote(input) {
        let (id, path) = split(input)?;
        return Some(make(&id, &path));
    }
    let (id, base) = split(base_url)?;
    let input = input.replace('\\', "/");
    let path = if input.starts_with('/') { input } else { join(&base, &input) };
    Some(make(&id, &path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_normalizes() {
        assert_eq!(split("ftp://srv/a/b/"), Some(("srv".into(), "/a/b".into())));
        assert_eq!(split("FTP://srv"), Some(("srv".into(), "/".into())));
        assert_eq!(split("ftp://srv/a/../b/./c"), Some(("srv".into(), "/b/c".into())));
        assert_eq!(split("ftp:///x"), None);
        assert_eq!(split("C:\\x"), None);
    }

    #[test]
    fn parents_and_resolve() {
        assert_eq!(parent_url("ftp://s/a/b").as_deref(), Some("ftp://s/a"));
        assert_eq!(parent_url("ftp://s/a").as_deref(), Some("ftp://s/"));
        assert_eq!(parent_url("ftp://s/"), None);
        assert_eq!(resolve("ftp://s/a", "b/c").as_deref(), Some("ftp://s/a/b/c"));
        assert_eq!(resolve("ftp://s/a", "..").as_deref(), Some("ftp://s/"));
        assert_eq!(resolve("ftp://s/a", "/x").as_deref(), Some("ftp://s/x"));
        assert_eq!(file_name("/a/b.txt"), "b.txt");
    }
}
