//! Media for the browser's player. What the backend resolves (the torrent
//! session's stream on 127.0.0.1, debrid links, files on disk) only the
//! server can reach, so the page gets `/media/<id>` in its place and the
//! server streams it:
//! - `/media/<id>`: as it is, with byte ranges, when the browser can play it;
//! - `/media/<id>/remux.mp4`: through ffmpeg as fragmented MP4, the video
//!   copied or turned into H.264 (`transcode.rs`) and the audio into stereo
//!   AAC, from any position (the page starts a new one to seek outside what
//!   it has);
//! - `/media/<id>/info`: what ffprobe says of it, for the page to choose.
//!
//! A text subtitle inside the file comes out of the same ffmpeg as the video,
//! into a WebVTT file the page reads as it grows (`/media/live-sub/<token>`):
//! the file is read once, at the pace the page plays it.
//! `/media/subtitle` turns a subtitle file of an addon into WebVTT.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::StreamExt;
use parking_lot::Mutex;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::ffprobe::{self, ProbeInfo};

use super::transcode::Transcoder;
use super::Server;

/// A stream nobody asked for in this long is forgotten.
const IDLE_TTL: Duration = Duration::from_secs(12 * 3600);
const SUBTITLE_MAX_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone)]
pub(super) enum Source {
    Url(String),
    File(PathBuf),
}

impl Source {
    fn parse(url: &str) -> Option<Self> {
        if url.starts_with("file:") {
            url::Url::parse(url).ok()?.to_file_path().ok().map(Source::File)
        } else if url.starts_with("http://") || url.starts_with("https://") {
            Some(Source::Url(url.to_string()))
        } else {
            None
        }
    }

    /// What ffprobe and ffmpeg open.
    pub(super) fn input(&self) -> String {
        match self {
            Source::Url(url) => url.clone(),
            Source::File(path) => path.to_string_lossy().into_owned(),
        }
    }
}

struct Entry {
    source: Source,
    probe: Option<ProbeInfo>,
    used: Instant,
}

#[derive(Default)]
pub struct Media {
    entries: Mutex<HashMap<String, Entry>>,
}

impl Media {
    /// Registers a resolved stream and returns the address the page plays it
    /// at. Anything but a stream (never, in practice) is returned as it is.
    pub fn publish(&self, url: &str, probe: Option<ProbeInfo>) -> String {
        let Some(source) = Source::parse(url) else {
            return url.to_string();
        };
        let id = super::random_hex(16);
        let now = Instant::now();
        let mut entries = self.entries.lock();
        entries.retain(|_, e| now.duration_since(e.used) < IDLE_TTL);
        entries.insert(id.clone(), Entry { source, probe, used: now });
        format!("/media/{id}")
    }

    fn get(&self, id: &str) -> Option<(Source, Option<ProbeInfo>)> {
        let mut entries = self.entries.lock();
        let entry = entries.get_mut(id)?;
        entry.used = Instant::now();
        Some((entry.source.clone(), entry.probe.clone()))
    }

    fn set_probe(&self, id: &str, probe: ProbeInfo) {
        if let Some(entry) = self.entries.lock().get_mut(id) {
            entry.probe = Some(probe);
        }
    }
}

/// The stream and what ffprobe says of it, probing it now when the resolve
/// did not (downloaded files, a probe that timed out).
pub(super) async fn probed(server: &Server, id: &str) -> Result<(Source, ProbeInfo), Response> {
    let Some((source, probe)) = server.media.get(id) else {
        return Err(StatusCode::NOT_FOUND.into_response());
    };
    if let Some(probe) = probe {
        return Ok((source, probe));
    }
    match ffprobe::probe(&source.input()).await {
        Ok(probe) => {
            server.media.set_probe(id, probe.clone());
            Ok((source, probe))
        }
        Err(e) => {
            tracing::warn!("[media] ffprobe of {id} failed: {e:#}");
            Err((StatusCode::BAD_GATEWAY, "cannot read the file").into_response())
        }
    }
}

pub async fn info(State(server): State<Server>, Path(id): Path<String>) -> Response {
    match probed(&server, &id).await {
        Ok((_, probe)) => Json(probe).into_response(),
        Err(response) => response,
    }
}

/// The stream as it is, byte ranges included, for what the browser plays.
pub async fn direct(State(server): State<Server>, Path(id): Path<String>, req: Request) -> Response {
    let Some((source, probe)) = server.media.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = match source {
        Source::File(path) => match ServeFile::new(path).oneshot(req).await {
            Ok(response) => response.map(Body::new).into_response(),
            Err(never) => match never {},
        },
        Source::Url(url) => proxy(&server.streams, &url, req.headers().get(header::RANGE)).await,
    };
    // Browsers turn `video/x-matroska` down, not the files: Matroska is the
    // container of WebM.
    if probe.as_ref().is_some_and(|p| p.container == "matroska") {
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static("video/webm"));
    }
    response
}

async fn proxy(client: &reqwest::Client, url: &str, range: Option<&HeaderValue>) -> Response {
    let mut upstream = client.get(url);
    if let Some(range) = range {
        upstream = upstream.header(header::RANGE, range.clone());
    }
    let reply = match upstream.send().await {
        Ok(reply) => reply,
        // Without the URL: debrid links open the file to whoever has them.
        Err(e) => {
            tracing::warn!("[media] upstream request failed: {}", e.without_url());
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    let mut response = Response::builder().status(reply.status());
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_RANGE,
        header::ACCEPT_RANGES,
        header::LAST_MODIFIED,
        header::ETAG,
    ] {
        if let Some(value) = reply.headers().get(&name) {
            response = response.header(name, value.clone());
        }
    }
    response
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(reply.bytes_stream()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[derive(Deserialize)]
pub struct RemuxParams {
    /// Where to start, in seconds.
    #[serde(default)]
    start: f64,
    /// Audio track, counted among the audio streams.
    #[serde(default)]
    audio: Option<u32>,
    /// `copy` (default) keeps the video as it is, `h264` transcodes it.
    #[serde(default)]
    video: Option<String>,
    /// Text subtitle to extract, counted among the subtitle streams...
    #[serde(default)]
    sub: Option<u32>,
    /// ...into the live subtitle file with this token (chosen by the page).
    #[serde(default)]
    subfile: Option<String>,
}

/// Subtitle codecs WebVTT can carry; bitmap ones (PGS, DVD) cannot.
pub(super) const TEXT_SUBTITLES: &[&str] = &["subrip", "ass", "ssa", "webvtt", "mov_text", "text"];

fn live_sub_path(token: &str) -> Option<PathBuf> {
    let valid = token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit());
    valid.then(|| std::env::temp_dir().join("siiishub-live-subs").join(format!("{token}.vtt")))
}

/// Removes the live subtitle file when its stream ends.
struct LiveSubFile(PathBuf);

impl Drop for LiveSubFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The WebVTT file a remuxed stream is writing, read by the page with byte
/// ranges as it grows; 416 while there is nothing new.
pub async fn live_sub(Path(token): Path<String>, req: Request) -> Response {
    let Some(path) = live_sub_path(&token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !path.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut response = match ServeFile::new(path).oneshot(req).await {
        Ok(response) => response.map(Body::new).into_response(),
        Err(never) => match never {},
    };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/vtt; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The stream through ffmpeg as fragmented MP4, which browsers play while it
/// arrives. The response owns ffmpeg: when the page stops reading (a seek
/// elsewhere, the player closed) it is dropped and ffmpeg with it.
pub async fn remux(
    State(server): State<Server>,
    Path(id): Path<String>,
    Query(params): Query<RemuxParams>,
) -> Response {
    let (source, probe) = match probed(&server, &id).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    let mut transcoder = match params.video.as_deref() {
        Some("h264") => Some(server.gpu.transcoder(probe.video.as_ref()).await),
        _ => None,
    };
    // The subtitle file exists (empty) from the start, so the page finds it.
    let live_sub = match (params.sub, params.subfile.as_deref().and_then(live_sub_path)) {
        (Some(index), Some(path))
            if probe
                .subs
                .get(index as usize)
                .is_some_and(|s| TEXT_SUBTITLES.contains(&s.codec.as_str())) =>
        {
            let created = match path.parent() {
                Some(dir) => std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&path, b"")),
                None => Ok(()),
            };
            match created {
                Ok(()) => Some((index, LiveSubFile(path))),
                Err(e) => {
                    tracing::warn!("[media] live subtitle file: {e}");
                    None
                }
            }
        }
        _ => None,
    };
    let (child, stdout, head) = loop {
        let live_sub = live_sub.as_ref().map(|(i, f)| (*i, f.0.as_path()));
        let args = remux_args(&source, &probe, &params, transcoder.as_ref(), live_sub);
        let spawned = Command::new(ffmpeg_path())
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                tracing::error!("[media] starting ffmpeg failed: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, "ffmpeg is not available").into_response();
            }
        };
        let (Some(mut stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!("[ffmpeg] {line}");
            }
        });
        tracing::info!(
            "[media] {id}: remux from {:.0}s, video {}, audio track {}",
            params.start,
            transcoder.as_ref().map_or("copied".to_string(), |t| format!("transcoded {}", t.describe())),
            params.audio.unwrap_or(0)
        );
        let Some(transcoder_now) = transcoder.as_ref().filter(|t| t.on_gpu()) else {
            break (child, stdout, Vec::new());
        };
        // A GPU that cannot do this video fails before the first fragment:
        // held back until then, the response can still come from the CPU.
        if let Some(head) = first_fragment(&mut stdout).await {
            break (child, stdout, head);
        }
        let next = transcoder_now.fallback(probe.video.as_ref()).unwrap_or_else(Transcoder::cpu);
        tracing::warn!("[media] {id}: the GPU could not transcode it, again {}", next.describe());
        transcoder = Some(next);
    };
    let body = futures::stream::iter([Ok::<_, std::io::Error>(Bytes::from(head))])
        .chain(tokio_util::io::ReaderStream::new(stdout))
        .map(move |chunk| {
            let _owners = (&child, &live_sub);
            chunk
        });
    Response::builder()
        .header(header::CONTENT_TYPE, "video/mp4")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

pub(super) fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// ffmpeg's global options and its input from `start` seconds, decoded on the
/// GPU when the transcode runs there.
pub(super) fn input_args(source: &Source, start: f64, transcode: Option<&Transcoder>) -> Vec<String> {
    let mut args = strings(&["-hide_banner", "-nostdin", "-nostats", "-loglevel", "error", "-y"]);
    if matches!(source, Source::Url(_)) {
        // The torrent session answers slowly while pieces arrive; debrid CDNs
        // drop long connections now and then.
        args.extend(strings(&[
            "-user_agent", "siiishub/0.1",
            "-reconnect", "1",
            "-reconnect_streamed", "1",
            "-reconnect_delay_max", "5",
        ]));
    }
    if let Some(transcoder) = transcode {
        args.extend(transcoder.input_args());
    }
    if start > 0.0 {
        args.extend(["-ss".to_string(), format!("{start:.3}")]);
    }
    args.extend(["-i".to_string(), source.input()]);
    args
}

/// The output's video and audio: the first video stream and audio track
/// `audio`, the video copied or transcoded (`transcode`) and the audio in
/// stereo AAC. `keyframes_every` forces transcoded keyframes on a grid of
/// that many seconds from where the job starts (with `-copyts`, a point of
/// the film's own grid), where HLS segments start.
pub(super) fn codec_args(
    probe: &ProbeInfo,
    audio: u32,
    transcode: Option<&Transcoder>,
    keyframes_every: Option<f64>,
) -> Vec<String> {
    let mut args = strings(&["-map", "0:v:0"]);
    if !probe.audios.is_empty() {
        let track = audio.min(probe.audios.len() as u32 - 1);
        args.extend(["-map".to_string(), format!("0:a:{track}")]);
    }
    let video = probe.video.as_ref();
    if let Some(transcoder) = transcode {
        args.extend(transcoder.video_args(video, keyframes_every));
    } else {
        args.extend(strings(&["-c:v", "copy"]));
        // Safari and Chrome want HEVC in MP4 tagged `hvc1`.
        if video.is_some_and(|v| v.codec == "hevc") {
            args.extend(strings(&["-tag:v", "hvc1"]));
        }
    }
    args.extend(strings(&["-c:a", "aac", "-ac", "2", "-b:a", "192k"]));
    args.extend(strings(&["-sn", "-dn", "-map_metadata", "-1", "-map_chapters", "-1", "-max_muxing_queue_size", "4096"]));
    args
}

/// An output of the text subtitle `index` as WebVTT, flushed cue by cue for
/// the page (or the HLS session) to read as it grows.
pub(super) fn subtitle_output_args(index: u32, path: &std::path::Path) -> Vec<String> {
    vec![
        "-map".to_string(),
        format!("0:s:{index}"),
        "-c:s".to_string(),
        "webvtt".to_string(),
        "-flush_packets".to_string(),
        "1".to_string(),
        "-f".to_string(),
        "webvtt".to_string(),
        path.to_string_lossy().into_owned(),
    ]
}

fn remux_args(
    source: &Source,
    probe: &ProbeInfo,
    params: &RemuxParams,
    transcode: Option<&Transcoder>,
    live_sub: Option<(u32, &std::path::Path)>,
) -> Vec<String> {
    let mut args = input_args(source, params.start, transcode);
    args.extend(codec_args(probe, params.audio.unwrap_or(0), transcode, None));
    args.extend(strings(&[
        "-movflags", "+frag_keyframe+empty_moov+default_base_moof",
        "-frag_duration", "2000000",
        "-f", "mp4",
        "pipe:1",
    ]));
    // Second output: the subtitle.
    if let Some((index, path)) = live_sub {
        args.extend(subtitle_output_args(index, path));
    }
    args
}

/// ffmpeg's output up to the end of its first media data, or `None` when it
/// ends before.
async fn first_fragment(stdout: &mut tokio::process::ChildStdout) -> Option<Vec<u8>> {
    let mut head = Vec::new();
    loop {
        let (kind, bytes) = super::hls::read_box(stdout).await.ok()??;
        head.extend_from_slice(&bytes);
        if &kind == b"mdat" {
            return Some(head);
        }
    }
}

pub(super) fn ffmpeg_path() -> PathBuf {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Some(bundled) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("binaries").join(name)))
        .filter(|path| path.exists())
    {
        return bundled;
    }
    PathBuf::from(name)
}

#[derive(Deserialize)]
pub struct SubtitleParams {
    url: String,
}

/// A subtitle file of an addon as WebVTT, the only format browsers show.
/// SRT is converted here; ASS, SSA and the rest go through ffmpeg.
pub async fn subtitle(State(server): State<Server>, Query(params): Query<SubtitleParams>) -> Response {
    if !(params.url.starts_with("https://") || params.url.starts_with("http://")) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let fetched = async {
        let reply = server.app.http.get(&params.url).send().await?.error_for_status()?;
        reply.bytes().await
    }
    .await;
    let bytes = match fetched {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!("[media] subtitle download failed: {}", e.without_url());
            return StatusCode::BAD_GATEWAY.into_response();
        }
    };
    if bytes.len() > SUBTITLE_MAX_BYTES {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let text = decode_text(&bytes);
    let vtt = if text.trim_start().starts_with("WEBVTT") {
        text
    } else if is_srt(&text) {
        srt_to_vtt(&text)
    } else {
        match ffmpeg_to_vtt(text.into_bytes()).await {
            Ok(vtt) => vtt,
            Err(e) => {
                tracing::warn!("[media] subtitle conversion failed: {e:#}");
                return (StatusCode::UNPROCESSABLE_ENTITY, "unsupported subtitle format").into_response();
            }
        }
    };
    (
        [
            (header::CONTENT_TYPE, "text/vtt; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        vtt,
    )
        .into_response()
}

/// UTF-8, or else Windows-1252, the usual encoding of Western subtitles.
fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => bytes.iter().map(|&b| cp1252(b)).collect(),
    }
}

fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
        '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9f => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

fn is_srt(text: &str) -> bool {
    static CUE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"(?m)^\s*\d{1,2}:\d{2}:\d{2},\d{1,3}\s*-->").expect("valid regex")
    });
    CUE.is_match(text)
}

/// SRT and WebVTT differ, for what browsers read, in the header and in the
/// decimal comma of the timestamps.
fn srt_to_vtt(srt: &str) -> String {
    let mut vtt = String::with_capacity(srt.len() + 16);
    vtt.push_str("WEBVTT\n\n");
    for line in srt.lines() {
        if line.contains("-->") {
            vtt.push_str(&line.replace(',', "."));
        } else {
            vtt.push_str(line);
        }
        vtt.push('\n');
    }
    vtt
}

async fn ffmpeg_to_vtt(input: Vec<u8>) -> anyhow::Result<String> {
    let mut child = Command::new(ffmpeg_path())
        .args(["-hide_banner", "-nostats", "-loglevel", "error", "-i", "pipe:0", "-f", "webvtt", "pipe:1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
    let mut stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?;
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&input).await;
    });
    let mut out = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(30), stdout.read_to_end(&mut out)).await;
    let _ = writer.await;
    let status = child.wait().await?;
    if read.is_err() || !status.success() || out.is_empty() {
        anyhow::bail!("ffmpeg could not convert it");
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}
