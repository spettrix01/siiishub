use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::ops::settings::SettingsPatch;
use crate::remote;
use crate::settings::PublicSettings;
use crate::state::AppState;

use super::shared::{err, CmdResult};

#[tauri::command]
pub async fn settings_get(state: State<'_, Arc<AppState>>) -> CmdResult<PublicSettings> {
    Ok(crate::ops::settings::get(&state))
}

#[tauri::command]
pub async fn settings_save(
    state: State<'_, Arc<AppState>>,
    patch: SettingsPatch,
) -> CmdResult<PublicSettings> {
    let (settings, port_changed) = crate::ops::settings::save(&state, patch).await?;
    if let Some(port) = port_changed {
        remote::reconfigure(state.inner().clone(), true, port).await;
    }
    Ok(settings)
}

#[tauri::command]
pub async fn open_download_dir(app: AppHandle, state: State<'_, Arc<AppState>>) -> CmdResult<()> {
    let path = state.download_dir.clone();
    tokio::fs::create_dir_all(&path).await.map_err(err)?;
    super::shared::open_folder(&app, &path).map_err(err)?;
    Ok(())
}

/// Opens a web page in the browser (the "get your key" links, cast pages).
#[tauri::command]
pub async fn open_url(app: AppHandle, url: String) -> CmdResult<()> {
    super::shared::open_url(&app, &url).map_err(err)
}
