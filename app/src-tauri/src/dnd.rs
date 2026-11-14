//! Dragging files OUT of twinyard into Explorer / other apps (OLE DoDragDrop
//! via the `drag` crate). Drops INTO the window come as Tauri drag-drop events.

use std::path::PathBuf;

const DRAG_ICON: &[u8] = include_bytes!("../icons/32x32.png");

/// Starts a native file drag (must run on the main/UI thread). `mode`:
/// "copy" or "move" — the effect the target app should prefer.
#[tauri::command]
pub fn drag_out(window: tauri::WebviewWindow, paths: Vec<String>, mode: String) -> Result<(), String> {
    let files: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).filter(|p| p.exists()).collect();
    if files.is_empty() {
        return Err(twinyard_core::i18n::tr("Przeciągać na zewnątrz można tylko pliki z dysku (nie z FTP / ZIP)", "Only files on a disk can be dragged out (not from FTP / ZIP)").into());
    }
    let w = window.clone();
    window
        .run_on_main_thread(move || {
            let options = drag::Options {
                skip_animatation_on_cancel_or_failure: false,
                mode: if mode == "move" { drag::DragMode::Move } else { drag::DragMode::Copy },
            };
            if let Err(e) = drag::start_drag(&w, drag::DragItem::Files(files), drag::Image::Raw(DRAG_ICON.to_vec()), |_, _| {}, options) {
                log::warn!("drag out: {e}");
            }
        })
        .map_err(|e| e.to_string())
}
