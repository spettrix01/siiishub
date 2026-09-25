//! What the apps ask of an account on the server: a token for the device,
//! signed in with the account's username and password once, the sync of the
//! account's profile (`ops::sync`) with that token, and a wait for news: the
//! answer comes as soon as the profile changes (in the browser, or on
//! another device), so the change reaches every device at once. What an app
//! sends reaches the account's pages open in the browser the same way.

use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::ops::sync::{self, Change};

use super::accounts::{AccountView, Viewer};
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

/// The account of the request's token.
fn account(server: &Server, headers: &HeaderMap) -> Option<AccountView> {
    server.accounts.token_account(bearer(headers)?)
}

#[derive(Deserialize)]
pub struct TokenBody {
    username: String,
    password: String,
    #[serde(default)]
    device: String,
}

/// A token for an app, for the account's username and password.
pub async fn token(State(server): State<Server>, Json(body): Json<TokenBody>) -> Response {
    if !server.auth.attempts_left() {
        return (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "too-many-attempts" }))).into_response();
    }
    let Some(account) = server.accounts.verify(&body.username, &body.password).await else {
        server.auth.record_failure();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "wrong-credentials" }))).into_response();
    };
    let device = if body.device.trim().is_empty() { "App" } else { body.device.trim() };
    let Some(token) = server.accounts.issue_token(&account.id, device) else {
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
pub async fn sync(State(server): State<Server>, headers: HeaderMap, Json(body): Json<SyncBody>) -> Response {
    let Some(account) = account(&server, &headers) else {
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
/// again.
pub async fn wait(State(server): State<Server>, headers: HeaderMap, Query(query): Query<WaitQuery>) -> Response {
    let Some(account) = account(&server, &headers) else {
        return unauthorized();
    };
    let profile = match server.profiles.get(&Viewer::Account(account.id)).await {
        Ok(profile) => profile,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    };
    // Further on than the profile: not the one the app synced with.
    let changed = query.since > profile.sync.seq()
        || tokio::time::timeout(WAIT, profile.sync.past(query.since)).await.is_ok();
    Json(json!({ "changed": changed })).into_response()
}
