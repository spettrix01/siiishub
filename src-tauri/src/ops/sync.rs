//! An account's sync (`sync.rs`), as the server and the app run it: the
//! changes after a point in a profile's order, and taking the other side's
//! newer ones.

use serde::{Deserialize, Serialize};

use crate::settings::{Settings, SettingsStore};
use crate::state::AppState;
use crate::sync::{self, SyncLog};
use crate::userdata::UserDataStore;

use super::{err, CmdResult};

/// A key with its value (none: deleted) as it was changed at `ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
    pub ts: u64,
}

/// What a sync works on: a profile's settings, its user data and when
/// they changed.
#[derive(Clone, Copy)]
pub struct Stores<'a> {
    pub settings: &'a SettingsStore,
    pub userdata: &'a UserDataStore,
    pub log: &'a SyncLog,
}

impl<'a> From<&'a AppState> for Stores<'a> {
    fn from(state: &'a AppState) -> Self {
        Self { settings: &state.settings, userdata: &state.userdata, log: &state.sync }
    }
}

/// Most changes taken at once, and the longest value: a profile holds
/// settings and small records, nothing near these.
const MAX_CHANGES: usize = 50_000;
const MAX_VALUE: usize = 1 << 20;

/// The changes after `seq` (with `own`, only the ones made on this side),
/// and the place in the order they reach.
pub fn changes_since(stores: Stores<'_>, seq: u64, own: bool) -> (Vec<Change>, u64) {
    let settings = sync::setting_values(&stores.settings.read());
    let stamps = stores.log.since(seq);
    let cursor = stamps.last().map_or(seq, |(_, stamp)| stamp.seq);
    let changes = stamps
        .into_iter()
        .filter(|(_, stamp)| !(own && stamp.remote))
        .map(|(key, stamp)| {
            let value = match sync::setting_field(&key) {
                Some(field) => settings.get(field).map(|v| v.to_string()),
                None => stores.userdata.get(&key),
            };
            Change { key, value, ts: stamp.ts }
        })
        .collect();
    (changes, cursor)
}

/// What `apply` changed.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct Applied {
    pub userdata: bool,
    pub settings: bool,
}

/// Takes the other side's changes newer than this side's, key by key. From
/// the server (`from_server`), its values also win over the ones this side
/// had before it ever synced.
pub async fn apply(stores: Stores<'_>, changes: Vec<Change>, from_server: bool) -> CmdResult<Applied> {
    if changes.len() > MAX_CHANGES {
        return Err("too many changes".to_string());
    }
    let mut applied = Applied::default();
    let mut settings = serde_json::to_value(stores.settings.read()).map_err(err)?;
    let mut userdata = Vec::new();
    let mut taken = Vec::new();
    for change in changes {
        if change.value.as_ref().is_some_and(|v| v.len() > MAX_VALUE) {
            continue;
        }
        let newer = match stores.log.stamp(&change.key) {
            None => true,
            Some(stamp) => change.ts > stamp.ts || (from_server && stamp.ts == 0 && !stamp.remote),
        };
        if !newer {
            continue;
        }
        if let Some(field) = sync::setting_field(&change.key) {
            // A setting is never deleted, only changed.
            let Some(value) = change.value.as_deref().and_then(|v| serde_json::from_str(v).ok()) else {
                continue;
            };
            settings[field] = value;
            applied.settings = true;
        } else if sync::synced_userdata(&change.key) {
            userdata.push((change.key.clone(), change.value));
            applied.userdata = true;
        } else {
            continue;
        }
        taken.push((change.key, change.ts));
    }
    if applied.settings {
        let next: Settings = serde_json::from_value(settings).map_err(err)?;
        stores.settings.write(next).await.map_err(err)?;
    }
    stores.userdata.update(userdata).await.map_err(err)?;
    stores.log.record(&taken, from_server).await;
    Ok(applied)
}
