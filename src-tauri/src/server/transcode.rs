//! How the video is transcoded when the browser cannot decode it: on the
//! server's GPU when it has one that works, else on the CPU.
//!
//! The GPU is looked for at startup as Stremio's streaming server does, by
//! trying it: NVENC on an NVIDIA GPU, then VAAPI (Intel, AMD) on every render
//! node, each with a second of video through the pipeline of a real
//! transcode. The first that works is used; `SIIISHUB_HWACCEL` and
//! `SIIISHUB_HWACCEL_DEVICE` choose instead. A job the GPU fails before its
//! first fragment is made again with less of it (`Transcoder::fallback`): HDR
//! mapped on the CPU, then everything on the CPU.
//!
//! On the GPU the video is decoded there when the GPU knows the codec (FFmpeg
//! decodes the rest and the frames are uploaded), scaled there and encoded
//! there. HDR10 is mapped to SDR there with Intel's VAAPI driver
//! (`tonemap_vaapi`); with the others the frames come back to the CPU for that
//! step alone.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::OnceCell;

use crate::ffprobe::VideoInfo;

use super::media::{self, strings};

/// Longest a test may take: a GPU driver can hang.
const TEST_TIMEOUT: Duration = Duration::from_secs(20);

/// HDR (PQ or HLG) to SDR, from linear light (the zscale before it); zscale
/// reads the input's colours from the frames.
const TONEMAP: &str = "format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv";

/// `SIIISHUB_HWACCEL`: which GPU to transcode on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wanted {
    Auto,
    Off,
    Vaapi,
    Nvenc,
}

impl Wanted {
    pub fn parse(value: Option<&str>) -> anyhow::Result<Self> {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") | Some("auto") => Ok(Self::Auto),
            Some("off") => Ok(Self::Off),
            Some("vaapi") => Ok(Self::Vaapi),
            Some("nvenc") => Ok(Self::Nvenc),
            Some(other) => anyhow::bail!("SIIISHUB_HWACCEL is `{other}`: it can be auto, off, vaapi or nvenc"),
        }
    }
}

#[derive(Clone, Debug)]
enum Kind {
    /// A render node, `/dev/dri/renderD128`.
    Vaapi(PathBuf),
    /// A CUDA device.
    Nvenc(u32),
}

#[derive(Clone, Copy, Debug)]
enum RateControl {
    /// A bitrate from the picture's size.
    Vbr,
    /// A constant quantizer, for drivers without VBR.
    Cqp,
}

/// A GPU that passed the startup tests, and how it did.
#[derive(Clone, Debug)]
pub struct Accel {
    kind: Kind,
    rate: RateControl,
    /// Intel's low-power encoder, the only one some chips have.
    low_power: bool,
    /// HDR10 mapped to SDR on the GPU.
    hdr_on_gpu: bool,
    /// HDR mapped on the CPU between the GPU's decoding and encoding.
    hdr_via_cpu: bool,
    /// The driver or the card, for the log.
    name: String,
}

impl Accel {
    fn api(&self) -> &'static str {
        match self.kind {
            Kind::Vaapi(_) => "VAAPI",
            Kind::Nvenc(_) => "NVENC",
        }
    }

    fn summary(&self) -> String {
        let device = match &self.kind {
            Kind::Vaapi(node) => node.display().to_string(),
            Kind::Nvenc(index) => format!("GPU {index}"),
        };
        let mut text = format!("{} on {device}", self.api());
        if !self.name.is_empty() {
            text.push_str(&format!(" ({})", self.name));
        }
        if self.low_power {
            text.push_str(", low-power encoder");
        }
        if matches!(self.rate, RateControl::Cqp) {
            text.push_str(", constant quantizer");
        }
        text.push_str(if self.hdr_on_gpu {
            "; HDR10 mapped to SDR on the GPU"
        } else if self.hdr_via_cpu {
            "; HDR mapped to SDR on the CPU"
        } else {
            "; HDR transcoded on the CPU"
        });
        text
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Gpu,
    /// The GPU, with HDR mapped on the CPU.
    GpuHdrOnCpu,
    Cpu,
}

/// How a video is transcoded.
#[derive(Clone, Debug)]
pub struct Transcoder {
    accel: Option<Arc<Accel>>,
    step: Step,
}

impl Transcoder {
    pub fn cpu() -> Self {
        Self { accel: None, step: Step::Cpu }
    }

    /// The best way `accel` has for `video`.
    fn new(accel: Option<Arc<Accel>>, video: Option<&VideoInfo>) -> Self {
        let (Some(accel), Some(video)) = (accel, video) else {
            return Self::cpu();
        };
        let step = if !is_hdr(video) || (is_pq(video) && accel.hdr_on_gpu) {
            Step::Gpu
        } else if accel.hdr_via_cpu {
            Step::GpuHdrOnCpu
        } else {
            return Self::cpu();
        };
        Self { accel: Some(accel), step }
    }

    /// What to try after a failure: HDR mapped on the CPU, then the CPU alone.
    pub fn fallback(&self, video: Option<&VideoInfo>) -> Option<Self> {
        let accel = self.accel.as_ref()?;
        if self.step == Step::Gpu && video.is_some_and(is_hdr) && accel.hdr_via_cpu {
            return Some(Self { accel: Some(accel.clone()), step: Step::GpuHdrOnCpu });
        }
        Some(Self::cpu())
    }

    pub fn on_gpu(&self) -> bool {
        self.accel.is_some()
    }

    /// For the log.
    pub fn describe(&self) -> String {
        match (&self.accel, self.step) {
            (Some(accel), Step::GpuHdrOnCpu) => format!("on the GPU ({}), HDR mapped on the CPU", accel.api()),
            (Some(accel), _) => format!("on the GPU ({})", accel.api()),
            (None, _) => "on the CPU".to_string(),
        }
    }

    /// ffmpeg's options for the GPU, before the input: decoding there, into
    /// frames that stay there.
    pub fn input_args(&self) -> Vec<String> {
        let Some(accel) = &self.accel else {
            return Vec::new();
        };
        let (device, api) = match &accel.kind {
            Kind::Vaapi(node) => (format!("vaapi=gpu:{}", node.display()), "vaapi"),
            Kind::Nvenc(index) => (format!("cuda=gpu:{index}"), "cuda"),
        };
        vec![
            "-init_hw_device".to_string(),
            device,
            "-filter_hw_device".to_string(),
            "gpu".to_string(),
            "-hwaccel".to_string(),
            api.to_string(),
            "-hwaccel_device".to_string(),
            "gpu".to_string(),
            "-hwaccel_output_format".to_string(),
            api.to_string(),
        ]
    }

    /// The filters and the encoder: H.264 of at most 1080p, HDR mapped to SDR
    /// (or the picture comes out grey and washed out). `keyframes_every`
    /// forces keyframes on a grid of that many seconds from the first frame,
    /// where HLS segments start.
    pub fn video_args(&self, video: Option<&VideoInfo>, keyframes_every: Option<f64>) -> Vec<String> {
        let mut args = match (&self.accel, video) {
            (Some(accel), Some(video)) => gpu_args(accel, video, self.step == Step::Gpu),
            _ => cpu_args(video),
        };
        match keyframes_every {
            Some(every) => args.extend([
                "-force_key_frames".to_string(),
                format!("expr:if(isnan(prev_forced_t),1,gte(floor(t/{every}),floor(prev_forced_t/{every})+1))"),
            ]),
            None => args.extend(strings(&["-g", "48"])),
        }
        args
    }
}

fn is_hdr(video: &VideoInfo) -> bool {
    matches!(video.color_transfer.as_str(), "smpte2084" | "arib-std-b67")
}

/// HDR10 (and Dolby Vision's base): what the GPU's tone mapping takes.
fn is_pq(video: &VideoInfo) -> bool {
    video.color_transfer == "smpte2084"
}

/// The transcoded picture: at most 1920 wide, even sides.
pub fn output_size(video: &VideoInfo) -> (u32, u32) {
    let (width, height) = (video.width.max(2), video.height.max(2));
    if width > 1920 {
        (1920, ((height as u64 * 1920 / width as u64) as u32).max(2) & !1)
    } else {
        (width & !1, height & !1)
    }
}

/// What a GPU encode aims at (the CPU's aims at a quality): 8 Mb/s at 1080p,
/// in proportion below, at least 2.
fn target_bitrate((width, height): (u32, u32)) -> u64 {
    let pixels = width as u64 * height as u64;
    (8_000_000 * pixels / (1920 * 1080)).clamp(2_000_000, 8_000_000)
}

/// The most a transcoded video should take, for the playlist.
pub fn peak_bitrate(video: Option<&VideoInfo>) -> u64 {
    video.map_or(12_000_000, |v| target_bitrate(output_size(v)) * 3 / 2)
}

fn cpu_args(video: Option<&VideoInfo>) -> Vec<String> {
    // Scaled down before the mapping, which then costs a quarter (4K HDR10
    // HEVC on 16 threads: 4.3x real time instead of 1.2x).
    let chain = if video.is_some_and(is_hdr) {
        format!("zscale=w='min(1920,iw)':h=-2:t=linear:npl=100,{TONEMAP},format=yuv420p")
    } else {
        "scale='min(1920,iw)':-2,format=yuv420p".to_string()
    };
    let mut args = vec!["-vf".to_string(), chain];
    args.extend(strings(&["-c:v", "libx264", "-preset", "veryfast", "-crf", "21", "-profile:v", "high"]));
    args
}

/// `format=...|vaapi,hwupload` takes both the GPU's frames, which it passes
/// on, and FFmpeg's when the GPU cannot decode the codec, which it uploads.
fn gpu_args(accel: &Accel, video: &VideoInfo, hdr_on_gpu: bool) -> Vec<String> {
    let (w, h) = output_size(video);
    let hdr = is_hdr(video);
    let chain = match &accel.kind {
        Kind::Vaapi(_) if !hdr => format!("format=nv12|p010|vaapi,hwupload,scale_vaapi=w={w}:h={h}:format=nv12"),
        Kind::Vaapi(_) if hdr_on_gpu && is_pq(video) => format!(
            "format=p010|vaapi,hwupload,scale_vaapi=w={w}:h={h}:format=p010,\
             tonemap_vaapi=format=nv12:t=bt709:m=bt709:p=bt709"
        ),
        Kind::Vaapi(_) => format!(
            "format=p010|vaapi,hwupload,scale_vaapi=w={w}:h={h}:format=p010,hwdownload,format=p010,\
             zscale=t=linear:npl=100,{TONEMAP},format=nv12,hwupload"
        ),
        Kind::Nvenc(_) if !hdr => format!("format=nv12|p010|cuda,hwupload,scale_cuda=w={w}:h={h}:format=nv12"),
        Kind::Nvenc(_) => format!(
            "format=p010|cuda,hwupload,scale_cuda=w={w}:h={h}:format=p010,hwdownload,format=p010,\
             zscale=t=linear:npl=100,{TONEMAP},format=nv12"
        ),
    };
    let mut args = vec!["-vf".to_string(), chain];
    let bitrate = target_bitrate((w, h));
    let vbr = [
        "-b:v".to_string(),
        bitrate.to_string(),
        "-maxrate".to_string(),
        (bitrate * 3 / 2).to_string(),
        "-bufsize".to_string(),
        (bitrate * 2).to_string(),
    ];
    match &accel.kind {
        Kind::Vaapi(_) => {
            args.extend(strings(&["-c:v", "h264_vaapi", "-profile:v", "high"]));
            if accel.low_power {
                args.extend(strings(&["-low_power", "1"]));
            }
            match accel.rate {
                RateControl::Vbr => {
                    args.extend(strings(&["-rc_mode", "VBR"]));
                    args.extend(vbr);
                }
                RateControl::Cqp => args.extend(strings(&["-rc_mode", "CQP", "-qp", "23"])),
            }
        }
        Kind::Nvenc(_) => {
            // Forced keyframes as IDR frames, where a segment can start.
            args.extend(strings(&[
                "-c:v", "h264_nvenc", "-preset", "p4", "-tune", "hq", "-profile:v", "high",
                "-forced-idr", "1", "-rc", "vbr",
            ]));
            args.extend(vbr);
        }
    }
    args
}

// ---------- Finding the GPU ----------

/// The server's GPU for transcoding, tested once.
pub struct Gpu {
    wanted: Wanted,
    device: Option<String>,
    found: OnceCell<Option<Arc<Accel>>>,
}

impl Gpu {
    pub fn new(wanted: Wanted, device: Option<String>) -> Self {
        Self {
            wanted,
            device,
            found: OnceCell::new(),
        }
    }

    /// Tests the GPU now, so the first transcode does not wait for it.
    pub fn start(self: &Arc<Self>) {
        let gpu = self.clone();
        tokio::spawn(async move {
            gpu.accel().await;
        });
    }

    async fn accel(&self) -> Option<Arc<Accel>> {
        self.found
            .get_or_init(|| detect(self.wanted, self.device.clone()))
            .await
            .clone()
    }

    /// How to transcode `video`: the GPU's best way, else the CPU.
    pub async fn transcoder(&self, video: Option<&VideoInfo>) -> Transcoder {
        Transcoder::new(self.accel().await, video)
    }
}

async fn detect(wanted: Wanted, device: Option<String>) -> Option<Arc<Accel>> {
    if wanted == Wanted::Off {
        tracing::info!("[transcode] on the CPU (SIIISHUB_HWACCEL=off)");
        return None;
    }
    let (Some(encoders), Some(filters)) = (ffmpeg_names("-encoders").await, ffmpeg_names("-filters").await) else {
        tracing::warn!("[transcode] ffmpeg does not answer: no GPU test");
        return None;
    };
    let dir = std::env::temp_dir().join(format!("siiishub-gpu-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let sample = hdr_sample(&dir, &encoders).await;
    let mut notes = Vec::new();
    let found = find(wanted, device.as_deref(), &encoders, &filters, sample.as_deref(), &mut notes).await;
    let _ = std::fs::remove_dir_all(&dir);
    match &found {
        Some(accel) => tracing::info!("[transcode] on the GPU: {}", accel.summary()),
        None if notes.is_empty() => tracing::info!(
            "[transcode] on the CPU: no GPU (in Docker, pass /dev/dri for Intel and AMD, \
             or an NVIDIA GPU: docs/WEB.md)"
        ),
        None => tracing::info!("[transcode] on the CPU: {}", notes.join("; ")),
    }
    found.map(Arc::new)
}

/// The first GPU that works: NVIDIA's, then the render nodes in order (a
/// computer with two GPUs may have the one that encodes on renderD129).
async fn find(
    wanted: Wanted,
    device: Option<&str>,
    encoders: &HashSet<String>,
    filters: &HashSet<String>,
    sample: Option<&Path>,
    notes: &mut Vec<String>,
) -> Option<Accel> {
    let index = device.and_then(|d| d.trim().parse::<u32>().ok());
    let node = device.filter(|_| index.is_none()).map(|d| PathBuf::from(d.trim()));

    let nvenc = match wanted {
        Wanted::Nvenc => true,
        Wanted::Auto => node.is_none() && nvidia_present(),
        _ => false,
    };
    if nvenc {
        let index = index.unwrap_or(0);
        if !encoders.contains("h264_nvenc") || !filters.contains("scale_cuda") {
            notes.push("this ffmpeg has no NVENC".to_string());
        } else {
            match try_gpu(Kind::Nvenc(index), filters, sample).await {
                Ok(accel) => return Some(accel),
                Err(e) => notes.push(format!("NVIDIA GPU {index}: {e}")),
            }
        }
    }

    let vaapi = match wanted {
        Wanted::Vaapi => true,
        Wanted::Auto => index.is_none(),
        _ => false,
    };
    if vaapi {
        let nodes = node.map(|n| vec![n]).unwrap_or_else(render_nodes);
        if !nodes.is_empty() && (!encoders.contains("h264_vaapi") || !filters.contains("scale_vaapi")) {
            notes.push("this ffmpeg has no VAAPI".to_string());
            return None;
        }
        for node in nodes {
            if let Err(e) = std::fs::OpenOptions::new().read(true).write(true).open(&node) {
                notes.push(if e.kind() == std::io::ErrorKind::PermissionDenied {
                    format!(
                        "{}: permission denied (the server's user must be in the device's group; \
                         in Docker: group_add)",
                        node.display()
                    )
                } else {
                    format!("{}: {e}", node.display())
                });
                continue;
            }
            match try_gpu(Kind::Vaapi(node.clone()), filters, sample).await {
                Ok(accel) => return Some(accel),
                Err(e) => notes.push(format!("{}: {e}", node.display())),
            }
        }
    }
    None
}

/// Tests a GPU: the encoder settings it takes (VAAPI drivers differ), then
/// what it does with HDR.
async fn try_gpu(kind: Kind, filters: &HashSet<String>, sample: Option<&Path>) -> Result<Accel, String> {
    let variants: &[(RateControl, bool)] = match kind {
        Kind::Vaapi(_) => &[
            (RateControl::Vbr, false),
            (RateControl::Vbr, true),
            (RateControl::Cqp, false),
            (RateControl::Cqp, true),
        ],
        Kind::Nvenc(_) => &[(RateControl::Vbr, false)],
    };
    let mut error = String::new();
    for &(rate, low_power) in variants {
        let mut accel = Accel {
            kind: kind.clone(),
            rate,
            low_power,
            hdr_on_gpu: false,
            hdr_via_cpu: false,
            name: String::new(),
        };
        let log = match test(&accel, Step::Gpu, false, sample).await {
            Ok(log) => log,
            Err(e) => {
                // The usual settings' failure says the most.
                if error.is_empty() {
                    error = e;
                }
                continue;
            }
        };
        accel.name = device_name(&kind, &log).await;
        accel.hdr_via_cpu = test(&accel, Step::GpuHdrOnCpu, true, sample).await.is_ok();
        if matches!(kind, Kind::Vaapi(_)) && filters.contains("tonemap_vaapi") {
            accel.hdr_on_gpu = test(&accel, Step::Gpu, true, sample).await.is_ok();
        }
        return Ok(accel);
    }
    Err(error)
}

/// A second of video through the pipeline of a real transcode: 1440p SDR
/// (scaled to 1080p on the way), or 720p HDR10.
async fn test(accel: &Accel, step: Step, hdr: bool, sample: Option<&Path>) -> Result<String, String> {
    let video = VideoInfo {
        codec: if hdr { "hevc" } else { "h264" }.to_string(),
        profile: String::new(),
        level: 0,
        width: if hdr { 1280 } else { 2560 },
        height: if hdr { 720 } else { 1440 },
        pix_fmt: String::new(),
        bit_depth: if hdr { 10 } else { 8 },
        color_transfer: if hdr { "smpte2084" } else { "" }.to_string(),
    };
    let transcoder = Transcoder {
        accel: Some(Arc::new(accel.clone())),
        step,
    };
    let mut args = strings(&["-hide_banner", "-nostdin", "-nostats", "-loglevel", "verbose", "-y"]);
    args.extend(transcoder.input_args());
    match (hdr, sample) {
        (true, Some(sample)) => args.extend(["-i".to_string(), sample.to_string_lossy().into_owned()]),
        (true, None) => args.extend(strings(&[
            "-f", "lavfi", "-i",
            "testsrc2=s=1280x720:r=24:d=1,format=yuv420p10le,\
             setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc",
        ])),
        (false, _) => args.extend(strings(&["-f", "lavfi", "-i", "testsrc2=s=2560x1440:r=24:d=1"])),
    }
    args.extend(transcoder.video_args(Some(&video), Some(1.0)));
    args.extend(strings(&["-an", "-f", "null", "-"]));
    run(&args).await
}

/// A second of 720p HDR10 HEVC with the mastering metadata of a real film
/// (`tonemap_vaapi` wants it), when this ffmpeg has libx265.
async fn hdr_sample(dir: &Path, encoders: &HashSet<String>) -> Option<PathBuf> {
    if !encoders.contains("libx265") {
        return None;
    }
    let path = dir.join("hdr10.mkv");
    let mut args = strings(&[
        "-hide_banner", "-nostdin", "-loglevel", "error", "-y",
        "-f", "lavfi", "-i", "testsrc2=s=1280x720:r=24:d=1",
        "-vf", "format=yuv420p10le",
        "-c:v", "libx265", "-preset", "ultrafast",
        "-x265-params",
        "log-level=error:hdr10=1:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:\
         master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1):max-cll=1000,400",
        "-color_primaries", "bt2020", "-color_trc", "smpte2084", "-colorspace", "bt2020nc",
    ]);
    args.push(path.to_string_lossy().into_owned());
    match run(&args).await {
        Ok(_) => Some(path),
        Err(e) => {
            tracing::debug!("[transcode] HDR sample: {e}");
            None
        }
    }
}

/// Runs ffmpeg; its log when it succeeds, what went wrong otherwise.
async fn run(args: &[String]) -> Result<String, String> {
    let child = Command::new(media::ffmpeg_path())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let output = match tokio::time::timeout(TEST_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => return Err(format!("no answer in {} s", TEST_TIMEOUT.as_secs())),
    };
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        return Ok(log);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = output.status.signal() {
            return Err(format!("ffmpeg crashed (signal {signal})"));
        }
    }
    // The last complaints, not the whole verbose log (nor its closing
    // statistics, "0 decode errors").
    let complaints: Vec<&str> = log
        .lines()
        .map(str::trim)
        .filter(|line| !line.contains("frames decoded") && !line.contains("Conversion failed"))
        .filter(|line| {
            ["rror", "ailed", "annot", "not supported", "nvalid", "mpossible", "No device"]
                .iter()
                .any(|word| line.contains(word))
        })
        .collect();
    let tail = complaints[complaints.len().saturating_sub(2)..].join(" / ");
    Err(if tail.is_empty() {
        format!("ffmpeg failed ({})", output.status)
    } else {
        tail.chars().take(300).collect()
    })
}

/// The encoders or the filters this ffmpeg has.
async fn ffmpeg_names(list: &str) -> Option<HashSet<String>> {
    let output = Command::new(media::ffmpeg_path())
        .args(["-hide_banner", list])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(TEST_TIMEOUT, output).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Some(
        text.lines()
            .filter_map(|line| line.split_whitespace().nth(1))
            .map(str::to_string)
            .collect(),
    )
}

/// The VAAPI driver, from the test's log; the card's name from nvidia-smi.
async fn device_name(kind: &Kind, log: &str) -> String {
    match kind {
        Kind::Vaapi(_) => log
            .lines()
            .find_map(|line| line.split_once("VAAPI driver: ").map(|(_, name)| name))
            .map(|name| name.trim().trim_end_matches('.').to_string())
            .unwrap_or_default(),
        Kind::Nvenc(index) => {
            let output = Command::new("nvidia-smi")
                .args(["--query-gpu=name", "--format=csv,noheader", "-i", &index.to_string()])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .output();
            match tokio::time::timeout(Duration::from_secs(5), output).await {
                Ok(Ok(output)) if output.status.success() => {
                    String::from_utf8_lossy(&output.stdout).trim().to_string()
                }
                _ => String::new(),
            }
        }
    }
}

/// An NVIDIA GPU's device files (WSL's GPU on Windows' Linux).
fn nvidia_present() -> bool {
    cfg!(windows) || ["/dev/nvidiactl", "/dev/nvidia0", "/dev/dxg"].iter().any(|p| Path::new(p).exists())
}

/// `/dev/dri/renderD*`, in order.
fn render_nodes() -> Vec<PathBuf> {
    let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .map(|dir| {
            dir.filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("renderD"))
                })
                .collect()
        })
        .unwrap_or_default();
    nodes.sort();
    nodes
}
