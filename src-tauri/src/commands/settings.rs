use std::sync::Arc;

use serde::Deserialize;
use tauri::{AppHandle, State};

use crate::remote;
use crate::settings::{PublicSettings, Settings};
use crate::state::AppState;

use super::shared::{err, CmdResult};

#[derive(Debug, Deserialize)]
pub struct SettingsPatch {
    #[serde(default)]
    pub tmdb_key: Option<String>,
    #[serde(default)]
    pub addons: Option<Vec<crate::settings::AddonConfig>>,
    #[serde(default)]
    pub rd_token: Option<String>,
    #[serde(default)]
    pub debrid_provider: Option<String>,
    #[serde(default)]
    pub debrid_token: Option<String>,
    #[serde(default)]
    pub tracker_fallbacks: Option<Vec<String>>,
    #[serde(default)]
    pub player_audio_langs: Option<Vec<String>>,
    #[serde(default)]
    pub player_sub_langs: Option<Vec<String>>,
    #[serde(default)]
    pub remote_port: Option<u16>,
    #[serde(default)]
    pub language: Option<String>,
}

#[tauri::command]
pub async fn settings_get(state: State<'_, Arc<AppState>>) -> CmdResult<PublicSettings> {
    Ok(state.public_settings())
}

#[tauri::command]
pub async fn settings_save(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    patch: SettingsPatch,
) -> CmdResult<PublicSettings> {
    let mut next: Settings = state.settings.read();
    let prev_remote_port = next.remote_port;
    if let Some(k) = patch.tmdb_key {
        next.tmdb_key = k.trim().to_string();
    }
    if let Some(a) = patch.addons {
        next.addons = a;
    }
    if let Some(t) = patch.rd_token {
        next.rd_token = t.trim().to_string();
    }
    if let Some(p) = patch.debrid_provider {
        next.debrid_provider = p.trim().to_lowercase();
    }
    if let Some(t) = patch.debrid_token {
        next.debrid_token = t.trim().to_string();
    }
    if let Some(t) = patch.tracker_fallbacks {
        next.tracker_fallbacks = t
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(a) = patch.player_audio_langs {
        next.player_audio_langs = a
            .into_iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(s) = patch.player_sub_langs {
        next.player_sub_langs = s
            .into_iter()
            .map(|x| x.trim().to_lowercase())
            .filter(|x| !x.is_empty())
            .collect();
    }
    if let Some(p) = patch.remote_port {
        if p >= 1024 {
            next.remote_port = p;
        }
    }
    if let Some(l) = patch.language {
        let l = l.trim().to_lowercase();
        if !l.is_empty() {
            next.language = l;
        }
    }
    let new_port = next.remote_port;
    state.settings.write(next).await.map_err(err)?;

    if prev_remote_port != new_port {
        remote::reconfigure(&app, state.inner().clone(), true, new_port).await;
    }

    Ok(state.public_settings())
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
