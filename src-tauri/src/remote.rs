//! The phone as a remote control: a page for its browser and a WebSocket
//! whose commands the interface carries out (`remote://cmd`), approved on
//! the screen first (`remote://pending`) unless the phone was remembered.
//! The app serves them on a port of their own (`start`); the web server at
//! `/remote/` of its own port (server/mod.rs). Either hands the events to
//! its interface through the emitter it sets.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
#[cfg(feature = "app")]
use axum::extract::{ConnectInfo, Query, State as AxState};
#[cfg(feature = "app")]
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};
#[cfg(feature = "app")]
use axum::routing::get;
#[cfg(feature = "app")]
use axum::Router;
use futures::{SinkExt, StreamExt};
use parking_lot::Mutex;
use qrcode::render::svg;
use qrcode::QrCode;
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

use crate::settings::{RemoteDevice, Settings, SettingsStore};
use crate::state::AppState;

const PHONE_PAGE: &str = include_str!("remote_page.html");

/// Hands an event to the interface: Tauri's in the app, the pages'
/// WebSocket in the web server.
pub type Emit = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
struct RemoteCmd {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Approval {
    Pending,
    Approved,
    Denied,
}

#[cfg(feature = "app")]
#[derive(Clone)]
struct AxumCtx {
    state: Arc<AppState>,
}

/// A live WebSocket client.
#[derive(Debug, Clone)]
struct ClientInfo {
    id: u64,
    /// Pairing token of a remembered device.
    token: Option<String>,
    ip: String,
    device: String,
    connected_at: u64,
    approved: bool,
    remembered: bool,
}

/// What the settings UI shows: connected clients plus remembered devices that
/// are currently offline. `id` identifies a live connection, `key` a stored
/// pairing (a fingerprint of the token, never the token itself).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceView {
    pub id: Option<u64>,
    pub key: String,
    pub ip: String,
    pub device: String,
    pub connected_at: u64,
    pub approved: bool,
    pub remembered: bool,
    pub online: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingDevice {
    pub id: u64,
    pub ip: String,
    pub device: String,
}

pub struct RemoteController {
    inner: Arc<Inner>,
}

struct Inner {
    settings: SettingsStore,
    state_tx: broadcast::Sender<String>,
    handle: Mutex<Option<JoinHandle<()>>>,
    clients: Mutex<Vec<ClientInfo>>,
    approvals: Mutex<HashMap<u64, watch::Sender<Approval>>>,
    /// Per-client channel for messages addressed to a single phone
    /// (pairing token, unpairing notice).
    direct: Mutex<HashMap<u64, mpsc::UnboundedSender<String>>>,
    next_id: AtomicU64,
    shutdown: Mutex<Option<watch::Sender<bool>>>,
    emit: Mutex<Option<Emit>>,
    /// Served by the web server's own listener, not one of its own.
    mounted: AtomicBool,
}

impl RemoteController {
    pub fn new(settings: SettingsStore) -> Self {
        let (state_tx, _) = broadcast::channel::<String>(32);
        Self {
            inner: Arc::new(Inner {
                settings,
                state_tx,
                handle: Mutex::new(None),
                clients: Mutex::new(Vec::new()),
                approvals: Mutex::new(HashMap::new()),
                direct: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
                shutdown: Mutex::new(None),
                emit: Mutex::new(None),
                mounted: AtomicBool::new(false),
            }),
        }
    }

    /// Where the events for the interface go.
    pub fn set_emitter(&self, emit: Emit) {
        *self.inner.emit.lock() = Some(emit);
    }

    /// The web server serves the phone page and its WebSocket itself.
    #[cfg_attr(not(feature = "server"), allow(dead_code))]
    pub fn mount(&self) {
        self.inner.mounted.store(true, Ordering::Relaxed);
    }

    fn emit(&self, name: &str, payload: impl serde::Serialize) {
        let emit = self.inner.emit.lock().clone();
        if let (Some(emit), Ok(payload)) = (emit, serde_json::to_value(payload)) {
            emit(name, payload);
        }
    }

    pub fn is_running(&self) -> bool {
        self.inner.handle.lock().is_some() || self.inner.mounted.load(Ordering::Relaxed)
    }

    pub fn client_count(&self) -> u32 {
        self.inner.clients.lock().len() as u32
    }

    /// Connected clients first, then remembered devices that are offline.
    pub fn devices(&self) -> Vec<DeviceView> {
        let clients = self.inner.clients.lock().clone();
        let mut out: Vec<DeviceView> = clients
            .iter()
            .map(|c| DeviceView {
                id: Some(c.id),
                key: c.token.as_deref().map(token_key).unwrap_or_default(),
                ip: c.ip.clone(),
                device: c.device.clone(),
                connected_at: c.connected_at,
                approved: c.approved,
                remembered: c.remembered,
                online: true,
            })
            .collect();
        let online_tokens: HashSet<String> = clients.iter().filter_map(|c| c.token.clone()).collect();
        let settings = self.inner.settings.read();
        for d in settings
            .remote_devices
            .iter()
            .filter(|d| !online_tokens.contains(&d.token))
        {
            out.push(DeviceView {
                id: None,
                key: token_key(&d.token),
                ip: d.ip.clone(),
                device: d.device.clone(),
                connected_at: d.last_seen,
                approved: false,
                remembered: true,
                online: false,
            });
        }
        out
    }

    fn emit_clients(&self) {
        self.emit("remote://client-count", self.devices());
    }

    pub fn broadcast_state(&self, payload: String) {
        let _ = self.inner.state_tx.send(payload);
    }

    fn send_direct(&self, id: u64, msg: serde_json::Value) {
        if let Some(tx) = self.inner.direct.lock().get(&id) {
            let _ = tx.send(msg.to_string());
        }
    }

    /// An approval only covers the current connection: a device that is not
    /// remembered is asked again every time it connects.
    pub fn set_approval(&self, id: u64, approved: bool) {
        if approved {
            if let Some(c) = self.inner.clients.lock().iter_mut().find(|c| c.id == id) {
                c.approved = true;
            }
        }

        if let Some(tx) = self.inner.approvals.lock().get(&id) {
            let _ = tx.send(if approved {
                Approval::Approved
            } else {
                Approval::Denied
            });
        }
        self.emit_clients();
    }

    /// Pairs an approved, connected client on the user's request: a fresh
    /// token is stored in the settings and handed to the phone, which will
    /// present it on every later connection to skip the approval prompt.
    pub fn remember_device(&self, id: u64) {
        let token = new_token();
        let entry = {
            let mut clients = self.inner.clients.lock();
            let Some(c) = clients.iter_mut().find(|c| c.id == id && c.approved) else {
                return;
            };
            if c.remembered {
                return;
            }
            c.token = Some(token.clone());
            c.remembered = true;
            let now = now_millis();
            RemoteDevice {
                token: token.clone(),
                device: c.device.clone(),
                ip: c.ip.clone(),
                first_seen: now,
                last_seen: now,
            }
        };
        store_device(&self.inner.settings, entry);
        self.send_direct(id, serde_json::json!({ "type": "paired", "token": token }));
        self.emit_clients();
    }

    /// Drops a stored pairing, addressed either by the live connection (`id`)
    /// or by the stored entry (`key`, for devices currently offline). A phone
    /// still connected keeps working for now but is told to drop its token, so
    /// it will have to be approved again next time.
    pub fn forget_device(&self, id: Option<u64>, key: Option<&str>) {
        let mut token: Option<String> = None;
        if let Some(id) = id {
            if let Some(c) = self.inner.clients.lock().iter().find(|c| c.id == id) {
                token = c.token.clone();
            }
        }
        if token.is_none() {
            if let Some(key) = key {
                token = self
                    .inner
                    .settings
                    .read()
                    .remote_devices
                    .iter()
                    .find(|d| token_key(&d.token) == key)
                    .map(|d| d.token.clone());
            }
        }
        let Some(token) = token else {
            return;
        };

        let notify: Vec<u64> = {
            let mut clients = self.inner.clients.lock();
            let mut ids = Vec::new();
            for c in clients
                .iter_mut()
                .filter(|c| c.token.as_deref() == Some(token.as_str()))
            {
                c.token = None;
                c.remembered = false;
                ids.push(c.id);
            }
            ids
        };
        let mut next = self.inner.settings.read();
        next.remote_devices.retain(|d| d.token != token);
        persist_settings(&self.inner.settings, next);
        for cid in notify {
            self.send_direct(cid, serde_json::json!({ "type": "unpaired" }));
        }
        tracing::info!("[remote] pairing forgotten ({})", token_key(&token));
        self.emit_clients();
    }
}

/// The QR code of the phone page's address, as SVG.
pub fn make_qr_svg(url: &str) -> Option<String> {
    let code = QrCode::new(url.as_bytes()).ok()?;
    Some(
        code.render::<svg::Color>()
            .min_dimensions(200, 200)
            .quiet_zone(false)
            .dark_color(svg::Color("#1a0f04"))
            .light_color(svg::Color("#ffffff"))
            .build(),
    )
}

fn new_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Short, non-reversible identifier of a pairing token for the UI.
fn token_key(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn store_device(settings: &SettingsStore, dev: RemoteDevice) {
    let mut next = settings.read();
    // A device pairing again from the same address lost its previous token
    // (browser data cleared, page reloaded before it was stored...): replace
    // the stale entry instead of piling up duplicates.
    next.remote_devices
        .retain(|d| d.token != dev.token && !(d.device == dev.device && d.ip == dev.ip));
    tracing::info!("[remote] device remembered: {} ({})", dev.device, dev.ip);
    next.remote_devices.push(dev);
    persist_settings(settings, next);
}

/// Refreshes the last-seen data of a remembered device on reconnect and drops
/// stale entries left behind for the same device at the same address.
fn touch_device(settings: &SettingsStore, token: &str, ip: &str) {
    let mut next = settings.read();
    let Some(d) = next.remote_devices.iter_mut().find(|d| d.token == token) else {
        return;
    };
    d.ip = ip.to_string();
    d.last_seen = now_millis();
    let device = d.device.clone();
    next.remote_devices
        .retain(|d| d.token == token || !(d.device == device && d.ip == ip));
    persist_settings(settings, next);
}

/// Settings writes are async and the callers hold no locks: fire and forget.
fn persist_settings(settings: &SettingsStore, next: Settings) {
    let store = settings.clone();
    crate::util::spawn(async move {
        if let Err(e) = store.write(next).await {
            tracing::warn!("[remote] saving remembered devices failed: {e:#}");
        }
    });
}

#[cfg(feature = "app")]
fn is_likely_virtual_name(name: &str) -> bool {
    let l = name.to_lowercase();
    l.contains("virtual")
        || l.contains("vbox")
        || l.contains("vmware")
        || l.contains("vmnet")
        || l.contains("hyper-v")
        || l.contains("docker")
        || l.contains("wsl")
        || l.contains("default switch")
        || l.contains("vethernet")
        || l.contains("npcap")
        || l.contains("tap-")
        || l.contains("tunnel")
        || l.contains("tailscale")
}

#[cfg(feature = "app")]
fn is_likely_virtual_ip(addr: &std::net::Ipv4Addr) -> bool {
    let o = addr.octets();
    if o[0] == 172 && (16..=31).contains(&o[1]) {
        return true;
    }
    if o[0] == 192 && o[1] == 168 && o[2] == 56 {
        return true;
    }
    if o[0] == 192 && o[1] == 168 && o[2] == 99 {
        return true;
    }
    false
}

/// Every usable IPv4 interface as (name, ip, is_virtual), physical ones first.
/// Virtual adapters (VPN, hypervisor bridges) are included so their addresses
/// can be listed too — e.g. reaching the remote over Tailscale.
#[cfg(feature = "app")]
pub fn local_ipv4_interfaces() -> Vec<(String, String, bool)> {
    let mut real: Vec<(String, String, bool)> = Vec::new();
    let mut maybe_virtual: Vec<(String, String, bool)> = Vec::new();
    if let Ok(list) = local_ip_address::list_afinet_netifas() {
        for (name, addr) in list {
            if let IpAddr::V4(v4) = addr {
                let s = v4.octets();
                if s[0] == 127 {
                    continue;
                }
                if s[0] == 169 && s[1] == 254 {
                    continue;
                }
                if name.to_lowercase().contains("loopback") {
                    continue;
                }
                let is_virtual = is_likely_virtual_name(&name) || is_likely_virtual_ip(&v4);
                let entry = (name, v4.to_string(), is_virtual);
                if is_virtual {
                    maybe_virtual.push(entry);
                } else {
                    real.push(entry);
                }
            }
        }
    }
    real.sort();
    real.dedup();
    maybe_virtual.sort();
    maybe_virtual.dedup();
    real.extend(maybe_virtual);
    real
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn client_ip(addr: SocketAddr) -> String {
    match addr.ip() {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(|v4| v4.to_string())
            .unwrap_or_else(|| v6.to_string()),
    }
}

fn device_label(ua: &str) -> String {
    let u = ua.to_lowercase();
    if u.contains("iphone") {
        "iPhone"
    } else if u.contains("ipad") {
        "iPad"
    } else if u.contains("ipod") {
        "iPod"
    } else if u.contains("android") {
        "Android"
    } else if u.contains("cros") {
        "Chromebook"
    } else if u.contains("windows") {
        "PC Windows"
    } else if u.contains("macintosh") || u.contains("mac os") {
        "Mac"
    } else if u.contains("linux") {
        "PC Linux"
    } else {
        "Dispositivo"
    }
    .to_string()
}

#[cfg(feature = "app")]
pub async fn reconfigure(state: Arc<AppState>, enabled: bool, port: u16) {
    stop(&state.remote).await;
    if enabled {
        if let Err(e) = start(state.clone(), port).await {
            tracing::warn!("[remote] failed to start server on port {port}: {e:#}");
        }
    }
}

#[cfg(feature = "app")]
async fn stop(controller: &RemoteController) {
    if let Some(tx) = controller.inner.shutdown.lock().take() {
        let _ = tx.send(true);
    }
    let handle = controller.inner.handle.lock().take();
    if let Some(h) = handle {
        h.abort();
        let _ = h.await.ok();
        tracing::info!("[remote] server stopped");
    }
    controller.inner.clients.lock().clear();
    controller.inner.approvals.lock().clear();
    controller.inner.direct.lock().clear();
}

/// The app's own listener for the phone page and its WebSocket.
#[cfg(feature = "app")]
pub async fn start(state: Arc<AppState>, port: u16) -> Result<(), String> {
    let ctx = AxumCtx { state: state.clone() };
    let router: Router = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_upgrade_handler))
        .with_state(ctx);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => return Err(format!("bind 0.0.0.0:{port} failed: {e}")),
    };

    let (shutdown_tx, _) = watch::channel(false);
    *state.remote.inner.shutdown.lock() = Some(shutdown_tx);

    let handle = tokio::spawn(async move {
        let service = router.into_make_service_with_connect_info::<SocketAddr>();
        if let Err(e) = axum::serve(listener, service).await {
            tracing::warn!("[remote] axum::serve ended: {e:#}");
        }
    });

    *state.remote.inner.handle.lock() = Some(handle);
    tracing::info!("[remote] server listening on 0.0.0.0:{port}");
    Ok(())
}

#[cfg(feature = "app")]
async fn index_handler(AxState(ctx): AxState<AxumCtx>) -> Response {
    phone_page(&ctx.state)
}

/// The phone page, in the interface's theme. Served with `no-store` so a
/// reopened tab always runs the current version (an old page would not know
/// how to keep its pairing token).
pub fn phone_page(state: &AppState) -> Response {
    let mut attrs = String::new();
    if state.userdata.get("siiishub-theme").as_deref() == Some("light") {
        attrs.push_str(" data-theme=\"light\"");
    }
    match state.userdata.get("siiishub-accent").as_deref() {
        Some("purple") => attrs.push_str(" data-accent=\"purple\""),
        Some("teal") => attrs.push_str(" data-accent=\"teal\""),
        _ => {}
    }
    let page = if attrs.is_empty() {
        PHONE_PAGE.to_string()
    } else {
        PHONE_PAGE.replacen("<html lang=\"it\">", &format!("<html lang=\"it\"{attrs}>"), 1)
    };
    ([(axum::http::header::CACHE_CONTROL, "no-store")], Html(page)).into_response()
}

#[cfg(feature = "app")]
async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    AxState(ctx): AxState<AxumCtx>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let ua = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    upgrade(ws, ctx.state, client_ip(addr), ua, query.get("token").map(String::as_str))
}

/// A phone's WebSocket: `token` is the pairing it presents, if any.
pub fn upgrade(
    ws: WebSocketUpgrade,
    state: Arc<AppState>,
    ip: String,
    user_agent: &str,
    token: Option<&str>,
) -> Response {
    let ua = user_agent.to_string();
    let token = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    ws.on_upgrade(move |socket| ws_session(socket, state, ip, ua, token))
}

fn hello_msg(lang: &str) -> String {
    serde_json::json!({ "type": "hello", "lang": lang }).to_string()
}

fn pending_msg(lang: &str) -> String {
    serde_json::json!({ "type": "pending", "lang": lang }).to_string()
}

async fn ws_session(
    socket: WebSocket,
    state: Arc<AppState>,
    ip: String,
    ua: String,
    token: Option<String>,
) {
    let (mut sender, mut receiver) = socket.split();
    let device = device_label(&ua);

    // Only a client presenting a valid pairing token (remembered by the user)
    // skips the approval prompt; everyone else is asked on every connection.
    let remembered_token = token.filter(|t| {
        state
            .settings
            .read()
            .remote_devices
            .iter()
            .any(|d| &d.token == t)
    });
    if let Some(t) = &remembered_token {
        touch_device(&state.settings, t, &ip);
    }
    let pre_approved = remembered_token.is_some();
    let (atx, arx) = watch::channel(if pre_approved {
        Approval::Approved
    } else {
        Approval::Pending
    });

    let client_id = state.remote.inner.next_id.fetch_add(1, Ordering::Relaxed);
    {
        let mut clients = state.remote.inner.clients.lock();
        clients.push(ClientInfo {
            id: client_id,
            token: remembered_token.clone(),
            ip: ip.clone(),
            device: device.clone(),
            connected_at: now_millis(),
            approved: pre_approved,
            remembered: remembered_token.is_some(),
        });
    }
    state
        .remote
        .inner
        .approvals
        .lock()
        .insert(client_id, atx);
    let (direct_tx, mut direct_rx) = mpsc::unbounded_channel::<String>();
    state
        .remote
        .inner
        .direct
        .lock()
        .insert(client_id, direct_tx);

    tracing::info!(
        "[remote] client connected ({device} {ip}) — {} (total {})",
        if pre_approved { "remembered" } else { "pending" },
        state.remote.client_count()
    );
    state.remote.emit_clients();
    if !pre_approved {
        state.remote.emit(
            "remote://pending",
            PendingDevice {
                id: client_id,
                ip: ip.clone(),
                device: device.clone(),
            },
        );
    }

    let lang = state.settings.read().language;
    let mut rx = state.remote.inner.state_tx.subscribe();
    let mut shutdown_rx = state
        .remote
        .inner
        .shutdown
        .lock()
        .as_ref()
        .map(|tx| tx.subscribe());

    let state_for_recv = state.clone();
    let arx_recv = arx.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(t) if *arx_recv.borrow() == Approval::Approved => {
                    if let Ok(cmd) = serde_json::from_str::<RemoteCmd>(&t) {
                        state_for_recv.remote.emit("remote://cmd", cmd);
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    });

    let mut arx_send = arx;
    let mut send_task = tokio::spawn(async move {
        let hello = hello_msg(&lang);
        let pending = pending_msg(&lang);
        let denied = serde_json::json!({ "type": "denied" }).to_string();

        let initial_state = *arx_send.borrow();
        let mut approved = initial_state == Approval::Approved;
        let first = match initial_state {
            Approval::Approved => hello.clone(),
            Approval::Denied => denied.clone(),
            Approval::Pending => pending.clone(),
        };
        if sender.send(Message::Text(first.into())).await.is_err() {
            return;
        }
        if initial_state == Approval::Denied {
            return;
        }

        let mut ping = tokio::time::interval(std::time::Duration::from_secs(15));
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                changed = arx_send.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let st = *arx_send.borrow();
                    match st {
                        Approval::Approved => {
                            if !approved {
                                approved = true;
                                if sender.send(Message::Text(hello.clone().into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Approval::Denied => {
                            let _ = sender.send(Message::Text(denied.clone().into())).await;
                            break;
                        }
                        Approval::Pending => {}
                    }
                }
                direct = direct_rx.recv() => {
                    match direct {
                        Some(payload) => {
                            if sender.send(Message::Text(payload.into())).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                msg = rx.recv() => {
                    match msg {
                        Ok(payload) => {
                            if approved
                                && sender.send(Message::Text(payload.into())).await.is_err()
                            {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = ping.tick() => {
                    if sender.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    tokio::select! {
        _ = &mut recv_task => {},
        _ = &mut send_task => {},
        _ = await_shutdown(&mut shutdown_rx) => {},
    }
    recv_task.abort();
    send_task.abort();

    state
        .remote
        .inner
        .clients
        .lock()
        .retain(|c| c.id != client_id);
    state.remote.inner.approvals.lock().remove(&client_id);
    state.remote.inner.direct.lock().remove(&client_id);
    tracing::info!(
        "[remote] client disconnected (total {})",
        state.remote.client_count()
    );
    state.remote.emit_clients();
}

async fn await_shutdown(rx: &mut Option<watch::Receiver<bool>>) {
    match rx {
        Some(r) => {
            let _ = r.wait_for(|v| *v).await;
        }
        None => std::future::pending::<()>().await,
    }
}
