//! Saved connections and pinned certificates (`<data dir>/connections.json`).
//! Passwords are never stored here — see `creds` (DPAPI-encrypted passwords.json).

use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Tls {
    /// Plain FTP.
    None,
    /// AUTH TLS on the normal port (FTPES).
    #[default]
    Explicit,
    /// TLS from the first byte, usually port 990.
    Implicit,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    /// Stable, URL-safe; used in panel paths (`ftp://<id>/…`).
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub tls: Tls,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub initial_dir: String,
    #[serde(default = "yes")]
    pub passive: bool,
    /// Use MLSD when the server supports it (exact sizes/dates); off = LIST.
    #[serde(default = "yes")]
    pub mlsd: bool,
    /// Keepalive interval in seconds (0 = off).
    #[serde(default)]
    pub keepalive: u32,
    /// File-name encoding on the server.
    #[serde(default)]
    pub encoding: NameEncoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NameEncoding {
    /// UTF-8 if the server announces it, otherwise detect (UTF-8 / CP1250).
    #[default]
    Auto,
    Utf8,
    Cp1250,
}

/// A trusted certificate for `host:port` (SHA-256 or, from TC imports, SHA-1),
/// stored as uppercase hex without separators.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    pub host_port: String,
    pub hash: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub pins: Vec<Pin>,
}

pub fn store_path() -> PathBuf {
    twinyard_core::config::config_path().with_file_name("connections.json")
}

pub fn load() -> io::Result<Store> {
    match std::fs::read_to_string(store_path()) {
        Ok(s) => serde_json::from_str(s.trim_start_matches('\u{feff}')).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(e),
    }
}

pub fn save(store: &Store) -> io::Result<()> {
    let path = store_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(store).map_err(io::Error::other)?)?;
    twinyard_core::win::move_path(&tmp, &path, true)
}

impl Store {
    pub fn get(&self, id: &str) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id.eq_ignore_ascii_case(id))
    }

    pub fn pins_for(&self, host_port: &str) -> Vec<String> {
        self.pins.iter().filter(|p| p.host_port.eq_ignore_ascii_case(host_port)).map(|p| p.hash.clone()).collect()
    }

    pub fn add_pin(&mut self, host_port: &str, hash: &str) {
        let pin = Pin { host_port: host_port.to_ascii_lowercase(), hash: normalize_hash(hash) };
        if !self.pins.contains(&pin) {
            self.pins.push(pin);
        }
    }

    /// Fills in a unique id for connections that have none (new / imported).
    pub fn assign_ids(&mut self) {
        for i in 0..self.connections.len() {
            if !self.connections[i].id.is_empty() {
                continue;
            }
            let base = slug(&self.connections[i].name);
            let mut id = base.clone();
            let mut n = 2;
            while self.connections.iter().any(|c| c.id.eq_ignore_ascii_case(&id)) {
                id = format!("{base}-{n}");
                n += 1;
            }
            self.connections[i].id = id;
        }
    }
}

impl Connection {
    pub fn host_port(&self) -> String {
        format!("{}:{}", self.host.to_ascii_lowercase(), self.port)
    }
}

pub fn normalize_hash(h: &str) -> String {
    h.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_ascii_uppercase()
}

/// "Mój hosting (nowy)" -> "moj-hosting-nowy".
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        let c = match c.to_lowercase().next().unwrap_or(c) {
            'ą' => 'a',
            'ć' => 'c',
            'ę' => 'e',
            'ł' => 'l',
            'ń' => 'n',
            'ó' => 'o',
            'ś' => 's',
            'ź' | 'ż' => 'z',
            c => c,
        };
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "ftp".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Mój hosting (nowy) "), "moj-hosting-nowy");
        assert_eq!(slug("  ŻÓŁW:21 "), "zolw-21");
        assert_eq!(slug("!!!"), "ftp");
    }

    #[test]
    fn unique_ids() {
        let c = |n: &str| Connection {
            id: String::new(),
            name: n.into(),
            host: "h".into(),
            port: 21,
            tls: Tls::None,
            user: String::new(),
            initial_dir: String::new(),
            passive: true,
            mlsd: true,
            keepalive: 0,
            encoding: NameEncoding::Auto,
        };
        let mut s = Store { connections: vec![c("A b"), c("a-b"), c("A B")], pins: vec![] };
        s.assign_ids();
        let ids: Vec<_> = s.connections.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["a-b", "a-b-2", "a-b-3"]);
    }

    #[test]
    fn hashes_normalize() {
        assert_eq!(normalize_hash("ab:cd:0F"), "ABCD0F");
    }
}
