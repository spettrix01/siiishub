use crate::state::AppState;
use crate::stremio::{self, AddonManifest, StreamsResult, SubtitlesResult};

use super::{err, CmdResult};

pub async fn meta(state: &AppState, url: &str) -> CmdResult<AddonManifest> {
    stremio::fetch_manifest(&state.http, url).await.map_err(err)
}

pub async fn streams(state: &AppState, kind: &str, id: &str) -> StreamsResult {
    let addons = state.settings.read().addons;
    stremio::fetch_streams(&state.http, &addons, kind, id).await
}

pub async fn subtitles(state: &AppState, kind: &str, id: &str) -> SubtitlesResult {
    let addons = state.settings.read().addons;
    stremio::fetch_subtitles(&state.http, &addons, kind, id).await
}
