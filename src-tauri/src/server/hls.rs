//! HLS for the browser's player, as Stremio's streaming server does it. The
//! page gets a VOD playlist of the whole film in 6-second segments, and
//! ffmpeg makes them on demand from the one asked for: a seek is the
//! player's own business (hls.js, or Safari by itself), and what was made
//! stays on disk for a while, so going back is immediate.
//!
//! One ffmpeg at a time per session. Its fragmented MP4 (the video copied or
//! transcoded, the audio in AAC) comes out on stdout and is cut here into the
//! playlist's segments. ffmpeg's MP4 starts every track at zero, so each
//! fragment's decode time (`tfdt`) gets the film time where its track started
//! in the job. A copied video starts at the keyframe before the target,
//! reported by a one-packet side output of the same job, which seeks the same
//! way; a transcoded one at the target itself. The audio starts about there
//! too, and another one-packet side output, encoded the same way, says
//! exactly where. Every text subtitle inside the file comes out of the
//! same job as WebVTT with the film's times (`-copyts`) and is merged per
//! track for the page, so switching subtitles restarts nothing.
//!
//! A transcode runs on the GPU when the server has one (`transcode.rs`). The
//! init segment comes from the first job that makes a fragment, so a GPU that
//! fails before that gives way to the CPU with nothing of it in the playlist.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;

use crate::ffprobe::ProbeInfo;

use super::media::{self, Source, TEXT_SUBTITLES};
use super::transcode::{self, Transcoder};
use super::Server;

/// Segment length, in seconds.
const SEGMENT: f64 = 6.0;
/// Segments made ahead of the last one the player asked for (3 minutes):
/// further on, ffmpeg waits (it blocks on its output).
const AHEAD: usize = 30;
/// Segments kept on disk (12 minutes); the farthest from the player go.
const KEEP: usize = 120;
/// A segment this close after the job's next one is waited for, not started
/// again from.
const REACH: usize = 2;
/// Longest wait for a segment or the init segment.
const WAIT: Duration = Duration::from_secs(90);
/// A session nobody asked anything of for this long ends.
const IDLE: Duration = Duration::from_secs(120);

fn root_dir() -> PathBuf {
    std::env::temp_dir().join("siiishub-hls")
}

#[derive(Default)]
pub struct Hls {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    jobs: AtomicU64,
}

struct Session {
    id: String,
    key: String,
    source: Source,
    probe: ProbeInfo,
    transcode: bool,
    /// How the video is transcoded (`None`: copied).
    transcoder: Mutex<Option<Transcoder>>,
    audio: u32,
    /// `sub_idx` of the text subtitles, in the order the page lists them.
    subs: Vec<u32>,
    count: usize,
    dir: PathBuf,
    state: Mutex<SessionState>,
    changed: Notify,
    used: Mutex<Instant>,
}

#[derive(Default)]
struct SessionState {
    init: bool,
    ready: BTreeSet<usize>,
    wanted: usize,
    job: Option<Job>,
    cues: Vec<Vec<Cue>>,
    seen: Vec<HashSet<(i64, String)>>,
    failed: bool,
}

struct Job {
    id: u64,
    from: usize,
    next: usize,
    /// It made a fragment.
    produced: bool,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Serialize)]
struct Cue {
    start: f64,
    end: f64,
    text: String,
}

impl Session {
    fn touch(&self) {
        *self.used.lock() = Instant::now();
    }

    fn segment_path(&self, k: usize) -> PathBuf {
        self.dir.join(format!("{k}.m4s"))
    }

    fn end(&self) {
        if let Some(job) = self.state.lock().job.take() {
            job.task.abort();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// Replaces the running job with one from segment `from`.
    fn start_job(self: &Arc<Self>, hls: &Hls, state: &mut SessionState, from: usize) {
        if let Some(old) = state.job.take() {
            old.task.abort();
        }
        let id = hls.jobs.fetch_add(1, Ordering::Relaxed) + 1;
        let session = self.clone();
        let task = tokio::spawn(async move { run_job(session, id, from).await });
        state.job = Some(Job {
            id,
            from,
            next: from,
            produced: false,
            task,
        });
        state.failed = false;
    }

    fn is_current(&self, job: u64) -> bool {
        self.state.lock().job.as_ref().is_some_and(|j| j.id == job)
    }

    /// Moves the transcode one step off the GPU; false when it cannot go
    /// further (on the CPU already, or the video is copied).
    fn fall_back(&self) -> bool {
        let mut transcoder = self.transcoder.lock();
        let Some(next) = transcoder.as_ref().and_then(|t| t.fallback(self.probe.video.as_ref())) else {
            return false;
        };
        tracing::warn!("[hls] {}: the GPU could not transcode it, again {}", self.id, next.describe());
        *transcoder = Some(next);
        true
    }
}

impl Hls {
    fn get(&self, id: &str) -> Option<Arc<Session>> {
        let session = self.sessions.lock().get(id).cloned()?;
        session.touch();
        Some(session)
    }

    /// Ends the sessions nobody used lately (players closed without saying).
    pub fn sweep(&self) {
        let now = Instant::now();
        let idle: Vec<Arc<Session>> = {
            let mut sessions = self.sessions.lock();
            let ids: Vec<String> = sessions
                .iter()
                .filter(|(_, s)| now.duration_since(*s.used.lock()) > IDLE)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter().filter_map(|id| sessions.remove(id)).collect()
        };
        for session in idle {
            tracing::info!("[hls] session {} idle, ended", session.id);
            session.end();
        }
    }
}

/// Clears what a previous run left behind, then ends idle sessions now and
/// then.
pub fn start_sweeper(server: Server) {
    let _ = std::fs::remove_dir_all(root_dir());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            server.hls.sweep();
        }
    });
}

// ---------- Session creation ----------

#[derive(Deserialize)]
pub struct CreateParams {
    /// `copy` (default) or `h264`.
    #[serde(default)]
    video: Option<String>,
    #[serde(default)]
    audio: u32,
    /// Where the player starts: the first job begins there right away.
    #[serde(default)]
    start: f64,
}

#[derive(Serialize)]
struct Created {
    session: String,
    playlist: String,
    /// `sub_idx` of the subtitles `/hls/<session>/subtitles/<n>` serves.
    subtitles: Vec<u32>,
}

/// `POST /media/<id>/hls`: an HLS session for a published stream, or the
/// one already open for it with the same choices.
pub async fn create(
    State(server): State<Server>,
    Path(id): Path<String>,
    Query(params): Query<CreateParams>,
) -> Response {
    let (source, probe) = match media::probed(&server, &id).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    if probe.duration <= 0.0 || probe.video.is_none() {
        return (StatusCode::UNPROCESSABLE_ENTITY, "no duration or no video: no playlist").into_response();
    }
    let transcode = params.video.as_deref() == Some("h264");
    let audio = params.audio.min(probe.audios.len().saturating_sub(1) as u32);
    let key = format!("{id}|{transcode}|{audio}");
    let from = ((params.start.max(0.0) / SEGMENT).floor() as usize).min(segment_count(probe.duration) - 1);

    let existing = server
        .hls
        .sessions
        .lock()
        .values()
        .find(|s| s.key == key)
        .cloned();
    let session = match existing {
        Some(session) => session,
        None => {
            let transcoder = match transcode {
                true => Some(server.gpu.transcoder(probe.video.as_ref()).await),
                false => None,
            };
            let sid = super::random_hex(16);
            let dir = root_dir().join(&sid);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                tracing::error!("[hls] {}: {e}", dir.display());
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            let subs: Vec<u32> = probe
                .subs
                .iter()
                .filter(|s| TEXT_SUBTITLES.contains(&s.codec.as_str()))
                .map(|s| s.sub_idx)
                .collect();
            let count = segment_count(probe.duration);
            let state = SessionState {
                cues: vec![Vec::new(); subs.len()],
                seen: vec![HashSet::new(); subs.len()],
                ..Default::default()
            };
            tracing::info!(
                "[hls] session {sid} for {id}: video {}, audio track {audio}, {count} segments",
                transcoder.as_ref().map_or("copied".to_string(), |t| format!("transcoded {}", t.describe()))
            );
            let session = Arc::new(Session {
                id: sid.clone(),
                key,
                source,
                probe,
                transcode,
                transcoder: Mutex::new(transcoder),
                audio,
                subs,
                count,
                dir,
                state: Mutex::new(state),
                changed: Notify::new(),
                used: Mutex::new(Instant::now()),
            });
            server.hls.sessions.lock().insert(sid.clone(), session.clone());
            session
        }
    };
    session.touch();
    {
        let mut state = session.state.lock();
        state.wanted = from;
        let covered = state
            .job
            .as_ref()
            .is_some_and(|j| j.from <= from && from <= j.next + REACH);
        if !covered && !state.ready.contains(&from) {
            session.start_job(&server.hls, &mut state, from);
        }
    }
    Json(Created {
        session: session.id.clone(),
        playlist: format!("/hls/{}/master.m3u8", session.id),
        subtitles: session.subs.clone(),
    })
    .into_response()
}

fn segment_count(duration: f64) -> usize {
    ((duration / SEGMENT).ceil() as usize).max(1)
}

/// `DELETE /hls/<session>`: the player closed.
pub async fn close(State(server): State<Server>, Path(sid): Path<String>) -> Response {
    let session = server.hls.sessions.lock().remove(&sid);
    if let Some(session) = session {
        session.end();
    }
    StatusCode::NO_CONTENT.into_response()
}

// ---------- Playlists ----------

fn playlist(body: String) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

pub async fn master(State(server): State<Server>, Path(sid): Path<String>) -> Response {
    let Some(session) = server.hls.get(&sid) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let probe = &session.probe;
    let video = probe.video.as_ref();
    let (width, height) = match video {
        Some(v) if session.transcode => transcode::output_size(v),
        Some(v) => (v.width, v.height),
        None => (1920, 1080),
    };
    let bandwidth = if session.transcode {
        transcode::peak_bitrate(video)
    } else {
        probe.bitrate.max(1_000_000)
    };
    playlist(format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-STREAM-INF:BANDWIDTH={bandwidth},RESOLUTION={width}x{height},CODECS=\"{}\"\nmedia.m3u8\n",
        codecs(probe, session.transcode)
    ))
}

pub async fn media_playlist(State(server): State<Server>, Path(sid): Path<String>) -> Response {
    let Some(session) = server.hls.get(&sid) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut body = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n",
        SEGMENT as u64
    );
    let duration = session.probe.duration;
    for k in 0..session.count {
        let length = (duration - k as f64 * SEGMENT).min(SEGMENT).max(0.001);
        body.push_str(&format!("#EXTINF:{length:.6},\nsegments/{k}\n"));
    }
    body.push_str("#EXT-X-ENDLIST\n");
    playlist(body)
}

/// The RFC 6381 codecs of the variant, for the player to set its decoders up
/// before the first segment.
fn codecs(probe: &ProbeInfo, transcode: bool) -> String {
    let video = match probe.video.as_ref() {
        _ if transcode => "avc1.640028".to_string(),
        Some(v) => match v.codec.as_str() {
            "h264" => {
                let profile = match v.profile.as_str() {
                    "baseline" | "constrained baseline" => 0x42,
                    "main" => 0x4d,
                    "high 10" => 0x6e,
                    _ => 0x64,
                };
                let level = if v.level > 0 { v.level } else { 40 };
                format!("avc1.{profile:02x}00{level:02x}")
            }
            "hevc" => {
                let level = if v.level > 0 { v.level } else { 150 };
                if v.bit_depth > 8 || v.profile.contains("10") {
                    format!("hvc1.2.4.L{level}.B0")
                } else {
                    format!("hvc1.1.6.L{level}.B0")
                }
            }
            "av1" => {
                let level = if v.level > 0 { v.level } else { 8 };
                format!("av01.0.{level:02}M.{:02}", v.bit_depth.max(8))
            }
            _ => "avc1.640028".to_string(),
        },
        None => "avc1.640028".to_string(),
    };
    if probe.audios.is_empty() {
        video
    } else {
        format!("{video},mp4a.40.2")
    }
}

// ---------- Init and segments ----------

fn media_file(bytes: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "video/mp4"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    )
        .into_response()
}

/// Waits, starting or restarting ffmpeg as needed, until `ready` holds.
async fn wait_for(
    server: &Server,
    session: &Arc<Session>,
    segment: Option<usize>,
) -> Result<(), Response> {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let notified = session.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        {
            let mut state = session.state.lock();
            if let Some(k) = segment {
                state.wanted = k;
            }
            let done = match segment {
                Some(k) => state.ready.contains(&k),
                None => state.init,
            };
            if done {
                return Ok(());
            }
            let target = segment.unwrap_or(state.wanted);
            let covered = !state.failed
                && state
                    .job
                    .as_ref()
                    .is_some_and(|j| j.from <= target && target <= j.next + REACH);
            if !covered {
                if state.failed && state.job.as_ref().is_some_and(|j| j.from == target) {
                    // Nothing made yet in the session: a GPU that cannot do
                    // this video gives way.
                    let nothing_yet = !state.init && !state.job.as_ref().is_some_and(|j| j.produced);
                    if !(nothing_yet && session.fall_back()) {
                        return Err((StatusCode::BAD_GATEWAY, "ffmpeg failed").into_response());
                    }
                }
                session.start_job(&server.hls, &mut state, target);
            }
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err() {
            return Err(StatusCode::GATEWAY_TIMEOUT.into_response());
        }
    }
}

pub async fn init(State(server): State<Server>, Path(sid): Path<String>) -> Response {
    let Some(session) = server.hls.get(&sid) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Err(response) = wait_for(&server, &session, None).await {
        return response;
    }
    match tokio::fs::read(session.dir.join("init.mp4")).await {
        Ok(bytes) => media_file(bytes),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn segment(State(server): State<Server>, Path((sid, k)): Path<(String, usize)>) -> Response {
    let Some(session) = server.hls.get(&sid) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if k >= session.count {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(response) = wait_for(&server, &session, Some(k)).await {
        return response;
    }
    match tokio::fs::read(session.segment_path(k)).await {
        Ok(bytes) => media_file(bytes),
        // Pruned between the wait and the read: the player asks again.
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[derive(Deserialize)]
pub struct SubtitleQuery {
    #[serde(default)]
    from: usize,
}

/// The cues of subtitle `n` (in the order of the session's `subtitles`)
/// found so far, from the `from`-th on.
pub async fn subtitles(
    State(server): State<Server>,
    Path((sid, n)): Path<(String, usize)>,
    Query(query): Query<SubtitleQuery>,
) -> Response {
    let Some(session) = server.hls.get(&sid) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let state = session.state.lock();
    let Some(cues) = state.cues.get(n) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let from = query.from.min(cues.len());
    Json(serde_json::json!({ "cues": &cues[from..], "next": cues.len() })).into_response()
}

// ---------- The job ----------

async fn run_job(session: Arc<Session>, job: u64, from: usize) {
    let start = from as f64 * SEGMENT;
    let crc = session.dir.join(format!("job{job}.crc"));
    let audio_map = media::audio_map(&session.probe, session.audio);
    let audio_crc = audio_map.as_ref().map(|_| session.dir.join(format!("job{job}.audio.crc")));
    let sub_files: Vec<PathBuf> = (0..session.subs.len())
        .map(|i| session.dir.join(format!("job{job}.sub{i}.vtt")))
        .collect();

    let transcoder = session.transcoder.lock().clone();
    let mut args = media::input_args(&session.source, start, transcoder.as_ref());
    // The film's own times, for the subtitles (the MP4 output starts at zero
    // regardless).
    args.push("-copyts".to_string());
    args.extend(media::codec_args(&session.probe, session.audio, transcoder.as_ref(), Some(SEGMENT)));
    for arg in [
        "-movflags", "+frag_keyframe+empty_moov+default_base_moof",
        "-frag_duration", "1000000",
        "-f", "mp4",
        "pipe:1",
    ] {
        args.push(arg.to_string());
    }
    if !session.transcode {
        // Flushed at once: ffmpeg would otherwise hold these few bytes until
        // the whole job ends.
        for arg in ["-map", "0:v:0", "-c:v", "copy", "-frames:v", "1", "-flush_packets", "1", "-f", "framecrc"] {
            args.push(arg.to_string());
        }
        args.push(crc.to_string_lossy().into_owned());
    }
    if let (Some(map), Some(path)) = (&audio_map, &audio_crc) {
        args.extend(["-map".to_string(), map.clone()]);
        args.extend(media::audio_codec_args());
        for arg in ["-frames:a", "1", "-flush_packets", "1", "-f", "framecrc"] {
            args.push(arg.to_string());
        }
        args.push(path.to_string_lossy().into_owned());
    }
    for (i, index) in session.subs.iter().enumerate() {
        args.extend(media::subtitle_output_args(*index, &sub_files[i]));
    }

    let spawned = Command::new(media::ffmpeg_path())
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            tracing::error!("[hls] starting ffmpeg failed: {e}");
            fail(&session, job);
            return;
        }
    };
    tracing::info!("[hls] {} job {job}: from segment {from} ({start:.0}s)", session.id);
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        fail(&session, job);
        return;
    };
    {
        // A job replaced after a seek dies of a broken pipe: not worth a word.
        let session = session.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if session.is_current(job) {
                    tracing::warn!("[ffmpeg] {line}");
                }
            }
        });
    }
    let subtitles = tokio::spawn(follow_subtitles(session.clone(), job, sub_files));

    let side = SideOutputs { video: crc, audio: audio_crc };
    let reached_end = cut_segments(&session, job, from, start, &side, stdout).await;
    drop(child);
    // The subtitle files are complete now: one more read, then stop.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    subtitles.abort();
    // Stopped before the end of the film (the source failed): the segments
    // still wanted will start a new job.
    if !reached_end && session.is_current(job) {
        tracing::warn!("[hls] {} job {job}: ended before the end of the film", session.id);
        fail(&session, job);
    }
}

fn fail(session: &Session, job: u64) {
    let mut state = session.state.lock();
    if state.job.as_ref().is_some_and(|j| j.id == job) {
        state.failed = true;
    }
    drop(state);
    session.changed.notify_waiters();
}

struct Track {
    timescale: u32,
    video: bool,
}

/// The job's one-packet framecrc outputs: the copied video's, the audio's.
struct SideOutputs {
    video: PathBuf,
    audio: Option<PathBuf>,
}

/// The film times where the job's video and audio tracks start.
#[derive(Clone, Copy)]
struct Origins {
    video: f64,
    audio: f64,
}

/// Reads ffmpeg's fragmented MP4 and writes the init segment and the media
/// segments, fragment by fragment. Returns whether it reached the end of the
/// film; replaced by another job, it just returns.
async fn cut_segments<R: AsyncRead + Unpin>(
    session: &Arc<Session>,
    job: u64,
    from: usize,
    start: f64,
    side: &SideOutputs,
    stdout: R,
) -> bool {
    let mut reader = BufReader::with_capacity(1 << 20, stdout);
    let mut init = Vec::new();
    let mut tracks: HashMap<u32, Track> = HashMap::new();
    let mut origins: Option<Origins> = None;
    let mut current = from;
    let mut buffer: Vec<u8> = Vec::new();
    let mut moof: Option<Vec<u8>> = None;
    let mut produced = false;

    loop {
        // Far enough ahead of the player: wait (ffmpeg blocks on its output).
        loop {
            let notified = session.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = session.state.lock();
                if state.job.as_ref().map(|j| j.id) != Some(job) {
                    return true;
                }
                if current <= state.wanted + AHEAD {
                    break;
                }
            }
            let _ = tokio::time::timeout(Duration::from_secs(5), notified).await;
        }

        let (kind, bytes) = match read_box(&mut reader).await {
            Ok(Some(found)) => found,
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("[hls] {} job {job}: reading ffmpeg's output: {e}", session.id);
                break;
            }
        };
        match &kind {
            b"ftyp" => init.extend_from_slice(&bytes),
            b"moov" => {
                init.extend_from_slice(&bytes);
                tracks = parse_tracks(&bytes);
            }
            b"moof" => moof = Some(bytes),
            b"mdat" => {
                let Some(mut fragment) = moof.take() else {
                    continue;
                };
                if !produced {
                    produced = true;
                    publish_init(session, job, &init).await;
                }
                let origins = match origins {
                    Some(origins) => origins,
                    None => {
                        let video = if session.transcode {
                            // Its first frame shows at the target; B-frames
                            // decode it earlier.
                            start - first_composition_offset(&fragment, &tracks).unwrap_or(0.0)
                        } else {
                            read_origin(&side.video, start).await
                        };
                        let audio = match &side.audio {
                            Some(path) => read_origin(path, start).await,
                            None => video,
                        };
                        let found = Origins { video, audio };
                        origins = Some(found);
                        found
                    }
                };
                let Some(time) = shift_fragment(&mut fragment, &tracks, origins) else {
                    continue;
                };
                // A fragment of a later segment closes the current one; one
                // step at a time, so a gap in the source never leaves holes.
                let target = ((time + 0.001) / SEGMENT).floor() as usize;
                if target > current && !buffer.is_empty() {
                    if !store_segment(session, job, current, &mut buffer).await {
                        return true;
                    }
                    current += 1;
                    if current >= session.count {
                        return true;
                    }
                }
                buffer.extend_from_slice(&fragment);
                buffer.extend_from_slice(&bytes);
            }
            _ => {}
        }
    }
    // ffmpeg is done. The last segment of the film may end a little early
    // (the playlist's last duration is an estimate); anything shorter is a
    // job that broke off.
    if buffer.is_empty() || !store_segment(session, job, current, &mut buffer).await {
        return false;
    }
    let reached_end = current + 2 >= session.count;
    if reached_end {
        let mut state = session.state.lock();
        if let Some(j) = state.job.as_mut().filter(|j| j.id == job) {
            j.next = session.count;
        }
        drop(state);
        session.changed.notify_waiters();
    }
    reached_end
}

/// The job made its first fragment: its init segment is the session's, if
/// the session has none yet.
async fn publish_init(session: &Session, job: u64, init: &[u8]) {
    let first = {
        let mut state = session.state.lock();
        if let Some(j) = state.job.as_mut().filter(|j| j.id == job) {
            j.produced = true;
        }
        !state.init
    };
    if first && write_atomic(&session.dir.join("init.mp4"), init).await.is_ok() {
        session.state.lock().init = true;
        session.changed.notify_waiters();
    }
}

async fn store_segment(session: &Session, job: u64, k: usize, buffer: &mut Vec<u8>) -> bool {
    let bytes = std::mem::take(buffer);
    if let Err(e) = write_atomic(&session.segment_path(k), &bytes).await {
        tracing::warn!("[hls] {} segment {k}: {e}", session.id);
        return false;
    }
    let pruned = {
        let mut state = session.state.lock();
        if state.job.as_ref().map(|j| j.id) != Some(job) {
            return false;
        }
        state.ready.insert(k);
        if let Some(j) = state.job.as_mut() {
            j.next = k + 1;
        }
        // Too many on disk: the farthest from the player go first.
        let mut pruned = Vec::new();
        while state.ready.len() > KEEP {
            let wanted = state.wanted as i64;
            let far = state
                .ready
                .iter()
                .copied()
                .max_by_key(|s| (*s as i64 - wanted).abs());
            match far {
                Some(far) => {
                    state.ready.remove(&far);
                    pruned.push(far);
                }
                None => break,
            }
        }
        pruned
    };
    for k in pruned {
        let _ = tokio::fs::remove_file(session.segment_path(k)).await;
    }
    session.changed.notify_waiters();
    true
}

async fn write_atomic(path: &FsPath, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await
}

/// The film time of the first packet of one of the job's framecrc side
/// outputs: the keyframe a copied video starts at, or the audio's first.
async fn read_origin(crc: &FsPath, fallback: f64) -> f64 {
    for _ in 0..50 {
        if let Ok(text) = tokio::fs::read_to_string(crc).await {
            if let Some(origin) = parse_framecrc(&text) {
                return origin;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tracing::warn!("[hls] no framecrc at {}: segments may be off", crc.display());
    fallback
}

/// `#tb 0: 1/1000` then `0, <dts>, <pts>, ...`: the first packet's dts in
/// seconds.
fn parse_framecrc(text: &str) -> Option<f64> {
    let mut timebase = None;
    for line in text.lines() {
        if let Some(tb) = line.strip_prefix("#tb 0:") {
            let (num, den) = tb.trim().split_once('/')?;
            timebase = Some(num.trim().parse::<f64>().ok()? / den.trim().parse::<f64>().ok()?);
        } else if !line.starts_with('#') {
            let dts: f64 = line.split(',').nth(1)?.trim().parse().ok()?;
            return Some(dts * timebase?);
        }
    }
    None
}

// ---------- MP4 boxes ----------

/// One whole top-level box: its type and all its bytes, header included.
pub(super) async fn read_box<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<Option<([u8; 4], Vec<u8>)>> {
    let mut header = [0u8; 8];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let kind = [header[4], header[5], header[6], header[7]];
    let mut size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as u64;
    let mut bytes = header.to_vec();
    if size == 1 {
        let mut large = [0u8; 8];
        reader.read_exact(&mut large).await?;
        size = u64::from_be_bytes(large);
        bytes.extend_from_slice(&large);
    }
    if size < bytes.len() as u64 || size > (1 << 32) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad MP4 box size"));
    }
    let rest = size as usize - bytes.len();
    let at = bytes.len();
    bytes.resize(size as usize, 0);
    reader.read_exact(&mut bytes[at..at + rest]).await?;
    Ok(Some((kind, bytes)))
}

/// The children of the box whose body spans `start..end`: (type, body start,
/// end).
fn children(bytes: &[u8], start: usize, end: usize) -> Vec<([u8; 4], usize, usize)> {
    let mut out = Vec::new();
    let mut at = start;
    while at + 8 <= end {
        let size = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        let kind = [bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]];
        let (body, total) = if size == 1 {
            if at + 16 > end {
                break;
            }
            let large = u64::from_be_bytes(bytes[at + 8..at + 16].try_into().unwrap_or([0; 8])) as usize;
            (at + 16, large)
        } else {
            (at + 8, if size == 0 { end - at } else { size })
        };
        if total < 8 || at + total > end {
            break;
        }
        out.push((kind, body, at + total));
        at += total;
    }
    out
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Track ids, timescales and kinds from the `moov` box.
fn parse_tracks(moov: &[u8]) -> HashMap<u32, Track> {
    let mut tracks = HashMap::new();
    for (kind, body, end) in children(moov, 8, moov.len()) {
        if &kind != b"trak" {
            continue;
        }
        let mut id = None;
        let mut timescale = None;
        let mut video = false;
        for (kind, body, end) in children(moov, body, end) {
            match &kind {
                b"tkhd" => {
                    let v1 = moov.get(body) == Some(&1);
                    id = read_u32(moov, body + if v1 { 20 } else { 12 });
                }
                b"mdia" => {
                    for (kind, body, _) in children(moov, body, end) {
                        match &kind {
                            b"mdhd" => {
                                let v1 = moov.get(body) == Some(&1);
                                timescale = read_u32(moov, body + if v1 { 20 } else { 12 });
                            }
                            b"hdlr" => video = moov.get(body + 8..body + 12) == Some(b"vide"),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        if let (Some(id), Some(timescale)) = (id, timescale) {
            tracks.insert(id, Track { timescale: timescale.max(1), video });
        }
    }
    tracks
}

/// The `traf` boxes of a `moof`, with the track each is for.
fn trafs<'a>(moof: &[u8], tracks: &'a HashMap<u32, Track>) -> Vec<(&'a Track, Vec<([u8; 4], usize, usize)>)> {
    children(moof, 8, moof.len())
        .into_iter()
        .filter(|(kind, _, _)| kind == b"traf")
        .filter_map(|(_, body, end)| {
            let parts = children(moof, body, end);
            let track_id = parts
                .iter()
                .find(|(kind, _, _)| kind == b"tfhd")
                .and_then(|(_, body, _)| read_u32(moof, body + 4))?;
            Some((tracks.get(&track_id)?, parts))
        })
        .collect()
}

/// How long after its decode time the fragment's first video frame shows, in
/// seconds: the composition offset of the first sample of its `trun`.
fn first_composition_offset(moof: &[u8], tracks: &HashMap<u32, Track>) -> Option<f64> {
    let (track, parts) = trafs(moof, tracks).into_iter().find(|(track, _)| track.video)?;
    let &(_, trun, _) = parts.iter().find(|(kind, _, _)| kind == b"trun")?;
    let version = *moof.get(trun)?;
    let flags = read_u32(moof, trun)? & 0x00ff_ffff;
    if flags & 0x800 == 0 {
        return Some(0.0);
    }
    // sample_count, then the optional data offset and first sample flags,
    // then the first sample's duration, size and flags when present.
    let mut at = trun + 8;
    for bit in [0x1, 0x4, 0x100, 0x200, 0x400] {
        if flags & bit != 0 {
            at += 4;
        }
    }
    let raw = read_u32(moof, at)?;
    let offset = if version == 1 { raw as i32 as f64 } else { raw as f64 };
    Some(offset / track.timescale as f64)
}

/// Moves each track's decode time in a `moof` to the film's timeline, by the
/// film time where that track starts in the job. Returns the fragment's start
/// in film time: its video's, else its first track's.
fn shift_fragment(moof: &mut [u8], tracks: &HashMap<u32, Track>, origins: Origins) -> Option<f64> {
    let mut video_time = None;
    let mut any_time = None;
    for (track, parts) in trafs(moof, tracks) {
        let Some(&(_, tfdt, _)) = parts.iter().find(|(kind, _, _)| kind == b"tfdt") else {
            continue;
        };
        let origin = if track.video { origins.video } else { origins.audio };
        let shift = (origin * track.timescale as f64).round().max(0.0) as u64;
        let seconds = if moof.get(tfdt) == Some(&1) {
            let field = moof.get_mut(tfdt + 4..tfdt + 12)?;
            let value = u64::from_be_bytes(field.try_into().ok()?) + shift;
            field.copy_from_slice(&value.to_be_bytes());
            value as f64 / track.timescale as f64
        } else {
            let field = moof.get_mut(tfdt + 4..tfdt + 8)?;
            let value = u32::from_be_bytes(field.try_into().ok()?) as u64 + shift;
            let Ok(small) = u32::try_from(value) else {
                tracing::warn!("[hls] 32-bit tfdt overflow");
                return None;
            };
            field.copy_from_slice(&small.to_be_bytes());
            value as f64 / track.timescale as f64
        };
        any_time.get_or_insert(seconds);
        if track.video {
            video_time = Some(seconds);
        }
    }
    video_time.or(any_time)
}

// ---------- Subtitles ----------

/// Reads the job's growing WebVTT files and adds their cues, already in film
/// time, to the session's.
async fn follow_subtitles(session: Arc<Session>, job: u64, files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    let mut offsets = vec![0usize; files.len()];
    let mut tails = vec![String::new(); files.len()];
    loop {
        for (i, file) in files.iter().enumerate() {
            let Ok(bytes) = tokio::fs::read(file).await else {
                continue;
            };
            if bytes.len() <= offsets[i] {
                continue;
            }
            let chunk = String::from_utf8_lossy(&bytes[offsets[i]..]).into_owned();
            offsets[i] = bytes.len();
            let text = std::mem::take(&mut tails[i]) + &chunk;
            // Whole cues only: the last one may still be being written.
            let Some(cut) = text.rfind("\n\n") else {
                tails[i] = text;
                continue;
            };
            tails[i] = text[cut + 2..].to_string();
            let cues = parse_vtt(&text[..cut]);
            if cues.is_empty() {
                continue;
            }
            let mut state = session.state.lock();
            let state = &mut *state;
            if let (Some(list), Some(seen)) = (state.cues.get_mut(i), state.seen.get_mut(i)) {
                for cue in cues {
                    if seen.insert(((cue.start * 1000.0) as i64, cue.text.clone())) {
                        list.push(cue);
                    }
                }
            }
        }
        if !session.is_current(job) {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn parse_timestamp(s: &str) -> Option<f64> {
    let mut seconds = 0.0;
    for part in s.trim().split(':') {
        seconds = seconds * 60.0 + part.replace(',', ".").parse::<f64>().ok()?;
    }
    Some(seconds)
}

fn parse_vtt(text: &str) -> Vec<Cue> {
    let mut cues = Vec::new();
    for block in text.replace('\r', "").split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(at) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let Some((from, rest)) = lines[at].split_once("-->") else {
            continue;
        };
        let to = rest.split_whitespace().next().unwrap_or("");
        let body = lines[at + 1..].join("\n").trim().to_string();
        if let (Some(start), Some(end)) = (parse_timestamp(from), parse_timestamp(to)) {
            if !body.is_empty() {
                cues.push(Cue { start, end, text: body });
            }
        }
    }
    cues
}
