//! The pages: the app's interface (`dist/`) with the browser additions of
//! `web/` injected into `index.html`.
//!
//! `index.html` loads the interface from `/v/<version>/`, the version being
//! the time of its newest file: a browser keeps those files for good and
//! still gets the new ones after an update, the modules all of the same
//! version (their imports are relative). What the server sends otherwise is
//! checked again at every use (`revalidate`).

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::ServeDir;

use super::Server;

/// `index.html` of the app, with what the app gets from Tauri: the initial
/// theme (Tauri's initialization script) and the bridge standing in for
/// `window.__TAURI__` (before any script of the app runs); then the styles
/// and the script of the browser additions (after the app's).
pub async fn index(State(server): State<Server>) -> Response {
    let path = server.files.app_dir.join("index.html");
    let html = match tokio::fs::read_to_string(&path).await {
        Ok(html) => html,
        Err(e) => {
            tracing::error!("[web] reading {} failed: {e}", path.display());
            return (StatusCode::INTERNAL_SERVER_ERROR, "index.html not found").into_response();
        }
    };
    let theme = server.app.userdata.get("siiishub-theme").unwrap_or_default();
    let theme = serde_json::to_string(&theme).unwrap_or_else(|_| "\"\"".to_string());
    let dirs = [server.files.app_dir.clone(), server.files.web_dir.clone()];
    let version = tokio::task::spawn_blocking(move || version(&dirs))
        .await
        .unwrap_or_else(|_| "0".to_string());
    let v = format!("/v/{version}");
    let scripts = format!(
        "<head>\n<script>window.__INITIAL_THEME__ = {theme};</script>\n<script src=\"{v}/web/bridge.js\"></script>"
    );
    let html = html
        .replacen("<head>", &scripts, 1)
        .replacen("href=\"/style.css\"", &format!("href=\"{v}/style.css\""), 1)
        .replacen("src=\"/app.js", &format!("src=\"{v}/app.js"), 1)
        .replacen("</head>", &format!("<link rel=\"stylesheet\" href=\"{v}/web/web.css\" />\n</head>"), 1)
        .replacen("</body>", &format!("<script type=\"module\" src=\"{v}/web/web.js\"></script>\n</body>"), 1);
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        html,
    )
        .into_response()
}

/// The time of the newest file of the interface, in milliseconds (hex).
fn version(dirs: &[PathBuf]) -> String {
    fn newest(dir: &Path, depth: u32) -> u128 {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                if meta.is_dir() {
                    return (depth < 8).then(|| newest(&entry.path(), depth + 1));
                }
                Some(meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis())
            })
            .max()
            .unwrap_or(0)
    }
    format!("{:x}", dirs.iter().map(|dir| newest(dir, 0)).max().unwrap_or(0))
}

/// `/v/<version>/<path>`: a file of the interface (`web/...`: of the browser
/// additions), at an address that changes with it, so kept for good. Any
/// version is served the current files, for pages opened before an update.
pub async fn versioned(State(server): State<Server>, mut req: Request) -> Response {
    let path = req.uri().path();
    // "", "v", the version, the rest.
    let rest = path.splitn(4, '/').nth(3).unwrap_or("");
    let (dir, rest) = match rest.strip_prefix("web/") {
        Some(rest) => (&server.files.web_dir, rest),
        None => (&server.files.app_dir, rest),
    };
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let Ok(uri) = format!("/{rest}{query}").parse() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    *req.uri_mut() = uri;
    let mut response = match ServeDir::new(dir).oneshot(req).await {
        Ok(response) => response.map(Body::new).into_response(),
        Err(never) => match never {},
    };
    if response.status().is_success() {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    response
}

/// Everything else is checked again at every use (a 304 when unchanged):
/// left alone, browsers keep files for hours after an update.
pub async fn revalidate(mut response: Response) -> Response {
    response
        .headers_mut()
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-cache"));
    response
}

/// A small file of `web/` read whole (the login page).
pub async fn file(path: &Path) -> Response {
    match tokio::fs::read(path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("[web] reading {} failed: {e}", path.display());
            StatusCode::NOT_FOUND.into_response()
        }
    }
}
