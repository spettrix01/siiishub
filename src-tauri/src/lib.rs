// Two builds share this crate: the app (feature `app`: Tauri, the embedded
// player, the remote control) and the web server (feature `server`: the same
// interface in a browser). What both need lives outside the app modules, and
// `ops` holds what the interface asks of the backend.
#[cfg(feature = "app")]
use std::sync::Arc;

#[cfg(feature = "app")]
use tauri::Manager;

mod alldebrid;
#[cfg(feature = "app")]
mod account;
#[cfg(feature = "app")]
mod commands;
mod download;
mod ffprobe;
#[cfg(feature = "app")]
mod mpv;
pub mod ops;
#[cfg(feature = "app")]
mod power;
mod realdebrid;
mod remote;
#[cfg(all(windows, feature = "real-mpv"))]
mod render;
#[cfg(all(target_os = "linux", feature = "real-mpv"))]
mod render_linux;
#[cfg(feature = "server")]
pub mod server;
mod settings;
mod state;
mod stremio;
mod sync;
mod torrent;
mod torrent_file;
mod userdata;
mod util;

pub use state::AppState;

#[cfg(feature = "app")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(not(target_os = "android"))]
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,siiishub_desktop_lib=debug".into()),
        )
        .with_target(false)
        .try_init();

    // Android has no stdout: logs and panics go to logcat (tag "siiishub",
    // `adb logcat -s siiishub mpv`).
    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        if let Ok(layer) = tracing_android::layer("siiishub") {
            let _ = tracing_subscriber::registry()
                .with(tracing_subscriber::EnvFilter::new(
                    "info,siiishub_desktop_lib=debug",
                ))
                .with(layer)
                .try_init();
        }
        std::panic::set_hook(Box::new(|info| {
            tracing::error!("panic: {info}");
        }));
    }

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init());

    // Android: libmpv is embedded through a Kotlin plugin
    // (src-tauri/plugins/android-player), see src/mpv/android.rs.
    #[cfg(target_os = "android")]
    let builder = builder.plugin(siiishub_android_player::init());

    builder
        .setup(|app| {
            let handle = app.handle().clone();
            let state = tauri::async_runtime::block_on(AppState::initialize(handle))
                .expect("failed to initialize AppState");

            util::set_log_dir(state.data_dir.clone());
            power::init(app.handle().clone());

            let data_dir = state.data_dir.clone();
            let webview_dir = data_dir.join("webview");
            let initial_theme = state.userdata.get("siiishub-theme").unwrap_or_default();

            let _ = std::fs::remove_file(data_dir.join("_webview_relocated"));

            app.manage(Arc::new(state));

            // The account's sync (account.rs): what comes in reaches the
            // interface as `sync://changed`.
            let account = Arc::new(account::Account::load(data_dir.join("account.json")));
            app.manage(account.clone());
            let sync_handle = app.handle().clone();
            account::start(app.state::<Arc<AppState>>().inner().clone(), account, move |applied| {
                use tauri::Emitter;
                let _ = sync_handle.emit("sync://changed", applied);
            });

            // The phone is the player itself: no remote-control server on an
            // Android phone. On a PC and in the TV APK the phone drives it.
            #[cfg(any(not(target_os = "android"), feature = "tv"))]
            {
                let state_for_remote = app.state::<Arc<AppState>>().inner().clone();
                let cfg = state_for_remote.settings.read();
                let port = if cfg.remote_port == 0 { 9871 } else { cfg.remote_port };
                drop(cfg);
                let app_handle = app.handle().clone();
                state_for_remote.remote.set_emitter(Arc::new(move |name: &str, payload| {
                    use tauri::Emitter;
                    let _ = app_handle.emit(name, payload);
                }));
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = remote::start(state_for_remote, port).await {
                        tracing::warn!("[remote] boot start failed: {e}");
                    }
                });
            }

            #[allow(unused_mut)]
            let mut init_script = format!(
                "window.__INITIAL_THEME__ = {};",
                serde_json::to_string(&initial_theme).unwrap_or_else(|_| "\"\"".to_string())
            );
            // The TV APK: the interface takes its TV look (js/platform.js).
            #[cfg(feature = "tv")]
            init_script.push_str(" window.__SIIISHUB_TV__ = true;");

            let builder = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .transparent(true)
            .initialization_script(&init_script);

            // Window chrome and sizing only exist on desktop; on Android the
            // activity is the window and the webview data dir is fixed.
            #[cfg(desktop)]
            let builder = builder
                .title("SIIISHUB")
                .inner_size(1400.0, 880.0)
                .min_inner_size(960.0, 600.0)
                .decorations(false)
                .shadow(false)
                .data_directory(webview_dir);
            #[cfg(mobile)]
            let _ = webview_dir;

            let main = builder.build().expect("failed to create main window");

            #[cfg(all(windows, feature = "real-mpv"))]
            {
                let state = app.state::<Arc<AppState>>().inner().clone();
                if let Ok(hwnd) = main.hwnd() {
                    state.set_main_hwnd(hwnd.0 as isize);
                    tracing::info!("[setup] main HWND captured: 0x{:x}", hwnd.0 as isize);
                }

                tauri::async_runtime::spawn(async move {
                    if let Err(e) = render::attach_to_window(state, main).await {
                        tracing::error!("compositor attach failed: {e:#}");
                    }
                });
            }

            #[cfg(target_os = "android")]
            {
                // libmpv renders into a SurfaceView under the webview; the
                // Kotlin side creates it lazily on the first player command.
                let state = app.state::<Arc<AppState>>().inner().clone();
                match mpv::Mpv::new(app.handle().clone()) {
                    Ok(bridge) => {
                        let bridge = Arc::new(bridge);
                        bridge.start_event_pump(app.handle().clone());
                        state.set_mpv(bridge);
                        tracing::info!("[mpv] android bridge ready");
                    }
                    Err(e) => tracing::error!("[mpv] android bridge failed: {e:#}"),
                }
                let _ = main;
            }

            #[cfg(all(target_os = "linux", feature = "real-mpv"))]
            {
                // GTK widgets must be touched from the main thread, which is
                // where `setup` runs: attach synchronously.
                let state = app.state::<Arc<AppState>>().inner().clone();
                if let Err(e) = render_linux::attach_to_window(state, main) {
                    tracing::error!("video pipeline attach failed: {e:#}");
                }
            }

            #[cfg(not(any(
                all(windows, feature = "real-mpv"),
                all(target_os = "linux", feature = "real-mpv"),
                target_os = "android"
            )))]
            {
                // No embedded video pipeline: development-only configuration.
                let _ = main;
                tracing::warn!("no video pipeline for this platform/feature set");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::settings_get,
            commands::settings_save,
            commands::userdata_load,
            commands::userdata_set,
            commands::userdata_remove,
            commands::sync_status,
            commands::sync_sign_in,
            commands::sync_sign_out,
            commands::open_download_dir,
            commands::open_url,
            commands::player_mode,
            commands::addon_meta,
            commands::streams_fetch,
            commands::subtitles_fetch,
            commands::media_resolve,
            commands::media_resolve_all,
            commands::media_cancel,
            commands::media_probe,
            commands::torrent_stats,
            commands::session_destroy,
            commands::download_start,
            commands::download_start_group,
            commands::download_add_local,
            commands::download_list,
            commands::download_remove,
            commands::download_pause,
            commands::download_resume,
            commands::download_play,
            commands::download_files,
            commands::download_open_folder,
            commands::torrent_parse_file,
            commands::remote_info,
            commands::remote_push_state,
            commands::remote_set_approval,
            commands::remote_remember_device,
            commands::remote_forget_device,
            commands::mpv_load,
            commands::mpv_command,
            commands::mpv_set_property,
            commands::mpv_get_property,
            commands::mpv_observe,
            commands::mpv_set_geometry,
            commands::mpv_set_visible,
            commands::window_set_fullscreen,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
