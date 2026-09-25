//! Backend events for the pages open in the browser, over one WebSocket per
//! page: what the app sends as Tauri events (`media://progress`, ...) arrives
//! as `{"event": name, "payload": value}` and `web/bridge.js` hands it to
//! the listeners registered with `event.listen`. An event goes to every page,
//! or only to the pages of one profile (an account's, or the server's own).

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, State};
use axum::response::Response;
use tokio::sync::broadcast;

use super::accounts::Viewer;
use super::Server;

/// An event on its way, and whose pages it is for (none: every page's).
type Outgoing = Arc<(Option<Viewer>, String)>;

#[derive(Clone)]
pub struct Events {
    tx: broadcast::Sender<Outgoing>,
}

impl Events {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    fn send(&self, to: Option<Viewer>, name: &str, payload: serde_json::Value) {
        let msg = serde_json::json!({ "event": name, "payload": payload }).to_string();
        // No page listening is not an error.
        let _ = self.tx.send(Arc::new((to, msg)));
    }

    /// Sends `payload` to every open page as the event `name`.
    pub fn emit(&self, name: &str, payload: serde_json::Value) {
        self.send(None, name, payload);
    }

    /// Sends `payload` to the pages of `viewer`'s profile only.
    pub fn emit_to(&self, viewer: Viewer, name: &str, payload: serde_json::Value) {
        self.send(Some(viewer), name, payload);
    }

    /// The resolve steps for the loading screen (`media://progress`).
    pub fn media_progress(&self) -> impl Fn(&str) + Clone + Send + Sync + 'static {
        let events = self.clone();
        move |msg: &str| events.emit("media://progress", serde_json::Value::String(msg.to_string()))
    }
}

pub async fn socket(
    ws: WebSocketUpgrade,
    State(server): State<Server>,
    Extension(viewer): Extension<Viewer>,
) -> Response {
    let rx = server.events.tx.subscribe();
    ws.on_upgrade(move |socket| pump(socket, rx, viewer))
}

async fn pump(mut socket: WebSocket, mut rx: broadcast::Receiver<Outgoing>, viewer: Viewer) {
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(msg) => {
                    let (to, text) = &*msg;
                    if to.as_ref().is_some_and(|to| *to != viewer) {
                        continue;
                    }
                    if socket.send(Message::Text(text.clone().into())).await.is_err() {
                        break;
                    }
                }
                // A slow page misses a few progress lines, nothing more.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                // The page sends nothing; axum answers the pings.
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
}
