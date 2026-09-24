use crate::state::AppState;
use crate::torrent::TorrentStats;

pub fn stats(state: &AppState, info_hash: &str) -> Option<TorrentStats> {
    state.torrents.stats(info_hash)
}

pub fn destroy(state: &AppState, info_hash: &str) {
    state.torrents.destroy(info_hash);
}
