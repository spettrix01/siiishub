use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::torrent::sanitize_for_path;

pub const HTTP_RUN: u8 = 0;
pub const HTTP_CANCEL: u8 = 1;
pub const HTTP_PAUSE: u8 = 2;

fn http_record_path(download_dir: &Path, title: &str, filename: &str) -> PathBuf {
    let safe = sanitize_for_path(title);
    if safe.is_empty() {
        download_dir.join(filename)
    } else {
        download_dir.join(safe).join(filename)
    }
}

pub fn part_path(final_path: &Path) -> PathBuf {
    let name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    final_path.with_file_name(format!("{name}.part"))
}

fn torrent_folder_path(download_dir: &Path, title: &str) -> PathBuf {
    let safe = sanitize_for_path(title);
    if safe.is_empty() {
        download_dir.to_path_buf()
    } else {
        download_dir.join(safe)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadKind {
    Torrent,
    Http,
    /// A local audio/video file dragged into the app. Referenced in place via
    /// `source_path` — never copied, and never deleted when removed from the list.
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: String,
    pub kind: DownloadKind,
    pub title: String,
    pub poster_url: Option<String>,
    pub addon: Option<String>,
    pub filename: Option<String>,
    pub started_at: i64,
    #[serde(default, rename = "tmdbId")]
    pub tmdb_id: Option<i64>,
    #[serde(default, rename = "tmdbType")]
    pub tmdb_type: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub season: Option<i64>,
    #[serde(default)]
    pub episode: Option<i64>,
    #[serde(default)]
    pub completed_size: Option<u64>,
    /// Absolute path for `Local` records; None for torrent/http downloads.
    #[serde(default, rename = "sourcePath")]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HttpStats {
    pub downloaded: u64,
    pub length: u64,
    pub progress: f64,
    pub download_speed: f64,
    pub done: bool,
    #[serde(default)]
    pub paused: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct DownloadManager {
    inner: Arc<Inner>,
}

struct Inner {
    records: Mutex<Vec<DownloadRecord>>,
    http_progress: Mutex<HashMap<String, HttpStats>>,
    http_controls: Mutex<HashMap<String, Arc<AtomicU8>>>,
    http_urls: Mutex<HashMap<String, String>>,
    persist_path: PathBuf,
    download_dir: PathBuf,
}

impl DownloadManager {
    pub fn new(persist_path: PathBuf, download_dir: PathBuf) -> Self {
        let mut records: Vec<DownloadRecord> = Vec::new();
        if let Ok(bytes) = std::fs::read(&persist_path) {
            if let Ok(persisted) = serde_json::from_slice::<Vec<DownloadRecord>>(&bytes) {
                records = persisted
                    .into_iter()
                    .filter(|r| match r.kind {
                        DownloadKind::Http => r
                            .filename
                            .as_deref()
                            .map(|n| {
                                let final_path = http_record_path(&download_dir, &r.title, n);
                                final_path.exists() || part_path(&final_path).exists()
                            })
                            .unwrap_or(false),
                        DownloadKind::Torrent => {
                            torrent_folder_path(&download_dir, &r.title).exists()
                        }
                        DownloadKind::Local => r
                            .source_path
                            .as_deref()
                            .map(|p| Path::new(p).exists())
                            .unwrap_or(false),
                    })
                    .collect();
            }
        }

        let mut http_progress: HashMap<String, HttpStats> = HashMap::new();
        for r in &records {
            match r.kind {
                DownloadKind::Http => {
                    if let Some(name) = r.filename.as_deref() {
                        let path = http_record_path(&download_dir, &r.title, name);
                        if path.exists() {
                            let length = std::fs::metadata(&path)
                                .ok()
                                .map(|m| m.len())
                                .unwrap_or(0);
                            http_progress.insert(
                                r.id.clone(),
                                HttpStats {
                                    downloaded: length,
                                    length,
                                    progress: if length > 0 { 1.0 } else { 0.0 },
                                    download_speed: 0.0,
                                    done: true,
                                    paused: false,
                                    error: None,
                                },
                            );
                        } else {
                            let downloaded = std::fs::metadata(part_path(&path))
                                .ok()
                                .map(|m| m.len())
                                .unwrap_or(0);
                            http_progress.insert(
                                r.id.clone(),
                                HttpStats {
                                    downloaded,
                                    length: 0,
                                    progress: 0.0,
                                    download_speed: 0.0,
                                    done: false,
                                    paused: false,
                                    error: Some(crate::util::loc("error.download.interrupted")),
                                },
                            );
                        }
                    }
                }
                DownloadKind::Torrent => {
                    if r.completed_size.is_none() {
                        http_progress.insert(
                            r.id.clone(),
                            HttpStats {
                                downloaded: 0,
                                length: 0,
                                progress: 0.0,
                                download_speed: 0.0,
                                done: false,
                                paused: false,
                                error: Some(crate::util::loc("error.download.interrupted")),
                            },
                        );
                    }
                }
                // Local files are always complete on disk; download_list
                // computes their stats fresh from the source path.
                DownloadKind::Local => {}
            }
        }

        Self {
            inner: Arc::new(Inner {
                records: Mutex::new(records),
                http_progress: Mutex::new(http_progress),
                http_controls: Mutex::new(HashMap::new()),
                http_urls: Mutex::new(HashMap::new()),
                persist_path,
                download_dir,
            }),
        }
    }

    pub fn upsert(&self, rec: DownloadRecord) {
        {
            let mut g = self.inner.records.lock();
            if let Some(slot) = g.iter_mut().find(|r| r.id == rec.id) {
                *slot = rec;
            } else {
                g.push(rec);
            }
        }
        self.save();
    }

    pub fn set_filename(&self, id: &str, filename: String) {
        {
            let mut g = self.inner.records.lock();
            if let Some(slot) = g.iter_mut().find(|r| r.id == id) {
                slot.filename = Some(filename);
            }
        }
        self.save();
    }

    pub fn mark_torrent_done(&self, id: &str, size: u64) {
        {
            let mut g = self.inner.records.lock();
            match g.iter_mut().find(|r| r.id == id) {
                Some(slot) if slot.completed_size != Some(size) => {
                    slot.completed_size = Some(size);
                }
                _ => return,
            }
        }
        self.save();
    }

    pub fn list(&self) -> Vec<DownloadRecord> {
        self.inner.records.lock().clone()
    }

    pub fn remove(&self, id: &str) {
        self.inner.records.lock().retain(|r| r.id != id);
        self.inner.http_progress.lock().remove(id);
        self.inner.http_controls.lock().remove(id);
        self.inner.http_urls.lock().remove(id);
        self.save();
    }

    pub fn set_http_stats(&self, id: &str, stats: HttpStats) {
        self.inner.http_progress.lock().insert(id.to_string(), stats);
    }

    pub fn http_stats(&self, id: &str) -> Option<HttpStats> {
        self.inner.http_progress.lock().get(id).cloned()
    }

    pub fn try_register_http_control(&self, id: &str) -> Option<Arc<AtomicU8>> {
        use std::collections::hash_map::Entry;
        match self.inner.http_controls.lock().entry(id.to_string()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(slot) => {
                let flag = Arc::new(AtomicU8::new(HTTP_RUN));
                slot.insert(flag.clone());
                Some(flag)
            }
        }
    }

    pub fn cancel_http(&self, id: &str) {
        if let Some(flag) = self.inner.http_controls.lock().get(id) {
            flag.store(HTTP_CANCEL, Ordering::Relaxed);
        }
    }

    pub fn pause_http(&self, id: &str) {
        if let Some(flag) = self.inner.http_controls.lock().get(id) {
            flag.store(HTTP_PAUSE, Ordering::Relaxed);
        }
    }

    pub fn clear_http_control(&self, id: &str) {
        self.inner.http_controls.lock().remove(id);
    }

    pub fn set_http_url(&self, id: &str, url: String) {
        self.inner.http_urls.lock().insert(id.to_string(), url);
    }

    pub fn http_url(&self, id: &str) -> Option<String> {
        self.inner.http_urls.lock().get(id).cloned()
    }

    pub fn download_dir(&self) -> &PathBuf {
        &self.inner.download_dir
    }

    fn save(&self) {
        let to_save: Vec<DownloadRecord> = self
            .inner
            .records
            .lock()
            .iter()
            .cloned()
            .collect();
        let bytes = match serde_json::to_vec_pretty(&to_save) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("[downloads] serialize failed: {e:#}");
                return;
            }
        };
        let dest = &self.inner.persist_path;
        let tmp = dest.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, &bytes) {
            tracing::warn!(
                "[downloads] tmp write failed ({}): {e:#}",
                tmp.display()
            );
            return;
        }
        if let Err(e) = std::fs::rename(&tmp, dest) {
            tracing::warn!(
                "[downloads] rename failed ({} -> {}): {e:#}",
                tmp.display(),
                dest.display()
            );
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

pub fn http_id_for(url: &str) -> String {
    use sha2::{Digest, Sha256};
    let bytes = Sha256::digest(url.as_bytes());
    let mut hex = String::with_capacity(40);
    for b in &bytes[..20] {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{:02x}", b);
    }
    hex
}
