use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddonConfig {
    #[serde(default)]
    pub name: String,
    pub url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub resources: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// A phone (or other client) approved once for the remote control that may
/// reconnect without asking again. The token is a secret generated at approval
/// time and kept by the client, which presents it on every connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteDevice {
    pub token: String,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub first_seen: u64,
    #[serde(default)]
    pub last_seen: u64,
}

fn default_remote_port() -> u16 {
    9871
}

fn default_language() -> String {
    "eng".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub tmdb_key: String,
    #[serde(default)]
    pub addons: Vec<AddonConfig>,
    #[serde(default)]
    pub rd_token: String,
    #[serde(default)]
    pub debrid_provider: String,
    #[serde(default)]
    pub debrid_token: String,
    #[serde(default)]
    pub tracker_fallbacks: Vec<String>,
    #[serde(default)]
    pub player_audio_langs: Vec<String>,
    #[serde(default)]
    pub player_sub_langs: Vec<String>,
    #[serde(default = "default_remote_port")]
    pub remote_port: u16,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub remote_devices: Vec<RemoteDevice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicSettings {
    pub tmdb_key: String,
    pub addons: Vec<AddonConfig>,
    pub rd_available: bool,
    pub debrid_configured: bool,
    pub debrid_provider: String,
    pub debrid_token: String,
    pub tracker_fallbacks: Vec<String>,
    pub player_audio_langs: Vec<String>,
    pub player_sub_langs: Vec<String>,
    pub remote_port: u16,
    pub language: String,
    pub download_dir: String,
}

impl PublicSettings {
    pub fn build(s: &Settings, download_dir: String) -> Self {
        let provider = s.debrid_provider.trim().to_lowercase();
        let token = s.debrid_token.clone();
        let rd_available = provider == "rd" && !token.is_empty();
        let debrid_configured = (provider == "rd" || provider == "ad") && !token.is_empty();
        let language = {
            let l = s.language.trim().to_lowercase();
            if l.is_empty() {
                default_language()
            } else {
                l
            }
        };
        Self {
            tmdb_key: s.tmdb_key.clone(),
            addons: s.addons.clone(),
            rd_available,
            debrid_configured,
            debrid_provider: provider,
            debrid_token: token,
            tracker_fallbacks: s.tracker_fallbacks.clone(),
            player_audio_langs: s.player_audio_langs.clone(),
            player_sub_langs: s.player_sub_langs.clone(),
            remote_port: if s.remote_port == 0 { default_remote_port() } else { s.remote_port },
            language,
            download_dir,
        }
    }
}

#[derive(Clone)]
pub struct SettingsStore {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    data: RwLock<Settings>,
    write_lock: tokio::sync::Mutex<()>,
}

impl SettingsStore {
    pub async fn open(path: PathBuf) -> Result<Self> {
        let data = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice::<Settings>(&bytes).unwrap_or_default(),
            Err(_) => Settings::default(),
        };
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                data: RwLock::new(data),
                write_lock: tokio::sync::Mutex::new(()),
            }),
        })
    }

    pub fn read(&self) -> Settings {
        self.inner.data.read().clone()
    }

    pub async fn write(&self, next: Settings) -> Result<()> {
        let _write = self.inner.write_lock.lock().await;
        {
            let mut g = self.inner.data.write();
            *g = next;
        }
        let bytes = serde_json::to_vec_pretty(&self.read())?;

        let tmp = self.inner.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes)
            .await
            .with_context(|| format!("writing {}", tmp.display()))?;
        tokio::fs::rename(&tmp, &self.inner.path)
            .await
            .context("renaming settings tmp into place")?;
        Ok(())
    }
}
