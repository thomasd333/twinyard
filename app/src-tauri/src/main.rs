// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // When started from a VS Code terminal (or by another Electron app) we inherit
    // ELECTRON_RUN_AS_NODE etc. Every program we launch would inherit them too —
    // Code.exe then runs as plain Node and tries to *execute* the file to edit.
    let polluted: Vec<_> = std::env::vars_os()
        .map(|(k, _)| k)
        .filter(|k| {
            let k = k.to_string_lossy().to_ascii_uppercase();
            k.starts_with("ELECTRON_") || k.starts_with("VSCODE_")
        })
        .collect();
    for k in polluted {
        // SAFETY: first thing in main, before any other thread exists.
        unsafe { std::env::remove_var(k) };
    }
    // Portable: WebView2's profile (localStorage = tabs, theme; caches) lives in
    // data\webview next to the exe instead of %LOCALAPPDATA%. The env var takes
    // precedence over the folder Tauri passes to WebView2.
    let webview = twinyard_core::config::data_dir().join("webview");
    // SAFETY: still single-threaded.
    unsafe { std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", webview) };
    // Already running with this data folder: bring that window up instead.
    if !twinyard_lib::housekeeping::single_instance() {
        return; // the running one was brought to the front
    }
    twinyard_lib::run()
}
