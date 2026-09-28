//! What the apps ask of an account on the server: a token for the device,
//! signed in with the account's username and password once, the sync of the
//! account's profile (`ops::sync`) with that token, and a wait for news: the
//! answer comes as soon as the profile changes (in the browser, or on
//! another device), so the change reaches every device at once. What an app
//! sends reaches the account's pages open in the browser the same way.
//! Every request tells of the device (`Visit`), for the administrator's list.

use std::net::SocketAddr;
use std::time::Duration;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::ops::sync::{self, Change};

use super::accounts::{AccountView, Viewer, Visit};
use super::Server;

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": "unauthorized" }))).into_response()
}

/// Longest wait for news, under the minute proxies give a request.
const WAIT: Duration = Duration::from_secs(50);

/// The app's User-Agent from 1.3.7 on, `SIIISHUB/<version> (<system>;
/// <name>)`: the version and the device's name. The apps before sent
/// `siiishub/0.1`.
fn app_agent(agent: &str) -> Option<(String, String)> {
    let (version, comment) = agent.strip_prefix("SIIISHUB/")?.split_once(" (")?;
    let comment = comment.strip_suffix(')')?;
    let name = comment.split_once(';').map(|(_, name)| name).unwrap_or("");
    let clean = |s: &str, max: usize| -> String {
        s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
    };
    Some((clean(version, 24), clean(name, 64)))
}

/// What the request tells of the app's device.
fn visit(headers: &HeaderMap, peer: SocketAddr) -> Visit {
    let (version, name) = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .and_then(app_agent)
        .unwrap_or_default();
    let address = super::auth::client_address(headers, Some(peer))
        .map(|ip| ip.to_string())
        .unwrap_or_default();
    Visit { name, version, address }
}

/// The account of the request's token; its device is seen now.
fn account(server: &Server, headers: &HeaderMap, peer: SocketAddr) -> Option<AccountView> {
    server.accounts.visit(bearer(headers)?, &visit(headers, peer))
}

#[derive(Deserialize)]
pub struct TokenBody {
    username: String,
    password: String,
    #[serde(default)]
    device: String,
}

/// A token for an app, for the account's username and password.
pub async fn token(
    State(server): State<Server>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<TokenBody>,
) -> Response {
    if !server.auth.attempts_left() {
        return (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "too-many-attempts" }))).into_response();
    }
    let Some(account) = server.accounts.verify(&body.username, &body.password).await else {
        server.auth.record_failure();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "wrong-credentials" }))).into_response();
    };
    let device = if body.device.trim().is_empty() { "App" } else { body.device.trim() };
    let Some(token) = server.accounts.issue_token(&account.id, device, &visit(&headers, peer)) else {
        return unauthorized();
    };
    tracing::info!("[accounts] {} signed in on {device}", account.username);
    Json(json!({ "token": token, "username": account.username })).into_response()
}

/// An app signs out: its token stops working.
pub async fn revoke(State(server): State<Server>, headers: HeaderMap) -> Response {
    if let Some(token) = bearer(&headers) {
        server.accounts.revoke_token(token);
    }
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
pub struct SyncBody {
    /// The place in the profile's order of changes the app has everything
    /// up to.
    #[serde(default)]
    since: u64,
    #[serde(default)]
    changes: Vec<Change>,
}

/// Takes the app's changes, and answers with the profile's after `since`.
pub async fn sync(
    State(server): State<Server>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SyncBody>,
) -> Response {
    let Some(account) = account(&server, &headers, peer) else {
        return unauthorized();
    };
    let viewer = Viewer::Account(account.id.clone());
    let profile = match server.profiles.get(&viewer).await {
        Ok(profile) => profile,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    };
    // Further on than the profile has come: the profile is not the one the
    // app synced with (restored from a backup, say), and it gets it all.
    let since = if body.since > profile.sync.seq() { 0 } else { body.since };
    let applied = match sync::apply(profile.as_ref().into(), body.changes, false).await {
        Ok(applied) => applied,
        Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": e }))).into_response(),
    };
    // The account's pages in the browser show it at once.
    if applied.userdata || applied.settings {
        server.events.emit_to(viewer, "sync://changed", json!(applied));
    }
    let (changes, cursor) = sync::changes_since(profile.as_ref().into(), since, false);
    Json(json!({
        "username": account.username,
        "cursor": cursor,
        "changes": changes,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct WaitQuery {
    #[serde(default)]
    since: u64,
}

/// Answers when the account's profile goes past `since` (`changed`), or
/// after a while with nothing (`changed` false): the app then syncs, or asks
/// again. A device signed out meanwhile gets its 401 at once.
pub async fn wait(
    State(server): State<Server>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<WaitQuery>,
) -> Response {
    let Some(account) = account(&server, &headers, peer) else {
        return unauthorized();
    };
    let profile = match server.profiles.get(&Viewer::Account(account.id)).await {
        Ok(profile) => profile,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    };
    // Further on than the profile: not the one the app synced with.
    if query.since > profile.sync.seq() {
        return Json(json!({ "changed": true })).into_response();
    }
    let deadline = tokio::time::Instant::now() + WAIT;
    let changed = loop {
        let kicked = server.accounts.kicked();
        tokio::select! {
            _ = profile.sync.past(query.since) => break true,
            _ = tokio::time::sleep_until(deadline) => break false,
            _ = kicked => {
                let token = bearer(&headers).unwrap_or_default();
                if server.accounts.token_account(token).is_none() {
                    return unauthorized();
                }
            }
        }
    };
    Json(json!({ "changed": changed })).into_response()
}

#[cfg(test)]
mod tests {
    use super::app_agent;

    #[test]
    fn agents() {
        assert_eq!(
            app_agent("SIIISHUB/1.3.7 (Windows; DESKTOP-7H2K9)"),
            Some(("1.3.7".into(), "DESKTOP-7H2K9".into()))
        );
        assert_eq!(
            app_agent("SIIISHUB/1.3.7 (Android TV; Amazon AFTKA)"),
            Some(("1.3.7".into(), "Amazon AFTKA".into()))
        );
        assert_eq!(app_agent("SIIISHUB/1.3.7 (Linux)"), Some(("1.3.7".into(), String::new())));
        assert_eq!(app_agent("siiishub/0.1"), None);
        assert_eq!(app_agent("Mozilla/5.0 (X11; Linux x86_64)"), None);
    }
}
