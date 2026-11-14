//! File-name encodings on the FTP control channel. Old Polish servers send
//! names in Windows-1250 and don't announce UTF8.

use twinyard_core::text::{decode_cp1250, encode_cp1250};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use suppaftp::Encoding;

#[derive(Debug, Default)]
pub struct Cp1250;

impl Encoding for Cp1250 {
    fn decode(&self, bytes: &[u8]) -> String {
        decode_cp1250(bytes)
    }
    fn encode(&self, text: &str) -> io::Result<Vec<u8>> {
        encode_cp1250(text).ok_or_else(|| io::Error::other(twinyard_core::trf!("Nazwy „{text}” nie da się zapisać w Windows-1250", "The name “{text}” cannot be written in Windows-1250")))
    }
}

/// For servers not in UTF-8 mode: valid UTF-8 stays UTF-8, anything else is
/// read as CP1250. Names are SENT in CP1250 (what such Polish servers store)
/// unless the server showed real UTF-8 names and never CP1250 ones.
#[derive(Debug, Default)]
pub struct Sniff {
    cp1250: AtomicBool,
    utf8: AtomicBool,
}

impl Encoding for Sniff {
    fn decode(&self, bytes: &[u8]) -> String {
        match std::str::from_utf8(bytes) {
            Ok(s) => {
                if !s.is_ascii() {
                    self.utf8.store(true, Ordering::Relaxed);
                }
                s.to_string()
            }
            Err(_) => {
                self.cp1250.store(true, Ordering::Relaxed);
                decode_cp1250(bytes)
            }
        }
    }
    fn encode(&self, text: &str) -> io::Result<Vec<u8>> {
        let utf8_server = self.utf8.load(Ordering::Relaxed) && !self.cp1250.load(Ordering::Relaxed);
        if !utf8_server && let Some(b) = encode_cp1250(text) {
            return Ok(b);
        }
        Ok(text.as_bytes().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs() {
        let s = Sniff::default();
        assert_eq!(s.encode("ą").unwrap(), vec![0xB9]); // nothing seen yet: CP1250 (old Polish server)
        assert_eq!(s.decode("ąę".as_bytes()), "ąę"); // real UTF-8 names on the server…
        assert_eq!(s.encode("ą").unwrap(), "ą".as_bytes()); // …-> send UTF-8
        assert_eq!(s.decode(&[b'a', 0xB9]), "aą"); // a CP1250 name -> CP1250 wins
        assert_eq!(s.encode("ą").unwrap(), vec![0xB9]);
        assert!(Cp1250.encode("日本").is_err());
    }
}
