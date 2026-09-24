//! Login for the web server: one password (`SIIISHUB_PASSWORD`), sessions in
//! an HttpOnly cookie. The sessions are kept in `web-sessions.json`, so
//! restarting the server or the container does not log anyone out.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::json;

use super::Server;

const COOKIE: &str = "siiishub_session";
const SESSION_SECS: u64 = 30 * 24 * 3600;
/// Wrong passwords accepted per minute, all clients together, before the
/// login refuses every attempt until the minute is over.
const FAILURES_PER_MINUTE: u32 = 10;

pub struct Auth {
    /// `None`: no login at all (`SIIISHUB_AUTH=off`).
    password: Option<String>,
    sessions: Mutex<HashMap<String, u64>>,
    failures: Mutex<(u64, u32)>,
    path: PathBuf,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Auth {
    pub fn load(password: Option<String>, path: PathBuf) -> Self {
        let now = now();
        let sessions: HashMap<String, u64> = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let sessions = sessions.into_iter().filter(|(_, expiry)| *expiry > now).collect();
        Self {
            password,
            sessions: Mutex::new(sessions),
            failures: Mutex::new((now, 0)),
            path,
        }
    }

    pub fn enabled(&self) -> bool {
        self.password.is_some()
    }

    /// Compares in constant time: how long it takes says nothing about how
    /// much of the password was right.
    fn password_matches(&self, candidate: &str) -> bool {
        let Some(expected) = self.password.as_deref() else {
            return true;
        };
        let (a, b) = (expected.as_bytes(), candidate.as_bytes());
        let mut diff = a.len() ^ b.len();
        for i in 0..a.len().max(b.len()) {
            diff |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as usize;
        }
        diff == 0
    }

    fn attempts_left(&self) -> bool {
        let now = now();
        let mut failures = self.failures.lock();
        if now.saturating_sub(failures.0) >= 60 {
            *failures = (now, 0);
        }
        failures.1 < FAILURES_PER_MINUTE
    }

    fn record_failure(&self) {
        self.failures.lock().1 += 1;
    }

    fn open_session(&self) -> String {
        let token = super::random_hex(32);
        let mut sessions = self.sessions.lock();
        sessions.insert(token.clone(), now() + SESSION_SECS);
        self.persist(&sessions);
        token
    }

    fn is_valid(&self, token: &str) -> bool {
        self.sessions
            .lock()
            .get(token)
            .is_some_and(|expiry| *expiry > now())
    }

    fn close_session(&self, token: &str) {
        let mut sessions = self.sessions.lock();
        if sessions.remove(token).is_some() {
            self.persist(&sessions);
        }
    }

    /// The tokens open the app like the password does: only the server's
    /// user may read the file.
    fn persist(&self, sessions: &HashMap<String, u64>) {
        let Ok(bytes) = serde_json::to_vec(sessions) else {
            return;
        };
        let tmp = self.path.with_extension("json.tmp");
        let written = std::fs::write(&tmp, bytes).and_then(|_| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
            }
            std::fs::rename(&tmp, &self.path)
        });
        if let Err(e) = written {
            tracing::warn!("[web] saving the sessions to {} failed: {e}", self.path.display());
        }
    }
}

fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| pair.trim().strip_prefix("siiishub_session="))
}

/// The API only answers the pages of this server. Without the check a page
/// on another site could use the session cookie of whoever visits it: a form
/// posting to the API, or a WebSocket opened on it. Requests without an
/// `Origin` header are not cross-site browser requests.
fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let origin_host = origin.split_once("://").map(|(_, host)| host).unwrap_or(origin);
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let forwarded_host = headers.get("x-forwarded-host").and_then(|v| v.to_str().ok());
    Some(origin_host) == host || Some(origin_host) == forwarded_host
}

/// The login page and what it loads: the app's styles, icons and
/// translations, none of which holds anything about the user.
fn is_public(path: &str) -> bool {
    matches!(
        path,
        "/login" | "/api/login" | "/style.css" | "/web/login.css" | "/web/login.js"
    ) || (path.starts_with("/js/locales/") && path.ends_with(".js") && !path.contains(".."))
        || (path.starts_with("/img/icon-") && path.ends_with(".png") && !path.contains(".."))
}

/// A phone on the home network opens the remote's page as it opens the one of
/// the app on a PC: the screen approves it, no password (the Android app's
/// frame could not keep the session's cookie anyway). What a proxy forwards,
/// or what comes from a public address, signs in first.
fn home_network(req: &Request) -> bool {
    let headers = req.headers();
    let forwarded = ["forwarded", "x-forwarded-for", "x-real-ip", "cf-connecting-ip"]
        .iter()
        .any(|name| headers.contains_key(*name));
    !forwarded
        && req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .is_some_and(|ConnectInfo(addr)| is_private(addr.ip()))
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            // With the carrier-grade NAT range, where Tailscale's addresses are.
            v4.is_private() || v4.is_loopback() || v4.is_link_local() || (a == 100 && (b & 0xc0) == 64)
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_private(IpAddr::V4(v4)),
            // Loopback, unique local, link-local.
            None => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
        },
    }
}

/// Every request but the login page needs a session; the API answers 401
/// without one, pages send the browser to the login.
pub async fn guard(State(server): State<Server>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let reads = matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD);
    let socket = path == "/remote/ws";
    if (path.starts_with("/api/") || socket || !reads) && !same_origin(req.headers()) {
        return (StatusCode::FORBIDDEN, "cross-origin request").into_response();
    }
    let remote = matches!(path.as_str(), "/remote" | "/remote/" | "/remote/ws");
    if !server.auth.enabled() || is_public(&path) || (remote && home_network(&req)) {
        return next.run(req).await;
    }
    let signed_in = session_cookie(req.headers()).is_some_and(|token| server.auth.is_valid(token));
    if signed_in {
        return next.run(req).await;
    }
    if path.starts_with("/api/") {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized" }))).into_response();
    }
    if path == "/" || path.ends_with(".html") {
        return Redirect::to("/login").into_response();
    }
    // The phone remote's page: back to it once signed in.
    if path == "/remote" || path == "/remote/" {
        return Redirect::to("/login?next=/remote/").into_response();
    }
    StatusCode::UNAUTHORIZED.into_response()
}

#[derive(Deserialize)]
pub struct LoginBody {
    password: String,
}

pub async fn login(
    State(server): State<Server>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    if !server.auth.attempts_left() {
        return (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "too-many-attempts" }))).into_response();
    }
    if !server.auth.password_matches(&body.password) {
        server.auth.record_failure();
        // Slows down guessing from a single client.
        tokio::time::sleep(Duration::from_secs(1)).await;
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "wrong-password" }))).into_response();
    }
    if !server.auth.enabled() {
        return Json(json!({ "ok": true })).into_response();
    }
    let token = server.auth.open_session();
    // Behind an HTTPS reverse proxy the cookie only travels encrypted.
    let secure = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|proto| proto.eq_ignore_ascii_case("https"));
    let cookie = format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_SECS}{}",
        if secure { "; Secure" } else { "" }
    );
    ([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true }))).into_response()
}

pub async fn logout(State(server): State<Server>, headers: HeaderMap) -> Response {
    if let Some(token) = session_cookie(&headers) {
        server.auth.close_session(token);
    }
    let cookie = format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    ([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true }))).into_response()
}

/// The login page; without a login configured it just leads to the app.
pub async fn login_page(State(server): State<Server>) -> Response {
    if !server.auth.enabled() {
        return Redirect::to("/").into_response();
    }
    super::web::file(&server.files.web_dir.join("login.html")).await
}
