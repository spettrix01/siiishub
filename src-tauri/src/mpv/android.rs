//! Android player backend: libmpv lives in the Kotlin plugin
//! (`plugins/android-player`), rendering into a SurfaceView under the
//! transparent webview. This type keeps the same surface as the desktop
//! backends and forwards everything to the plugin; mpv events come back
//! through a channel and are re-emitted as `mpv://event`, so the web player
//! UI runs unchanged.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use serde_json::Value;
use siiishub_android_player::AndroidMpv;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager};

pub struct Mpv {
    app: AppHandle,
    observed: Mutex<Vec<String>>,
}

impl Mpv {
    pub fn new(app: AppHandle) -> Result<Self> {
        if app.try_state::<AndroidMpv<tauri::Wry>>().is_none() {
            return Err(anyhow!("android-player plugin not registered"));
        }
        Ok(Self {
            app,
            observed: Mutex::new(Vec::new()),
        })
    }

    fn plugin(&self) -> tauri::State<'_, AndroidMpv<tauri::Wry>> {
        self.app.state::<AndroidMpv<tauri::Wry>>()
    }

    pub fn start_event_pump(self: &Arc<Self>, app: AppHandle) {
        let emitter = app.clone();
        let channel = Channel::<Value>::new(move |body: InvokeResponseBody| {
            let event: Value = body.deserialize()?;
            let _ = emitter.emit("mpv://event", event);
            Ok(())
        });
        // Plugin calls block until Kotlin answers: keep them off the main
        // thread, which is where `setup` runs.
        tauri::async_runtime::spawn_blocking(move || {
            let plugin = app.state::<AndroidMpv<tauri::Wry>>();
            match plugin.start(channel) {
                Ok(()) => tracing::info!("[mpv] android event channel registered"),
                Err(e) => tracing::error!("[mpv] android event channel failed: {e}"),
            }
        });
    }

    pub fn loadfile(&self, url: &str, options: Option<&str>) -> Result<()> {
        if let Some(opts) = options {
            tracing::warn!("[mpv] loadfile options ignored on android: {opts}");
        }
        let args = vec![
            "loadfile".to_string(),
            url.to_string(),
            "replace".to_string(),
        ];
        self.plugin().command(&args).map_err(|e| anyhow!(e))
    }

    pub fn command(&self, args: &[String]) -> Result<()> {
        if args.is_empty() {
            return Err(anyhow!("empty mpv command"));
        }
        self.plugin().command(args).map_err(|e| anyhow!(e))
    }

    pub fn set_property(&self, name: &str, value: Value) -> Result<()> {
        self.plugin()
            .set_property(name, &value)
            .map_err(|e| anyhow!(e))
    }

    pub fn get_property(&self, name: &str) -> Result<Value> {
        let raw = self
            .plugin()
            .get_property(name)
            .map_err(|e| anyhow!(e))?
            .ok_or_else(|| anyhow!("property {name} unavailable"))?;
        Ok(match raw.as_str() {
            "yes" => Value::Bool(true),
            "no" => Value::Bool(false),
            _ => raw
                .parse::<f64>()
                .ok()
                .map(|n| serde_json::json!(n))
                .unwrap_or(Value::String(raw)),
        })
    }

    pub fn observe(&self, name: &str) -> Result<()> {
        {
            let mut obs = self.observed.lock();
            if obs.iter().any(|n| n == name) {
                return Ok(());
            }
            obs.push(name.to_string());
        }
        self.plugin().observe(name).map_err(|e| anyhow!(e))
    }

    pub fn set_geometry(&self, x: i32, y: i32, w: i32, h: i32) -> Result<()> {
        self.plugin()
            .set_geometry(x, y, w, h)
            .map_err(|e| anyhow!(e))
    }

    pub fn set_visible(&self, visible: bool) -> Result<()> {
        self.plugin().set_visible(visible).map_err(|e| anyhow!(e))
    }
}
