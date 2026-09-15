use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::settings::AddonConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonStream {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, rename = "infoHash", skip_serializing_if = "Option::is_none")]
    pub info_hash: Option<String>,
    #[serde(default, rename = "fileIdx", skip_serializing_if = "Option::is_none")]
    pub file_idx: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(flatten)]
    pub extra: serde_json::Value,
    #[serde(default, rename = "_addon", skip_serializing_if = "Option::is_none")]
    pub addon_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonStreamsResp {
    #[serde(default)]
    pub streams: Vec<AddonStream>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamsResult {
    pub streams: Vec<AddonStream>,
    pub errors: Vec<AddonError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddonError {
    pub addon: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonManifest {
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub resources: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subtitle {
    pub id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(default, rename = "_addon", skip_serializing_if = "Option::is_none")]
    pub addon_label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AddonSubtitlesResp {
    #[serde(default)]
    subtitles: Vec<Subtitle>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtitlesResult {
    pub subtitles: Vec<Subtitle>,
    pub errors: Vec<AddonError>,
}

fn supports_subtitles(addon: &AddonConfig) -> bool {
    if addon.resources.is_empty() {
        return false;
    }
    addon.resources.iter().any(|r| r == "subtitles")
}

pub fn normalize_base(raw: &str) -> String {
    let mut s = raw.trim().trim_end_matches('/').to_string();
    if s.ends_with("/manifest.json") {
        s.truncate(s.len() - "/manifest.json".len());
    }
    s
}

pub async fn fetch_streams(
    http: &reqwest::Client,
    addons: &[AddonConfig],
    type_: &str,
    id: &str,
) -> StreamsResult {
    let enabled: Vec<&AddonConfig> = addons.iter().filter(|a| a.enabled).collect();
    let mut labels = Vec::with_capacity(enabled.len());
    let mut tasks = Vec::with_capacity(enabled.len());
    for addon in &enabled {
        let http = http.clone();
        let base = normalize_base(&addon.url);
        let label = if addon.name.is_empty() {
            base.clone()
        } else {
            addon.name.clone()
        };
        labels.push(label);
        let id_enc = urlencoding::encode(id).to_string();
        let url = format!("{base}/stream/{type_}/{id_enc}.json");
        tasks.push(async move {
            let res = http
                .get(&url)
                .send()
                .await
                .context("addon HTTP send")?
                .error_for_status()
                .context("addon HTTP status")?
                .json::<AddonStreamsResp>()
                .await
                .context("addon JSON parse")?;
            anyhow::Ok(res.streams)
        });
    }

    let results = futures::future::join_all(tasks).await;
    let mut out = StreamsResult {
        streams: Vec::new(),
        errors: Vec::new(),
    };
    for (i, r) in results.into_iter().enumerate() {
        let label = &labels[i];
        match r {
            Ok(streams) => {
                let total = streams.len();
                let with_sources = streams.iter().filter(|s| !s.sources.is_empty()).count();
                tracing::info!(
                    "[stremio] addon \"{label}\" returned {total} streams ({with_sources} with sources)"
                );
                for mut s in streams {
                    s.addon_label = Some(label.clone());
                    out.streams.push(s);
                }
            }
            Err(e) => out.errors.push(AddonError {
                addon: label.clone(),
                error: format!("{e:#}"),
            }),
        }
    }
    out
}

pub async fn fetch_subtitles(
    http: &reqwest::Client,
    addons: &[AddonConfig],
    type_: &str,
    id: &str,
) -> SubtitlesResult {
    let enabled: Vec<&AddonConfig> = addons
        .iter()
        .filter(|a| a.enabled && supports_subtitles(a))
        .collect();
    let mut labels = Vec::with_capacity(enabled.len());
    let mut tasks = Vec::with_capacity(enabled.len());
    for addon in &enabled {
        let http = http.clone();
        let base = normalize_base(&addon.url);
        let label = if addon.name.is_empty() {
            base.clone()
        } else {
            addon.name.clone()
        };
        labels.push(label);
        let id_enc = urlencoding::encode(id).to_string();
        let url = format!("{base}/subtitles/{type_}/{id_enc}.json");
        tasks.push(async move {
            let res = http
                .get(&url)
                .send()
                .await
                .context("addon HTTP send")?
                .error_for_status()
                .context("addon HTTP status")?
                .json::<AddonSubtitlesResp>()
                .await
                .context("addon JSON parse")?;
            anyhow::Ok(res.subtitles)
        });
    }

    let results = futures::future::join_all(tasks).await;
    let mut out = SubtitlesResult {
        subtitles: Vec::new(),
        errors: Vec::new(),
    };
    for (i, r) in results.into_iter().enumerate() {
        let label = &labels[i];
        match r {
            Ok(subs) => {
                tracing::info!(
                    "[stremio] subtitles addon \"{label}\" returned {} entries",
                    subs.len()
                );
                for mut s in subs {
                    s.addon_label = Some(label.clone());
                    out.subtitles.push(s);
                }
            }
            Err(e) => out.errors.push(AddonError {
                addon: label.clone(),
                error: format!("{e:#}"),
            }),
        }
    }
    out
}

pub async fn fetch_manifest(http: &reqwest::Client, raw_url: &str) -> Result<AddonManifest> {
    let base = normalize_base(raw_url);
    let url = format!("{base}/manifest.json");
    let resp = http
        .get(&url)
        .send()
        .await?
        .error_for_status()
        .map_err(|e| anyhow!("HTTP {}", e.status().map(|s| s.as_u16()).unwrap_or(0)))?;
    let m = resp.json::<AddonManifest>().await?;
    Ok(m)
}
