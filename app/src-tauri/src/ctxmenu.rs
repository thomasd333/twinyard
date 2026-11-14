//! Explorer's context menu for files on disk (IContextMenu): 7-Zip, Send to,
//! Open with, Properties… Shown at the mouse cursor on the UI thread. While
//! it is open the window is subclassed so owner-drawn / lazy submenus
//! (IContextMenu2/3) work.

use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CMF_EXPLORE, CMF_NORMAL, CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, DefSubclassProc,
    IContextMenu, IContextMenu2, IContextMenu3, ILFindLastID, IShellFolder, RemoveWindowSubclass, SHBindToParent,
    SHParseDisplayName, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_DRAWITEM,
    WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};
use windows::core::{HSTRING, Interface, PCSTR, PCWSTR};

/// Not exported by this `windows` version (shobjidl_core.h).
const CMIC_MASK_UNICODE: u32 = 0x0000_4000;

thread_local! {
    static MENU2: RefCell<Option<IContextMenu2>> = const { RefCell::new(None) };
    static MENU3: RefCell<Option<IContextMenu3>> = const { RefCell::new(None) };
}

unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        let mut res = LRESULT(0);
        // SAFETY: forwarding the window message to the shell menu handler.
        let handled = MENU3.with(|m| m.borrow().as_ref().map(|c| unsafe { c.HandleMenuMsg2(msg, wp, lp, Some(&mut res)).is_ok() }))
            .or_else(|| MENU2.with(|m| m.borrow().as_ref().map(|c| unsafe { c.HandleMenuMsg(msg, wp, lp).is_ok() })))
            .unwrap_or(false);
        if handled {
            return res;
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

/// All paths must be in the same folder.
fn show(hwnd: HWND, paths: &[String]) -> windows::core::Result<()> {
    let mut abs: Vec<*mut ITEMIDLIST> = Vec::new();
    let result = (|| unsafe {
        for p in paths {
            let mut pidl = std::ptr::null_mut();
            SHParseDisplayName(&HSTRING::from(p.as_str()), None, &mut pidl, 0, None)?;
            abs.push(pidl);
        }
        let folder: IShellFolder = SHBindToParent(abs[0], None)?;
        let kids: Vec<*const ITEMIDLIST> = abs.iter().map(|&p| ILFindLastID(p) as *const _).collect();
        let cm: IContextMenu = folder.GetUIObjectOf(hwnd, &kids, None)?;
        let menu = CreatePopupMenu()?;
        cm.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL | CMF_EXPLORE).ok()?;

        MENU2.with(|m| *m.borrow_mut() = cm.cast().ok());
        MENU3.with(|m| *m.borrow_mut() = cm.cast().ok());
        let _ = SetWindowSubclass(hwnd, Some(subclass), 0xF11E, 0);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, pt.x, pt.y, hwnd, None).0;
        let _ = RemoveWindowSubclass(hwnd, Some(subclass), 0xF11E);
        MENU2.with(|m| *m.borrow_mut() = None);
        MENU3.with(|m| *m.borrow_mut() = None);

        if cmd > 0 {
            let id = (cmd - 1) as usize;
            let info = CMINVOKECOMMANDINFOEX {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE,
                hwnd,
                lpVerb: PCSTR(id as *const u8),
                lpVerbW: PCWSTR(id as *const u16),
                nShow: SW_SHOWNORMAL.0,
                ptInvoke: pt,
                ..Default::default()
            };
            let r = cm.InvokeCommand(&info as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO);
            let _ = DestroyMenu(menu);
            r?;
        } else {
            let _ = DestroyMenu(menu);
        }
        Ok(())
    })();
    for p in abs {
        unsafe { CoTaskMemFree(Some(p as *const _)) };
    }
    result
}

/// Right click / Menu key: Explorer's menu for local files of one folder.
#[tauri::command]
pub fn shell_context_menu(window: tauri::WebviewWindow, paths: Vec<String>) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    if paths.iter().any(|p| twinyard_remote::is_remote(p) || twinyard_core::archive::is_inside(p)) {
        return Err(twinyard_core::i18n::tr("Menu Eksploratora jest dostępne dla plików na dysku (nie FTP / ZIP)", "The Explorer menu is available for files on a disk (not FTP / ZIP)").into());
    }
    // Raw handle as a number: HWND itself is not Send.
    let raw = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    window
        .run_on_main_thread(move || {
            if let Err(e) = show(HWND(raw as _), &paths) {
                log::warn!("context menu: {e}");
            }
        })
        .map_err(|e| e.to_string())
}
