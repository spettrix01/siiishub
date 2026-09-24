//! The phone as a remote control, on the server's own port: the page at
//! `/remote/` and its WebSocket at `/remote/ws` (crate::remote does the
//! pairing and the commands). From the home network they need no password,
//! as the app's on a PC; from elsewhere the phone signs in first (auth.rs).
//! The screen that approves a phone is any page of the interface: they all
//! get the events.

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Serialize;
use serde_json::Value;

use crate::remote::{self as hub, DeviceView};

use super::Server;

pub async fn page(State(server): State<Server>) -> Response {
    hub::phone_page(&server.app)
}

/// `/remote` without the slash: the page's relative address of its
/// WebSocket needs it.
pub async fn page_slash() -> Response {
    Redirect::permanent("/remote/").into_response()
}

pub async fn socket(
    State(server): State<Server>,
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    // Behind a reverse proxy the connection is the proxy's: the phone's own
    // address, for the list of devices, is the first it forwards.
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| hub::client_ip(addr));
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    hub::upgrade(ws, server.app.clone(), ip, ua, query.get("token").map(String::as_str))
}

#[derive(Serialize)]
pub struct Iface {
    name: String,
    ip: String,
    url: String,
    qr_svg: String,
    #[serde(rename = "isVirtual")]
    is_virtual: bool,
}

/// What the settings show (the app's `remote_info`).
#[derive(Serialize)]
pub struct Info {
    running: bool,
    port: u16,
    interfaces: Vec<Iface>,
    clients: u32,
    devices: Vec<DeviceView>,
}

/// The phone page's address is the one the screen opened the interface at
/// (`origin`, from the page): what the server sees of itself in a container
/// (its network, its port inside) is no use to a phone.
pub fn info(server: &Server, origin: Option<&str>) -> Info {
    let interfaces = origin
        .and_then(|o| url::Url::parse(o).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
        .map(|u| {
            let host = u.host_str().unwrap_or_default().to_string();
            let url = format!("{}/remote/", u.origin().ascii_serialization());
            Iface {
                name: "SIIISHUB".to_string(),
                ip: host,
                qr_svg: hub::make_qr_svg(&url).unwrap_or_default(),
                url,
                is_virtual: false,
            }
        })
        .into_iter()
        .collect();
    Info {
        running: true,
        port: 0,
        interfaces,
        clients: server.app.remote.client_count(),
        devices: server.app.remote.devices(),
    }
}

/// The interface's state for the phones (the app's `remote_push_state`).
pub fn push_state(server: &Server, payload: Value) {
    let Value::Object(mut state) = payload else {
        return;
    };
    state.insert("type".to_string(), Value::String("state".to_string()));
    if let Ok(json) = serde_json::to_string(&Value::Object(state)) {
        server.app.remote.broadcast_state(json);
    }
}
