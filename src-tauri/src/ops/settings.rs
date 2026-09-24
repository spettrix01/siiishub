use serde::Deserialize;

use crate::settings::{PublicSettings, Settings};
use crate::state::AppState;

use super::{err, CmdResult};

#[derive(Debug, Deserialize)]
pub struct SettingsPatch {
    #[serde(default)]
    pub tmdb_key: Option<String>,
    #[serde(default)]
    pub addons: Option<Vec<crate::settings::AddonConfig>>,
    #[serde(default)]
    pub rd_token: Option<String>,
    #[serde(default)]
    pub debrid_provider: Option<String>,
    #[serde(default)]
    pub debrid_token: Option<String>,
    #[serde(default)]
    pub tracker_fallbacks: Option<Vec<String>>,
    #[serde(default)]
    pub player_audio_langs: Option<Vec<String>>,
    #[serde(default)]
    pub player_sub_langs: Option<Vec<String>>,
    #[serde(default)]
    pub remote_port: Option<u16>,
    #[serde(default)]
    pub language: Option<String>,
}

pub fn get(state: &AppState) -> PublicSettings {
    state.public_settings()
}

/// Applies `patch` and stores the settings. Also returns the new port of the
/// remote-control server when it changed, for the app to restart it there.
pub async fn save(state: &AppState, patch: SettingsPatch) -> CmdResult<(PublicSettings, Option<u16>)> {
    let mut next: Settings = state.settings.read();
    let prev_remote_port = next.remote_port;
    if let Some(k) = patch.tmdb_key {
        next.tmdb_key = k.trim().to_string();
    }
    if let Some(a) = patch.addons {
        next.addons = a;
    }
    if let Some(t) = patch.rd_token {
        next.rd_token = t.trim().to_string();
    }
    if let Some(p) = patch.debrid_provider {
        next.debrid_provider = p.trim().to_lowercase();
    }
    if let Some(t) = patch.debrid_token {
        next.debrid_token = t.trim().to_string();
    }
    if let Some(t) = patch.tracker_fallbacks {
        next.tracker_fallbacks = t
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(a) = patch.player_audio_langs {
        next.player_audio_langs = a
            .into_iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(s) = patch.player_sub_langs {
        next.player_sub_langs = s
            .into_iter()
            .map(|x| x.trim().to_lowercase())
            .filter(|x| !x.is_empty())
            .collect();
    }
    if let Some(p) = patch.remote_port {
        if p >= 1024 {
            next.remote_port = p;
        }
    }
    if let Some(l) = patch.language {
        let l = l.trim().to_lowercase();
        if !l.is_empty() {
            next.language = l;
        }
    }
    let new_port = next.remote_port;
    state.settings.write(next).await.map_err(err)?;

    let port_changed = (prev_remote_port != new_port).then_some(new_port);
    Ok((state.public_settings(), port_changed))
}
