use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use serde_json::Value;

const RD_BASE: &str = "https://api.real-debrid.com/rest/1.0";
const POLL_INTERVAL: Duration = Duration::from_millis(1500);
const POLL_MAX_ATTEMPTS: u32 = 60;

#[derive(Debug, thiserror::Error)]
pub enum RdError {
    #[error("Debrid: HTTP {status} — {message}")]
    Http { status: u16, message: String },
    #[error("debrid: missing or invalid token")]
    NoToken,
    #[error("debrid: no playable file in torrent")]
    NoPlayableFile,
    #[error("debrid: torrent not available in time")]
    Timeout,
    #[error("debrid: cancelled by user")]
    Cancelled,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedStream {
    pub url: String,
    pub filename: String,
    pub filesize: u64,
    pub mime_type: String,
    pub info_hash: Option<String>,
}

// The provider API needs every one of these inputs; a params struct would only move the noise.
#[allow(clippy::too_many_arguments)]
pub async fn resolve(
    http: &reqwest::Client,
    token: &str,
    info_hash: &str,
    display_name: Option<&str>,
    file_hint: Option<&str>,
    trackers: &[String],
    cancel: Option<&tokio_util::sync::CancellationToken>,
    on_progress: impl Fn(&str),
) -> Result<ResolvedStream, RdError> {
    if token.is_empty() {
        return Err(RdError::NoToken);
    }
    if crate::util::is_cancelled(cancel) {
        return Err(RdError::Cancelled);
    }
    let auth = format!("Bearer {token}");

    on_progress(&crate::util::loc("progress.rd.sendingTorrent"));
    let magnet = build_magnet(info_hash, display_name, trackers);
    let add_resp: Value = http
        .post(format!("{RD_BASE}/torrents/addMagnet"))
        .header("Authorization", &auth)
        .form(&[("magnet", magnet.as_str())])
        .send()
        .await
        .map_err(rd_other)?
        .error_for_status()
        .map_err(map_status)?
        .json()
        .await
        .map_err(rd_other)?;
    let id = add_resp
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RdError::Other(anyhow!("RD addMagnet: id missing")))?
        .to_string();

    on_progress(&crate::util::loc("progress.rd.selectingFiles"));
    let info: Value = http
        .get(format!("{RD_BASE}/torrents/info/{id}"))
        .header("Authorization", &auth)
        .send()
        .await
        .map_err(rd_other)?
        .error_for_status()
        .map_err(map_status)?
        .json()
        .await
        .map_err(rd_other)?;
    let files = info
        .get("files")
        .and_then(|f| f.as_array())
        .ok_or_else(|| RdError::Other(anyhow!("RD info: files missing")))?;
    let chosen_id = pick_file_id(files, file_hint).ok_or(RdError::NoPlayableFile)?;
    http.post(format!("{RD_BASE}/torrents/selectFiles/{id}"))
        .header("Authorization", &auth)
        .form(&[("files", chosen_id.to_string())])
        .send()
        .await
        .map_err(rd_other)?
        .error_for_status()
        .map_err(map_status)?;

    let mut links: Vec<String> = Vec::new();
    for attempt in 0..POLL_MAX_ATTEMPTS {
        let info: Value = http
            .get(format!("{RD_BASE}/torrents/info/{id}"))
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(rd_other)?
            .error_for_status()
            .map_err(map_status)?
            .json()
            .await
            .map_err(rd_other)?;
        let status = info
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let progress = info
            .get("progress")
            .and_then(|v| v.as_f64().or(v.as_u64().map(|u| u as f64)))
            .unwrap_or(0.0);
        on_progress(&crate::util::loc_p(
            "progress.rd.status",
            serde_json::json!({ "status": status, "pct": progress.round() as i64 }),
        ));
        if status == "downloaded" {
            if let Some(arr) = info.get("links").and_then(|v| v.as_array()) {
                links = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
            }
            break;
        }
        if matches!(status, "magnet_error" | "error" | "virus" | "dead") {
            return Err(RdError::Http {
                status: 502,
                message: format!("RD torrent {status}"),
            });
        }
        if attempt + 1 == POLL_MAX_ATTEMPTS {
            return Err(RdError::Timeout);
        }
        tokio::select! {
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
            _ = crate::util::cancelled_opt(cancel) => return Err(RdError::Cancelled),
        }
    }
    if crate::util::is_cancelled(cancel) {
        return Err(RdError::Cancelled);
    }
    let restricted = links.into_iter().next().ok_or(RdError::NoPlayableFile)?;

    on_progress(&crate::util::loc("progress.rd.resolvingLink"));
    let unrestrict: Value = http
        .post(format!("{RD_BASE}/unrestrict/link"))
        .header("Authorization", &auth)
        .form(&[("link", restricted.as_str())])
        .send()
        .await
        .map_err(rd_other)?
        .error_for_status()
        .map_err(map_status)?
        .json()
        .await
        .map_err(rd_other)?;

    let download_url = unrestrict
        .get("download")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            let rd_err = unrestrict.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
            RdError::Other(anyhow!("RD unrestrict: no URL ({rd_err})"))
        })?
        .to_string();

    Ok(ResolvedStream {
        url: download_url,
        filename: unrestrict
            .get("filename")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        filesize: unrestrict
            .get("filesize")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        mime_type: unrestrict
            .get("mimeType")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        info_hash: Some(info_hash.to_string()),
    })
}

pub async fn resolve_all(
    http: &reqwest::Client,
    token: &str,
    info_hash: &str,
    display_name: Option<&str>,
    trackers: &[String],
    cancel: Option<&tokio_util::sync::CancellationToken>,
    on_progress: impl Fn(&str),
) -> Result<Vec<ResolvedStream>, RdError> {
    if token.is_empty() {
        return Err(RdError::NoToken);
    }
    if crate::util::is_cancelled(cancel) {
        return Err(RdError::Cancelled);
    }
    let auth = format!("Bearer {token}");

    on_progress(&crate::util::loc("progress.rd.sendingTorrent"));
    let magnet = build_magnet(info_hash, display_name, trackers);
    let add_resp: Value = http
        .post(format!("{RD_BASE}/torrents/addMagnet"))
        .header("Authorization", &auth)
        .form(&[("magnet", magnet.as_str())])
        .send()
        .await
        .map_err(rd_other)?
        .error_for_status()
        .map_err(map_status)?
        .json()
        .await
        .map_err(rd_other)?;
    let id = add_resp
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RdError::Other(anyhow!("RD addMagnet: id missing")))?
        .to_string();

    on_progress(&crate::util::loc("progress.rd.selectingFiles"));
    let info: Value = http
        .get(format!("{RD_BASE}/torrents/info/{id}"))
        .header("Authorization", &auth)
        .send()
        .await
        .map_err(rd_other)?
        .error_for_status()
        .map_err(map_status)?
        .json()
        .await
        .map_err(rd_other)?;
    let files = info
        .get("files")
        .and_then(|f| f.as_array())
        .ok_or_else(|| RdError::Other(anyhow!("RD info: files missing")))?;
    let file_ids: Vec<u64> = files
        .iter()
        .filter_map(|f| f.get("id").and_then(|v| v.as_u64()))
        .collect();
    if file_ids.is_empty() {
        return Err(RdError::NoPlayableFile);
    }
    let csv = file_ids
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    http.post(format!("{RD_BASE}/torrents/selectFiles/{id}"))
        .header("Authorization", &auth)
        .form(&[("files", csv.as_str())])
        .send()
        .await
        .map_err(rd_other)?
        .error_for_status()
        .map_err(map_status)?;

    let mut links: Vec<String> = Vec::new();
    for attempt in 0..POLL_MAX_ATTEMPTS {
        let info: Value = http
            .get(format!("{RD_BASE}/torrents/info/{id}"))
            .header("Authorization", &auth)
            .send()
            .await
            .map_err(rd_other)?
            .error_for_status()
            .map_err(map_status)?
            .json()
            .await
            .map_err(rd_other)?;
        let status = info
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let progress = info
            .get("progress")
            .and_then(|v| v.as_f64().or(v.as_u64().map(|u| u as f64)))
            .unwrap_or(0.0);
        on_progress(&crate::util::loc_p(
            "progress.rd.status",
            serde_json::json!({ "status": status, "pct": progress.round() as i64 }),
        ));
        if status == "downloaded" {
            if let Some(arr) = info.get("links").and_then(|v| v.as_array()) {
                links = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
            }
            break;
        }
        if matches!(status, "magnet_error" | "error" | "virus" | "dead") {
            return Err(RdError::Http {
                status: 502,
                message: format!("RD torrent {status}"),
            });
        }
        if attempt + 1 == POLL_MAX_ATTEMPTS {
            return Err(RdError::Timeout);
        }
        tokio::select! {
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
            _ = crate::util::cancelled_opt(cancel) => return Err(RdError::Cancelled),
        }
    }
    if links.is_empty() {
        return Err(RdError::NoPlayableFile);
    }

    let total = links.len();
    let mut out: Vec<ResolvedStream> = Vec::with_capacity(total);
    for (i, restricted) in links.into_iter().enumerate() {
        if crate::util::is_cancelled(cancel) {
            return Err(RdError::Cancelled);
        }
        on_progress(&crate::util::loc_p(
            "progress.rd.resolvingLinkN",
            serde_json::json!({ "n": i + 1, "total": total }),
        ));
        let resolved: Result<ResolvedStream, RdError> = async {
            let unrestrict: Value = http
                .post(format!("{RD_BASE}/unrestrict/link"))
                .header("Authorization", &auth)
                .form(&[("link", restricted.as_str())])
                .send()
                .await
                .map_err(rd_other)?
                .error_for_status()
                .map_err(map_status)?
                .json()
                .await
                .map_err(rd_other)?;
            let download_url = unrestrict
                .get("download")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    let rd_err = unrestrict.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
                    RdError::Other(anyhow!("RD unrestrict: no URL ({rd_err})"))
                })?
                .to_string();
            let filename = unrestrict
                .get("filename")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
                .unwrap_or_else(|| crate::util::filename_from_url(&download_url));
            Ok(ResolvedStream {
                url: download_url,
                filename,
                filesize: unrestrict
                    .get("filesize")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                mime_type: unrestrict
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                info_hash: Some(info_hash.to_string()),
            })
        }
        .await;
        match resolved {
            Ok(s) => out.push(s),
            Err(e) => tracing::warn!("[realdebrid] resolve_all: file {} saltato ({e})", i + 1),
        }
    }
    if out.is_empty() {
        return Err(RdError::NoPlayableFile);
    }
    Ok(out)
}

fn pick_file_id(files: &[Value], hint: Option<&str>) -> Option<u64> {
    let video_ext = regex::Regex::new(r"(?i)\.(mkv|mp4|m4v|mov|avi|webm|ts|m2ts|mts|flv|wmv)$")
        .expect("video ext regex");
    let candidates: Vec<&Value> = files
        .iter()
        .filter(|f| {
            f.get("path")
                .and_then(|p| p.as_str())
                .map(|p| video_ext.is_match(p))
                .unwrap_or(false)
        })
        .collect();
    let pool = if !candidates.is_empty() {
        candidates
    } else {
        files.iter().collect()
    };
    if let Some(h) = hint {
        if !h.is_empty() {
            let h_lower = h.to_lowercase();
            if let Some(found) = pool.iter().find(|f| {
                f.get("path")
                    .and_then(|p| p.as_str())
                    .map(|p| p.to_lowercase().contains(&h_lower))
                    .unwrap_or(false)
            }) {
                return found.get("id").and_then(|v| v.as_u64());
            }
        }
    }

    pool.iter()
        .max_by_key(|f| f.get("bytes").and_then(|b| b.as_u64()).unwrap_or(0))
        .and_then(|f| f.get("id").and_then(|v| v.as_u64()))
}

fn build_magnet(info_hash: &str, display_name: Option<&str>, trackers: &[String]) -> String {
    let mut s = format!("magnet:?xt=urn:btih:{info_hash}");
    if let Some(dn) = display_name {
        if !dn.is_empty() {
            s.push_str(&format!("&dn={}", urlencoding::encode(dn)));
        }
    }

    for tr in trackers.iter().filter(|t| !t.is_empty()) {
        s.push_str(&format!("&tr={}", urlencoding::encode(tr)));
    }
    s
}

fn rd_other(e: reqwest::Error) -> RdError {
    if let Some(status) = e.status() {
        return RdError::Http {
            status: status.as_u16(),
            message: e.to_string(),
        };
    }
    RdError::Other(anyhow!(e))
}

fn map_status(e: reqwest::Error) -> RdError {
    if let Some(status) = e.status() {
        return RdError::Http {
            status: status.as_u16(),
            message: format!("RD HTTP {status}"),
        };
    }
    RdError::Other(anyhow!(e))
}

pub async fn follow_to_final(http: &reqwest::Client, url: &str) -> Result<ResolvedStream> {
    let resp = http
        .get(url)
        .header("Range", "bytes=0-0")
        .send()
        .await?
        .error_for_status()
        .map_err(|e| anyhow!("HTTP {}", e.status().map(|s| s.as_u16()).unwrap_or(0)))?;
    let status = resp.status();
    if !status.is_success() && status.as_u16() != 206 {
        bail!("upstream HTTP {}", status.as_u16());
    }
    let final_url = resp.url().clone();
    let filesize = resp
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mime_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let filename = crate::util::filename_from_url(final_url.as_str());
    Ok(ResolvedStream {
        url: final_url.to_string(),
        filename,
        filesize,
        mime_type,
        info_hash: None,
    })
}
