use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
use windows::Win32::{
    Foundation::{BOOL, HANDLE, HWND, LPARAM, TRUE},
    System::Pipes::PeekNamedPipe,
    UI::WindowsAndMessaging::{
        EnumChildWindows, GetClassNameW, IsWindow, SetWindowPos, HWND_BOTTOM, HWND_TOP,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    },
};

const PIPE_NAME: &str = r"\\.\pipe\siiishub-mpv";

pub struct Mpv {
    inner: Arc<Mutex<MpvInner>>,
}

struct MpvInner {

    proc: Option<Child>,

    parent_hwnd: Option<isize>,

    mpv_hwnd: Option<isize>,

    app_handle: Option<AppHandle>,

    tx: Option<std::sync::mpsc::Sender<Vec<u8>>>,

    tx_generation: u64,

    observed: Vec<String>,

    next_request_id: u64,
}

impl Mpv {
    pub fn new(parent_hwnd: Option<isize>) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Mutex::new(MpvInner {
                proc: None,
                parent_hwnd,
                mpv_hwnd: None,
                app_handle: None,
                tx: None,
                tx_generation: 0,
                observed: Vec::new(),
                next_request_id: 1,
            })),
        })
    }

    pub fn start_event_pump(self: &Arc<Self>, app: AppHandle) {
        self.inner.lock().app_handle = Some(app);
    }

    fn ensure_communicator(&self) {
        let mut g = self.inner.lock();
        if g.tx.is_some() {
            return;
        }
        let app = g.app_handle.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        g.tx = Some(tx.clone());
        let inner_arc = Arc::clone(&self.inner);

        g.tx_generation = g.tx_generation.wrapping_add(1);
        let my_generation = g.tx_generation;

        let observed_snapshot: Vec<String> = g.observed.clone();
        for name in &observed_snapshot {
            let id = g.next_request_id;
            g.next_request_id += 1;
            let line = format!(
                "{}\n",
                json!({"command": ["observe_property", id, name]})
            );
            let _ = tx.send(line.into_bytes());
        }
        drop(g);
        drop(tx);

        std::thread::spawn(move || {

            let mut file: Option<std::fs::File> = None;
            for _ in 0..60 {
                match std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(PIPE_NAME)
                {
                    Ok(f) => {
                        file = Some(f);
                        break;
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(150)),
                }
            }
            let Some(mut file) = file else {
                tracing::error!("[mpv-stub] ipc pipe never appeared");
                let mut g = inner_arc.lock();
                if g.tx_generation == my_generation {
                    g.tx = None;
                }
                return;
            };
            tracing::info!("[mpv-stub] ipc connected (duplex), gen={my_generation}");

            #[cfg(windows)]
            {
                use std::io::{Read, Write};
                use std::os::windows::io::AsRawHandle;

                let mut leftover: Vec<u8> = Vec::with_capacity(8192);
                let mut read_buf = [0u8; 8192];

                'pump: loop {

                    loop {
                        match rx.try_recv() {
                            Ok(buf) => {
                                if let Err(e) = file.write_all(&buf) {
                                    tracing::error!(
                                        "[mpv-stub] ipc write failed: {e}"
                                    );
                                    break 'pump;
                                }
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => break,
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                break 'pump;
                            }
                        }
                    }

                    let raw = file.as_raw_handle();
                    let mut bytes_avail: u32 = 0;
                    let peek_ok = unsafe {
                        PeekNamedPipe(
                            HANDLE(raw),
                            None,
                            0,
                            None,
                            Some(&mut bytes_avail as *mut _),
                            None,
                        )
                    };
                    if peek_ok.is_err() {

                        tracing::warn!("[mpv-stub] PeekNamedPipe failed; ipc pump exiting");
                        break 'pump;
                    }
                    if bytes_avail > 0 {
                        let want = (bytes_avail as usize).min(read_buf.len());
                        match file.read(&mut read_buf[..want]) {
                            Ok(0) => break 'pump,
                            Ok(n) => leftover.extend_from_slice(&read_buf[..n]),
                            Err(e) => {
                                tracing::error!("[mpv-stub] ipc read failed: {e}");
                                break 'pump;
                            }
                        }

                        while let Some(pos) =
                            leftover.iter().position(|&b| b == b'\n')
                        {
                            let line: Vec<u8> = leftover.drain(..=pos).collect();

                            let end = if line.len() >= 2
                                && line[line.len() - 2] == b'\r'
                            {
                                line.len() - 2
                            } else {
                                line.len() - 1
                            };
                            let Ok(s) = std::str::from_utf8(&line[..end]) else {
                                continue;
                            };
                            if s.is_empty() {
                                continue;
                            }
                            let Ok(v): Result<Value, _> =
                                serde_json::from_str(s)
                            else {
                                continue;
                            };
                            let Some(event_name) =
                                v.get("event").and_then(|e| e.as_str())
                            else {
                                continue;
                            };
                            let evt = match event_name {
                                "property-change" => {
                                    MpvFrontEvent::PropertyChanged {
                                        name: v
                                            .get("name")
                                            .and_then(|n| n.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        value: v
                                            .get("data")
                                            .cloned()
                                            .unwrap_or(Value::Null),
                                    }
                                }
                                "file-loaded" => MpvFrontEvent::FileLoaded,
                                "playback-restart" => {
                                    MpvFrontEvent::PlaybackRestart
                                }
                                "start-file" => MpvFrontEvent::StartFile,
                                "end-file" => MpvFrontEvent::EndFile {
                                    reason: v
                                        .get("reason")
                                        .and_then(|r| r.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                },
                                "seek" => MpvFrontEvent::Seek,
                                "idle" => MpvFrontEvent::Idle,
                                "shutdown" => MpvFrontEvent::Shutdown,
                                other => MpvFrontEvent::Other {
                                    name: other.to_string(),
                                },
                            };
                            if let Some(ref a) = app {
                                let _ = a.emit("mpv://event", &evt);
                            }
                        }
                    }

                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
            #[cfg(not(windows))]
            {
                let _ = (rx, file, app);
            }

            tracing::warn!("[mpv-stub] ipc pump exiting (gen={my_generation})");
            let mut g = inner_arc.lock();
            if g.tx_generation == my_generation {
                g.tx = None;
            }
        });
    }

    fn ensure_running(&self) -> Result<()> {
        let mut g = self.inner.lock();
        if g.proc.as_mut().map(|c| c.try_wait().ok().flatten().is_none()).unwrap_or(false) {
            return Ok(());
        }
        let bin = mpv_path()
            .ok_or_else(|| anyhow!("mpv.exe non trovato: aggiungilo a binaries/ o al PATH"))?;
        let log_file = std::env::temp_dir().join("siiishub-mpv.log");
        tracing::info!(
            "[mpv-stub] spawning {} → log {} parent_hwnd={:?}",
            bin.display(),
            log_file.display(),
            g.parent_hwnd
        );

        let mut args: Vec<String> = vec![
            "--idle=yes".into(),
            "--keep-open=yes".into(),
            "--force-window=yes".into(),
            "--no-config".into(),
            "--osc=no".into(),
            "--osd-level=0".into(),
            "--input-default-bindings=no".into(),
            "--input-vo-keyboard=no".into(),

            "--auto-window-resize=no".into(),
            "--no-window-dragging".into(),
            "--title=siiishub player".into(),
            "--msg-level=all=v".into(),
            format!("--log-file={}", log_file.display()),
            format!("--input-ipc-server={PIPE_NAME}"),
        ];
        if let Some(hwnd) = g.parent_hwnd {
            args.push(format!("--wid={}", hwnd));
        }

        let child = Command::new(&bin)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("spawning {}", bin.display()))?;
        g.proc = Some(child);
        g.mpv_hwnd = None;

        g.tx = None;
        Ok(())
    }

    fn send_ipc(&self, payload: Value) -> Result<()> {
        self.ensure_running()?;
        self.ensure_communicator();
        let line = format!("{}\n", payload.to_string());
        tracing::debug!("[mpv-stub] → {}", payload);

        let mut tx_opt = None;
        for _ in 0..60 {
            tx_opt = self.inner.lock().tx.clone();
            if tx_opt.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        let tx = tx_opt.ok_or_else(|| anyhow!("ipc non disponibile"))?;
        tx.send(line.into_bytes())
            .map_err(|e| anyhow!("ipc send fallito: {e}"))?;
        Ok(())
    }

    pub fn loadfile(&self, url: &str, options: Option<&str>) -> Result<()> {
        if let Some(opts) = options {
            self.send_ipc(json!({"command": ["loadfile", url, "replace", opts]}))
        } else {
            self.send_ipc(json!({"command": ["loadfile", url, "replace"]}))
        }
    }

    pub fn command(&self, args: &[String]) -> Result<()> {
        if args.is_empty() {
            return Err(anyhow!("empty mpv command"));
        }
        let mut cmd: Vec<Value> = Vec::with_capacity(args.len());
        for a in args {
            cmd.push(Value::String(a.clone()));
        }
        self.send_ipc(json!({"command": cmd}))
    }

    pub fn set_property(&self, name: &str, value: Value) -> Result<()> {
        self.send_ipc(json!({"command": ["set_property", name, value]}))
    }

    pub fn get_property(&self, _name: &str) -> Result<Value> {

        Ok(Value::Null)
    }

    pub fn observe(&self, name: &str) -> Result<()> {

        let (already_observed, id) = {
            let mut g = self.inner.lock();
            let already = g.observed.iter().any(|n| n == name);
            if !already {
                g.observed.push(name.to_string());
            }
            let id = g.next_request_id;
            g.next_request_id += 1;
            (already, id)
        };
        if already_observed {
            return Ok(());
        }
        self.send_ipc(json!({"command": ["observe_property", id, name]}))
    }

    pub fn set_geometry(&self, x: i32, y: i32, w: i32, h: i32) -> Result<()> {
        #[cfg(not(windows))]
        {
            let _ = (x, y, w, h);
            return Ok(());
        }
        #[cfg(windows)]
        {
            self.ensure_running()?;
            let hwnd = self.find_mpv_child()?;
            tracing::debug!("[mpv-stub] geometry → x={x} y={y} w={w} h={h}");
            unsafe {
                let _ = SetWindowPos(
                    HWND(hwnd as *mut _),
                    Some(HWND_TOP),
                    x,
                    y,
                    w,
                    h,
                    SWP_NOACTIVATE,
                );
            }
            Ok(())
        }
    }

    pub fn set_visible(&self, visible: bool) -> Result<()> {
        #[cfg(not(windows))]
        {
            let _ = visible;
            return Ok(());
        }
        #[cfg(windows)]
        {
            self.ensure_running()?;
            let hwnd = self.find_mpv_child()?;
            tracing::debug!("[mpv-stub] visible={visible}");
            unsafe {
                let _ = SetWindowPos(
                    HWND(hwnd as *mut _),
                    Some(if visible { HWND_TOP } else { HWND_BOTTOM }),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
            }
            Ok(())
        }
    }

    #[cfg(windows)]
    fn find_mpv_child(&self) -> Result<isize> {

        {
            let g = self.inner.lock();
            if let Some(h) = g.mpv_hwnd {
                let valid = unsafe { IsWindow(Some(HWND(h as *mut _))).as_bool() };
                if valid {
                    return Ok(h);
                }
            }
        }
        let parent_hwnd = self
            .inner
            .lock()
            .parent_hwnd
            .ok_or_else(|| anyhow!("Tauri main HWND non disponibile"))?;
        for _ in 0..30 {
            let mut found: isize = 0;
            unsafe {
                let _ = EnumChildWindows(
                    Some(HWND(parent_hwnd as *mut _)),
                    Some(enum_child_proc),
                    LPARAM(&mut found as *mut _ as isize),
                );
            }
            if found != 0 {
                self.inner.lock().mpv_hwnd = Some(found);
                tracing::info!("[mpv-stub] child HWND found: 0x{:x}", found);
                return Ok(found);
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        Err(anyhow!("mpv child HWND non trovato dopo 30 retry"))
    }
}

#[cfg(windows)]
unsafe extern "system" fn enum_child_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let mut buf = [0u16; 64];
    let n = GetClassNameW(hwnd, &mut buf);
    if n > 0 {
        let class = String::from_utf16_lossy(&buf[..n as usize]);

        if class == "mpv" {
            let result_ptr = lparam.0 as *mut isize;
            *result_ptr = hwnd.0 as isize;
            return BOOL(0);
        }
    }
    TRUE
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

fn mpv_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for sub in &["binaries", ".."] {
                let p = dir.join(sub).join("mpv.exe");
                if p.exists() {
                    return Some(p);
                }
            }
            let p = dir.join("mpv.exe");
            if p.exists() {
                return Some(p);
            }
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(';') {
            let p = PathBuf::from(dir).join("mpv.exe");
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}
