//! IPC for the FTP connection manager.

use crate::blocking;
use twinyard_remote::conn::{self, Connection};
use twinyard_remote::{ConnectResult, Remote, creds, tc};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnView {
    #[serde(flatten)]
    conn: Connection,
    saved_password: bool,
}

#[tauri::command]
pub async fn ftp_connections() -> Result<Vec<ConnView>, String> {
    blocking(|| {
        let store = conn::load().map_err(|e| format!("connections.json: {e}"))?;
        Ok(store
            .connections
            .into_iter()
            .map(|c| ConnView { saved_password: creds::get(&c.id).is_some(), conn: c })
            .collect())
    })
    .await
}

/// Insert (empty id) or update a connection. `password`: Some("") = forget,
/// Some(x) = save in the Credential Manager, None = leave as is.
#[tauri::command]
pub async fn ftp_save_connection(
    remote: State<'_, Arc<Remote>>,
    conn: Connection,
    password: Option<String>,
) -> Result<String, String> {
    let r = remote.inner().clone();
    blocking(move || {
        let mut store = conn::load().map_err(|e| e.to_string())?;
        let mut c = conn;
        c.name = c.name.trim().to_string();
        c.host = c.host.trim().to_string();
        if c.name.is_empty() || c.host.is_empty() {
            return Err(twinyard_core::i18n::tr("Podaj nazwę i adres serwera", "Enter a name and the server address").into());
        }
        if store.connections.iter().any(|x| x.name.eq_ignore_ascii_case(&c.name) && x.id != c.id) {
            return Err(twinyard_core::trf!("Połączenie „{}” już istnieje", "Connection “{}” already exists", c.name));
        }
        let id = if c.id.is_empty() {
            store.connections.push(c);
            store.assign_ids();
            store.connections.last().unwrap().id.clone()
        } else {
            let id = c.id.clone();
            let slot = store.connections.iter_mut().find(|x| x.id == id).ok_or(twinyard_core::i18n::tr("Nie ma takiego połączenia", "No such connection"))?;
            *slot = c;
            r.disconnect(&id); // settings changed: next use reconnects
            id
        };
        conn::save(&store).map_err(|e| e.to_string())?;
        match password.as_deref() {
            Some("") => r.forget_password(&id)?,
            Some(p) => {
                let user = store.get(&id).map(|c| c.user.clone()).unwrap_or_default();
                creds::set(&id, &user, p).map_err(|e| e.to_string())?;
            }
            None => {}
        }
        Ok(id)
    })
    .await
}

#[tauri::command]
pub async fn ftp_delete_connection(remote: State<'_, Arc<Remote>>, id: String) -> Result<(), String> {
    let r = remote.inner().clone();
    blocking(move || {
        let mut store = conn::load().map_err(|e| e.to_string())?;
        store.connections.retain(|c| c.id != id);
        // Trusted certificates of hosts no other connection uses go too.
        let used: Vec<String> = store.connections.iter().map(|c| c.host_port()).collect();
        store.pins.retain(|p| used.iter().any(|u| u.eq_ignore_ascii_case(&p.host_port)));
        conn::save(&store).map_err(|e| e.to_string())?;
        r.disconnect(&id);
        r.forget_password(&id)
    })
    .await
}

#[tauri::command]
pub async fn ftp_connect(
    remote: State<'_, Arc<Remote>>,
    id: String,
    password: Option<String>,
    remember: bool,
) -> Result<ConnectResult, String> {
    let r = remote.inner().clone();
    blocking(move || r.connect(&id, password, remember)).await
}

#[tauri::command]
pub async fn ftp_trust(remote: State<'_, Arc<Remote>>, id: String, sha256: String) -> Result<(), String> {
    let r = remote.inner().clone();
    blocking(move || r.trust(&id, &sha256)).await
}

#[tauri::command]
pub async fn ftp_forget_password(remote: State<'_, Arc<Remote>>, id: String) -> Result<(), String> {
    let r = remote.inner().clone();
    blocking(move || r.forget_password(&id)).await
}

#[tauri::command]
pub async fn ftp_disconnect(remote: State<'_, Arc<Remote>>, id: String) -> Result<(), String> {
    let r = remote.inner().clone();
    blocking(move || {
        r.disconnect(&id);
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn ftp_tc_default_path() -> String {
    tc::default_path().display().to_string()
}

#[tauri::command]
pub async fn ftp_import_tc(path: String) -> Result<tc::ImportReport, String> {
    blocking(move || {
        let mut store = conn::load().map_err(|e| e.to_string())?;
        let report = tc::import(&PathBuf::from(path.trim().trim_matches('"')), &mut store)?;
        conn::save(&store).map_err(|e| e.to_string())?;
        Ok(report)
    })
    .await
}

/// Opens the FTP log in the configured editor.
#[tauri::command]
pub async fn ftp_open_log() -> Result<(), String> {
    blocking(|| {
        let path = crate::filelog::log_path();
        if !path.exists() {
            return Err(twinyard_core::i18n::tr("Log jest pusty — nie było jeszcze połączeń", "The log is empty — no connections yet").into());
        }
        let cfg = twinyard_core::config::load().map_err(|e| e.to_string())?;
        twinyard_core::config::launch(&cfg.tools.editor, &[("%f", &path.display().to_string())]).map_err(|e| e.to_string())
    })
    .await
}
