use std::sync::Arc;

use anyhow::{anyhow, Result};
use libmpv2::Mpv as RawMpv;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;

/// CA bundle for HTTPS streams. Windows: the Mozilla store embedded in the
/// binary (resources/cacert.pem), written to the app data folder at startup.
/// Linux: the distribution bundle, when one of the usual paths exists.
fn ca_bundle_path() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        const MOZILLA_CA_BUNDLE: &[u8] = include_bytes!("../../resources/cacert.pem");
        let dir = crate::util::app_data_dir()?;
        let path = dir.join("cacert.pem");
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, MOZILLA_CA_BUNDLE)) {
            Ok(()) => Some(path),
            Err(e) => {
                tracing::warn!("[mpv] writing {}: {e}", path.display());
                None
            }
        }
    }
    #[cfg(not(windows))]
    {
        [
            "/etc/ssl/certs/ca-certificates.crt",
            "/etc/pki/tls/certs/ca-bundle.crt",
            "/etc/ssl/ca-bundle.pem",
            "/etc/ssl/cert.pem",
        ]
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
    }
}

pub struct Mpv {
    inner: Arc<MpvInner>,
}

struct MpvInner {
    raw: Mutex<RawMpv>,
    observed: Mutex<Vec<String>>,
}

impl Mpv {
    pub fn new(_parent_hwnd: Option<isize>) -> Result<Self> {

        let raw = RawMpv::with_initializer(|init| {
            init.set_property("vo", "libmpv")?;
            init.set_property("input-default-bindings", "no")?;
            init.set_property("input-vo-keyboard", "no")?;
            init.set_property("osc", "no")?;
            init.set_property("osd-level", "0")?;
            init.set_property("config", "no")?;
            init.set_property("idle", "yes")?;
            init.set_property("force-window", "no")?;
            init.set_property("keep-open", "yes")?;
            init.set_property("hwdec", "auto-safe")?;
            init.set_property("msg-level", "all=warn,cplayer=info")?;
            // HTTPS streams are verified: the libmpv builds do not do it by
            // default. Without a CA bundle the Windows build, whose FFmpeg
            // uses OpenSSL without a store of its own, could open no HTTPS
            // stream at all, so it keeps verification off in that case; the
            // Linux TLS libraries fall back to the system trust store.
            match ca_bundle_path() {
                Some(ca) => {
                    init.set_property("tls-verify", "yes")?;
                    init.set_property("tls-ca-file", ca.to_string_lossy().into_owned())?;
                }
                None if cfg!(windows) => {
                    tracing::warn!("[mpv] no CA bundle: HTTPS certificates are not verified");
                    init.set_property("tls-verify", "no")?;
                }
                None => init.set_property("tls-verify", "yes")?,
            }
            // Debug hook: expose the mpv JSON IPC endpoint (unix socket on
            // Linux, named pipe on Windows) so playback can be driven and
            // inspected from outside the app, e.g. for automated tests.
            if let Ok(path) = std::env::var("SIIISHUB_MPV_IPC") {
                if !path.trim().is_empty() {
                    init.set_property("input-ipc-server", path)?;
                }
            }
            Ok(())
        })
        .map_err(|e| anyhow!("creating mpv handle: {e}"))?;

        Ok(Self {
            inner: Arc::new(MpvInner {
                raw: Mutex::new(raw),
                observed: Mutex::new(Vec::new()),
            }),
        })
    }

    pub fn start_event_pump(self: &Arc<Self>, app: AppHandle) {
        use tauri::Emitter;

        let raw_addr: usize = {
            let raw = self.inner.raw.lock();
            raw.ctx.as_ptr() as usize
        };
        std::thread::spawn(move || {
            let ctx = raw_addr as *mut libmpv2_sys::mpv_handle;
            tracing::info!("[mpv] event pump started");

            unsafe {
                let level = std::ffi::CString::new("info").expect("'info' is a valid C string");
                let _ = libmpv2_sys::mpv_request_log_messages(ctx, level.as_ptr());
            }

            // With vo=libmpv the host app must inhibit display sleep itself;
            // the pump tracks "file loaded + not paused" via its own observer.
            const KEEPAWAKE_OBS_ID: u64 = 1_000_001;
            unsafe {
                let name = std::ffi::CString::new("pause").expect("'pause' is a valid C string");
                let _ = libmpv2_sys::mpv_observe_property(
                    ctx,
                    KEEPAWAKE_OBS_ID,
                    name.as_ptr(),
                    libmpv2_sys::mpv_format_MPV_FORMAT_FLAG,
                );
            }
            let mut ka_paused = true;
            let mut ka_has_file = false;
            let mut ka_awake = false;
            loop {

                let event_ptr = unsafe { libmpv2_sys::mpv_wait_event(ctx, 0.5) };
                if event_ptr.is_null() {
                    continue;
                }
                let event = unsafe { *event_ptr };
                let evt = match event.event_id {
                    libmpv2_sys::mpv_event_id_MPV_EVENT_PROPERTY_CHANGE => {
                        let prop = event.data as *mut libmpv2_sys::mpv_event_property;
                        if prop.is_null() {
                            continue;
                        }
                        let name = unsafe { std::ffi::CStr::from_ptr((*prop).name) }
                            .to_string_lossy()
                            .into_owned();
                        let value = unsafe { property_data_to_json(&*prop) };

                        if name == "pause" {
                            if let Some(b) = value.as_bool() {
                                ka_paused = b;
                            }
                        }
                        if name == "track-list" {
                            log_track_list(&value);
                        }
                        if event.reply_userdata == KEEPAWAKE_OBS_ID {
                            None
                        } else {
                            Some(MpvFrontEvent::PropertyChanged { name, value })
                        }
                    }
                    libmpv2_sys::mpv_event_id_MPV_EVENT_FILE_LOADED => {
                        ka_has_file = true;
                        Some(MpvFrontEvent::FileLoaded)
                    }
                    libmpv2_sys::mpv_event_id_MPV_EVENT_START_FILE => {
                        ka_has_file = true;
                        Some(MpvFrontEvent::StartFile)
                    }
                    libmpv2_sys::mpv_event_id_MPV_EVENT_END_FILE => {
                        ka_has_file = false;
                        let ef = event.data as *mut libmpv2_sys::mpv_event_end_file;
                        let reason = if ef.is_null() {
                            String::from("unknown")
                        } else {
                            match unsafe { (*ef).reason } {
                                libmpv2_sys::mpv_end_file_reason_MPV_END_FILE_REASON_EOF => "eof",
                                libmpv2_sys::mpv_end_file_reason_MPV_END_FILE_REASON_STOP => "stop",
                                libmpv2_sys::mpv_end_file_reason_MPV_END_FILE_REASON_QUIT => "quit",
                                libmpv2_sys::mpv_end_file_reason_MPV_END_FILE_REASON_ERROR => "error",
                                libmpv2_sys::mpv_end_file_reason_MPV_END_FILE_REASON_REDIRECT => "redirect",
                                _ => "unknown",
                            }
                            .to_string()
                        };
                        Some(MpvFrontEvent::EndFile { reason })
                    }
                    libmpv2_sys::mpv_event_id_MPV_EVENT_PLAYBACK_RESTART => {
                        Some(MpvFrontEvent::PlaybackRestart)
                    }
                    libmpv2_sys::mpv_event_id_MPV_EVENT_SEEK => Some(MpvFrontEvent::Seek),
                    libmpv2_sys::mpv_event_id_MPV_EVENT_IDLE => {
                        ka_has_file = false;
                        Some(MpvFrontEvent::Idle)
                    }
                    libmpv2_sys::mpv_event_id_MPV_EVENT_SHUTDOWN => {
                        crate::power::keep_display_awake(false);
                        let _ = app.emit("mpv://event", &MpvFrontEvent::Shutdown);
                        tracing::info!("[mpv] event pump exiting (Shutdown received)");
                        break;
                    }
                    libmpv2_sys::mpv_event_id_MPV_EVENT_NONE => None,
                    libmpv2_sys::mpv_event_id_MPV_EVENT_LOG_MESSAGE => {
                        let msg = event.data as *mut libmpv2_sys::mpv_event_log_message;
                        if !msg.is_null() {
                            let prefix = unsafe {
                                std::ffi::CStr::from_ptr((*msg).prefix).to_string_lossy().into_owned()
                            };
                            let text = unsafe {
                                std::ffi::CStr::from_ptr((*msg).text).to_string_lossy().into_owned()
                            };
                            let lvl = unsafe {
                                std::ffi::CStr::from_ptr((*msg).level).to_string_lossy().into_owned()
                            };
                            crate::util::app_log_append(
                                "mpv.log",
                                &format!("[{lvl}] {prefix}: {}", text.trim_end()),
                            );
                        }

                        None
                    }
                    other => Some(MpvFrontEvent::Other {
                        name: format!("event_{other}"),
                    }),
                };

                let want_awake = ka_has_file && !ka_paused;
                if want_awake != ka_awake {
                    ka_awake = want_awake;
                    crate::power::keep_display_awake(want_awake);
                }

                if let Some(evt) = evt {
                    let _ = app.emit("mpv://event", &evt);
                }
            }
        });
    }

    pub fn loadfile(&self, url: &str, options: Option<&str>) -> Result<()> {
        crate::util::app_log_append(
            "mpv.log",
            &format!("loadfile url_len={} options={:?}", url.len(), options),
        );

        let preview = if url.len() > 240 {
            format!("{}…{}", &url[..160], &url[url.len() - 60..])
        } else {
            url.to_string()
        };
        crate::util::app_log_append("mpv.log", &format!("loadfile preview: {preview}"));
        let mpv = self.inner.raw.lock();

        let args: Vec<&str> = if let Some(opts) = options {
            vec![url, "replace", "-1", opts]
        } else {
            vec![url, "replace"]
        };
        mpv.command("loadfile", &args)
            .map_err(|e| anyhow!(e.to_string()))?;
        Ok(())
    }

    pub fn command(&self, args: &[String]) -> Result<()> {
        if args.is_empty() {
            return Err(anyhow!("empty mpv command"));
        }
        let mpv = self.inner.raw.lock();
        let head = args[0].as_str();
        let tail: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();
        mpv.command(head, &tail).map_err(|e| anyhow!(e.to_string()))?;
        Ok(())
    }

    pub fn set_property(&self, name: &str, value: Value) -> Result<()> {
        let mpv = self.inner.raw.lock();
        match value {
            Value::Bool(b) => mpv.set_property(name, b).map_err(|e| anyhow!(e.to_string()))?,
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    mpv.set_property(name, i).map_err(|e| anyhow!(e.to_string()))?;
                } else if let Some(f) = n.as_f64() {
                    mpv.set_property(name, f).map_err(|e| anyhow!(e.to_string()))?;
                }
            }
            Value::String(s) => mpv.set_property(name, s).map_err(|e| anyhow!(e.to_string()))?,
            Value::Null => {}
            other => mpv
                .set_property(name, other.to_string())
                .map_err(|e| anyhow!(e.to_string()))?,
        }
        Ok(())
    }

    pub fn get_property(&self, name: &str) -> Result<Value> {
        let mpv = self.inner.raw.lock();
        if let Ok(s) = mpv.get_property::<String>(name) {
            return Ok(Value::String(s));
        }
        if let Ok(n) = mpv.get_property::<f64>(name) {
            return Ok(serde_json::json!(n));
        }
        if let Ok(b) = mpv.get_property::<bool>(name) {
            return Ok(Value::Bool(b));
        }
        Err(anyhow!("property {name} unavailable"))
    }

    pub fn raw_handle(&self) -> *mut std::ffi::c_void {
        let raw = self.inner.raw.lock();
        raw.ctx.as_ptr() as *mut std::ffi::c_void
    }

    pub fn observe(&self, name: &str) -> Result<()> {

        let id = {
            let mut obs = self.inner.observed.lock();
            if obs.iter().any(|n| n == name) {
                return Ok(());
            }
            let id = (obs.len() as u64) + 1;
            obs.push(name.to_string());
            id
        };
        let cname = std::ffi::CString::new(name)
            .map_err(|e| anyhow!("invalid property name: {e}"))?;
        let raw = self.inner.raw.lock();
        let err = unsafe {
            libmpv2_sys::mpv_observe_property(
                raw.ctx.as_ptr(),
                id,
                cname.as_ptr(),
                libmpv2_sys::mpv_format_MPV_FORMAT_NODE,
            )
        };
        if err < 0 {
            return Err(anyhow!("mpv_observe_property({name}) failed: {err}"));
        }
        Ok(())
    }

    pub fn set_geometry(&self, _x: i32, _y: i32, _w: i32, _h: i32) -> Result<()> {
        Ok(())
    }

    pub fn set_visible(&self, _visible: bool) -> Result<()> {
        Ok(())
    }
}

unsafe fn property_data_to_json(prop: &libmpv2_sys::mpv_event_property) -> Value {
    if prop.data.is_null() {
        return Value::Null;
    }
    match prop.format {
        libmpv2_sys::mpv_format_MPV_FORMAT_NONE => Value::Null,
        libmpv2_sys::mpv_format_MPV_FORMAT_FLAG => {
            Value::Bool(unsafe { *(prop.data as *mut std::ffi::c_int) } != 0)
        }
        libmpv2_sys::mpv_format_MPV_FORMAT_INT64 => {
            serde_json::json!(unsafe { *(prop.data as *mut i64) })
        }
        libmpv2_sys::mpv_format_MPV_FORMAT_DOUBLE => {
            serde_json::json!(unsafe { *(prop.data as *mut f64) })
        }
        libmpv2_sys::mpv_format_MPV_FORMAT_STRING
        | libmpv2_sys::mpv_format_MPV_FORMAT_OSD_STRING => {
            let cstr = unsafe { *(prop.data as *mut *mut std::ffi::c_char) };
            if cstr.is_null() {
                Value::Null
            } else {
                Value::String(
                    unsafe { std::ffi::CStr::from_ptr(cstr) }
                        .to_string_lossy()
                        .into_owned(),
                )
            }
        }
        libmpv2_sys::mpv_format_MPV_FORMAT_NODE => {
            let node = unsafe { &*(prop.data as *mut libmpv2_sys::mpv_node) };
            mpv_node_to_json(node)
        }
        _ => Value::Null,
    }
}

unsafe fn mpv_node_to_json(node: &libmpv2_sys::mpv_node) -> Value {
    match node.format {
        libmpv2_sys::mpv_format_MPV_FORMAT_NONE => Value::Null,
        libmpv2_sys::mpv_format_MPV_FORMAT_FLAG => {
            Value::Bool(unsafe { node.u.flag } != 0)
        }
        libmpv2_sys::mpv_format_MPV_FORMAT_INT64 => {
            serde_json::json!(unsafe { node.u.int64 })
        }
        libmpv2_sys::mpv_format_MPV_FORMAT_DOUBLE => {
            serde_json::json!(unsafe { node.u.double_ })
        }
        libmpv2_sys::mpv_format_MPV_FORMAT_STRING => {
            let s = unsafe { node.u.string };
            if s.is_null() {
                Value::Null
            } else {
                Value::String(
                    unsafe { std::ffi::CStr::from_ptr(s) }
                        .to_string_lossy()
                        .into_owned(),
                )
            }
        }
        libmpv2_sys::mpv_format_MPV_FORMAT_NODE_ARRAY => {
            let list = unsafe { &*node.u.list };
            let n = list.num as usize;
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let child = unsafe { &*list.values.add(i) };
                out.push(unsafe { mpv_node_to_json(child) });
            }
            Value::Array(out)
        }
        libmpv2_sys::mpv_format_MPV_FORMAT_NODE_MAP => {
            let list = unsafe { &*node.u.list };
            let n = list.num as usize;
            let mut map = serde_json::Map::with_capacity(n);
            for i in 0..n {
                let key_ptr = unsafe { *list.keys.add(i) };
                let key = if key_ptr.is_null() {
                    format!("_{i}")
                } else {
                    unsafe { std::ffi::CStr::from_ptr(key_ptr) }
                        .to_string_lossy()
                        .into_owned()
                };
                let child = unsafe { &*list.values.add(i) };
                map.insert(key, unsafe { mpv_node_to_json(child) });
            }
            Value::Object(map)
        }
        _ => Value::Null,
    }
}

fn log_track_list(value: &Value) {
    let arr = match value.as_array() {
        Some(a) => a,
        None => {
            crate::util::app_log_append("mpv.log", "track-list: <not array>");
            return;
        }
    };
    let mut audio = 0usize;
    let mut sub = 0usize;
    let mut video = 0usize;
    let mut other = 0usize;
    let mut samples: Vec<String> = Vec::new();
    for t in arr {
        let kind = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "audio" => audio += 1,
            "sub" => sub += 1,
            "video" => video += 1,
            _ => other += 1,
        }
        if samples.len() < 8 {
            let id = t.get("id").map(|v| v.to_string()).unwrap_or_default();
            let lang = t
                .get("lang")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let title = t
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            samples.push(format!("[{kind} id={id} lang={lang} title={title:?}]"));
        }
    }
    crate::util::app_log_append(
        "mpv.log",
        &format!(
            "track-list: total={} video={} audio={} sub={} other={} | {}",
            arr.len(),
            video,
            audio,
            sub,
            other,
            samples.join(" "),
        ),
    );
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MpvFrontEvent {
    PropertyChanged { name: String, value: Value },
    PlaybackRestart,
    StartFile,
    EndFile { reason: String },
    FileLoaded,
    Seek,
    Idle,
    Shutdown,
    Other { name: String },
}
