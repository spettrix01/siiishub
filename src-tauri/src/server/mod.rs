//! siiishub-server: the app's interface in a browser, for a server or a
//! Docker container. The pages are the app's (`dist/`) plus the browser
//! additions of `web/`; the backend is the app's too, through `ops`. The
//! embedded player has no place here: the browser plays the video.

mod api;
mod auth;
mod events;
mod hls;
mod media;
mod remote;
mod transcode;
mod web;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::{middleware, Router};
use tower_http::services::ServeDir;

use crate::state::AppState;

pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub download_dir: PathBuf,
    /// The app's interface (`dist/`).
    pub app_dir: PathBuf,
    /// The browser additions (`web/`).
    pub web_dir: PathBuf,
    /// `None`: no login.
    pub password: Option<String>,
    /// The GPU to transcode on.
    pub hwaccel: transcode::Wanted,
    /// Its render node or CUDA device, instead of the first that works.
    pub hwaccel_device: Option<String>,
}

impl Config {
    /// Reads the configuration from the environment (docs/WEB.md).
    pub fn from_env() -> anyhow::Result<Self> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let port: u16 = match var("SIIISHUB_PORT") {
            Some(p) => p.trim().parse().context("SIIISHUB_PORT is not a port number")?,
            None => 8080,
        };
        let address: IpAddr = match var("SIIISHUB_ADDRESS") {
            Some(a) => a.trim().parse().context("SIIISHUB_ADDRESS is not an IP address")?,
            None => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        let data_dir = var("SIIISHUB_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"));
        let download_dir = var("SIIISHUB_DOWNLOAD_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("download"));
        let app_dir = var("SIIISHUB_APP_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("dist"));
        let web_dir = var("SIIISHUB_WEB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("web"));
        let no_login = var("SIIISHUB_AUTH").is_some_and(|v| v.trim().eq_ignore_ascii_case("off"));
        let password = var("SIIISHUB_PASSWORD");
        if password.is_none() && !no_login {
            anyhow::bail!(
                "set SIIISHUB_PASSWORD, the password to sign in with \
                 (SIIISHUB_AUTH=off runs without a login: only on a network you trust)"
            );
        }
        Ok(Self {
            bind: SocketAddr::new(address, port),
            data_dir,
            download_dir,
            app_dir,
            web_dir,
            password: if no_login { None } else { password },
            hwaccel: transcode::Wanted::parse(var("SIIISHUB_HWACCEL").as_deref())?,
            hwaccel_device: var("SIIISHUB_HWACCEL_DEVICE"),
        })
    }
}

pub struct Files {
    pub app_dir: PathBuf,
    pub web_dir: PathBuf,
}

#[derive(Clone)]
pub struct Server {
    pub app: Arc<AppState>,
    pub events: events::Events,
    pub auth: Arc<auth::Auth>,
    pub files: Arc<Files>,
    pub media: Arc<media::Media>,
    pub hls: Arc<hls::Hls>,
    pub gpu: Arc<transcode::Gpu>,
    /// For relaying streams: no overall timeout (a film lasts hours) and no
    /// compression (it would break byte ranges).
    pub streams: reqwest::Client,
}

/// `bytes` random bytes in hex, for session tokens and media ids.
pub(crate) fn random_hex(bytes: usize) -> String {
    use rand::RngCore;
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

pub async fn run(config: Config) -> anyhow::Result<()> {
    for (file, var) in [
        (config.app_dir.join("index.html"), "SIIISHUB_APP_DIR"),
        (config.web_dir.join("bridge.js"), "SIIISHUB_WEB_DIR"),
    ] {
        if !file.is_file() {
            anyhow::bail!("{} not found: point {var} at the right folder", file.display());
        }
    }
    if config.password.is_none() {
        tracing::warn!("[web] no login (SIIISHUB_AUTH=off): whoever reaches the server can use it");
    }

    let app = AppState::open(config.data_dir.clone(), config.download_dir.clone()).await?;
    let server = Server {
        app: Arc::new(app),
        events: events::Events::new(),
        auth: Arc::new(auth::Auth::load(
            config.password.clone(),
            config.data_dir.join("web-sessions.json"),
        )),
        files: Arc::new(Files {
            app_dir: config.app_dir.clone(),
            web_dir: config.web_dir.clone(),
        }),
        media: Arc::new(media::Media::default()),
        hls: Arc::new(hls::Hls::default()),
        gpu: Arc::new(transcode::Gpu::new(config.hwaccel, config.hwaccel_device.clone())),
        streams: reqwest::Client::builder()
            .user_agent("siiishub/0.1")
            .connect_timeout(std::time::Duration::from_secs(15))
            .no_gzip()
            .build()
            .context("building the streaming HTTP client")?,
    };

    hls::start_sweeper(server.clone());
    server.gpu.start();
    // The phone remote's events (approvals, commands) go to the pages.
    let events = server.events.clone();
    server
        .app
        .remote
        .set_emitter(Arc::new(move |name: &str, payload| events.emit(name, payload)));
    server.app.remote.mount();

    let router = Router::new()
        .route("/", get(web::index))
        .route("/index.html", get(web::index))
        .route("/login", get(auth::login_page))
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .route("/api/invoke/{command}", post(api::invoke))
        .route("/api/events", get(events::socket))
        .route("/media/subtitle", get(media::subtitle))
        .route("/media/live-sub/{token}", get(media::live_sub))
        .route("/media/{id}", get(media::direct))
        .route("/media/{id}/info", get(media::info))
        .route("/media/{id}/remux.mp4", get(media::remux))
        .route("/media/{id}/hls", post(hls::create))
        .route("/hls/{sid}", axum::routing::delete(hls::close))
        .route("/hls/{sid}/master.m3u8", get(hls::master))
        .route("/hls/{sid}/media.m3u8", get(hls::media_playlist))
        .route("/hls/{sid}/init.mp4", get(hls::init))
        .route("/hls/{sid}/segments/{k}", get(hls::segment))
        .route("/hls/{sid}/subtitles/{n}", get(hls::subtitles))
        .route("/v/{version}/{*path}", get(web::versioned))
        .route("/remote", get(remote::page_slash))
        .route("/remote/", get(remote::page))
        .route("/remote/ws", get(remote::socket))
        .nest_service("/web", ServeDir::new(&config.web_dir))
        .fallback_service(ServeDir::new(&config.app_dir))
        // A .torrent file reaches `torrent_parse_file` as a JSON array.
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(middleware::map_response(web::revalidate))
        .layer(middleware::from_fn_with_state(server.clone(), auth::guard))
        .with_state(server);

    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("cannot listen on {}", config.bind))?;
    tracing::info!(
        "[web] SIIISHUB on http://{} (data in {}, downloads in {})",
        config.bind,
        config.data_dir.display(),
        config.download_dir.display()
    );
    // The peer's address, for the phone remote's list of devices.
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Ctrl+C, or SIGTERM from `docker stop`. Open requests get a few seconds to
/// finish; a video being streamed never would, so then the process exits.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("[web] shutting down");
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        std::process::exit(0);
    });
}
