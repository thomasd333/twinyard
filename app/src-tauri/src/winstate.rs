//! Window size / position / maximized, remembered in `data\window.json`
//! (portable — not the window-state plugin, which writes to %APPDATA%).
//! The window starts hidden (tauri.conf.json) and is shown after restoring,
//! so it doesn't jump from the centre to its place.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow, WindowEvent};

#[derive(Clone, Copy, Serialize, Deserialize, Default)]
struct Geom {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    maximized: bool,
}

fn path() -> std::path::PathBuf {
    twinyard_core::config::data_dir().join("window.json")
}

/// Restores the saved geometry (if it still fits a monitor), shows the
/// window and keeps track of changes.
pub fn attach(win: &WebviewWindow) {
    let saved: Option<Geom> = std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok());
    if let Some(g) = saved.filter(|g| on_screen(win, g)) {
        let _ = win.set_size(PhysicalSize::new(g.w, g.h));
        let _ = win.set_position(PhysicalPosition::new(g.x, g.y));
        if g.maximized {
            let _ = win.maximize();
        }
    }
    let _ = win.show();
    let _ = win.set_focus();

    // Last *normal* (not maximized/minimized) geometry: what to restore to.
    let normal = Mutex::new(saved.unwrap_or_default());
    remember_normal(win, &normal); // a window never moved still gets saved
    let w = win.clone();
    win.on_window_event(move |ev| match ev {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => remember_normal(&w, &normal),
        // Also when the close is then refused (jobs running): harmless.
        WindowEvent::CloseRequested { .. } => {
            remember_normal(&w, &normal);
            let mut g = *normal.lock().unwrap();
            if g.w == 0 {
                return;
            }
            g.maximized = w.is_maximized().unwrap_or(false);
            if let Ok(json) = serde_json::to_string(&g) {
                let _ = std::fs::write(path(), json);
            }
        }
        _ => {}
    });
}

/// Current geometry, unless maximized / minimized (then the last normal one stays).
fn remember_normal(w: &WebviewWindow, normal: &Mutex<Geom>) {
    if w.is_maximized().unwrap_or(false) || w.is_minimized().unwrap_or(false) {
        return;
    }
    // inner size: set_size() sets the inner size (outer would grow the window every start).
    if let (Ok(p), Ok(s)) = (w.outer_position(), w.inner_size()) {
        *normal.lock().unwrap() = Geom { x: p.x, y: p.y, w: s.width, h: s.height, maximized: false };
    }
}

/// At least the top-left 100×50 px of the window lies on some monitor
/// (a monitor that was unplugged must not leave the window invisible).
fn on_screen(win: &WebviewWindow, g: &Geom) -> bool {
    if g.w < 400 || g.h < 300 {
        return false;
    }
    win.available_monitors().unwrap_or_default().iter().any(|m| {
        let (p, s) = (m.position(), m.size());
        g.x + 100 > p.x && g.x < p.x + s.width as i32 - 100 && g.y >= p.y - 10 && g.y + 50 < p.y + s.height as i32
    })
}
