//! Login for the web server. With an account (`accounts.rs`) one uses its
//! profile; without one, the server's own profile opens from the home
//! network (unless `SIIISHUB_GUEST=off`), and from anywhere with
//! `SIIISHUB_PASSWORD` when it is set, the login of the versions before
//! accounts. Sessions live in an HttpOnly cookie and in `web-sessions.json`,
//! so restarting the server or the container signs nobody out. The apps
//! sign in with a token of their own instead (`sync_api.rs`).

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
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::accounts::Viewer;
use super::Server;

const COOKIE: &str = "siiishub_session";
const SESSION_SECS: u64 = 30 * 24 * 3600;
/// Wrong passwords accepted per minute, all clients together, before every
/// login is refused until the minute is over.
const FAILURES_PER_MINUTE: u32 = 10;

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    expiry: u64,
    #[serde(default)]
    account: Option<String>,
    /// The server's profile opened without a password: only from the home
    /// network, request by request.
    #[serde(default)]
    home: bool,
}

/// `web-sessions.json` before accounts held the expiry alone.
#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Before(u64),
    Now(Session),
}

pub struct Auth {
    /// No login at all (`SIIISHUB_AUTH=off`): everyone uses the server's
    /// profile.
    open: bool,
    password: Option<String>,
    guest_home: bool,
    sessions: Mutex<HashMap<String, Session>>,
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
    pub fn load(open: bool, password: Option<String>, guest_home: bool, path: PathBuf) -> Self {
        let now = now();
        let stored: HashMap<String, Stored> = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let sessions = stored
            .into_iter()
            .map(|(token, s)| {
                let session = match s {
                    Stored::Before(expiry) => Session { expiry, account: None, home: false },
                    Stored::Now(session) => session,
                };
                (token, session)
            })
            .filter(|(_, s)| s.expiry > now)
            .collect();
        Self {
            open,
            password,
            guest_home,
            sessions: Mutex::new(sessions),
            failures: Mutex::new((now, 0)),
            path,
        }
    }

    pub fn enabled(&self) -> bool {
        !self.open
    }

    /// The password of the versions before accounts, compared in constant
    /// time: how long it takes says nothing about how much of it was right.
    fn password_matches(&self, candidate: &str) -> bool {
        let Some(expected) = self.password.as_deref() else {
            return false;
        };
        let (a, b) = (expected.as_bytes(), candidate.as_bytes());
        let mut diff = a.len() ^ b.len();
        for i in 0..a.len().max(b.len()) {
            diff |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as usize;
        }
        diff == 0
    }

    pub fn attempts_left(&self) -> bool {
        let now = now();
        let mut failures = self.failures.lock();
        if now.saturating_sub(failures.0) >= 60 {
            *failures = (now, 0);
        }
        failures.1 < FAILURES_PER_MINUTE
    }

    pub fn record_failure(&self) {
        self.failures.lock().1 += 1;
    }

    fn open_session(&self, account: Option<String>, home: bool) -> String {
        let token = super::random_hex(32);
        let mut sessions = self.sessions.lock();
        sessions.insert(token.clone(), Session { expiry: now() + SESSION_SECS, account, home });
        self.persist(&sessions);
        token
    }

    fn session(&self, token: &str) -> Option<Session> {
        self.sessions
            .lock()
            .get(token)
            .filter(|s| s.expiry > now())
            .cloned()
    }

    fn close_session(&self, token: &str) {
        let mut sessions = self.sessions.lock();
        if sessions.remove(token).is_some() {
            self.persist(&sessions);
        }
    }

    /// An account deleted: its sessions end.
    pub fn close_account(&self, id: &str) {
        let mut sessions = self.sessions.lock();
        let before = sessions.len();
        sessions.retain(|_, s| s.account.as_deref() != Some(id));
        if sessions.len() != before {
            self.persist(&sessions);
        }
    }

    /// The tokens open the app like a password does: only the server's user
    /// may read the file.
    fn persist(&self, sessions: &HashMap<String, Session>) {
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

/// The login page and what it loads (the app's styles, icons and
/// translations, none of which holds anything about the user), and the
/// endpoints that check credentials themselves.
fn is_public(path: &str) -> bool {
    matches!(
        path,
        "/login"
            | "/api/login"
            | "/api/login/options"
            | "/api/login/guest"
            | "/api/account/setup"
            | "/api/account/token"
            | "/api/account/revoke"
            | "/api/sync"
            | "/api/sync/wait"
            | "/style.css"
            | "/web/login.css"
            | "/web/login.js"
    ) || (path.starts_with("/js/locales/") && path.ends_with(".js") && !path.contains(".."))
        || (path.starts_with("/img/icon-") && path.ends_with(".png") && !path.contains(".."))
}

/// A request from the home network: a private address, not forwarded by a
/// proxy. What a proxy forwards, or what comes from a public address, is
/// from outside.
pub fn home_network(headers: &HeaderMap, peer: Option<SocketAddr>) -> bool {
    let forwarded = ["forwarded", "x-forwarded-for", "x-real-ip", "cf-connecting-ip"]
        .iter()
        .any(|name| headers.contains_key(*name));
    !forwarded && peer.is_some_and(|addr| is_private(addr.ip()))
}

fn peer(req: &Request) -> Option<SocketAddr> {
    req.extensions().get::<ConnectInfo<SocketAddr>>().map(|ConnectInfo(addr)| *addr)
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

/// Every request but the login page needs a session, which tells whose
/// profile it uses (`Viewer`, for the handlers). The API answers 401 without
/// one; pages send the browser to the login.
pub async fn guard(State(server): State<Server>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let reads = matches!(*req.method(), axum::http::Method::GET | axum::http::Method::HEAD);
    let socket = path == "/remote/ws";
    if (path.starts_with("/api/") || socket || !reads) && !same_origin(req.headers()) {
        return (StatusCode::FORBIDDEN, "cross-origin request").into_response();
    }
    if !server.auth.enabled() {
        req.extensions_mut().insert(Viewer::Guest);
        return next.run(req).await;
    }
    // A phone on the home network opens the remote's page as it opens the
    // one of the app on a PC: the screen approves it, no password (the
    // Android app's frame could not keep the session's cookie anyway).
    let home = home_network(req.headers(), peer(&req));
    let remote = matches!(path.as_str(), "/remote" | "/remote/" | "/remote/ws");
    if is_public(&path) || (remote && home) {
        return next.run(req).await;
    }
    let viewer = session_cookie(req.headers())
        .and_then(|token| server.auth.session(token))
        .and_then(|session| match session.account {
            Some(id) => server.accounts.get(&id).map(|_| Viewer::Account(id)),
            None => (!session.home || (home && server.auth.guest_home)).then_some(Viewer::Guest),
        });
    if let Some(viewer) = viewer {
        req.extensions_mut().insert(viewer);
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

/// Signs the browser in: a session for `account` (none: the server's
/// profile), in a cookie.
fn signed_in(server: &Server, headers: &HeaderMap, account: Option<String>, home: bool) -> Response {
    let token = server.auth.open_session(account, home);
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

async fn refused(server: &Server, error: &str) -> Response {
    server.auth.record_failure();
    // Slows down guessing from a single client.
    tokio::time::sleep(Duration::from_secs(1)).await;
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": error }))).into_response()
}

fn too_many() -> Response {
    (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "too-many-attempts" }))).into_response()
}

/// What the login page offers this browser.
pub async fn options(
    State(server): State<Server>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let home = home_network(&headers, Some(addr));
    Json(json!({
        // No account yet: the first one, the administrator's, is made here.
        "setup": server.accounts.is_empty(),
        "home": home,
        "guest": home && server.auth.guest_home,
        "password": server.auth.password.is_some(),
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct LoginBody {
    #[serde(default)]
    username: String,
    password: String,
}

/// An account's username and password; or, with no username, the server's
/// password of the versions before accounts.
pub async fn login(State(server): State<Server>, headers: HeaderMap, Json(body): Json<LoginBody>) -> Response {
    if !server.auth.enabled() {
        return Json(json!({ "ok": true })).into_response();
    }
    if !server.auth.attempts_left() {
        return too_many();
    }
    if body.username.trim().is_empty() {
        if !server.auth.password_matches(&body.password) {
            return refused(&server, "wrong-password").await;
        }
        return signed_in(&server, &headers, None, false);
    }
    match server.accounts.verify(&body.username, &body.password).await {
        Some(account) => signed_in(&server, &headers, Some(account.id), false),
        None => refused(&server, "wrong-credentials").await,
    }
}

/// The server's profile without an account, from the home network.
pub async fn guest(
    State(server): State<Server>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !(server.auth.guest_home && home_network(&headers, Some(addr))) {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "not-home" }))).into_response();
    }
    signed_in(&server, &headers, None, true)
}

#[derive(Deserialize)]
pub struct SetupBody {
    username: String,
    password: String,
}

/// The first account, the administrator's: made from the home network while
/// the server has none.
pub async fn setup(
    State(server): State<Server>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SetupBody>,
) -> Response {
    if !server.accounts.is_empty() {
        return (StatusCode::CONFLICT, Json(json!({ "error": "setup-done" }))).into_response();
    }
    if !home_network(&headers, Some(addr)) {
        return (StatusCode::FORBIDDEN, Json(json!({ "error": "not-home" }))).into_response();
    }
    match server.accounts.create(&body.username, &body.password, true).await {
        Ok(account) => {
            tracing::info!("[accounts] {} made, the administrator", account.username);
            signed_in(&server, &headers, Some(account.id), false)
        }
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": e.code() }))).into_response(),
    }
}

pub async fn logout(State(server): State<Server>, headers: HeaderMap) -> Response {
    if let Some(token) = session_cookie(&headers) {
        server.auth.close_session(token);
    }
    let cookie = format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    ([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true }))).into_response()
}

/// The login page; without a login it just leads to the app.
pub async fn login_page(State(server): State<Server>) -> Response {
    if !server.auth.enabled() {
        return Redirect::to("/").into_response();
    }
    super::web::file(&server.files.web_dir.join("login.html")).await
}
