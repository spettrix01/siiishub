use std::collections::BTreeMap;

use crate::state::AppState;
use crate::sync;

use super::{err, CmdResult};

pub fn load(state: &AppState) -> BTreeMap<String, String> {
    state.userdata.snapshot()
}

pub async fn set(state: &AppState, key: String, value: String) -> CmdResult<()> {
    state.userdata.set(key.clone(), value).await.map_err(err)?;
    if sync::synced_userdata(&key) {
        state.sync.touch(&[key]).await;
    }
    Ok(())
}

pub async fn remove(state: &AppState, key: &str) -> CmdResult<()> {
    state.userdata.remove(key).await.map_err(err)?;
    if sync::synced_userdata(key) {
        state.sync.touch(&[key.to_string()]).await;
    }
    Ok(())
}
