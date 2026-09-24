use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use librqbit::api::TorrentIdOrHash;
use librqbit::http_api::HttpApi;
use librqbit::dht::{Id20, PersistentDhtConfig};
use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Api, Session, SessionOptions};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "m4v", "wmv", "flv", "ts", "m2ts",
];

const METADATA_TIMEOUT: Duration = Duration::from_secs(60);
const ADD_TORRENT_TIMEOUT: Duration = Duration::from_secs(60);
const FOLDER_MAX_NAME_CHARS: usize = 80;

#[derive(Clone)]
pub struct TorrentManager {
    inner: Arc<Inner>,
}

struct Inner {
    session: Arc<Session>,
    http_base: String,
    temp_dir: PathBuf,
    permanent_dir: PathBuf,
    active: Mutex<Vec<ActiveHandle>>,
    permanent_set: Mutex<HashSet<String>>,
    pending_destroys: Mutex<HashMap<String, JoinHandle<()>>>,
    display_names: Mutex<HashMap<String, String>>,
}

pub fn sanitize_for_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_space = false;
    for c in s.chars() {
        let safe = match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        };
        let is_space = safe == ' ';
        if is_space && last_was_space {
            continue;
        }
        last_was_space = is_space;
        out.push(safe);
    }
    let trimmed = out.trim_matches(|c: char| c == ' ' || c == '.' || c == '_');
    trimmed.chars().take(FOLDER_MAX_NAME_CHARS).collect()
}

fn temp_folder_name(display_name: Option<&str>, info_hash_lc: &str) -> String {
    let safe = display_name
        .map(sanitize_for_path)
        .filter(|s| !s.is_empty());
    match safe {
        Some(name) => {
            let short = &info_hash_lc[..8.min(info_hash_lc.len())];
            format!("{name} [{short}]")
        }
        None => info_hash_lc.to_string(),
    }
}

#[derive(Clone)]
struct ActiveHandle {
    info_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TorrentStats {
    pub peers: u32,
    pub download_speed: f64,
    pub upload_speed: f64,
    pub progress: f64,
    pub downloaded: u64,
    pub length: u64,
    pub ready: bool,
    /// True only while the torrent is actively live (not fetching metadata,
    /// not re-checking files, not paused) — the only state librqbit can pause.
    pub live: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TorrentResolved {
    pub url: String,
    pub filename: String,
    pub filesize: u64,
    pub info_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TorrentFileInfo {
    pub idx: usize,
    pub path: String,
    pub basename: String,
    pub size: u64,
}

impl TorrentManager {
    /// `state_dir` holds the DHT routing table dump. librqbit would otherwise
    /// derive it from the OS cache directory, which Android does not expose
    /// (no HOME): the session creation failed and the app died at startup.
    pub async fn new(temp_dir: PathBuf, permanent_dir: PathBuf, state_dir: &Path) -> Result<Self> {
        let opts = SessionOptions {
            dht_config: Some(PersistentDhtConfig {
                config_filename: Some(state_dir.join("dht.json")),
                ..Default::default()
            }),
            ..Default::default()
        };
        let session = Session::new_with_opts(temp_dir.clone(), opts)
            .await
            .context("starting librqbit session")?;

        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .context("binding librqbit HTTP API listener")?;
        let local_addr = listener.local_addr().context("HTTP API local_addr")?;
        let http_base = format!("http://{}", local_addr);

        let api = Api::new(session.clone(), None, None);
        let http = HttpApi::new(api, None);
        tokio::spawn(async move {
            if let Err(e) = http.make_http_api_and_run(listener, None).await {
                tracing::error!("librqbit HTTP API stopped: {e:#}");
            }
        });

        Ok(Self {
            inner: Arc::new(Inner {
                session,
                http_base,
                temp_dir,
                permanent_dir,
                active: Mutex::new(Vec::new()),
                permanent_set: Mutex::new(HashSet::new()),
                pending_destroys: Mutex::new(HashMap::new()),
                display_names: Mutex::new(HashMap::new()),
            }),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn resolve(
        &self,
        info_hash: &str,
        file_hint: Option<&str>,
        display_name: Option<&str>,
        sources: &[String],
        user_fallbacks: &[String],
        permanent: bool,
        cancel: Option<&tokio_util::sync::CancellationToken>,
        on_progress: impl Fn(&str),
    ) -> Result<TorrentResolved> {
        let info_hash_lc = info_hash.to_ascii_lowercase();

        if crate::util::is_cancelled(cancel) {
            bail!("{}", crate::util::loc("details.playback.cancelled"));
        }

        if permanent {
            self.inner.permanent_set.lock().insert(info_hash_lc.clone());
        }

        if let Some(dn) = display_name.filter(|s| !s.is_empty()) {
            self.inner
                .display_names
                .lock()
                .entry(info_hash_lc.clone())
                .or_insert_with(|| dn.to_string());
        }

        let pending = self.inner.pending_destroys.lock().remove(&info_hash_lc);
        if let Some(task) = pending {
            tracing::info!(
                "[torrent] {info_hash_lc} awaiting in-flight destroy before re-add"
            );
            on_progress(&crate::util::loc("progress.torrent.cleaning"));
            let _ = task.await;
        }

        let existing = parse_id(&info_hash_lc)
            .and_then(|id| self.inner.session.get(id));

        let handle = if let Some(h) = existing {
            tracing::info!(
                "[torrent] {info_hash_lc} already in session, reusing handle (skipping add_torrent)"
            );
            on_progress(&crate::util::loc("progress.torrent.resuming"));
            h
        } else {
            let mut magnet = format!("magnet:?xt=urn:btih:{}", info_hash_lc);
            let mut seen_trackers: std::collections::HashSet<String> = HashSet::new();
            let mut addon_tracker_count = 0usize;
            let push = |magnet: &mut String, seen: &mut HashSet<String>, url: &str| -> bool {
                if !seen.insert(url.to_string()) {
                    return false;
                }
                magnet.push_str("&tr=");
                magnet.push_str(&urlencoding::encode(url));
                true
            };
            for raw in sources {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let url = trimmed.strip_prefix("tracker:").unwrap_or(trimmed);
                let known_scheme = url.starts_with("http://")
                    || url.starts_with("https://")
                    || url.starts_with("udp://")
                    || url.starts_with("ws://")
                    || url.starts_with("wss://");
                if known_scheme && push(&mut magnet, &mut seen_trackers, url) {
                    addon_tracker_count += 1;
                }
            }

            let mut fallback_count = 0usize;
            for url in user_fallbacks.iter().filter(|s| !s.is_empty()) {
                if push(&mut magnet, &mut seen_trackers, url.as_str()) {
                    fallback_count += 1;
                }
            }
            let tracker_count = addon_tracker_count + fallback_count;
            tracing::info!(
                "[torrent] {info_hash_lc} {addon_tracker_count} addon trackers + {fallback_count} fallback = {tracker_count} total"
            );
            tracing::info!("[torrent] adding magnet for {info_hash_lc}");

            on_progress(&crate::util::loc("progress.torrent.connectingPeers"));

            let is_permanent = self
                .inner
                .permanent_set
                .lock()
                .contains(&info_hash_lc);
            let output_folder = if is_permanent {
                let dn = self.inner.display_names.lock().get(&info_hash_lc).cloned();
                let safe = dn.as_deref().map(sanitize_for_path).filter(|s| !s.is_empty());
                let path = match safe {
                    Some(name) => self.inner.permanent_dir.join(&name),
                    None => self.inner.permanent_dir.clone(),
                };
                Some(path.to_string_lossy().into_owned())
            } else {
                let dn = self.inner.display_names.lock().get(&info_hash_lc).cloned();
                let folder = temp_folder_name(dn.as_deref(), &info_hash_lc);
                Some(
                    self.inner
                        .temp_dir
                        .join(&folder)
                        .to_string_lossy()
                        .into_owned(),
                )
            };
            let opts = AddTorrentOptions {
                overwrite: true,
                output_folder,
                ..Default::default()
            };
            let add_fut = self
                .inner
                .session
                .add_torrent(AddTorrent::from_url(magnet), Some(opts));
            let add_res = tokio::select! {
                r = tokio::time::timeout(ADD_TORRENT_TIMEOUT, add_fut) => r,
                _ = crate::util::cancelled_opt(cancel) => {
                    tracing::info!("[torrent] {info_hash_lc} resolve cancelled during add");
                    self.destroy(&info_hash_lc);
                    bail!("{}", crate::util::loc("details.playback.cancelled"));
                }
            };
            let resp = match add_res {
                Ok(r) => r.context("add_torrent failed")?,
                Err(_) => {
                    tracing::error!(
                        "[torrent] {info_hash_lc} add_torrent timed out after {}s",
                        ADD_TORRENT_TIMEOUT.as_secs()
                    );
                    bail!("{}", crate::util::loc_p(
                        "error.torrent.noPeersReachable",
                        serde_json::json!({ "seconds": ADD_TORRENT_TIMEOUT.as_secs() }),
                    ));
                }
            };

            match resp {
                AddTorrentResponse::Added(_, h) | AddTorrentResponse::AlreadyManaged(_, h) => h,
                AddTorrentResponse::ListOnly(_) => {
                    bail!("{}", crate::util::loc("error.torrent.unexpectedResponse"))
                }
            }
        };

        on_progress(&crate::util::loc("progress.torrent.fetchingInfo"));
        tracing::info!(
            "[torrent] {info_hash_lc} added; waiting for metadata (timeout {}s)…",
            METADATA_TIMEOUT.as_secs()
        );
        let init_res = tokio::select! {
            r = tokio::time::timeout(METADATA_TIMEOUT, handle.wait_until_initialized()) => r,
            _ = crate::util::cancelled_opt(cancel) => {
                tracing::info!("[torrent] {info_hash_lc} resolve cancelled while waiting for metadata");
                self.destroy(&info_hash_lc);
                bail!("{}", crate::util::loc("details.playback.cancelled"));
            }
        };
        match init_res {
            Ok(Ok(())) => {
                tracing::info!("[torrent] {info_hash_lc} metadata ready");
            }
            Ok(Err(e)) => {
                return Err(anyhow!(e)).context("waiting for metadata/initialization");
            }
            Err(_) => {
                tracing::warn!(
                    "[torrent] {info_hash_lc} metadata timeout after {}s",
                    METADATA_TIMEOUT.as_secs()
                );

                if !permanent {
                    let session = self.inner.session.clone();
                    if let Some(id) = parse_id(&info_hash_lc) {
                        tokio::spawn(async move {
                            let _ = session.delete(id, true).await;
                        });
                    }
                }
                bail!("{}", crate::util::loc("error.torrent.noPeersFound"));
            }
        }

        let (file_idx, filesize, filename) = handle.with_metadata(|md| {
            let infos = &md.file_infos;
            let mut best_hint: Option<(usize, u64, String)> = None;
            let mut best_video: Option<(usize, u64, String)> = None;
            let mut best_any: Option<(usize, u64, String)> = None;
            for (i, fi) in infos.iter().enumerate() {
                let path_str = fi.relative_filename.to_string_lossy().to_string();
                let lc = path_str.to_ascii_lowercase();
                let is_video = VIDEO_EXTS
                    .iter()
                    .any(|ext| lc.ends_with(&format!(".{ext}")));
                if let Some(hint) = file_hint {
                    let h = hint.to_ascii_lowercase();
                    if !h.is_empty() && lc.contains(&h) {
                        match &best_hint {
                            Some((_, sz, _)) if *sz >= fi.len => {}
                            _ => best_hint = Some((i, fi.len, path_str.clone())),
                        }
                    }
                }
                if is_video {
                    match &best_video {
                        Some((_, sz, _)) if *sz >= fi.len => {}
                        _ => best_video = Some((i, fi.len, path_str.clone())),
                    }
                }
                match &best_any {
                    Some((_, sz, _)) if *sz >= fi.len => {}
                    _ => best_any = Some((i, fi.len, path_str)),
                }
            }
            best_hint
                .or(best_video)
                .or(best_any)
                .ok_or_else(|| anyhow!("torrent has no files"))
        })??;

        on_progress(&crate::util::loc_p(
            "progress.torrent.file",
            serde_json::json!({ "name": filename }),
        ));

        {
            let mut active = self.inner.active.lock();
            if !active.iter().any(|h| h.info_hash == info_hash_lc) {
                active.push(ActiveHandle {
                    info_hash: info_hash_lc.clone(),
                });
            }
        }

        let url = format!(
            "{}/torrents/{}/stream/{}",
            self.inner.http_base, info_hash_lc, file_idx
        );
        on_progress(&crate::util::loc("progress.torrent.starting"));
        Ok(TorrentResolved {
            url,
            filename,
            filesize,
            info_hash: info_hash_lc,
        })
    }

    pub fn stats(&self, info_hash: &str) -> Option<TorrentStats> {
        let id = parse_id(info_hash)?;
        let handle = self.inner.session.get(id)?;
        let s = handle.stats();

        let (down, up, peers) = if let Some(live) = &s.live {
            (
                live.download_speed.mbps * 1024.0 * 1024.0,
                live.upload_speed.mbps * 1024.0 * 1024.0,
                u32::try_from(live.snapshot.peer_stats.live).unwrap_or(u32::MAX),
            )
        } else {
            (0.0, 0.0, 0)
        };

        let progress = if s.total_bytes > 0 {
            s.progress_bytes as f64 / s.total_bytes as f64
        } else {
            0.0
        };

        Some(TorrentStats {
            peers,
            download_speed: down,
            upload_speed: up,
            progress,
            downloaded: s.progress_bytes,
            length: s.total_bytes,
            ready: handle.metadata.load().is_some(),
            live: s.live.is_some(),
        })
    }

    pub async fn pause(&self, info_hash: &str) -> Result<()> {
        let id = parse_id(info_hash).ok_or_else(|| anyhow!("info hash non valido"))?;
        let handle = self
            .inner
            .session
            .get(id)
            .ok_or_else(|| anyhow!("torrent non attivo nella sessione"))?;
        self.inner.session.pause(&handle).await?;
        Ok(())
    }

    pub async fn resume(&self, info_hash: &str) -> Result<()> {
        let id = parse_id(info_hash).ok_or_else(|| anyhow!("info hash non valido"))?;
        let handle = self
            .inner
            .session
            .get(id)
            .ok_or_else(|| anyhow!("torrent non attivo nella sessione"))?;
        self.inner.session.unpause(&handle).await?;
        Ok(())
    }

    pub fn is_paused(&self, info_hash: &str) -> bool {
        parse_id(info_hash)
            .and_then(|id| self.inner.session.get(id))
            .map(|h| h.is_paused())
            .unwrap_or(false)
    }

    pub fn list_video_files(&self, info_hash: &str) -> Option<Vec<TorrentFileInfo>> {
        let id = parse_id(info_hash)?;
        let handle = self.inner.session.get(id)?;
        handle.with_metadata(|md| {
            let mut out: Vec<TorrentFileInfo> = md
                .file_infos
                .iter()
                .enumerate()
                .filter_map(|(i, fi)| {
                    let path = fi.relative_filename.to_string_lossy().into_owned();
                    let lc = path.to_ascii_lowercase();
                    let is_video = VIDEO_EXTS
                        .iter()
                        .any(|ext| lc.ends_with(&format!(".{ext}")));
                    if !is_video {
                        return None;
                    }
                    let basename = std::path::Path::new(&path)
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.clone());
                    Some(TorrentFileInfo {
                        idx: i,
                        path,
                        basename,
                        size: fi.len,
                    })
                })
                .collect();

            out.sort_by(|a, b| a.path.cmp(&b.path));
            out
        })
        .ok()
    }

    pub fn stream_url_for_file(&self, info_hash: &str, file_idx: usize) -> Option<String> {
        let info_hash_lc = info_hash.to_ascii_lowercase();
        let id = parse_id(&info_hash_lc)?;

        self.inner.session.get(id)?;
        Some(format!(
            "{}/torrents/{}/stream/{}",
            self.inner.http_base, info_hash_lc, file_idx
        ))
    }

    pub fn destroy(&self, info_hash: &str) {
        let info_hash_lc = info_hash.to_ascii_lowercase();
        if self.inner.permanent_set.lock().contains(&info_hash_lc) {
            tracing::info!(
                "[torrent] {info_hash_lc} is permanent — skipping destroy on player close"
            );
            return;
        }
        self.destroy_internal(info_hash_lc, false);
    }

    pub fn destroy_force(&self, info_hash: &str) {
        let info_hash_lc = info_hash.to_ascii_lowercase();
        let was_permanent = self.inner.permanent_set.lock().remove(&info_hash_lc);
        self.destroy_internal(info_hash_lc, was_permanent);
    }

    fn destroy_internal(&self, info_hash_lc: String, was_permanent: bool) {
        {
            let mut g = self.inner.active.lock();
            if let Some(pos) = g.iter().position(|h| h.info_hash == info_hash_lc) {
                g.remove(pos);
            }
        }
        let Some(id) = parse_id(&info_hash_lc) else {
            return;
        };
        let session = self.inner.session.clone();

        let (temp_root, perm_root) = if was_permanent {
            let dn = self.inner.display_names.lock().remove(&info_hash_lc);
            let safe = dn.as_deref().map(sanitize_for_path).filter(|s| !s.is_empty());
            (None, safe.map(|name| self.inner.permanent_dir.join(name)))
        } else {
            let dn = self.inner.display_names.lock().remove(&info_hash_lc);
            let folder = temp_folder_name(dn.as_deref(), &info_hash_lc);
            (Some(self.inner.temp_dir.join(&folder)), None)
        };
        let inner = self.inner.clone();
        let info_hash_lc_task = info_hash_lc.clone();

        let task = tokio::spawn(async move {
            let start = std::time::Instant::now();
            let max_register_wait = Duration::from_secs(15);
            let mut registered = false;
            while start.elapsed() < max_register_wait {
                if session.get(id).is_some() {
                    registered = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
            }

            if registered {
                match session.delete(id, true).await {
                    Ok(()) => tracing::info!(
                        "[torrent] {info_hash_lc_task} removed from session (files deleted by librqbit)"
                    ),
                    Err(e) => tracing::warn!(
                        "[torrent] {info_hash_lc_task} session removal failed (continuing to folder cleanup): {e:#}"
                    ),
                }
            } else {
                tracing::info!(
                    "[torrent] {info_hash_lc_task} not registered after {}s — skipping session.delete and going straight to folder cleanup",
                    max_register_wait.as_secs()
                );
            }

            if let Some(root) = perm_root.as_ref() {
                tokio::time::sleep(Duration::from_millis(800)).await;
                crate::ops::downloads::cleanup_title_folder_if_empty(root).await;
                if root.exists() {
                    tracing::debug!(
                        "[torrent] {info_hash_lc_task} title folder kept (siblings present): {}",
                        root.display()
                    );
                } else {
                    tracing::info!(
                        "[torrent] {info_hash_lc_task} title folder removed: {}",
                        root.display()
                    );
                }
                inner.pending_destroys.lock().remove(&info_hash_lc_task);
                return;
            }

            let torrent_roots: HashSet<PathBuf> = temp_root.into_iter().collect();
            tracing::info!(
                "[torrent] {info_hash_lc_task} cleanup roots: {:?}",
                torrent_roots
            );

            tokio::time::sleep(Duration::from_millis(600)).await;
            let mut backoff = Duration::from_millis(400);
            for attempt in 1..=12 {
                let mut all_gone = true;
                for root in &torrent_roots {
                    if !root.exists() {
                        continue;
                    }
                    if let Err(e) = try_remove_path(root).await {
                        tracing::debug!(
                            "[torrent] {info_hash_lc_task} delete attempt {attempt}/12 partial for {}: {e:#}",
                            root.display()
                        );
                        all_gone = false;
                    }
                }
                if all_gone {
                    tracing::info!(
                        "[torrent] {info_hash_lc_task} cleanup complete (attempt {attempt})"
                    );
                    break;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
            }

            let leftover: Vec<_> = torrent_roots.iter().filter(|p| p.exists()).collect();
            if !leftover.is_empty() {
                tracing::warn!(
                    "[torrent] {info_hash_lc_task} cleanup gave up after 12 attempts. Leftover: {:?}",
                    leftover
                );
            }

            inner.pending_destroys.lock().remove(&info_hash_lc_task);
        });

        if let Some(prev) = self
            .inner
            .pending_destroys
            .lock()
            .insert(info_hash_lc.clone(), task)
        {
            tokio::spawn(async move {
                let _ = prev.await;
            });
        }
    }
}

fn parse_id(info_hash_hex: &str) -> Option<TorrentIdOrHash> {
    let id = Id20::from_str(info_hash_hex).ok()?;
    Some(TorrentIdOrHash::from(id))
}

async fn try_remove_path(path: &std::path::Path) -> std::io::Result<()> {
    let meta = tokio::fs::metadata(path).await?;
    if meta.is_dir() {
        if let Ok(mut entries) = tokio::fs::read_dir(path).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let _ = Box::pin(try_remove_path(&entry.path())).await;
            }
        }
        tokio::fs::remove_dir(path).await
    } else {
        tokio::fs::remove_file(path).await
    }
}
