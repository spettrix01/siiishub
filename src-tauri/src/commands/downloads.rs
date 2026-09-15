use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::download::{http_id_for, DownloadKind, DownloadRecord, HttpStats};
use crate::state::AppState;
use crate::torrent::{sanitize_for_path, TorrentFileInfo, TorrentStats};
use crate::torrent_file::{parse_torrent_bytes, ParsedTorrent};

use super::shared::{err, CmdResult};

#[derive(Debug, Deserialize)]
pub struct DownloadStartArgs {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "infoHash")]
    pub info_hash: Option<String>,
    pub title: String,
    #[serde(default, rename = "posterUrl")]
    pub poster_url: Option<String>,
    #[serde(default)]
    pub addon: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default, rename = "fileHint")]
    pub file_hint: Option<String>,
    #[serde(default, rename = "tmdbId")]
    pub tmdb_id: Option<i64>,
    #[serde(default, rename = "tmdbType")]
    pub tmdb_type: Option<String>,
    #[serde(default)]
    pub season: Option<i64>,
    #[serde(default)]
    pub episode: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct UnifiedStats {
    pub downloaded: u64,
    pub length: u64,
    pub progress: f64,
    pub download_speed: f64,
    pub upload_speed: f64,
    pub peers: u32,
    pub ready: bool,
    pub paused: bool,
    pub live: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<TorrentStats> for UnifiedStats {
    fn from(s: TorrentStats) -> Self {
        Self {
            downloaded: s.downloaded,
            length: s.length,
            progress: s.progress,
            download_speed: s.download_speed,
            upload_speed: s.upload_speed,
            peers: s.peers,
            ready: s.ready,
            paused: false,
            live: s.live,
            error: None,
        }
    }
}

impl From<HttpStats> for UnifiedStats {
    fn from(s: HttpStats) -> Self {
        Self {
            live: !s.done && !s.paused && s.error.is_none(),
            downloaded: s.downloaded,
            length: s.length,
            progress: s.progress,
            download_speed: s.download_speed,
            upload_speed: 0.0,
            peers: 0,
            ready: s.done,
            paused: s.paused,
            error: s.error,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DownloadEntry {
    #[serde(flatten)]
    pub record: DownloadRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<UnifiedStats>,
}

fn http_download_path(download_dir: &Path, title: &str, filename: &str) -> PathBuf {
    let safe = sanitize_for_path(title);
    if safe.is_empty() {
        download_dir.join(filename)
    } else {
        download_dir.join(safe).join(filename)
    }
}

fn permanent_folder_path(download_dir: &Path, title: &str) -> PathBuf {
    let safe = sanitize_for_path(title);
    if safe.is_empty() {
        download_dir.to_path_buf()
    } else {
        download_dir.join(safe)
    }
}

fn completed_torrent_stats(size: u64) -> UnifiedStats {
    UnifiedStats {
        downloaded: size,
        length: size,
        progress: 1.0,
        download_speed: 0.0,
        upload_speed: 0.0,
        peers: 0,
        ready: true,
        paused: false,
        live: false,
        error: None,
    }
}

const LOCAL_AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "wma", "aiff", "aif", "mka", "alac",
];
const LOCAL_EXTRA_VIDEO_EXTS: &[&str] = &["mpg", "mpeg", "3gp", "ogv", "mts", "vob", "divx"];

fn is_local_media_ext(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    crate::torrent::VIDEO_EXTS.iter().any(|v| *v == e)
        || LOCAL_AUDIO_EXTS.contains(&e.as_str())
        || LOCAL_EXTRA_VIDEO_EXTS.contains(&e.as_str())
}

fn local_stats(rec: &DownloadRecord) -> UnifiedStats {
    let exists = rec
        .source_path
        .as_deref()
        .map(|p| Path::new(p).exists())
        .unwrap_or(false);
    if !exists {
        return UnifiedStats {
            error: Some(crate::util::loc("error.download.fileNotFound")),
            ..Default::default()
        };
    }
    let size = rec
        .source_path
        .as_deref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .or(rec.completed_size)
        .unwrap_or(0);
    UnifiedStats {
        downloaded: size,
        length: size,
        progress: 1.0,
        ready: true,
        live: false,
        ..Default::default()
    }
}

fn torrent_disk_files(folder: &Path) -> Vec<(PathBuf, u64)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                let is_video = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| {
                        let lc = e.to_ascii_lowercase();
                        crate::torrent::VIDEO_EXTS.iter().any(|v| *v == lc)
                    })
                    .unwrap_or(false);
                if is_video {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    out.push((path, size));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(folder, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn http_download_failed(state: &Arc<AppState>, id: &str, error: String, progress: Option<(u64, u64)>) {
    let (downloaded, length) = progress.unwrap_or((0, 0));
    state.downloads.set_http_stats(id, HttpStats {
        downloaded,
        length,
        error: Some(error),
        ..Default::default()
    });
}

fn find_download_record(state: &AppState, id: &str) -> CmdResult<DownloadRecord> {
    state
        .downloads
        .list()
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| "Download non trovato".to_string())
}

#[cfg(windows)]
fn run_attrib(path: &Path, flags: &[&str]) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    Command::new("attrib")
        .args(flags)
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|_| ())
}

fn cover_present(folder: &Path) -> bool {
    #[cfg(windows)]
    {
        folder.join("cover.ico").exists() && folder.join("desktop.ini").exists()
    }
    #[cfg(not(windows))]
    {
        folder.join(unix_cover::COVER_FILE).exists()
    }
}

/// Unix folder cover: the poster is stored as a hidden `.cover.jpg`; on Linux
/// it is also set as the folder's `metadata::custom-icon`, which Nautilus,
/// Nemo and Caja display like Explorer does with `desktop.ini`.
#[cfg(not(windows))]
mod unix_cover {
    use std::path::Path;

    pub const COVER_FILE: &str = ".cover.jpg";

    pub async fn save(folder: &Path, bytes: &[u8]) {
        let path = folder.join(COVER_FILE);
        if let Err(e) = tokio::fs::write(&path, bytes).await {
            tracing::debug!("[cover] write {} failed: {e:#}", path.display());
            return;
        }
        tracing::info!("[cover] saved cover → {}", path.display());
        #[cfg(target_os = "linux")]
        set_custom_icon(folder, Some(&path));
    }

    pub async fn cleanup(folder: &Path) {
        #[cfg(target_os = "linux")]
        set_custom_icon(folder, None);
        let _ = tokio::fs::remove_file(folder.join(COVER_FILE)).await;
    }

    #[cfg(target_os = "linux")]
    fn set_custom_icon(folder: &Path, icon: Option<&Path>) {
        use gtk::gio::prelude::*;
        let file = gtk::gio::File::for_path(folder);
        let uri = icon.and_then(|p| gtk::glib::filename_to_uri(p, None).ok());
        let res = match uri {
            Some(uri) => file.set_attribute_string(
                "metadata::custom-icon",
                uri.as_str(),
                gtk::gio::FileQueryInfoFlags::NONE,
                gtk::gio::Cancellable::NONE,
            ),
            // gio-rs does not bind g_file_set_attribute (needed to unset an
            // attribute), so call the C function directly.
            None => unsafe {
                use gtk::glib::translate::from_glib_full;
                let mut error = std::ptr::null_mut();
                let ok = gtk::gio::ffi::g_file_set_attribute(
                    file.as_ptr(),
                    b"metadata::custom-icon\0".as_ptr() as *const _,
                    gtk::gio::ffi::G_FILE_ATTRIBUTE_TYPE_INVALID,
                    std::ptr::null_mut(),
                    gtk::gio::ffi::G_FILE_QUERY_INFO_NONE,
                    std::ptr::null_mut(),
                    &mut error,
                );
                if ok == 0 {
                    Err(from_glib_full::<_, gtk::glib::Error>(error))
                } else {
                    Ok(())
                }
            },
        };
        if let Err(e) = res {
            tracing::debug!("[cover] custom-icon metadata for {}: {e}", folder.display());
        }
    }
}

async fn save_folder_cover(folder: &Path, poster_url: Option<&str>, http: &reqwest::Client) {
    let Some(url) = poster_url.filter(|s| !s.is_empty()) else {
        return;
    };
    let _ = tokio::fs::create_dir_all(folder).await;

    if cover_present(folder) {
        return;
    }

    let bytes = match http.get(url).send().await {
        Ok(r) if r.status().is_success() => match r.bytes().await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!("[cover] body read failed for {url}: {e:#}");
                return;
            }
        },
        Ok(r) => {
            tracing::debug!("[cover] HTTP {} for {url}", r.status().as_u16());
            return;
        }
        Err(e) => {
            tracing::debug!("[cover] fetch failed for {url}: {e:#}");
            return;
        }
    };

    #[cfg(windows)]
    windows_cover(folder, bytes.to_vec()).await;
    #[cfg(not(windows))]
    unix_cover::save(folder, &bytes).await;
}

/// Windows Explorer folder icon: `cover.ico` + hidden `desktop.ini`.
#[cfg(windows)]
async fn windows_cover(folder: &Path, bytes: Vec<u8>) {
    let ico_path = folder.join("cover.ico");
    let ini_path = folder.join("desktop.ini");

    if !ico_path.exists() {
        let bytes_clone = bytes.clone();
        let ico_path_clone = ico_path.clone();
        let conv = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let img = image::load_from_memory(&bytes_clone)
                .map_err(|e| format!("decode: {e}"))?;

            let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
            for &target in &[256u32, 128, 64, 48, 32, 16] {
                let resized = img.resize(
                    target,
                    target,
                    image::imageops::FilterType::Lanczos3,
                );
                let rw = resized.width();
                let rh = resized.height();
                let dx = ((target as i64 - rw as i64) / 2).max(0) as u32;
                let dy = ((target as i64 - rh as i64) / 2).max(0) as u32;
                let mut canvas = image::RgbaImage::from_pixel(
                    target,
                    target,
                    image::Rgba([0u8, 0, 0, 0]),
                );
                let resized_rgba = resized.to_rgba8();
                image::imageops::overlay(&mut canvas, &resized_rgba, dx as i64, dy as i64);
                let icon_img = ico::IconImage::from_rgba_data(target, target, canvas.into_raw());
                dir.add_entry(
                    ico::IconDirEntry::encode(&icon_img)
                        .map_err(|e| format!("encode {target}: {e}"))?,
                );
            }
            let mut buf = Vec::new();
            dir.write(&mut buf).map_err(|e| format!("write ico: {e}"))?;
            std::fs::write(&ico_path_clone, &buf).map_err(|e| format!("save ico: {e}"))?;
            Ok(())
        })
        .await;

        match conv {
            Ok(Ok(())) => tracing::info!("[cover] saved icon → {}", ico_path.display()),
            Ok(Err(e)) => {
                tracing::debug!("[cover] ico conversion failed: {e}");
                return;
            }
            Err(e) => {
                tracing::debug!("[cover] ico task failed: {e}");
                return;
            }
        }
    }

    if !ini_path.exists() {
        let ini = "[.ShellClassInfo]\r\nIconResource=cover.ico,0\r\nIconFile=cover.ico\r\nIconIndex=0\r\nConfirmFileOp=0\r\n";
        if let Err(e) = tokio::fs::write(&ini_path, ini).await {
            tracing::debug!("[cover] write desktop.ini failed: {e:#}");
            return;
        }
    }

    #[cfg(windows)]
    {
        let _ = run_attrib(&ico_path, &["+h"]);
        let _ = run_attrib(&ini_path, &["+h", "+s"]);
        let _ = run_attrib(folder, &["+r"]);
    }
}

pub async fn cleanup_cover_files(folder: &Path) {
    #[cfg(windows)]
    {
        let _ = run_attrib(&folder.join("cover.ico"), &["-h", "-s"]);
        let _ = run_attrib(&folder.join("desktop.ini"), &["-h", "-s"]);
        let _ = run_attrib(folder, &["-r", "-s"]);
    }
    #[cfg(not(windows))]
    unix_cover::cleanup(folder).await;
    let _ = tokio::fs::remove_file(folder.join("Folder.jpg")).await;
    let _ = tokio::fs::remove_file(folder.join("cover.ico")).await;
    let _ = tokio::fs::remove_file(folder.join("desktop.ini")).await;
}

pub async fn cleanup_title_folder_if_empty(folder: &Path) {
    let mut entries = match tokio::fs::read_dir(folder).await {
        Ok(e) => e,
        Err(_) => return,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(
            name_str.as_ref(),
            "Folder.jpg" | "cover.ico" | "desktop.ini" | ".cover.jpg"
        ) {
            continue;
        }
        return;
    }
    cleanup_cover_files(folder).await;
    let _ = tokio::fs::remove_dir(folder).await;
}

#[tauri::command]
pub async fn download_start(
    state: State<'_, Arc<AppState>>,
    args: DownloadStartArgs,
) -> CmdResult<DownloadRecord> {
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    if let Some(url) = args.url.as_deref().filter(|s| !s.is_empty()) {
        let id = http_id_for(url);
        let filename = crate::util::filename_from_url(url);
        let record = DownloadRecord {
            id: id.clone(),
            kind: DownloadKind::Http,
            title: args.title.clone(),
            poster_url: args.poster_url.clone(),
            addon: args.addon.clone(),
            filename: Some(filename.clone()),
            started_at,
            tmdb_id: args.tmdb_id,
            tmdb_type: args.tmdb_type.clone(),
            group: None,
            season: args.season,
            episode: args.episode,
            completed_size: None,
            source_path: None,
        };
        state.downloads.upsert(record.clone());
        spawn_http_download(
            state.inner().clone(),
            id,
            url.to_string(),
            filename,
            args.title.clone(),
            args.poster_url.clone(),
            false,
        );
        return Ok(record);
    }

    let info_hash = args
        .info_hash
        .as_deref()
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| crate::util::loc("error.missingUrlOrHash"))?;

    let record = DownloadRecord {
        id: info_hash.clone(),
        kind: DownloadKind::Torrent,
        title: args.title.clone(),
        poster_url: args.poster_url.clone(),
        addon: args.addon.clone(),
        filename: None,
        started_at,
        tmdb_id: args.tmdb_id,
        tmdb_type: args.tmdb_type.clone(),
        group: None,
        season: args.season,
        episode: args.episode,
        completed_size: None,
        source_path: None,
    };
    state.downloads.upsert(record.clone());

    {
        let folder = permanent_folder_path(&state.download_dir, &args.title);
        let http = state.http.clone();
        let poster = args.poster_url.clone();
        tauri::async_runtime::spawn(async move {
            save_folder_cover(&folder, poster.as_deref(), &http).await;
        });
    }

    let torrents = state.torrents.clone();
    let downloads = state.downloads.clone();
    let cfg = state.settings.read();
    let user_fallbacks = cfg.tracker_fallbacks.clone();
    let sources = args.sources.clone();
    let file_hint = args.file_hint.clone();
    let id_for_task = info_hash.clone();
    let title_for_task = args.title.clone();
    tauri::async_runtime::spawn(async move {
        let result = torrents
            .resolve(
                &id_for_task,
                file_hint.as_deref(),
                Some(&title_for_task),
                &sources,
                &user_fallbacks,
                true,
                None,
                |_msg| {},
            )
            .await;
        match result {
            Ok(t) => {
                downloads.set_filename(&t.info_hash, t.filename);
                tracing::info!(
                    "[download] {id_for_task} resolved, leaving torrent running"
                );
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    match torrents.stats(&id_for_task) {
                        Some(s) if s.length > 0 && s.progress >= 1.0 => {
                            downloads.mark_torrent_done(&id_for_task, s.length);
                            tracing::info!(
                                "[download] {id_for_task} torrent completo ({} byte)",
                                s.length
                            );
                            break;
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
            }
            Err(e) => {
                tracing::error!(
                    "[download] {id_for_task} resolve failed (record kept): {e:#}"
                );
            }
        }
    });

    Ok(record)
}

#[derive(Debug, Deserialize)]
pub struct DownloadGroupFile {
    pub url: String,
    #[serde(default)]
    pub filename: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DownloadStartGroupArgs {
    pub title: String,
    #[serde(default, rename = "infoHash")]
    pub info_hash: Option<String>,
    #[serde(default, rename = "posterUrl")]
    pub poster_url: Option<String>,
    #[serde(default)]
    pub addon: Option<String>,
    pub files: Vec<DownloadGroupFile>,
}

#[tauri::command]
pub async fn download_start_group(
    state: State<'_, Arc<AppState>>,
    args: DownloadStartGroupArgs,
) -> CmdResult<Vec<DownloadRecord>> {
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    if args.files.is_empty() {
        return Err(crate::util::loc("error.download.noFiles"));
    }

    let group = args
        .info_hash
        .as_deref()
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty());

    let mut records: Vec<DownloadRecord> = Vec::with_capacity(args.files.len());
    for file in &args.files {
        if file.url.is_empty() {
            continue;
        }
        let id = http_id_for(&file.url);
        let filename = file
            .filename
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(crate::util::safe_filename)
            .unwrap_or_else(|| crate::util::filename_from_url(&file.url));
        let record = DownloadRecord {
            id: id.clone(),
            kind: DownloadKind::Http,
            title: args.title.clone(),
            poster_url: args.poster_url.clone(),
            addon: args.addon.clone(),
            filename: Some(filename.clone()),
            started_at,
            tmdb_id: None,
            tmdb_type: None,
            group: group.clone(),
            season: None,
            episode: None,
            completed_size: None,
            source_path: None,
        };
        state.downloads.upsert(record.clone());
        spawn_http_download(
            state.inner().clone(),
            id,
            file.url.clone(),
            filename,
            args.title.clone(),
            args.poster_url.clone(),
            false,
        );
        records.push(record);
    }

    if records.is_empty() {
        return Err(crate::util::loc("error.download.noValidFiles"));
    }
    Ok(records)
}

#[tauri::command]
pub async fn download_add_local(
    state: State<'_, Arc<AppState>>,
    path: String,
) -> CmdResult<DownloadRecord> {
    let src = PathBuf::from(&path);
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or_default();
    if !is_local_media_ext(ext) {
        return Err(crate::util::loc("error.download.unsupportedLocal"));
    }
    let meta = tokio::fs::metadata(&src)
        .await
        .map_err(|_| crate::util::loc("error.download.fileNotFound"))?;
    if !meta.is_file() {
        return Err(crate::util::loc("error.download.fileNotFound"));
    }
    let basename = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let title = src
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| basename.clone());
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let record = DownloadRecord {
        id: http_id_for(&path),
        kind: DownloadKind::Local,
        title,
        poster_url: None,
        addon: None,
        filename: Some(basename),
        started_at,
        tmdb_id: None,
        tmdb_type: None,
        group: None,
        season: None,
        episode: None,
        completed_size: Some(meta.len()),
        source_path: Some(path),
    };
    state.downloads.upsert(record.clone());
    Ok(record)
}

#[tauri::command]
pub async fn download_list(
    state: State<'_, Arc<AppState>>,
) -> CmdResult<Vec<DownloadEntry>> {
    let mut out = Vec::new();
    for r in state.downloads.list() {
        let stats: Option<UnifiedStats> = match r.kind {
            DownloadKind::Torrent => {
                if let Some(mut s) = state.torrents.stats(&r.id).map(UnifiedStats::from) {
                    s.paused = state.torrents.is_paused(&r.id);
                    Some(s)
                } else {
                    r.completed_size
                        .map(completed_torrent_stats)
                        .or_else(|| state.downloads.http_stats(&r.id).map(Into::into))
                }
            }
            DownloadKind::Http => state.downloads.http_stats(&r.id).map(Into::into),
            DownloadKind::Local => Some(local_stats(&r)),
        };
        out.push(DownloadEntry { record: r, stats });
    }
    Ok(out)
}

#[tauri::command]
pub async fn download_remove(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CmdResult<()> {
    let record = state
        .downloads
        .list()
        .into_iter()
        .find(|r| r.id == id);

    state.downloads.cancel_http(&id);
    state.downloads.remove(&id);

    if let Some(rec) = record {
        match rec.kind {
            DownloadKind::Torrent => {
                let in_session = state.torrents.stats(&id).is_some();
                state.torrents.destroy_force(&id);
                if !in_session {
                    let folder = permanent_folder_path(&state.download_dir, &rec.title);
                    if folder != state.download_dir && folder.exists() {
                        let _ = tokio::fs::remove_dir_all(&folder).await;
                    }
                }
            }
            DownloadKind::Http => {
                if let Some(filename) = rec.filename.as_deref() {
                    let path = http_download_path(&state.download_dir, &rec.title, filename);
                    if path.exists() {
                        if let Err(e) = tokio::fs::remove_file(&path).await {
                            tracing::warn!(
                                "[download] {id} remove_file {} failed: {e:#}",
                                path.display()
                            );
                        } else {
                            tracing::info!(
                                "[download] {id} file removed: {}",
                                path.display()
                            );
                        }
                    }
                    let part = crate::download::part_path(&path);
                    if part.exists() {
                        let _ = tokio::fs::remove_file(&part).await;
                    }
                    if let Some(parent) = path.parent() {
                        if parent != state.download_dir {
                            cleanup_title_folder_if_empty(parent).await;
                        }
                    }
                }
            }
            // Same contract as the other kinds: the confirm dialog states the
            // file is removed from disk, so delete it where it lives.
            DownloadKind::Local => {
                if let Some(path) = rec.source_path.as_deref() {
                    match tokio::fs::remove_file(path).await {
                        Ok(()) => tracing::info!("[download] {id} local file deleted: {path}"),
                        Err(e) => tracing::warn!("[download] {id} delete {path} failed: {e:#}"),
                    }
                }
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn download_pause(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CmdResult<()> {
    let record = find_download_record(&state, &id)?;
    match record.kind {
        DownloadKind::Torrent => state.torrents.pause(&id).await.map_err(err)?,
        DownloadKind::Http => state.downloads.pause_http(&id),
        DownloadKind::Local => {}
    }
    Ok(())
}

#[tauri::command]
pub async fn download_resume(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CmdResult<()> {
    let record = find_download_record(&state, &id)?;
    match record.kind {
        DownloadKind::Torrent => state.torrents.resume(&id).await.map_err(err)?,
        DownloadKind::Http => {
            let url = state
                .downloads
                .http_url(&id)
                .ok_or_else(|| "URL non disponibile per la ripresa".to_string())?;
            let filename = record
                .filename
                .clone()
                .unwrap_or_else(|| crate::util::filename_from_url(&url));
            spawn_http_download(
                state.inner().clone(),
                id.clone(),
                url,
                filename,
                record.title.clone(),
                record.poster_url.clone(),
                true,
            );
        }
        DownloadKind::Local => {}
    }
    Ok(())
}

/// `file://` URL the embedded player opens for a file on disk.
fn local_file_url(path: &Path) -> CmdResult<String> {
    url::Url::from_file_path(path)
        .map(|u| u.to_string())
        .map_err(|_| crate::util::loc("error.download.fileUrlFailed"))
}

#[tauri::command]
pub async fn download_play(
    state: State<'_, Arc<AppState>>,
    id: String,
    file_index: Option<usize>,
) -> CmdResult<String> {
    let record = find_download_record(&state, &id)?;

    match record.kind {
        DownloadKind::Http => {
            let filename = record
                .filename
                .as_deref()
                .ok_or_else(|| crate::util::loc("error.download.missingFilename"))?;
            let path = http_download_path(&state.download_dir, &record.title, filename);
            if !path.exists() {
                return Err(crate::util::loc("error.download.fileNotReady"));
            }

            local_file_url(&path)
        }
        DownloadKind::Torrent => {
            if record.completed_size.is_some()
                && state.torrents.stats(&record.id).is_none()
            {
                let folder = permanent_folder_path(&state.download_dir, &record.title);
                let chosen = match file_index {
                    Some(idx) => torrent_disk_files(&folder)
                        .into_iter()
                        .nth(idx)
                        .map(|(p, _)| p),
                    None => record
                        .filename
                        .as_deref()
                        .map(|f| folder.join(f))
                        .filter(|p| p.exists()),
                };
                let path = chosen
                    .or_else(|| {
                        torrent_disk_files(&folder).into_iter().next().map(|(p, _)| p)
                    })
                    .ok_or_else(|| crate::util::loc("error.download.fileNotFound"))?;
                return local_file_url(&path);
            }
            let cfg = state.settings.read();
            let resolved = state
                .torrents
                .resolve(
                    &record.id,
                    record.filename.as_deref(),
                    Some(&record.title),
                    &[],
                    &cfg.tracker_fallbacks,
                    true,
                    None,
                    |_| {},
                )
                .await
                .map_err(err)?;

            if let Some(idx) = file_index {
                if let Some(url) = state.torrents.stream_url_for_file(&record.id, idx) {
                    return Ok(url);
                }
            }
            Ok(resolved.url)
        }
        DownloadKind::Local => {
            let path = record
                .source_path
                .as_deref()
                .ok_or_else(|| crate::util::loc("error.download.missingFilename"))?;
            let p = PathBuf::from(path);
            if !p.exists() {
                return Err(crate::util::loc("error.download.fileNotReady"));
            }
            local_file_url(&p)
        }
    }
}

#[tauri::command]
pub async fn torrent_parse_file(bytes: Vec<u8>) -> CmdResult<ParsedTorrent> {
    parse_torrent_bytes(&bytes).map_err(err)
}

#[tauri::command]
pub async fn download_open_folder(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CmdResult<()> {
    let record = find_download_record(&state, &id)?;
    let folder = match record.kind {
        DownloadKind::Http => {
            if let Some(filename) = record.filename.as_deref() {
                http_download_path(&state.download_dir, &record.title, filename)
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| state.download_dir.clone())
            } else {
                permanent_folder_path(&state.download_dir, &record.title)
            }
        }
        DownloadKind::Torrent => permanent_folder_path(&state.download_dir, &record.title),
        DownloadKind::Local => record
            .source_path
            .as_deref()
            .map(PathBuf::from)
            .and_then(|p| p.parent().map(|x| x.to_path_buf()))
            .unwrap_or_else(|| state.download_dir.clone()),
    };
    let target = if folder.exists() {
        folder
    } else {
        state.download_dir.clone()
    };
    tokio::fs::create_dir_all(&target).await.map_err(err)?;
    super::shared::open_folder(&app, &target).map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn download_files(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> CmdResult<Vec<TorrentFileInfo>> {
    let record = find_download_record(&state, &id)?;

    if record.kind == DownloadKind::Local {
        let path = record.source_path.clone().unwrap_or_default();
        let size = tokio::fs::metadata(&path)
            .await
            .ok()
            .map(|m| m.len())
            .unwrap_or(0);
        let basename = record
            .filename
            .clone()
            .unwrap_or_else(|| path.clone());
        return Ok(vec![TorrentFileInfo {
            idx: 0,
            path,
            basename,
            size,
        }]);
    }

    if record.kind == DownloadKind::Http {
        let filename = match record.filename.as_deref() {
            Some(f) if !f.is_empty() => f,
            _ => return Ok(Vec::new()),
        };
        let path = http_download_path(&state.download_dir, &record.title, filename);
        let size = tokio::fs::metadata(&path)
            .await
            .ok()
            .map(|m| m.len())
            .unwrap_or(0);

        return Ok(vec![TorrentFileInfo {
            idx: 0,
            path: filename.to_string(),
            basename: filename.to_string(),
            size,
        }]);
    }

    if record.completed_size.is_some() && state.torrents.stats(&record.id).is_none() {
        let folder = permanent_folder_path(&state.download_dir, &record.title);
        return Ok(torrent_disk_files(&folder)
            .into_iter()
            .enumerate()
            .map(|(idx, (path, size))| {
                let basename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                TorrentFileInfo {
                    idx,
                    path: path.to_string_lossy().into_owned(),
                    basename,
                    size,
                }
            })
            .collect());
    }

    let cfg = state.settings.read();
    state
        .torrents
        .resolve(
            &record.id,
            record.filename.as_deref(),
            Some(&record.title),
            &[],
            &cfg.tracker_fallbacks,
            true,
            None,
            |_| {},
        )
        .await
        .map_err(err)?;
    Ok(state
        .torrents
        .list_video_files(&record.id)
        .unwrap_or_default())
}

fn spawn_http_download(
    state: Arc<AppState>,
    id: String,
    url: String,
    filename: String,
    title: String,
    poster_url: Option<String>,
    resume: bool,
) {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let control = match state.downloads.try_register_http_control(&id) {
        Some(c) => c,
        None => {
            tracing::warn!("[download/http] {id} already in progress, skipping duplicate spawn");
            return;
        }
    };
    state.downloads.set_http_url(&id, url.clone());

    let dest = http_download_path(&state.download_dir, &title, &filename);
    let part = crate::download::part_path(&dest);
    let dest_dir = dest.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| state.download_dir.clone());
    let cover_dir = dest_dir.clone();
    let cover_http = state.http.clone();
    let cover_url = poster_url.clone();
    tauri::async_runtime::spawn(async move {
        save_folder_cover(&cover_dir, cover_url.as_deref(), &cover_http).await;
    });

    tauri::async_runtime::spawn(async move {
        let _ = tokio::fs::create_dir_all(&dest_dir).await;

        let resume_offset = if resume {
            tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0)
        } else {
            0
        };

        state.downloads.set_http_stats(
            &id,
            HttpStats {
                downloaded: resume_offset,
                length: resume_offset,
                progress: 0.0,
                download_speed: 0.0,
                done: false,
                paused: false,
                error: None,
            },
        );

        let dl_client = match reqwest::Client::builder()
            .user_agent("siiishub/0.1")
            .gzip(false)
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("[download/http] {id} client build failed: {e:#}");
                http_download_failed(&state, &id, format!("Errore client HTTP: {e}"), None);
                return;
            }
        };

        let mut req = dl_client.get(&url);
        if resume_offset > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={resume_offset}-"));
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("[download/http] {id} request failed: {e:#}");
                http_download_failed(&state, &id, format!("Errore di rete: {e}"), Some((resume_offset, resume_offset)));
                return;
            }
        };

        let status = resp.status();
        if !status.is_success() {
            tracing::error!("[download/http] {id} HTTP {} from {url}", status.as_u16());

            let body_preview = resp
                .text()
                .await
                .ok()
                .map(|s| s.chars().take(120).collect::<String>())
                .unwrap_or_default();
            let msg = if body_preview.is_empty() {
                format!("HTTP {} dal CDN — link probabilmente scaduto", status.as_u16())
            } else {
                format!("HTTP {} dal CDN: {}", status.as_u16(), body_preview)
            };
            http_download_failed(&state, &id, msg, Some((resume_offset, resume_offset)));
            return;
        }

        let is_partial = status == reqwest::StatusCode::PARTIAL_CONTENT;
        let (mut downloaded, total, mut file) = if is_partial && resume_offset > 0 {
            let remaining = resp.content_length().unwrap_or(0);
            let total = resume_offset.saturating_add(remaining);
            match tokio::fs::OpenOptions::new().append(true).open(&part).await {
                Ok(f) => (resume_offset, total, f),
                Err(e) => {
                    tracing::error!("[download/http] {id} open .part for append failed: {e:#}");
                    http_download_failed(&state, &id, format!("Errore scrittura: {e}"), Some((resume_offset, total)));
                    return;
                }
            }
        } else {
            let total = resp.content_length().unwrap_or(0);
            match tokio::fs::File::create(&part).await {
                Ok(f) => (0u64, total, f),
                Err(e) => {
                    tracing::error!("[download/http] {id} create file failed: {e:#}");
                    http_download_failed(&state, &id, format!("Errore scrittura: {e}"), None);
                    return;
                }
            }
        };

        let mut last_tick = std::time::Instant::now();
        let mut last_downloaded: u64 = downloaded;
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            match control.load(std::sync::atomic::Ordering::Relaxed) {
                crate::download::HTTP_CANCEL => {
                    drop(file);
                    let _ = tokio::fs::remove_file(&part).await;
                    if dest_dir != state.download_dir {
                        cleanup_title_folder_if_empty(&dest_dir).await;
                    }
                    state.downloads.clear_http_control(&id);
                    tracing::info!("[download/http] {id} cancelled, partial file removed");
                    return;
                }
                crate::download::HTTP_PAUSE => {
                    let _ = file.flush().await;
                    drop(file);
                    let progress = if total > 0 { downloaded as f64 / total as f64 } else { 0.0 };
                    state.downloads.set_http_stats(
                        &id,
                        HttpStats {
                            downloaded,
                            length: total,
                            progress,
                            download_speed: 0.0,
                            done: false,
                            paused: true,
                            error: None,
                        },
                    );
                    state.downloads.clear_http_control(&id);
                    tracing::info!("[download/http] {id} paused at {downloaded} bytes");
                    return;
                }
                _ => {}
            }
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("[download/http] {id} stream error: {e:#}");
                    http_download_failed(&state, &id, format!("Errore di rete: {e}"), Some((downloaded, total)));
                    return;
                }
            };
            if let Err(e) = file.write_all(&chunk).await {
                tracing::error!("[download/http] {id} write failed: {e:#}");
                http_download_failed(&state, &id, format!("Errore scrittura: {e}"), Some((downloaded, total)));
                return;
            }
            downloaded += chunk.len() as u64;

            let now = std::time::Instant::now();
            if now.duration_since(last_tick).as_millis() >= 250 {
                let dt = now.duration_since(last_tick).as_secs_f64();
                let speed = if dt > 0.0 {
                    (downloaded - last_downloaded) as f64 / dt
                } else {
                    0.0
                };
                let progress = if total > 0 {
                    downloaded as f64 / total as f64
                } else {
                    0.0
                };
                state.downloads.set_http_stats(
                    &id,
                    HttpStats {
                        downloaded,
                        length: total,
                        progress,
                        download_speed: speed,
                        done: false,
                        paused: false,
                        error: None,
                    },
                );
                last_tick = now;
                last_downloaded = downloaded;
            }
        }

        let _ = file.flush().await;
        drop(file);

        if total > 0 && downloaded < total {
            tracing::error!("[download/http] {id} truncated: {downloaded}/{total} bytes (.part kept for resume)");
            http_download_failed(
                &state,
                &id,
                format!("Download incompleto: ricevuti {downloaded} di {total} byte"),
                Some((downloaded, total)),
            );
            return;
        }

        if let Err(e) = tokio::fs::rename(&part, &dest).await {
            tracing::error!(
                "[download/http] {id} rename {} -> {} failed: {e:#}",
                part.display(),
                dest.display()
            );
            http_download_failed(&state, &id, format!("Errore scrittura: {e}"), Some((downloaded, total)));
            return;
        }
        state.downloads.clear_http_control(&id);
        state.downloads.set_http_stats(
            &id,
            HttpStats {
                downloaded,
                length: total.max(downloaded),
                progress: 1.0,
                download_speed: 0.0,
                done: true,
                paused: false,
                error: None,
            },
        );
        tracing::info!("[download/http] {id} done ({downloaded} bytes → {})", dest.display());
    });
}
