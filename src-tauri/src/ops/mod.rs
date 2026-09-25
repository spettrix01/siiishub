//! What the interface asks of the backend, whoever carries the request: the
//! app's Tauri commands (`commands/`) and the web server's HTTP API
//! (`server/`) are thin layers over these functions.

pub mod addons;
pub mod downloads;
pub mod media;
pub mod settings;
pub mod sync;
pub mod torrents;
pub mod userdata;

pub type CmdResult<T> = std::result::Result<T, String>;

pub fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

pub fn rd_err(e: crate::realdebrid::RdError) -> String {
    use crate::realdebrid::RdError;
    match e {
        RdError::NoToken => crate::util::loc("error.debrid.invalidToken"),
        RdError::NoPlayableFile => crate::util::loc("error.debrid.noPlayableFile"),
        RdError::Timeout => crate::util::loc("error.debrid.timeout"),
        RdError::Cancelled => crate::util::loc("details.playback.cancelled"),
        RdError::Http { status, .. } => {
            crate::util::loc_p("error.debrid.http", serde_json::json!({ "status": status }))
        }
        RdError::Other(inner) => err(inner),
    }
}
