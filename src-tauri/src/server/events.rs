//! Backend events for the pages open in the browser, over one WebSocket per
//! page: what the app sends as Tauri events (`media://progress`, ...) arrives
//! as `{"event": name, "payload": value}` and `web/bridge.js` hands it to
//! the listeners registered with `event.listen`.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use tokio::sync::broadcast;

use super::Server;

#[derive(Clone)]
pub struct Events {
    tx: broadcast::Sender<String>,
}

impl Events {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    /// Sends `payload` to every open page as the event `name`.
    pub fn emit(&self, name: &str, payload: serde_json::Value) {
        let msg = serde_json::json!({ "event": name, "payload": payload }).to_string();
        // No page listening is not an error.
        let _ = self.tx.send(msg);
    }

    /// The resolve steps for the loading screen (`media://progress`).
    pub fn media_progress(&self) -> impl Fn(&str) + Clone + Send + Sync + 'static {
        let events = self.clone();
        move |msg: &str| events.emit("media://progress", serde_json::Value::String(msg.to_string()))
    }
}

pub async fn socket(ws: WebSocketUpgrade, State(server): State<Server>) -> Response {
    let rx = server.events.tx.subscribe();
    ws.on_upgrade(move |socket| pump(socket, rx))
}

async fn pump(mut socket: WebSocket, mut rx: broadcast::Receiver<String>) {
    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(text) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
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
