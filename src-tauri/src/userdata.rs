use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::RwLock;

#[derive(Clone)]
pub struct UserDataStore {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    data: RwLock<BTreeMap<String, String>>,
    write_lock: tokio::sync::Mutex<()>,
}

impl UserDataStore {
    pub async fn open(path: PathBuf) -> Result<Self> {
        let data: BTreeMap<String, String> = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => BTreeMap::new(),
        };
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                data: RwLock::new(data),
                write_lock: tokio::sync::Mutex::new(()),
            }),
        })
    }

    pub fn snapshot(&self) -> BTreeMap<String, String> {
        self.inner.data.read().clone()
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.inner.data.read().get(key).cloned()
    }

    pub async fn set(&self, key: String, value: String) -> Result<()> {
        {
            self.inner.data.write().insert(key, value);
        }
        self.persist().await
    }

    pub async fn remove(&self, key: &str) -> Result<()> {
        {
            self.inner.data.write().remove(key);
        }
        self.persist().await
    }

    /// Sets (`Some`) or removes (`None`) several keys, saved once.
    pub async fn update(&self, changes: Vec<(String, Option<String>)>) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        {
            let mut data = self.inner.data.write();
            for (key, value) in changes {
                match value {
                    Some(value) => data.insert(key, value),
                    None => data.remove(&key),
                };
            }
        }
        self.persist().await
    }

    async fn persist(&self) -> Result<()> {
        let _write = self.inner.write_lock.lock().await;
        let bytes = serde_json::to_vec_pretty(&self.snapshot())?;
        let tmp = self.inner.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes)
            .await
            .with_context(|| format!("writing {}", tmp.display()))?;
        tokio::fs::rename(&tmp, &self.inner.path)
            .await
            .context("renaming userdata tmp into place")?;
        Ok(())
    }
}
