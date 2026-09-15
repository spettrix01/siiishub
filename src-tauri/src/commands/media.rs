use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use crate::alldebrid;
use crate::ffprobe::{probe, ProbeInfo};
use crate::realdebrid::{self, ResolvedStream};
use crate::state::AppState;

use super::shared::{err, rd_err, CmdResult};

#[derive(Debug, Deserialize)]
pub struct ResolveArgs {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "infoHash")]
    pub info_hash: Option<String>,
    #[serde(default, rename = "displayName")]
    pub display_name: Option<String>,
    #[serde(default, rename = "fileHint")]
    pub file_hint: Option<String>,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default, rename = "useDebrid")]
    pub use_debrid: bool,
    #[serde(default, rename = "requestId")]
    pub request_id: Option<String>,
}

fn cancelled_err() -> String {
    crate::util::loc("details.playback.cancelled")
}

#[derive(Debug, Serialize)]
pub struct ResolveResult {
    pub url: String,
    pub filename: String,
    pub filesize: u64,
    pub mime_type: String,
    pub probe: Option<ProbeInfo>,
    #[serde(rename = "infoHash", skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
}

fn into_result(r: ResolvedStream, p: Option<ProbeInfo>) -> ResolveResult {
    ResolveResult {
        url: r.url,
        filename: r.filename,
        filesize: r.filesize,
        mime_type: r.mime_type,
        probe: p,
        info_hash: r.info_hash,
    }
}

#[tauri::command]
pub async fn media_resolve(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    args: ResolveArgs,
) -> CmdResult<ResolveResult> {
    let cancel = args
        .request_id
        .as_deref()
        .map(|id| state.resolve_cancel_token(id));
    let out = media_resolve_inner(app, &state, &args, cancel.as_ref()).await;
    if let Some(id) = args.request_id.as_deref() {
        state.clear_resolve_cancel(id);
    }
    out
}

#[tauri::command]
pub async fn media_cancel(
    state: State<'_, Arc<AppState>>,
    request_id: String,
) -> CmdResult<()> {
    state.cancel_resolve(&request_id);
    Ok(())
}

async fn media_resolve_inner(
    app: AppHandle,
    state: &State<'_, Arc<AppState>>,
    args: &ResolveArgs,
    cancel: Option<&CancellationToken>,
) -> CmdResult<ResolveResult> {
    let progress = {
        let app = app.clone();
        move |msg: &str| {
            let _ = app.emit("media://progress", msg.to_string());
        }
    };

    if let Some(url) = args.url.as_deref() {
        if !url.is_empty() {
            progress(&crate::util::loc("progress.resolvingLink"));
            let resolved = realdebrid::follow_to_final(&state.http, url)
                .await
                .map_err(err)?;
            if crate::util::is_cancelled(cancel) {
                return Err(cancelled_err());
            }
            progress(&crate::util::loc("progress.analyzingFile"));
            let p = probe(&resolved.url).await.ok();
            return Ok(into_result(resolved, p));
        }
    }
    let info_hash = args
        .info_hash
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| crate::util::loc("error.missingUrlOrHash"))?;

    let cfg = state.settings.read();
    let provider = cfg.debrid_provider.trim().to_lowercase();
    let token = if cfg.debrid_token.is_empty() {
        None
    } else {
        Some(cfg.debrid_token.clone())
    };

    let mut local_torrent: Option<String> = None;
    let resolved = if args.use_debrid && provider == "rd" && token.is_some() {
        progress(&crate::util::loc("progress.rd.connecting"));
        let p2 = progress.clone();
        realdebrid::resolve(
            &state.http,
            token.as_deref().unwrap(),
            info_hash,
            args.display_name.as_deref(),
            args.file_hint.as_deref(),
            &cfg.tracker_fallbacks,
            cancel,
            move |msg| p2(msg),
        )
        .await
        .map_err(rd_err)?
    } else if args.use_debrid && provider == "ad" && token.is_some() {
        progress(&crate::util::loc("progress.ad.connecting"));
        let p2 = progress.clone();
        alldebrid::resolve(
            &state.http,
            token.as_deref().unwrap(),
            info_hash,
            args.display_name.as_deref(),
            args.file_hint.as_deref(),
            &cfg.tracker_fallbacks,
            cancel,
            move |msg| p2(msg),
        )
        .await
        .map_err(rd_err)?
    } else {
        let p2 = progress.clone();
        let t = state
            .torrents
            .resolve(
                info_hash,
                args.file_hint.as_deref(),
                args.display_name.as_deref(),
                &args.sources,
                &cfg.tracker_fallbacks,
                false,
                cancel,
                move |msg| p2(msg),
            )
            .await
            .map_err(err)?;
        local_torrent = Some(t.info_hash.clone());
        ResolvedStream {
            url: t.url,
            filename: t.filename,
            filesize: t.filesize,
            mime_type: String::new(),
            info_hash: Some(t.info_hash),
        }
    };

    // A cancel landing after the resolver returned would otherwise leave the
    // freshly added torrent seeding with no player attached to it.
    let ensure_not_cancelled = || -> CmdResult<()> {
        if crate::util::is_cancelled(cancel) {
            if let Some(ih) = local_torrent.as_deref() {
                state.torrents.destroy(ih);
            }
            return Err(cancelled_err());
        }
        Ok(())
    };

    ensure_not_cancelled()?;
    progress(&crate::util::loc("progress.analyzingFile"));
    let p = probe(&resolved.url).await.ok();
    ensure_not_cancelled()?;
    Ok(into_result(resolved, p))
}

#[tauri::command]
pub async fn media_resolve_all(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    args: ResolveArgs,
) -> CmdResult<Vec<ResolveResult>> {
    let cancel = args
        .request_id
        .as_deref()
        .map(|id| state.resolve_cancel_token(id));
    let out = media_resolve_all_inner(app, &state, &args, cancel.as_ref()).await;
    if let Some(id) = args.request_id.as_deref() {
        state.clear_resolve_cancel(id);
    }
    out
}

async fn media_resolve_all_inner(
    app: AppHandle,
    state: &State<'_, Arc<AppState>>,
    args: &ResolveArgs,
    cancel: Option<&CancellationToken>,
) -> CmdResult<Vec<ResolveResult>> {
    let progress = {
        let app = app.clone();
        move |msg: &str| {
            let _ = app.emit("media://progress", msg.to_string());
        }
    };

    let info_hash = args
        .info_hash
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| crate::util::loc("error.missingInfoHash"))?;

    let cfg = state.settings.read();
    let provider = cfg.debrid_provider.trim().to_lowercase();
    let token = if cfg.debrid_token.is_empty() {
        None
    } else {
        Some(cfg.debrid_token.clone())
    };

    let streams = if provider == "rd" && token.is_some() {
        progress(&crate::util::loc("progress.rd.connecting"));
        let p2 = progress.clone();
        realdebrid::resolve_all(
            &state.http,
            token.as_deref().unwrap(),
            info_hash,
            args.display_name.as_deref(),
            &cfg.tracker_fallbacks,
            cancel,
            move |msg| p2(msg),
        )
        .await
        .map_err(rd_err)?
    } else if provider == "ad" && token.is_some() {
        progress(&crate::util::loc("progress.ad.connecting"));
        let p2 = progress.clone();
        alldebrid::resolve_all(
            &state.http,
            token.as_deref().unwrap(),
            info_hash,
            args.display_name.as_deref(),
            &cfg.tracker_fallbacks,
            cancel,
            move |msg| p2(msg),
        )
        .await
        .map_err(rd_err)?
    } else {
        return Err(crate::util::loc("error.debridNotConfigured"));
    };

    Ok(streams.into_iter().map(|s| into_result(s, None)).collect())
}

#[tauri::command]
pub async fn media_probe(url: String) -> CmdResult<ProbeInfo> {
    probe(&url).await.map_err(err)
}
