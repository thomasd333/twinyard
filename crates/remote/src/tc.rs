//! Import of Total Commander's built-in FTP connections (`wcx_ftp.ini`).
//!
//! Connections and trusted certificates are imported; passwords are NOT —
//! twinyard asks on the first connect and keeps them in the Windows
//! Credential Manager.

use crate::conn::{Connection, Store, Tls, normalize_hash};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Default location of TC's FTP settings.
pub fn default_path() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("GHISLER").join("wcx_ftp.ini")
}

#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub added: Vec<String>,
    /// Already present (same name) — left untouched.
    pub skipped: Vec<String>,
    pub pins: usize,
}

/// Sections in file order; keys lowercased, values trimmed.
pub fn parse_ini(text: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut out: Vec<(String, HashMap<String, String>)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            out.push((line[1..line.len() - 1].to_string(), HashMap::new()));
        } else if let (Some((k, v)), Some(last)) = (line.split_once('='), out.last_mut()) {
            last.1.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    out
}

/// TC writes ANSI (cp1250 here) or UTF-16 LE with BOM; newer builds UTF-8.
pub fn decode(bytes: &[u8]) -> String {
    if let [0xFF, 0xFE, rest @ ..] = bytes {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| cp1250(b)).collect(),
    }
}

/// Polish letters of cp1250; other high bytes as Latin-1 (good enough for names).
fn cp1250(b: u8) -> char {
    match b {
        0xA5 => 'Ą',
        0xC6 => 'Ć',
        0xCA => 'Ę',
        0xA3 => 'Ł',
        0xD1 => 'Ń',
        0xD3 => 'Ó',
        0x8C => 'Ś',
        0x8F => 'Ź',
        0xAF => 'Ż',
        0xB9 => 'ą',
        0xE6 => 'ć',
        0xEA => 'ę',
        0xB3 => 'ł',
        0xF1 => 'ń',
        0xF3 => 'ó',
        0x9C => 'ś',
        0x9F => 'ź',
        0xBF => 'ż',
        b => b as char,
    }
}

/// `host` field: `host`, `host:port`, `ftp://host`, `ftps://host:990/dir`.
/// Returns (host, port, tls, dir).
pub fn parse_host(field: &str) -> (String, u16, Tls, String) {
    let f = field.trim();
    let lower = f.to_ascii_lowercase();
    let (rest, secure) = if let Some(r) = lower.strip_prefix("ftps://").map(|_| &f[7..]) {
        (r, true)
    } else if lower.starts_with("ftp://") {
        (&f[6..], false)
    } else {
        (f, false)
    };
    let (hostport, dir) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, String::new()),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() && !h.contains(':') => (h.to_string(), p.parse::<u16>().ok()),
        _ => (hostport.to_string(), None),
    };
    // TC's ftps:// = AUTH TLS (explicit) on whatever port is given — real
    // servers run explicit TLS on 990 too. (A truly implicit server still works:
    // the connection falls back the other way only for plain-text greeters,
    // so for those the user switches the mode in the connection editor.)
    let (tls, port) = match (secure, port) {
        (true, p) => (Tls::Explicit, p.unwrap_or(21)),
        (false, p) => (Tls::None, p.unwrap_or(21)),
    };
    (host, port, tls, dir)
}

/// Merges connections and pinned certificates from `wcx_ftp.ini` into `store`.
pub fn import(path: &Path, store: &mut Store) -> Result<ImportReport, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let ini = parse_ini(&decode(&bytes));
    let section = |name: &str| ini.iter().find(|(n, _)| n.trim().eq_ignore_ascii_case(name.trim())).map(|(_, m)| m);

    let mut report = ImportReport::default();
    let Some(list) = section("connections") else {
        return Err(twinyard_core::i18n::tr("To nie wygląda na wcx_ftp.ini (brak sekcji [connections])", "This does not look like wcx_ftp.ini (no [connections] section)").into());
    };
    // "1=Name", "2=Name"… in numeric order.
    let mut numbered: Vec<(u32, String)> =
        list.iter().filter_map(|(k, v)| k.parse::<u32>().ok().map(|n| (n, v.trim().to_string()))).collect();
    numbered.sort();

    for (_, name) in numbered {
        if name.is_empty() {
            continue;
        }
        let Some(sec) = section(&name) else { continue };
        let Some(hostfield) = sec.get("host").filter(|h| !h.is_empty()) else { continue };
        if store.connections.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
            report.skipped.push(name);
            continue;
        }
        let (host, port, tls, dir_in_host) = parse_host(hostfield);
        let keepalive = sec.get("keepalive").and_then(|v| v.split(',').next()?.trim().parse::<u32>().ok()).unwrap_or(0);
        store.connections.push(Connection {
            id: String::new(),
            name: name.clone(),
            host,
            port,
            tls,
            user: sec.get("username").cloned().unwrap_or_default(),
            initial_dir: sec.get("directory").cloned().filter(|d| !d.is_empty()).unwrap_or(dir_in_host),
            passive: sec.get("pasvmode").is_none_or(|v| v != "0"),
            // TC: MLSD=-1 → don't use MLSD.
            mlsd: sec.get("mlsd").is_none_or(|v| v != "-1" && v != "0"),
            keepalive,
            encoding: Default::default(),
        });
        report.added.push(name);
    }

    for sec_name in ["CertSHA256hashes", "CertSHA1hashes"] {
        if let Some(sec) = section(sec_name) {
            for (hp, hash) in sec {
                let before = store.pins.len();
                store.add_pin(hp, &normalize_hash(hash));
                report.pins += store.pins.len() - before;
            }
        }
    }
    store.assign_ids();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(parse_host("ftps://192.0.2.10:990"), ("192.0.2.10".into(), 990, Tls::Explicit, String::new()));
        assert_eq!(parse_host("ftps://example.com:21/www"), ("example.com".into(), 21, Tls::Explicit, "/www".into()));
        assert_eq!(parse_host("ftp.example.com:2121"), ("ftp.example.com".into(), 2121, Tls::None, String::new()));
        assert_eq!(parse_host("example.com"), ("example.com".into(), 21, Tls::None, String::new()));
    }

    #[test]
    fn imports_fixture() {
        let dir = std::env::temp_dir().join(format!("twinyard-tc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ini = dir.join("wcx_ftp.ini");
        // Anonymized: invented names, documentation IPs, fake hashes, no passwords.
        let text = "[connections]\r\n1=Hosting   \r\n2=Firma ftp.example.com:21\r\n3=Brak sekcji\r\ndefault=Hosting\r\n\
[General]\r\nConnectRetries=3\r\n\
[Hosting]\r\nhost=ftps://192.0.2.10:990\r\nusername=jan\r\npasvmode=1\r\nkeepalive=30,PWD\r\nMLSD=-1\r\n\
[Firma ftp.example.com:21]\r\nhost=ftp.example.com\r\nusername=anonymous\r\ndirectory=/pub\r\npasvmode=0\r\n\
[CertSHA256hashes]\r\n192.0.2.10:990=AA:BB:CC\r\n";
        let mut bytes = text.as_bytes().to_vec();
        // cp1250 "ż" in a comment line must not break parsing.
        bytes.extend_from_slice(b";\xBFaba\r\n");
        std::fs::write(&ini, bytes).unwrap();

        let mut store = Store::default();
        let r = import(&ini, &mut store).unwrap();
        assert_eq!(r.added, ["Hosting", "Firma ftp.example.com:21"]);
        assert_eq!(r.pins, 1);
        let h = &store.connections[0];
        assert_eq!((h.id.as_str(), h.host.as_str(), h.port, h.tls), ("hosting", "192.0.2.10", 990, Tls::Explicit));
        assert_eq!((h.user.as_str(), h.passive, h.mlsd, h.keepalive), ("jan", true, false, 30));
        let f = &store.connections[1];
        assert_eq!((f.port, f.tls, f.initial_dir.as_str(), f.passive), (21, Tls::None, "/pub", false));
        assert_eq!(store.pins_for("192.0.2.10:990"), ["AABBCC"]);

        // Second import: nothing duplicated.
        let r2 = import(&ini, &mut store).unwrap();
        assert!(r2.added.is_empty() && r2.skipped.len() == 2 && r2.pins == 0);
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Smoke test on a real file without revealing its contents:
/// `set TWINYARD_TC_INI=path && cargo test -p twinyard-remote real_file -- --nocapture`
#[cfg(test)]
mod real {
    #[test]
    fn real_file() {
        let Ok(p) = std::env::var("TWINYARD_TC_INI") else { return };
        let mut store = crate::conn::Store::default();
        let r = super::import(std::path::Path::new(&p), &mut store).unwrap();
        let modes: Vec<String> = store.connections.iter().map(|c| format!("{:?}/{}/mlsd={}", c.tls, c.port, c.mlsd)).collect();
        println!("connections: {}, skipped: {}, pins: {}, modes: {modes:?}", r.added.len(), r.skipped.len(), r.pins);
        assert!(store.connections.iter().all(|c| !c.host.is_empty() && !c.id.is_empty()));
    }
}
