//! Reading files as text for the built-in viewer, diff and search:
//! BOM-aware UTF-8 / UTF-16, CP1250 fallback (old Polish files), binary detection.

use serde::Serialize;
use std::io::{self, Read};
use std::path::Path;

/// Windows-1250, bytes 0x80..=0xFF (U+FFFD for the five undefined ones).
const CP1250_HIGH: &str = "€\u{FFFD}‚\u{FFFD}„…†‡\u{FFFD}‰Š‹ŚŤŽŹ\
\u{FFFD}‘’“”•–—\u{FFFD}™š›śťžź\
\u{A0}ˇ˘Ł¤Ą¦§¨©Ş«¬\u{AD}®Ż\
°±˛ł´µ¶·¸ąş»Ľ˝ľż\
ŔÁÂĂÄĹĆÇČÉĘËĚÍÎĎ\
ĐŃŇÓÔŐÖ×ŘŮÚŰÜÝŢß\
ŕáâăäĺćçčéęëěíîď\
đńňóôőö÷řůúűüýţ˙";

pub fn cp1250_char(b: u8) -> char {
    if b < 0x80 { b as char } else { CP1250_HIGH.chars().nth((b - 0x80) as usize).unwrap_or('\u{FFFD}') }
}

pub fn decode_cp1250(bytes: &[u8]) -> String {
    let table: Vec<char> = CP1250_HIGH.chars().collect();
    bytes.iter().map(|&b| if b < 0x80 { b as char } else { table[(b - 0x80) as usize] }).collect()
}

/// Text -> Windows-1250 bytes; None if a character has no CP1250 form.
pub fn encode_cp1250(text: &str) -> Option<Vec<u8>> {
    text.chars()
        .map(|c| {
            if (c as u32) < 0x80 {
                Some(c as u8)
            } else {
                CP1250_HIGH.chars().position(|x| x == c && x != '\u{FFFD}').map(|i| 0x80 + i as u8)
            }
        })
        .collect()
}

/// Binary = NUL bytes (outside UTF-16) or many control chars in the sample.
pub fn looks_binary(sample: &[u8]) -> bool {
    if sample.starts_with(&[0xFF, 0xFE]) || sample.starts_with(&[0xFE, 0xFF]) {
        return false;
    }
    if sample.contains(&0) {
        return true;
    }
    let ctrl = sample.iter().filter(|&&b| b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t' | 0x0C | 0x1B)).count();
    ctrl * 100 > sample.len().max(1) * 2
}

/// (text, encoding name)
pub fn decode(bytes: &[u8]) -> (String, &'static str) {
    if let Some(b) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return (String::from_utf8_lossy(b).into_owned(), "UTF-8 BOM");
    }
    if let Some(b) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return (String::from_utf16_lossy(&u), "UTF-16 LE");
    }
    if let Some(b) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return (String::from_utf16_lossy(&u), "UTF-16 BE");
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), if bytes.is_ascii() { "ASCII" } else { "UTF-8" }),
        // A multi-byte char cut by the read limit is still UTF-8.
        Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 > bytes.len() => {
            (String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(), "UTF-8")
        }
        Err(_) => (decode_cp1250(bytes), "CP1250"),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFile {
    pub text: String,
    pub encoding: &'static str,
    pub binary: bool,
    /// Only the first `limit` bytes were read.
    pub truncated: bool,
    pub size: u64,
}

pub fn read(path: &Path, limit: usize) -> io::Result<TextFile> {
    let f = std::fs::File::open(path)?;
    let size = f.metadata()?.len();
    let mut bytes = Vec::with_capacity(limit.min(size as usize));
    f.take(limit as u64).read_to_end(&mut bytes)?;
    let binary = looks_binary(&bytes[..bytes.len().min(8192)]);
    let (text, encoding) = if binary { (String::new(), "binary") } else { decode(&bytes) };
    Ok(TextFile { text, encoding, binary, truncated: (bytes.len() as u64) < size, size })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes() {
        assert_eq!(CP1250_HIGH.chars().count(), 128);
        assert_eq!(decode_cp1250(&[0xB9, 0xE6, 0xEA, 0xB3, 0xF1, 0xF3, 0x9C, 0x9F, 0xBF]), "ąćęłńóśźż");
        assert_eq!(decode_cp1250(&[0xA5, 0xC6, 0xCA, 0xA3, 0xD1, 0xD3, 0x8C, 0x8F, 0xAF]), "ĄĆĘŁŃÓŚŹŻ");
        assert_eq!(encode_cp1250("zażółć ĄĘ").map(|b| decode_cp1250(&b)).as_deref(), Some("zażółć ĄĘ"));
        assert_eq!(encode_cp1250("日本"), None);
        assert_eq!(decode("zażółć".as_bytes()), ("zażółć".into(), "UTF-8"));
        assert_eq!(decode(&[b'a', 0xB3]).1, "CP1250");
        assert_eq!(decode(&[0xFF, 0xFE, b'a', 0]), ("a".into(), "UTF-16 LE"));
        // UTF-8 char cut at the end of the read window.
        let cut = &"aż".as_bytes()[..2];
        assert_eq!(decode(cut).1, "UTF-8");
        assert!(looks_binary(b"PK\x03\x04\x00\x00"));
        assert!(!looks_binary(b"line\r\n\tok"));
    }
}
