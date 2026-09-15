use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::realdebrid::{RdError, ResolvedStream};

const AD_BASE: &str = "https://api.alldebrid.com";
const AD_AGENT: &str = "siiishub";
const POLL_INTERVAL: Duration = Duration::from_millis(1500);
const POLL_MAX_ATTEMPTS: u32 = 60;

const VIDEO_EXTS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "webm", "m4v", "wmv", "flv", "ts", "m2ts",
];

fn ad_other(e: reqwest::Error) -> RdError {
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
            message: format!("AD HTTP {status}"),
        };
    }
    RdError::Other(anyhow!(e))
}

fn check_ad_error(v: &Value) -> Result<&Value, RdError> {
    if v.get("status").and_then(|s| s.as_str()) == Some("success") {
        return v
            .get("data")
            .ok_or_else(|| RdError::Other(anyhow!("AD: data missing")));
    }
    let code = v
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(|c| c.as_str())
        .unwrap_or("UNKNOWN");
    let msg = v
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("unknown error");
    if code == "AUTH_BAD_APIKEY" || code == "AUTH_BLOCKED" {
        return Err(RdError::NoToken);
    }
    Err(RdError::Http {
        status: 502,
        message: format!("AllDebrid: {code} ({msg})"),
    })
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

    on_progress(&crate::util::loc("progress.ad.sendingMagnet"));
    let magnet = build_magnet(info_hash, display_name, trackers);
    let bearer = format!("Bearer {token}");
    let upload_resp: Value = http
        .post(format!("{AD_BASE}/v4/magnet/upload"))
        .query(&[("agent", AD_AGENT)])
        .header("Authorization", &bearer)
        .form(&[("magnets[]", magnet.as_str())])
        .send()
        .await
        .map_err(ad_other)?
        .error_for_status()
        .map_err(map_status)?
        .json()
        .await
        .map_err(ad_other)?;
    let data = check_ad_error(&upload_resp)?;
    let magnets = data
        .get("magnets")
        .and_then(|m| m.as_array())
        .ok_or_else(|| RdError::Other(anyhow!("AD upload: magnets missing")))?;
    let first = magnets
        .first()
        .ok_or_else(|| RdError::Other(anyhow!("AD upload: empty magnets")))?;
    if let Some(err) = first.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
        return Err(RdError::Http { status: 502, message: format!("AllDebrid: {err}") });
    }
    let id = first
        .get("id")
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .ok_or_else(|| RdError::Other(anyhow!("AD upload: id missing")))?;

    on_progress(&crate::util::loc("progress.ad.fetchingStatus"));
    let mut links_out: Vec<Value> = Vec::new();
    for attempt in 0..POLL_MAX_ATTEMPTS {
        let status_resp: Value = http
            .get(format!("{AD_BASE}/v4.1/magnet/status"))
            .query(&[
                ("agent", AD_AGENT),
                ("id", &id.to_string()),
            ])
            .header("Authorization", &bearer)
            .send()
            .await
            .map_err(ad_other)?
            .error_for_status()
            .map_err(map_status)?
            .json()
            .await
            .map_err(ad_other)?;
        let data = check_ad_error(&status_resp)?;
        let mg = data.get("magnets").cloned().unwrap_or(Value::Null);
        let m = if mg.is_array() {
            mg.as_array()
                .and_then(|a| a.first())
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            mg
        };
        let status_code = m
            .get("statusCode")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        let status_text = m
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let progress = m
            .get("downloaded")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let size = m
            .get("size")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let pct = if size > 0 {
            (progress as f64 / size as f64) * 100.0
        } else {
            0.0
        };
        on_progress(&crate::util::loc_p(
            "progress.ad.status",
            serde_json::json!({ "status": status_text.as_str(), "pct": pct.round() as i64 }),
        ));
        if status_code == 4 {
            if let Some(arr) = m.get("links").and_then(|v| v.as_array()) {
                links_out = arr.clone();
            }
            tracing::info!(
                "[alldebrid] magnet {id} ready, {} links",
                links_out.len()
            );
            break;
        }
        if status_code >= 5 {
            return Err(RdError::Http {
                status: 502,
                message: format!("AllDebrid: {status_text}"),
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
    let mut links = links_out;
    if links.is_empty() {
        on_progress(&crate::util::loc("progress.ad.fetchingFileLinks"));
        let files_resp: Value = http
            .get(format!("{AD_BASE}/v4/magnet/files"))
            .query(&[
                ("agent", AD_AGENT),
                ("id[]", &id.to_string()),
            ])
            .header("Authorization", &bearer)
            .send()
            .await
            .map_err(ad_other)?
            .error_for_status()
            .map_err(map_status)?
            .json()
            .await
            .map_err(ad_other)?;
        let data = check_ad_error(&files_resp)?;
        let mg = data.get("magnets").cloned().unwrap_or(Value::Null);
        let m = if mg.is_array() {
            mg.as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null)
        } else {
            mg
        };
        if let Some(files) = m.get("files") {
            flatten_files(files, &mut links);
        }
        tracing::info!(
            "[alldebrid] /magnet/files fallback returned {} links",
            links.len()
        );
    }

    if links.is_empty() {
        return Err(RdError::NoPlayableFile);
    }
    let chosen = pick_link(&links, file_hint).ok_or(RdError::NoPlayableFile)?;
    let chosen_url = chosen
        .get("link")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RdError::Other(anyhow!("AD link missing")))?
        .to_string();

    on_progress(&crate::util::loc("progress.ad.resolvingLink"));
    let unlock_resp: Value = http
        .get(format!("{AD_BASE}/v4/link/unlock"))
        .query(&[
            ("agent", AD_AGENT),
            ("link", chosen_url.as_str()),
        ])
        .header("Authorization", &bearer)
        .send()
        .await
        .map_err(ad_other)?
        .error_for_status()
        .map_err(map_status)?
        .json()
        .await
        .map_err(ad_other)?;
    let data = check_ad_error(&unlock_resp)?;
    let download_url = data
        .get("link")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| RdError::Other(anyhow!("AD unlock: no URL")))?
        .to_string();
    let filename = data
        .get("filename")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let filesize = data
        .get("filesize")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    Ok(ResolvedStream {
        url: download_url,
        filename,
        filesize,
        mime_type: String::new(),
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

    on_progress(&crate::util::loc("progress.ad.sendingMagnet"));
    let magnet = build_magnet(info_hash, display_name, trackers);
    let bearer = format!("Bearer {token}");
    let upload_resp: Value = http
        .post(format!("{AD_BASE}/v4/magnet/upload"))
        .query(&[("agent", AD_AGENT)])
        .header("Authorization", &bearer)
        .form(&[("magnets[]", magnet.as_str())])
        .send()
        .await
        .map_err(ad_other)?
        .error_for_status()
        .map_err(map_status)?
        .json()
        .await
        .map_err(ad_other)?;
    let data = check_ad_error(&upload_resp)?;
    let magnets = data
        .get("magnets")
        .and_then(|m| m.as_array())
        .ok_or_else(|| RdError::Other(anyhow!("AD upload: magnets missing")))?;
    let first = magnets
        .first()
        .ok_or_else(|| RdError::Other(anyhow!("AD upload: empty magnets")))?;
    if let Some(err) = first.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()) {
        return Err(RdError::Http { status: 502, message: format!("AllDebrid: {err}") });
    }
    let id = first
        .get("id")
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .ok_or_else(|| RdError::Other(anyhow!("AD upload: id missing")))?;

    on_progress(&crate::util::loc("progress.ad.fetchingStatus"));
    let mut links_out: Vec<Value> = Vec::new();
    for attempt in 0..POLL_MAX_ATTEMPTS {
        let status_resp: Value = http
            .get(format!("{AD_BASE}/v4.1/magnet/status"))
            .query(&[("agent", AD_AGENT), ("id", &id.to_string())])
            .header("Authorization", &bearer)
            .send()
            .await
            .map_err(ad_other)?
            .error_for_status()
            .map_err(map_status)?
            .json()
            .await
            .map_err(ad_other)?;
        let data = check_ad_error(&status_resp)?;
        let mg = data.get("magnets").cloned().unwrap_or(Value::Null);
        let m = if mg.is_array() {
            mg.as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null)
        } else {
            mg
        };
        let status_code = m.get("statusCode").and_then(|v| v.as_i64()).unwrap_or(-1);
        let status_text = m
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let progress = m.get("downloaded").and_then(|v| v.as_u64()).unwrap_or(0);
        let size = m.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        let pct = if size > 0 {
            (progress as f64 / size as f64) * 100.0
        } else {
            0.0
        };
        on_progress(&crate::util::loc_p(
            "progress.ad.status",
            serde_json::json!({ "status": status_text.as_str(), "pct": pct.round() as i64 }),
        ));
        if status_code == 4 {
            if let Some(arr) = m.get("links").and_then(|v| v.as_array()) {
                links_out = arr.clone();
            }
            break;
        }
        if status_code >= 5 {
            return Err(RdError::Http {
                status: 502,
                message: format!("AllDebrid: {status_text}"),
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
    let mut links = links_out;
    if links.is_empty() {
        on_progress(&crate::util::loc("progress.ad.fetchingFileLinks"));
        let files_resp: Value = http
            .get(format!("{AD_BASE}/v4/magnet/files"))
            .query(&[("agent", AD_AGENT), ("id[]", &id.to_string())])
            .header("Authorization", &bearer)
            .send()
            .await
            .map_err(ad_other)?
            .error_for_status()
            .map_err(map_status)?
            .json()
            .await
            .map_err(ad_other)?;
        let data = check_ad_error(&files_resp)?;
        let mg = data.get("magnets").cloned().unwrap_or(Value::Null);
        let m = if mg.is_array() {
            mg.as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null)
        } else {
            mg
        };
        if let Some(files) = m.get("files") {
            flatten_files(files, &mut links);
        }
    }
    if links.is_empty() {
        return Err(RdError::NoPlayableFile);
    }

    let total = links.len();
    let mut out: Vec<ResolvedStream> = Vec::with_capacity(total);
    for (i, l) in links.iter().enumerate() {
        if crate::util::is_cancelled(cancel) {
            return Err(RdError::Cancelled);
        }
        let link_url = match l.get("link").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };
        on_progress(&crate::util::loc_p(
            "progress.ad.resolvingLinkN",
            serde_json::json!({ "n": i + 1, "total": total }),
        ));
        let resolved: Result<ResolvedStream, RdError> = async {
            let unlock_resp: Value = http
                .get(format!("{AD_BASE}/v4/link/unlock"))
                .query(&[("agent", AD_AGENT), ("link", link_url.as_str())])
                .header("Authorization", &bearer)
                .send()
                .await
                .map_err(ad_other)?
                .error_for_status()
                .map_err(map_status)?
                .json()
                .await
                .map_err(ad_other)?;
            let data = check_ad_error(&unlock_resp)?;
            let download_url = data
                .get("link")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| RdError::Other(anyhow!("AD unlock: no URL")))?
                .to_string();
            let filename = data
                .get("filename")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
                .or_else(|| {
                    l.get("filename")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                })
                .unwrap_or_else(|| crate::util::filename_from_url(&download_url));
            Ok(ResolvedStream {
                url: download_url,
                filename,
                filesize: data.get("filesize").and_then(|v| v.as_u64()).unwrap_or(0),
                mime_type: String::new(),
                info_hash: Some(info_hash.to_string()),
            })
        }
        .await;
        match resolved {
            Ok(s) => out.push(s),
            Err(e) => tracing::warn!("[alldebrid] resolve_all: file {} saltato ({e})", i + 1),
        }
    }
    if out.is_empty() {
        return Err(RdError::NoPlayableFile);
    }
    Ok(out)
}

fn flatten_files(node: &Value, out: &mut Vec<Value>) {
    if let Some(arr) = node.as_array() {
        for item in arr {
            flatten_files(item, out);
        }
        return;
    }
    if let Some(obj) = node.as_object() {
        if let Some(l) = obj.get("l").and_then(|v| v.as_str()) {
            if !l.is_empty() {
                let name = obj
                    .get("n")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let size = obj.get("s").and_then(|v| v.as_u64()).unwrap_or(0);
                out.push(serde_json::json!({
                    "filename": name,
                    "size": size,
                    "link": l,
                }));
            }
        }
        if let Some(e) = obj.get("e") {
            flatten_files(e, out);
        }
    }
}

fn pick_link<'a>(links: &'a [Value], hint: Option<&str>) -> Option<&'a Value> {
    let is_video = |path: &str| {
        let lc = path.to_ascii_lowercase();
        VIDEO_EXTS.iter().any(|ext| lc.ends_with(&format!(".{ext}")))
    };
    let filename_of = |l: &Value| -> String {
        l.get("filename")
            .and_then(|v| v.as_str())
            .or_else(|| l.get("link").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string()
    };
    let size_of = |l: &Value| l.get("size").and_then(|v| v.as_u64()).unwrap_or(0);

    if let Some(h) = hint.filter(|s| !s.is_empty()) {
        let h_lc = h.to_lowercase();
        if let Some(found) = links
            .iter()
            .find(|l| filename_of(l).to_lowercase().contains(&h_lc))
        {
            return Some(found);
        }
    }
    let videos: Vec<&Value> = links.iter().filter(|l| is_video(&filename_of(l))).collect();
    let pool: &[&Value] = if !videos.is_empty() { &videos[..] } else { return links.iter().max_by_key(|l| size_of(l)); };
    pool.iter().copied().max_by_key(|l| size_of(l))
}
