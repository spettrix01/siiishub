//! Tauri plugin that embeds libmpv on Android.
//!
//! The Kotlin side (`android/src/main/java/MpvPlugin.kt`) owns a libmpv
//! instance (prebuilt `dev.jdtech.mpv:libmpv`) rendering into a SurfaceView
//! placed under the transparent webview, the same layering the desktop
//! builds use. The Rust side (`src/mpv/android.rs` in the app) forwards the
//! usual mpv commands and property calls here and receives mpv events back
//! through a channel. On desktop the crate still compiles (so it can be
//! type-checked) but every call answers with an error.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::Runtime;

#[derive(Serialize)]
struct StartArgs {
    channel: Channel<Value>,
}

#[derive(Serialize)]
struct CommandArgs<'a> {
    args: &'a [String],
}

#[derive(Serialize)]
struct SetPropertyArgs<'a> {
    name: &'a str,
    value: &'a Value,
}

#[derive(Serialize)]
struct NameArgs<'a> {
    name: &'a str,
}

#[derive(Serialize)]
struct GeometryArgs {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Serialize)]
struct VisibleArgs {
    visible: bool,
}

#[derive(Serialize)]
struct OpenFolderArgs<'a> {
    path: &'a str,
}

#[derive(Serialize)]
struct OpenUrlArgs<'a> {
    url: &'a str,
}

#[derive(Deserialize)]
struct ValueResponse {
    value: Option<String>,
}

/// Handle to the Kotlin plugin, managed in the app state on Android.
#[cfg(mobile)]
pub struct AndroidMpv<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Desktop placeholder: never registered, keeps the crate compiling.
#[cfg(not(mobile))]
pub struct AndroidMpv<R: Runtime>(std::marker::PhantomData<R>);

impl<R: Runtime> AndroidMpv<R> {
    fn call<T: DeserializeOwned>(&self, command: &str, payload: impl Serialize) -> Result<T, String> {
        #[cfg(mobile)]
        {
            self.0
                .run_mobile_plugin(command, payload)
                .map_err(|e| e.to_string())
        }
        #[cfg(not(mobile))]
        {
            let _ = (command, payload);
            Err("android-player plugin is only available on Android".to_string())
        }
    }

    /// Registers the channel that receives mpv events as JSON objects
    /// (`{"kind": "property_changed", "name": ..., "value": ...}` and the
    /// other `MpvFrontEvent` variants).
    pub fn start(&self, channel: Channel<Value>) -> Result<(), String> {
        self.call::<()>("start", StartArgs { channel })
    }

    pub fn command(&self, args: &[String]) -> Result<(), String> {
        self.call::<()>("command", CommandArgs { args })
    }

    pub fn set_property(&self, name: &str, value: &Value) -> Result<(), String> {
        self.call::<()>("setProperty", SetPropertyArgs { name, value })
    }

    /// The property as mpv formats it to a string, `None` when unavailable.
    pub fn get_property(&self, name: &str) -> Result<Option<String>, String> {
        self.call::<ValueResponse>("getProperty", NameArgs { name })
            .map(|r| r.value)
    }

    pub fn observe(&self, name: &str) -> Result<(), String> {
        self.call::<()>("observe", NameArgs { name })
    }

    /// Video rectangle in physical pixels, relative to the window.
    pub fn set_geometry(&self, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        self.call::<()>("setGeometry", GeometryArgs { x, y, w, h })
    }

    pub fn set_visible(&self, visible: bool) -> Result<(), String> {
        self.call::<()>("setVisible", VisibleArgs { visible })
    }

    /// Landscape, immersive player mode, independent of the video surface.
    pub fn set_player_mode(&self, on: bool) -> Result<(), String> {
        self.call::<()>("playerMode", VisibleArgs { visible: on })
    }

    /// Shows a folder of the shared storage in the phone's file manager.
    pub fn open_folder(&self, path: &str) -> Result<(), String> {
        self.call::<()>("openFolder", OpenFolderArgs { path })
    }

    /// Opens a web page with the app the system picks for it (the browser).
    pub fn open_url(&self, url: &str) -> Result<(), String> {
        self.call::<()>("openUrl", OpenUrlArgs { url })
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("android-player")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager;
                let handle = api.register_android_plugin("dev.siiis.siiishub.player", "MpvPlugin")?;
                app.manage(AndroidMpv(handle));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}
