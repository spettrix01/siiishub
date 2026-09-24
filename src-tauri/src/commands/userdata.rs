use std::collections::BTreeMap;
use std::sync::Arc;

use tauri::State;

use crate::state::AppState;

use super::shared::CmdResult;

#[tauri::command]
pub async fn userdata_load(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<BTreeMap<String, String>> {
    Ok(crate::ops::userdata::load(&state))
}

#[tauri::command]
pub async fn userdata_set(
    state: State<'_, Arc<AppState>>,
    key: String,
    value: String,
) -> CmdResult<()> {
    crate::ops::userdata::set(&state, key, value).await
}

#[tauri::command]
pub async fn userdata_remove(
    state: State<'_, Arc<AppState>>,
    key: String,
) -> CmdResult<()> {
    crate::ops::userdata::remove(&state, &key).await
}
