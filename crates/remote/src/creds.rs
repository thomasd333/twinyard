//! Saved FTP passwords: `<data dir>\passwords.json`, each value encrypted with
//! Windows DPAPI (CryptProtectData, bound to the current Windows user).
//! Portable on purpose: the file travels with the app; on another PC/account
//! it cannot be decrypted and twinyard simply asks for the password again.
//! Entries from earlier versions (Windows Credential Manager,
//! `filecmd:ftp:<id>`) are moved here on first read.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::ptr;
use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError, LocalFree};
use windows_sys::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW};
use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};

fn store_path() -> PathBuf {
    twinyard_core::config::data_dir().join("passwords.json")
}

/// A file that exists but can't be read is an error, not "no passwords":
/// saving over it would wipe every other stored password.
fn load() -> io::Result<BTreeMap<String, String>> {
    match std::fs::read_to_string(store_path()) {
        Ok(s) => serde_json::from_str(s.trim_start_matches('\u{feff}')).map_err(|e| io::Error::other(format!("passwords.json: {e}"))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(e),
    }
}

fn save(map: &BTreeMap<String, String>) -> io::Result<()> {
    let path = store_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(map).map_err(io::Error::other)?)?;
    twinyard_core::win::move_path(&tmp, &path, true)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn dpapi(data: &[u8], protect: bool) -> io::Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };
    // SAFETY: input lives for the call; the output buffer is LocalFree'd below.
    let ok = unsafe {
        if protect {
            CryptProtectData(&input, ptr::null(), ptr::null(), ptr::null(), ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, ptr::null_mut(), ptr::null(), ptr::null(), ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        }
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: DPAPI returned a valid buffer of cbData bytes.
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe { LocalFree(out.pbData as _) };
    Ok(v)
}

pub fn get(id: &str) -> Option<String> {
    if let Some(enc) = load().ok().and_then(|m| m.get(id).cloned()) {
        let plain = dpapi(&unhex(&enc)?, false).ok()?; // other PC/user: None -> ask again
        return String::from_utf8(plain).ok();
    }
    // Migrate from the Credential Manager (versions before portable data).
    let p = legacy::get(id)?;
    if set(id, "", &p).is_ok() {
        let _ = legacy::delete(id);
    }
    Some(p)
}

pub fn set(id: &str, _user: &str, password: &str) -> io::Result<()> {
    let enc = dpapi(password.as_bytes(), true)?;
    let mut map = load()?;
    map.insert(id.to_string(), hex(&enc));
    save(&map)
}

pub fn delete(id: &str) -> io::Result<()> {
    let mut map = load()?;
    if map.remove(id).is_some() {
        save(&map)?;
    }
    legacy::delete(id)
}

/// The Windows Credential Manager store used before the portable layout.
mod legacy {
    use super::*;

    fn target(id: &str) -> Vec<u16> {
        format!("filecmd:ftp:{id}").encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn get(id: &str) -> Option<String> {
        let t = target(id);
        let mut cred: *mut CREDENTIALW = ptr::null_mut();
        // SAFETY: valid NUL-terminated target; `cred` freed with CredFree.
        unsafe {
            if CredReadW(t.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) == 0 {
                return None;
            }
            let c = &*cred;
            let bytes = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize);
            let units: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
            CredFree(cred as *const _);
            String::from_utf16(&units).ok()
        }
    }

    pub fn delete(id: &str) -> io::Result<()> {
        let t = target(id);
        // SAFETY: valid NUL-terminated target.
        unsafe {
            if CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0) == 0 && GetLastError() != ERROR_NOT_FOUND {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn set(id: &str, password: &str) {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CredWriteW};
        let mut t = target(id);
        let blob: Vec<u8> = password.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        let cred = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: t.as_mut_ptr(),
            Comment: ptr::null_mut(),
            LastWritten: FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 },
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: ptr::null_mut(),
            TargetAlias: ptr::null_mut(),
            UserName: ptr::null_mut(),
        };
        // SAFETY: pointers live until the call returns.
        assert!(unsafe { CredWriteW(&cred, 0) } != 0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrip_and_migration() {
        let dir = std::env::temp_dir().join(format!("twinyard-creds-{}", std::process::id()));
        // SAFETY: the only test in this binary touching TWINYARD_DATA.
        unsafe { std::env::set_var("TWINYARD_DATA", &dir) };
        let id = "selftest-delete-me";
        super::set(id, "user", "zażółć 🔑").unwrap();
        assert_eq!(super::get(id).as_deref(), Some("zażółć 🔑"));
        let raw = std::fs::read_to_string(dir.join("passwords.json")).unwrap();
        assert!(!raw.contains("zażółć"), "stored encrypted");
        super::delete(id).unwrap();
        assert_eq!(super::get(id), None);

        // Old Credential Manager entry is moved into the file.
        super::legacy::set(id, "stare");
        assert_eq!(super::get(id).as_deref(), Some("stare"));
        assert_eq!(super::legacy::get(id), None);
        super::delete(id).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
