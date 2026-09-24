use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::ffprobe::ProbeInfo;
use crate::ops::media::{ResolveArgs, ResolveResult};
use crate::state::AppState;

use super::shared::CmdResult;

/// The resolve steps go to the loading screen as `media://progress` events.
fn progress_events(app: AppHandle) -> impl Fn(&str) + Clone + Send + Sync + 'static {
    move |msg: &str| {
        let _ = app.emit("media://progress", msg.to_string());
    }
}

#[tauri::command]
pub async fn media_resolve(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    args: ResolveArgs,
) -> CmdResult<ResolveResult> {
    crate::ops::media::resolve(&state, &args, progress_events(app)).await
}

#[tauri::command]
pub async fn media_cancel(
    state: State<'_, Arc<AppState>>,
    request_id: String,
) -> CmdResult<()> {
    crate::ops::media::cancel(&state, &request_id);
    Ok(())
}

#[tauri::command]
pub async fn media_resolve_all(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    args: ResolveArgs,
) -> CmdResult<Vec<ResolveResult>> {
    crate::ops::media::resolve_all(&state, &args, progress_events(app)).await
}

#[tauri::command]
pub async fn media_probe(url: String) -> CmdResult<ProbeInfo> {
    crate::ops::media::probe(&url).await
}
