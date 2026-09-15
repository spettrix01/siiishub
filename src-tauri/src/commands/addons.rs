use std::sync::Arc;

use tauri::State;

use crate::state::AppState;
use crate::stremio::{self, AddonManifest, StreamsResult, SubtitlesResult};

use super::shared::{err, CmdResult};

#[tauri::command]
pub async fn addon_meta(
    state: State<'_, Arc<AppState>>,
    url: String,
) -> CmdResult<AddonManifest> {
    stremio::fetch_manifest(&state.http, &url)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn streams_fetch(
    state: State<'_, Arc<AppState>>,
    kind: String,
    id: String,
) -> CmdResult<StreamsResult> {
    let addons = state.settings.read().addons;
    Ok(stremio::fetch_streams(&state.http, &addons, &kind, &id).await)
}

#[tauri::command]
pub async fn subtitles_fetch(
    state: State<'_, Arc<AppState>>,
    kind: String,
    id: String,
) -> CmdResult<SubtitlesResult> {
    let addons = state.settings.read().addons;
    Ok(stremio::fetch_subtitles(&state.http, &addons, &kind, &id).await)
}
