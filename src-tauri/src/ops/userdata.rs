use std::collections::BTreeMap;

use crate::state::AppState;

use super::{err, CmdResult};

pub fn load(state: &AppState) -> BTreeMap<String, String> {
    state.userdata.snapshot()
}

pub async fn set(state: &AppState, key: String, value: String) -> CmdResult<()> {
    state.userdata.set(key, value).await.map_err(err)
}

pub async fn remove(state: &AppState, key: &str) -> CmdResult<()> {
    state.userdata.remove(key).await.map_err(err)
}
