use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::mpv::Mpv;
use crate::state::AppState;

use super::shared::{err, CmdResult};

fn require_mpv(state: &State<'_, Arc<AppState>>) -> CmdResult<Arc<Mpv>> {
    state.mpv().ok_or_else(|| "Player non pronto".to_string())
}

/// What the device's hardware video decoders take, on Android
/// (`{"hevc4k": bool, "avc4k": bool, "av1": bool}`): the TV interface keeps
/// away from the video a device would decode in software. `None` elsewhere.
#[tauri::command]
pub async fn device_video_caps(app: tauri::AppHandle) -> CmdResult<Option<Value>> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let Some(plugin) = app.try_state::<siiishub_android_player::AndroidMpv<tauri::Wry>>() else {
            return Ok(None);
        };
        let caps = plugin.decoder_caps()?;
        Ok(Some(serde_json::to_value(caps).map_err(err)?))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(None)
    }
}

#[tauri::command]
pub async fn mpv_load(
    state: State<'_, Arc<AppState>>,
    url: String,
    options: Option<String>,
) -> CmdResult<()> {
    let mpv = require_mpv(&state)?;
    let cfg = state.settings.read();
    if !cfg.player_audio_langs.is_empty() {
        let csv = cfg.player_audio_langs.join(",");
        let _ = mpv.set_property("alang", Value::String(csv));
    }
    if !cfg.player_sub_langs.is_empty() {
        let csv = cfg.player_sub_langs.join(",");
        let _ = mpv.set_property("slang", Value::String(csv));
    }
    mpv.loadfile(&url, options.as_deref()).map_err(err)
}

#[tauri::command]
pub async fn mpv_command(state: State<'_, Arc<AppState>>, args: Vec<String>) -> CmdResult<()> {
    let mpv = require_mpv(&state)?;
    mpv.command(&args).map_err(err)
}

#[tauri::command]
pub async fn mpv_set_property(
    state: State<'_, Arc<AppState>>,
    name: String,
    value: Value,
) -> CmdResult<()> {
    let mpv = require_mpv(&state)?;
    mpv.set_property(&name, value).map_err(err)
}

#[tauri::command]
pub async fn mpv_get_property(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> CmdResult<Value> {
    let mpv = require_mpv(&state)?;
    mpv.get_property(&name).map_err(err)
}

#[tauri::command]
pub async fn mpv_observe(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> CmdResult<()> {
    let mpv = require_mpv(&state)?;
    mpv.observe(&name).map_err(err)
}

#[tauri::command]
pub async fn mpv_set_geometry(
    state: State<'_, Arc<AppState>>,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> CmdResult<()> {
    let mpv = require_mpv(&state)?;
    mpv.set_geometry(x, y, w, h).map_err(err)
}

#[tauri::command]
pub async fn mpv_set_visible(
    state: State<'_, Arc<AppState>>,
    visible: bool,
) -> CmdResult<()> {
    let mpv = require_mpv(&state)?;
    mpv.set_visible(visible).map_err(err)
}
