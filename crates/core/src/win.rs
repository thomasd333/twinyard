//! Thin safe wrappers over Win32 file APIs.

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    COPY_FILE_FAIL_IF_EXISTS, CopyFileExW, MOVEFILE_REPLACE_EXISTING, MoveFileExW, PROGRESS_CANCEL, PROGRESS_CONTINUE,
};

pub const ERROR_NOT_SAME_DEVICE: i32 = 17;

/// `<path>.twinyard-part`: a file is written here first and swapped in only
/// when complete, so an interrupted overwrite never destroys the old one.
pub fn part_path(p: &Path) -> std::path::PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".twinyard-part");
    s.into()
}

/// A real link — symlink, junction (reparse tags of the "name surrogate"
/// kind; std checks that). OneDrive / cloud placeholders are reparse points
/// too, but ordinary files and folders: they are copied, not skipped.
pub fn is_link(md: &std::fs::Metadata) -> bool {
    md.file_type().is_symlink()
}
pub const ERROR_REQUEST_ABORTED: i32 = 1235;

/// NUL-terminated wide path. Paths near/over MAX_PATH (260) get the `\\?\`
/// prefix (`\\?\UNC\` for shares), which the Win32 file APIs need to accept
/// them (std::fs does the same internally; these raw calls don't).
fn wide(p: &Path) -> Vec<u16> {
    let s = p.as_os_str();
    if s.len() >= 248 {
        let st = p.to_string_lossy().replace('/', "\\");
        let long = if st.starts_with(r"\\?\") {
            None
        } else if let Some(rest) = st.strip_prefix(r"\\") {
            Some(format!(r"\\?\UNC\{rest}"))
        } else if st.as_bytes().get(1) == Some(&b':') {
            Some(format!(r"\\?\{st}"))
        } else {
            None
        };
        if let Some(l) = long {
            return l.encode_utf16().chain(Some(0)).collect();
        }
    }
    s.encode_wide().chain(Some(0)).collect()
}

struct Ctx<'a> {
    on_progress: &'a mut dyn FnMut(u64) -> bool,
}

unsafe extern "system" fn progress_routine(
    _total: i64,
    transferred: i64,
    _stream_size: i64,
    _stream_transferred: i64,
    _stream_no: u32,
    _reason: u32,
    _src: HANDLE,
    _dst: HANDLE,
    data: *const c_void,
) -> u32 {
    // SAFETY: `data` is the `Ctx` passed by `copy_file`, alive for the whole
    // synchronous CopyFileExW call, and only touched on this thread.
    let ctx = unsafe { &mut *(data as *mut Ctx) };
    if (ctx.on_progress)(transferred as u64) {
        PROGRESS_CONTINUE
    } else {
        PROGRESS_CANCEL
    }
}

/// Copies one file with native speed, keeping attributes and timestamps.
/// `on_progress(bytes_done)` returns false to abort; then the error has
/// raw code `ERROR_REQUEST_ABORTED` and the partial target is removed by Windows.
pub fn copy_file(src: &Path, dst: &Path, overwrite: bool, on_progress: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
    let (s, d) = (wide(src), wide(dst));
    let mut ctx = Ctx { on_progress };
    let flags = if overwrite { 0 } else { COPY_FILE_FAIL_IF_EXISTS };
    // SAFETY: valid NUL-terminated paths; ctx outlives the call.
    let ok = unsafe {
        CopyFileExW(
            s.as_ptr(),
            d.as_ptr(),
            Some(progress_routine),
            &mut ctx as *mut Ctx as *const c_void,
            std::ptr::null_mut(),
            flags,
        )
    };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// Rename/move on the same volume (files or whole directories).
/// Fails with `ERROR_NOT_SAME_DEVICE` across volumes: caller falls back to copy+delete.
pub fn move_path(src: &Path, dst: &Path, overwrite: bool) -> io::Result<()> {
    let (s, d) = (wide(src), wide(dst));
    let flags = if overwrite { MOVEFILE_REPLACE_EXISTING } else { 0 };
    // SAFETY: valid NUL-terminated paths.
    let ok = unsafe { MoveFileExW(s.as_ptr(), d.as_ptr(), flags) };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// Clears the read-only attribute (needed to overwrite/delete such files).
pub fn clear_readonly(p: &Path) -> io::Result<()> {
    let md = std::fs::symlink_metadata(p)?;
    let mut perm = md.permissions();
    if perm.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(p, perm)?;
    }
    Ok(())
}

/// Opens a file with its associated program. Unlike plain ShellExecute it
/// never shows Windows' own error box: Err(..) lets the caller fall back
/// (e.g. to the configured editor) when there is no working association.
pub fn shell_open(path: &Path) -> io::Result<()> {
    use windows_sys::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let file = wide(path);
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let dir = wide(path.parent().unwrap_or(Path::new(".")));
    // SAFETY: all strings are NUL-terminated and outlive the call.
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpDirectory = dir.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&mut info) == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Windows' "Open with…" chooser for a file.
pub fn open_with_dialog(path: &Path) -> io::Result<()> {
    std::process::Command::new("rundll32.exe")
        .arg("shell32.dll,OpenAs_RunDLL")
        .arg(path)
        .spawn()
        .map(|_| ())
}

