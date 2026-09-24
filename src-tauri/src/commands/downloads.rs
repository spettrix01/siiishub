use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::download::DownloadRecord;
use crate::ops::downloads::{DownloadEntry, DownloadStartArgs, DownloadStartGroupArgs};
use crate::state::AppState;
use crate::torrent::TorrentFileInfo;
use crate::torrent_file::ParsedTorrent;

use super::shared::{err, CmdResult};

#[tauri::command]
pub async fn download_start(
    state: State<'_, Arc<AppState>>,
    args: DownloadStartArgs,
) -> CmdResult<DownloadRecord> {
    crate::ops::downloads::start(&state, args).await
}

#[tauri::command]
pub async fn download_start_group(
    state: State<'_, Arc<AppState>>,
    args: DownloadStartGroupArgs,
) -> CmdResult<Vec<DownloadRecord>> {
    crate::ops::downloads::start_group(&state, args).await
}

#[tauri::command]
pub async fn download_add_local(
    state: State<'_, Arc<AppState>>,
    path: String,
) -> CmdResult<DownloadRecord> {
    crate::ops::downloads::add_local(&state, path).await
}

#[tauri::command]
pub async fn download_list(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<Vec<DownloadEntry>> {
    Ok(crate::ops::downloads::list(&state))
}

#[tauri::command]
pub async fn download_remove(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CmdResult<()> {
    crate::ops::downloads::remove(&state, id).await
}

#[tauri::command]
pub async fn download_pause(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CmdResult<()> {
    crate::ops::downloads::pause(&state, id).await
}

#[tauri::command]
pub async fn download_resume(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CmdResult<()> {
    crate::ops::downloads::resume(&state, id).await
}

#[tauri::command]
pub async fn download_play(
    state: State<'_, Arc<AppState>>,
    id: String,
    file_index: Option<usize>,
) -> CmdResult<String> {
    crate::ops::downloads::play(&state, id, file_index).await
}

#[tauri::command]
pub async fn torrent_parse_file(bytes: Vec<u8>) -> CmdResult<ParsedTorrent> {
    crate::ops::downloads::parse_torrent_file(&bytes)
}

#[tauri::command]
pub async fn download_open_folder(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CmdResult<()> {
    let target = crate::ops::downloads::folder(&state, id).await?;
    super::shared::open_folder(&app, &target).map_err(err)
}

#[tauri::command]
pub async fn download_files(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CmdResult<Vec<TorrentFileInfo>> {
    crate::ops::downloads::files(&state, id).await
}
