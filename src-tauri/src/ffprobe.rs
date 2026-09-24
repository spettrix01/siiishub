// Android ships no ffprobe binary: `probe` fails fast there and the parsing
// helpers below go unused.
#![cfg_attr(target_os = "android", allow(dead_code, unused_imports))]

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const PROBE_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, Serialize)]
pub struct ProbeInfo {
    pub duration: f64,
    pub container: String,
    pub bitrate: u64,
    pub video: Option<VideoInfo>,
    pub audios: Vec<AudioInfo>,
    pub subs: Vec<SubInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VideoInfo {
    pub codec: String,
    pub profile: String,
    pub level: i64,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: String,
    pub bit_depth: u8,
    /// `smpte2084` (HDR10, Dolby Vision) or `arib-std-b67` (HLG) for HDR.
    pub color_transfer: String,
    /// Dolby Vision profile (5, 7, 8...), 0 without Dolby Vision.
    pub dv_profile: u8,
    /// What a player without Dolby Vision gets of it
    /// (`dv_bl_signal_compatibility_id`): 1 HDR10, 2 SDR, 4 HLG, 6 Blu-ray
    /// HDR10; 0 nothing but wrong colours (profile 5).
    pub dv_compat: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioInfo {
    pub index: u32,
    pub codec: String,
    pub profile: String,
    pub channels: u32,
    pub sample_rate: u32,
    pub lang: String,
    pub title: String,
    pub default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubInfo {
    pub sub_idx: u32,
    pub codec: String,
    pub lang: String,
    pub title: String,
    pub default: bool,
    pub forced: bool,
}

#[cfg(target_os = "android")]
pub async fn probe(_url: &str) -> Result<ProbeInfo> {
    Err(anyhow!("ffprobe non disponibile su Android"))
}

#[cfg(not(target_os = "android"))]
pub async fn probe(url: &str) -> Result<ProbeInfo> {
    let bin = ffprobe_path();
    let mut cmd = Command::new(&bin);
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-analyzeduration",
            "10000000",
            "-probesize",
            "10000000",
            "-headers",
            "User-Agent: siiishub/0.1\r\n",
            url,
        ]);
    let mut child = cmd.spawn().context("spawning ffprobe")?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("ffprobe stdout missing"))?;
    let mut buf = Vec::new();
    let read_fut = stdout.read_to_end(&mut buf);
    let stat_fut = child.wait();

    let stat = tokio::time::timeout(PROBE_TIMEOUT, async {
        let _ = read_fut.await?;
        let s = stat_fut.await?;
        Ok::<_, std::io::Error>(s)
    })
    .await
    .map_err(|_| anyhow!("ffprobe timeout"))??;
    if !stat.success() {
        return Err(anyhow!("ffprobe exit {}", stat.code().unwrap_or(-1)));
    }
    let v: Value = serde_json::from_slice(&buf).context("ffprobe JSON parse")?;
    Ok(parse(v))
}

fn parse(j: Value) -> ProbeInfo {
    let fmt = j.get("format").cloned().unwrap_or(Value::Null);
    let streams = j
        .get("streams")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    let format_name = fmt
        .get("format_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .split(',')
        .next()
        .unwrap_or("")
        .to_string();
    let bitrate = fmt
        .get("bit_rate")
        .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_u64()))
        .unwrap_or(0);
    let fmt_dur = fmt
        .get("duration")
        .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_f64()))
        .unwrap_or(0.0);

    // The Dolby Vision configuration of the video, when it has one.
    let dovi = |s: &Value, field: &str| -> u8 {
        s.get("side_data_list")
            .and_then(|l| l.as_array())
            .and_then(|l| {
                l.iter().find(|d| {
                    d.get("side_data_type").and_then(|t| t.as_str()) == Some("DOVI configuration record")
                })
            })
            .and_then(|d| d.get(field))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8
    };
    let v = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("video"))
        .map(|s| VideoInfo {
            codec: s.get("codec_name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            profile: s
                .get("profile")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase(),
            level: s.get("level").and_then(|v| v.as_i64()).unwrap_or(0),
            width: s.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            height: s.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            pix_fmt: s.get("pix_fmt").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            bit_depth: s
                .get("bits_per_raw_sample")
                .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_u64()))
                .unwrap_or(8) as u8,
            color_transfer: s
                .get("color_transfer")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            dv_profile: dovi(s, "dv_profile"),
            dv_compat: dovi(s, "dv_bl_signal_compatibility_id"),
        });

    let v_dur = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("video"))
        .and_then(|s| s.get("duration"))
        .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_f64()))
        .unwrap_or(0.0);
    let duration = fmt_dur.max(v_dur);

    let audios: Vec<AudioInfo> = streams
        .iter()
        .filter(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("audio"))
        .enumerate()
        .map(|(i, s)| AudioInfo {
            index: i as u32,
            codec: s.get("codec_name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            profile: s
                .get("profile")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase(),
            channels: s.get("channels").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            sample_rate: s
                .get("sample_rate")
                .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or(v.as_u64()))
                .unwrap_or(0) as u32,
            lang: tag_str(s, "language"),
            title: tag_str(s, "title"),
            default: disposition_bool(s, "default"),
        })
        .collect();

    let subs: Vec<SubInfo> = streams
        .iter()
        .filter(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("subtitle"))
        .enumerate()
        .map(|(i, s)| SubInfo {
            sub_idx: i as u32,
            codec: s.get("codec_name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            lang: tag_str(s, "language"),
            title: tag_str(s, "title"),
            default: disposition_bool(s, "default"),
            forced: disposition_bool(s, "forced"),
        })
        .collect();

    ProbeInfo {
        duration,
        container: format_name,
        bitrate,
        video: v,
        audios,
        subs,
    }
}

fn tag_str(s: &Value, k: &str) -> String {
    s.get("tags")
        .and_then(|t| t.get(k))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn disposition_bool(s: &Value, k: &str) -> bool {
    s.get("disposition")
        .and_then(|d| d.get(k))
        .and_then(|v| v.as_u64())
        .map(|n| n == 1)
        .unwrap_or(false)
}

pub fn ffprobe_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let candidate = exe
            .parent()
            .map(|p| p.join("binaries").join(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" }));
        if let Some(p) = candidate {
            if p.exists() {
                return p;
            }
        }
    }
    PathBuf::from(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" })
}
