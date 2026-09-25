//! What keeps the devices of a SIIISHUB account alike. The app on each
//! device and the server keep the same settings and user data, and every
//! change is stamped with the time it was made. Each key (a user data key,
//! or `settings.<field>` for a setting) remembers when it changed last (`ts`,
//! milliseconds on the clock of the device that changed it) and its place in
//! the profile's own order of changes (`seq`). A sync sends what changed
//! since the last one and takes what the other side has newer, key by key.
//! A deleted key keeps its stamp, so a deletion travels like a change.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// User data that belongs to the device: what it downloaded and its own
/// bookkeeping.
const LOCAL_KEYS: &[&str] = &["siiis:dl:", "_migrated"];

/// The settings a sync carries. The rest (the remote's port, the phones
/// paired with it) belong to the device.
pub const SYNCED_SETTINGS: &[&str] = &[
    "tmdb_key",
    "addons",
    "rd_token",
    "debrid_provider",
    "debrid_token",
    "tracker_fallbacks",
    "player_audio_langs",
    "player_sub_langs",
    "language",
];

const SETTING: &str = "settings.";

/// Whether a user data key travels with the account.
pub fn synced_userdata(key: &str) -> bool {
    !key.starts_with(SETTING) && !LOCAL_KEYS.iter().any(|prefix| key.starts_with(prefix))
}

/// The setting a sync key stands for, if it is a synced one.
pub fn setting_field(key: &str) -> Option<&'static str> {
    let field = key.strip_prefix(SETTING)?;
    SYNCED_SETTINGS.iter().copied().find(|f| *f == field)
}

pub fn setting_key(field: &str) -> String {
    format!("{SETTING}{field}")
}

/// The synced settings as JSON, field by field.
pub fn setting_values(settings: &Settings) -> BTreeMap<&'static str, serde_json::Value> {
    let all = serde_json::to_value(settings).unwrap_or_default();
    SYNCED_SETTINGS
        .iter()
        .map(|f| (*f, all.get(*f).cloned().unwrap_or_default()))
        .collect()
}

/// The keys a profile has before syncing ever ran: its user data, and the
/// synced settings that hold something.
pub fn existing_keys(settings: &Settings, userdata: &BTreeMap<String, String>) -> Vec<String> {
    let settings = setting_values(settings)
        .into_iter()
        .filter(|(_, value)| match value {
            serde_json::Value::String(s) => !s.is_empty(),
            serde_json::Value::Array(a) => !a.is_empty(),
            serde_json::Value::Null => false,
            _ => true,
        })
        .map(|(field, _)| setting_key(field));
    userdata
        .keys()
        .filter(|k| synced_userdata(k))
        .cloned()
        .chain(settings)
        .collect()
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Stamp {
    pub ts: u64,
    pub seq: u64,
    /// Taken from the server: the app has nothing to send back.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remote: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Log {
    seq: u64,
    stamps: BTreeMap<String, Stamp>,
}

#[derive(Clone)]
pub struct SyncLog {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    log: Mutex<Log>,
    write: tokio::sync::Mutex<()>,
    /// A change made here (the app's sync waits for it).
    changed: tokio::sync::Notify,
    /// The last place in the order, for whoever waits for any change (the
    /// server's answer to an app waiting for news).
    seq: tokio::sync::watch::Sender<u64>,
}

impl SyncLog {
    /// Opens the log at `path`. The first time, the keys already there get a
    /// stamp from before syncing existed (time 0): the account's values win
    /// over them, and the ones the account lacks are sent to it.
    pub async fn open(path: PathBuf, existing: impl IntoIterator<Item = String>) -> Self {
        let log = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => {
                let mut log = Log::default();
                for key in existing {
                    log.seq += 1;
                    log.stamps.insert(key, Stamp { ts: 0, seq: log.seq, remote: false });
                }
                log
            }
        };
        let fresh = !path.exists();
        let (seq, _) = tokio::sync::watch::channel(log.seq);
        let this = Self {
            inner: Arc::new(Inner {
                path,
                log: Mutex::new(log),
                write: tokio::sync::Mutex::new(()),
                changed: tokio::sync::Notify::new(),
                seq,
            }),
        };
        if fresh {
            this.persist().await;
        }
        this
    }

    pub fn stamp(&self, key: &str) -> Option<Stamp> {
        self.inner.log.lock().stamps.get(key).copied()
    }

    /// The last place in the order of changes.
    pub fn seq(&self) -> u64 {
        self.inner.log.lock().seq
    }

    /// Keys changed here, now.
    pub async fn touch(&self, keys: &[String]) {
        if keys.is_empty() {
            return;
        }
        let ts = now_ms();
        let last = {
            let mut log = self.inner.log.lock();
            for key in keys {
                log.seq += 1;
                let seq = log.seq;
                log.stamps.insert(key.clone(), Stamp { ts, seq, remote: false });
            }
            log.seq
        };
        self.persist().await;
        self.inner.changed.notify_one();
        self.inner.seq.send_replace(last);
    }

    /// Keys taken from the other side, with the times they were changed.
    pub async fn record(&self, changes: &[(String, u64)], remote: bool) {
        if changes.is_empty() {
            return;
        }
        let last = {
            let mut log = self.inner.log.lock();
            for (key, ts) in changes {
                log.seq += 1;
                let seq = log.seq;
                log.stamps.insert(key.clone(), Stamp { ts: *ts, seq, remote });
            }
            log.seq
        };
        self.persist().await;
        self.inner.seq.send_replace(last);
    }

    /// The keys changed after `seq`, in order.
    pub fn since(&self, seq: u64) -> Vec<(String, Stamp)> {
        let log = self.inner.log.lock();
        let mut out: Vec<(String, Stamp)> = log
            .stamps
            .iter()
            .filter(|(_, s)| s.seq > seq)
            .map(|(k, s)| (k.clone(), *s))
            .collect();
        out.sort_by_key(|(_, s)| s.seq);
        out
    }

    /// Waits for a change made here.
    pub async fn changed(&self) {
        self.inner.changed.notified().await;
    }

    /// Waits until the order goes past `seq`, whoever made the change.
    pub async fn past(&self, seq: u64) {
        let mut rx = self.inner.seq.subscribe();
        let _ = rx.wait_for(|last| *last > seq).await;
    }

    async fn persist(&self) {
        let _write = self.inner.write.lock().await;
        let bytes = {
            let log = self.inner.log.lock();
            serde_json::to_vec(&*log)
        };
        let Ok(bytes) = bytes else {
            return;
        };
        let tmp = self.inner.path.with_extension("json.tmp");
        let written = async {
            tokio::fs::write(&tmp, &bytes).await?;
            tokio::fs::rename(&tmp, &self.inner.path).await
        };
        if let Err(e) = written.await {
            tracing::warn!("[sync] saving {} failed: {e}", self.inner.path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert!(synced_userdata("siiis:resume:tv:1:1:1"));
        assert!(synced_userdata("siiishub-theme"));
        assert!(!synced_userdata("siiis:dl:seen"));
        assert!(!synced_userdata("_migrated_v1"));
        assert!(!synced_userdata("settings.addons"));
        assert_eq!(setting_field("settings.addons"), Some("addons"));
        assert_eq!(setting_field("settings.remote_port"), None);
        let values = setting_values(&Settings::default());
        assert_eq!(values.len(), SYNCED_SETTINGS.len());
        assert!(values.values().all(|v| !v.is_null()));
    }
}
