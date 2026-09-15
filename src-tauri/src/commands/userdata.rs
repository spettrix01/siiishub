use std::collections::BTreeMap;
use std::sync::Arc;

use tauri::State;

use crate::state::AppState;

use super::shared::{err, CmdResult};

#[tauri::command]
pub async fn userdata_load(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<BTreeMap<String, String>> {
    Ok(state.userdata.snapshot())
}

#[tauri::command]
pub async fn userdata_set(
    state: State<'_, Arc<AppState>>,
    key: String,
    value: String,
) -> CmdResult<()> {
    state.userdata.set(key, value).await.map_err(err)
}

#[tauri::command]
pub async fn userdata_remove(
    state: State<'_, Arc<AppState>>,
    key: String,
) -> CmdResult<()> {
    state.userdata.remove(&key).await.map_err(err)
}
