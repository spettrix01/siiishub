//! `POST /api/invoke/{command}`: the app's Tauri commands for the page in the
//! browser (`web/bridge.js` sends `invoke(command, args)` here). The body is
//! the object the page passes to `invoke`, named as Tauri names command
//! parameters (camelCase); a failed command answers 422 with the error string
//! the app would reject with. Commands about the window, the embedded player
//! or the remote control are the bridge's business and never reach here.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ops;
use crate::ops::downloads::{DownloadStartArgs, DownloadStartGroupArgs};
use crate::ops::media::ResolveArgs;
use crate::ops::settings::SettingsPatch;
use crate::state::AppState;

use super::accounts::Viewer;
use super::Server;

pub async fn invoke(
    State(server): State<Server>,
    Extension(viewer): Extension<Viewer>,
    Path(command): Path<String>,
    body: Bytes,
) -> Response {
    let args = if body.is_empty() {
        Value::Object(Default::default())
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return failure(format!("invalid arguments: {e}")),
        }
    };
    // Whose settings and user data: the account signed in, or the server's.
    let state = match server.profiles.get(&viewer).await {
        Ok(state) => state,
        Err(e) => return failure(e),
    };
    match dispatch(&server, &viewer, &state, &command, args).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => failure(e),
    }
}

fn failure(error: String) -> Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": error }))).into_response()
}

fn parse<T: DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| format!("invalid arguments: {e}"))
}

fn reply<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct IdArg {
    id: String,
}

#[derive(Deserialize)]
struct KeyArg {
    key: String,
}

#[derive(Deserialize)]
struct UrlArg {
    url: String,
}

#[derive(Deserialize)]
struct KindIdArgs {
    kind: String,
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InfoHashArg {
    info_hash: String,
}

#[derive(Deserialize)]
struct Wrapped<T> {
    args: T,
}

/// The administrator only.
fn admin(server: &Server, viewer: &Viewer) -> Result<(), String> {
    match viewer {
        Viewer::Account(id) if server.accounts.get(id).is_some_and(|a| a.admin) => Ok(()),
        _ => Err("not-admin".to_string()),
    }
}

async fn dispatch(
    server: &Server,
    viewer: &Viewer,
    state: &Arc<AppState>,
    command: &str,
    args: Value,
) -> Result<Value, String> {
    match command {
        "settings_get" => reply(ops::settings::get(state)),
        "settings_save" => {
            #[derive(Deserialize)]
            struct A {
                patch: SettingsPatch,
            }
            let A { patch } = parse(args)?;
            let (settings, _remote_port) = ops::settings::save(state, patch).await?;
            reply(settings)
        }

        "userdata_load" => reply(ops::userdata::load(state)),
        "userdata_set" => {
            #[derive(Deserialize)]
            struct A {
                key: String,
                value: String,
            }
            let A { key, value } = parse(args)?;
            reply(ops::userdata::set(state, key, value).await?)
        }
        "userdata_remove" => {
            let KeyArg { key } = parse(args)?;
            reply(ops::userdata::remove(state, &key).await?)
        }

        "addon_meta" => {
            let UrlArg { url } = parse(args)?;
            reply(ops::addons::meta(state, &url).await?)
        }
        "streams_fetch" => {
            let KindIdArgs { kind, id } = parse(args)?;
            reply(ops::addons::streams(state, &kind, &id).await)
        }
        "subtitles_fetch" => {
            let KindIdArgs { kind, id } = parse(args)?;
            reply(ops::addons::subtitles(state, &kind, &id).await)
        }

        // The player gets the stream at an address of this server: the one
        // resolved is only reachable from here (media.rs).
        "media_resolve" => {
            let Wrapped::<ResolveArgs> { args } = parse(args)?;
            let mut resolved = ops::media::resolve(state, &args, server.events.media_progress()).await?;
            resolved.url = server.media.publish(&resolved.url, resolved.probe.clone());
            reply(resolved)
        }
        "media_resolve_all" => {
            let Wrapped::<ResolveArgs> { args } = parse(args)?;
            reply(ops::media::resolve_all(state, &args, server.events.media_progress()).await?)
        }
        "media_cancel" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                request_id: String,
            }
            let A { request_id } = parse(args)?;
            ops::media::cancel(state, &request_id);
            reply(())
        }
        "media_probe" => {
            let UrlArg { url } = parse(args)?;
            reply(ops::media::probe(&url).await?)
        }

        "torrent_stats" => {
            let InfoHashArg { info_hash } = parse(args)?;
            reply(ops::torrents::stats(state, &info_hash))
        }
        "session_destroy" => {
            let InfoHashArg { info_hash } = parse(args)?;
            ops::torrents::destroy(state, &info_hash);
            reply(())
        }

        "download_start" => {
            let Wrapped::<DownloadStartArgs> { args } = parse(args)?;
            reply(ops::downloads::start(state, args).await?)
        }
        "download_start_group" => {
            let Wrapped::<DownloadStartGroupArgs> { args } = parse(args)?;
            reply(ops::downloads::start_group(state, args).await?)
        }
        "download_list" => reply(ops::downloads::list(state)),
        "download_remove" => {
            let IdArg { id } = parse(args)?;
            reply(ops::downloads::remove(state, id).await?)
        }
        "download_pause" => {
            let IdArg { id } = parse(args)?;
            reply(ops::downloads::pause(state, id).await?)
        }
        "download_resume" => {
            let IdArg { id } = parse(args)?;
            reply(ops::downloads::resume(state, id).await?)
        }
        "download_play" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct A {
                id: String,
                #[serde(default)]
                file_index: Option<usize>,
            }
            let A { id, file_index } = parse(args)?;
            let url = ops::downloads::play(state, id, file_index).await?;
            reply(server.media.publish(&url, None))
        }
        "download_files" => {
            let IdArg { id } = parse(args)?;
            reply(ops::downloads::files(state, id).await?)
        }
        "torrent_parse_file" => {
            #[derive(Deserialize)]
            struct A {
                bytes: Vec<u8>,
            }
            let A { bytes } = parse(args)?;
            reply(ops::downloads::parse_torrent_file(&bytes)?)
        }

        // The phone remote (server/remote.rs).
        "remote_info" => {
            #[derive(Deserialize)]
            struct A {
                /// The address the page is at (web/bridge.js adds it).
                #[serde(default)]
                origin: Option<String>,
            }
            let A { origin } = parse(args)?;
            reply(super::remote::info(server, origin.as_deref()))
        }
        "remote_push_state" => {
            #[derive(Deserialize)]
            struct A {
                payload: Value,
            }
            let A { payload } = parse(args)?;
            super::remote::push_state(server, payload);
            reply(())
        }
        "remote_set_approval" => {
            #[derive(Deserialize)]
            struct A {
                id: u64,
                approved: bool,
            }
            let Wrapped::<A> { args } = parse(args)?;
            server.app.remote.set_approval(args.id, args.approved);
            reply(())
        }
        "remote_remember_device" => {
            #[derive(Deserialize)]
            struct A {
                id: u64,
            }
            let Wrapped::<A> { args } = parse(args)?;
            server.app.remote.remember_device(args.id);
            reply(())
        }
        "remote_forget_device" => {
            #[derive(Deserialize)]
            struct A {
                #[serde(default)]
                id: Option<u64>,
                #[serde(default)]
                key: Option<String>,
            }
            let Wrapped::<A> { args } = parse(args)?;
            server.app.remote.forget_device(args.id, args.key.as_deref());
            reply(())
        }

        // Accounts (accounts.rs): who is signed in, their password, and the
        // administrator's list. Errors are codes the page words.
        "account_status" => reply(match viewer {
            Viewer::Guest => json!({ "account": null }),
            Viewer::Account(id) => json!({ "account": server.accounts.get(id) }),
        }),
        "account_password" => {
            let Viewer::Account(id) = viewer else {
                return Err("no-account".to_string());
            };
            #[derive(Deserialize)]
            struct A {
                current: String,
                next: String,
            }
            let A { current, next } = parse(args)?;
            server
                .accounts
                .set_password(id, &current, &next)
                .await
                .map_err(|e| e.code().to_string())?;
            reply(())
        }
        "accounts_list" => {
            admin(server, viewer)?;
            reply(server.accounts.list())
        }
        "account_create" => {
            admin(server, viewer)?;
            #[derive(Deserialize)]
            struct A {
                username: String,
                password: String,
                #[serde(default)]
                admin: bool,
            }
            let A { username, password, admin } = parse(args)?;
            let account = server
                .accounts
                .create(&username, &password, admin)
                .await
                .map_err(|e| e.code().to_string())?;
            tracing::info!("[accounts] {} made", account.username);
            reply(account)
        }
        "account_delete" => {
            admin(server, viewer)?;
            let IdArg { id } = parse(args)?;
            server.accounts.delete(&id).map_err(|e| e.code().to_string())?;
            server.auth.close_account(&id);
            server.profiles.remove(&id).await;
            reply(())
        }

        _ => Err(format!("unknown command: {command}")),
    }
}
