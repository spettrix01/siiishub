use std::sync::Arc;

use tauri::State;

use crate::state::AppState;
use crate::stremio::{AddonManifest, StreamsResult, SubtitlesResult};

use super::shared::CmdResult;

#[tauri::command]
pub async fn addon_meta(
    state: State<'_, Arc<AppState>>,
    url: String,
) -> CmdResult<AddonManifest> {
    crate::ops::addons::meta(&state, &url).await
}

#[tauri::command]
pub async fn streams_fetch(
    state: State<'_, Arc<AppState>>,
    kind: String,
    id: String,
) -> CmdResult<StreamsResult> {
    Ok(crate::ops::addons::streams(&state, &kind, &id).await)
}

#[tauri::command]
pub async fn subtitles_fetch(
    state: State<'_, Arc<AppState>>,
    kind: String,
    id: String,
) -> CmdResult<SubtitlesResult> {
    Ok(crate::ops::addons::subtitles(&state, &kind, &id).await)
}
