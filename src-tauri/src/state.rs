use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(feature = "app")]
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

use crate::download::DownloadManager;
#[cfg(feature = "app")]
use crate::mpv::Mpv;
use crate::remote::RemoteController;
use crate::settings::{PublicSettings, SettingsStore};
use crate::sync::SyncLog;
use crate::torrent::TorrentManager;
use crate::userdata::UserDataStore;

pub struct AppState {
    pub settings: SettingsStore,
    pub userdata: UserDataStore,
    /// When each synced setting and user data key changed (`sync.rs`).
    pub sync: SyncLog,
    pub http: reqwest::Client,
    pub torrents: TorrentManager,
    pub downloads: DownloadManager,
    #[cfg(feature = "app")]
    pub mpv: Mutex<Option<Arc<Mpv>>>,
    pub data_dir: PathBuf,
    pub download_dir: PathBuf,
    #[cfg(feature = "app")]
    pub main_hwnd: Mutex<Option<isize>>,
    pub remote: RemoteController,
    resolve_cancels: Mutex<HashMap<String, CancellationToken>>,
}

impl AppState {
    #[cfg(feature = "app")]
    pub async fn initialize(app: tauri::AppHandle) -> Result<Self> {
        use tauri::Manager;
        let data_dir = app
            .path()
            .app_data_dir()
            .context("failed to resolve app_data_dir")?;
        #[cfg(target_os = "android")]
        let download_dir = android_download_dir(&data_dir);
        #[cfg(not(target_os = "android"))]
        let download_dir = data_dir.join("download");
        Self::open(data_dir, download_dir).await
    }

    /// Opens the stores kept in `data_dir` and the torrent session writing to
    /// `download_dir`: the app passes its platform folders, the web server
    /// the ones it is configured with.
    pub async fn open(data_dir: PathBuf, download_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir).ok();

        let (settings, userdata, sync) = open_profile(&data_dir).await?;

        let http = reqwest::Client::builder()
            .user_agent("siiishub/0.1")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("building reqwest client")?;

        std::fs::create_dir_all(&download_dir).ok();
        let temp_dir = download_dir.join("download_temp");
        std::fs::create_dir_all(&temp_dir).ok();

        if let Ok(mut entries) = tokio::fs::read_dir(&temp_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                let _ = if is_dir {
                    tokio::fs::remove_dir_all(&path).await
                } else {
                    tokio::fs::remove_file(&path).await
                };
            }
        }

        let torrents = TorrentManager::new(temp_dir, download_dir.clone(), &data_dir).await?;

        let downloads_persist = data_dir.join("downloads.json");
        let downloads = DownloadManager::new(downloads_persist, download_dir.clone());

        let remote = RemoteController::new(settings.clone());

        Ok(Self {
            settings,
            userdata,
            sync,
            http,
            torrents,
            downloads,
            #[cfg(feature = "app")]
            mpv: Mutex::new(None),
            data_dir,
            download_dir,
            #[cfg(feature = "app")]
            main_hwnd: Mutex::new(None),
            remote,
            resolve_cancels: Mutex::new(HashMap::new()),
        })
    }

    /// Another profile on the same server: an account's settings and user
    /// data, kept in `dir`, over this state's torrents, downloads and remote.
    #[cfg(feature = "server")]
    pub async fn profile(&self, dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir).ok();
        let (settings, userdata, sync) = open_profile(&dir).await?;
        Ok(Self {
            settings,
            userdata,
            sync,
            http: self.http.clone(),
            torrents: self.torrents.clone(),
            downloads: self.downloads.clone(),
            #[cfg(feature = "app")]
            mpv: Mutex::new(None),
            data_dir: dir,
            download_dir: self.download_dir.clone(),
            #[cfg(feature = "app")]
            main_hwnd: Mutex::new(None),
            remote: self.remote.clone(),
            resolve_cancels: Mutex::new(HashMap::new()),
        })
    }

    /// `media_cancel` can race ahead of the resolve command that registers the
    /// id, so both sides share the same map entry instead of replacing it.
    pub fn resolve_cancel_token(&self, id: &str) -> CancellationToken {
        self.resolve_cancels
            .lock()
            .entry(id.to_string())
            .or_default()
            .clone()
    }

    pub fn cancel_resolve(&self, id: &str) {
        self.resolve_cancels
            .lock()
            .entry(id.to_string())
            .or_default()
            .cancel();
    }

    pub fn clear_resolve_cancel(&self, id: &str) {
        self.resolve_cancels.lock().remove(id);
    }

    pub fn public_settings(&self) -> PublicSettings {
        PublicSettings::build(
            &self.settings.read(),
            self.download_dir.to_string_lossy().into_owned(),
        )
    }

    #[cfg(feature = "app")]
    pub fn set_mpv(&self, mpv: Arc<Mpv>) {
        *self.mpv.lock() = Some(mpv);
    }

    #[cfg(feature = "app")]
    pub fn mpv(&self) -> Option<Arc<Mpv>> {
        self.mpv.lock().clone()
    }

    #[cfg(feature = "app")]
    pub fn set_main_hwnd(&self, hwnd: isize) {
        *self.main_hwnd.lock() = Some(hwnd);
    }

    #[cfg(feature = "app")]
    pub fn main_hwnd(&self) -> Option<isize> {
        *self.main_hwnd.lock()
    }
}

/// A profile's stores in `dir`: its settings, its user data and when each
/// synced key of them changed.
async fn open_profile(dir: &std::path::Path) -> Result<(SettingsStore, UserDataStore, SyncLog)> {
    let settings = SettingsStore::open(dir.join("settings.json")).await?;
    let userdata = UserDataStore::open(dir.join("userdata.json")).await?;
    let existing = crate::sync::existing_keys(&settings.read(), &userdata.snapshot());
    let sync = SyncLog::open(dir.join("sync.json"), existing).await;
    Ok((settings, userdata, sync))
}

/// Android keeps downloads in the shared `Download/SIIISHUB` folder so a file
/// manager can show them: from Android 11 an app may create its own files
/// there without any storage permission. Older releases, or a folder that
/// turns out not to be writable, fall back to the app's private folder. Files
/// already downloaded into the private folder are moved over once, unless
/// there are so many that the move would stall the start-up.
#[cfg(target_os = "android")]
fn android_download_dir(data_dir: &std::path::Path) -> PathBuf {
    use std::path::Path;

    let private = data_dir.join("download");
    let user = data_dir
        .to_string_lossy()
        .strip_prefix("/data/user/")
        .and_then(|rest| rest.split('/').next())
        .and_then(|n| n.parse::<u32>().ok())
        .unwrap_or(0);
    let public = PathBuf::from(format!("/storage/emulated/{user}/Download/SIIISHUB"));

    fn writable(dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let probe = dir.join(".siiishub-write-test");
        std::fs::write(&probe, b"ok")?;
        std::fs::remove_file(&probe)
    }

    if let Err(e) = writable(&public) {
        tracing::warn!(
            "cartella download condivisa non scrivibile ({e}): uso {}",
            private.display()
        );
        return private;
    }
    if private.is_dir() {
        match migrate_downloads(&private, &public) {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                "{n} elementi spostati da {} a {}",
                private.display(),
                public.display()
            ),
            Err(e) => {
                tracing::warn!("download precedenti lasciati in {} ({e})", private.display());
                return private;
            }
        }
    }
    tracing::info!("cartella download: {}", public.display());
    public
}

/// Moves the entries of `from` into `to` (rename, or copy + delete across
/// file systems), skipping the torrent temp folder. Refuses when there is
/// more than 512 MB to copy so a big library never blocks the start-up; the
/// caller then keeps using `from`.
#[cfg(target_os = "android")]
fn migrate_downloads(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<usize> {
    use std::path::Path;

    fn size_of(path: &Path) -> std::io::Result<u64> {
        let meta = std::fs::metadata(path)?;
        if !meta.is_dir() {
            return Ok(meta.len());
        }
        let mut total = 0;
        for entry in std::fs::read_dir(path)? {
            total += size_of(&entry?.path())?;
        }
        Ok(total)
    }
    fn move_entry(src: &Path, dst: &Path) -> std::io::Result<()> {
        if std::fs::rename(src, dst).is_ok() {
            return Ok(());
        }
        if src.is_dir() {
            std::fs::create_dir_all(dst)?;
            for entry in std::fs::read_dir(src)? {
                let entry = entry?;
                move_entry(&entry.path(), &dst.join(entry.file_name()))?;
            }
            std::fs::remove_dir(src)
        } else {
            std::fs::copy(src, dst)?;
            std::fs::remove_file(src)
        }
    }

    let entries: Vec<_> = std::fs::read_dir(from)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != "download_temp")
        .collect();
    if entries.is_empty() {
        return Ok(0);
    }
    let mut total = 0;
    for entry in &entries {
        total += size_of(&entry.path())?;
    }
    if total > 512 * 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "piu' di 512 MB da spostare",
        ));
    }
    let mut moved = 0;
    for entry in entries {
        let dst = to.join(entry.file_name());
        if dst.exists() {
            continue;
        }
        move_entry(&entry.path(), &dst)?;
        moved += 1;
    }
    Ok(moved)
}
