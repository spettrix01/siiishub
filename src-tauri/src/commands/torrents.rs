use std::sync::Arc;

use tauri::State;

use crate::state::AppState;
use crate::torrent::TorrentStats;

use super::shared::CmdResult;

#[tauri::command]
pub async fn torrent_stats(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> CmdResult<Option<TorrentStats>> {
    Ok(crate::ops::torrents::stats(&state, &info_hash))
}

#[tauri::command]
pub async fn session_destroy(
    state: State<'_, Arc<AppState>>,
    info_hash: String,
) -> CmdResult<()> {
    crate::ops::torrents::destroy(&state, &info_hash);
    Ok(())
}
