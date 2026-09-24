use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::remote::{local_ipv4_interfaces, make_qr_svg, DeviceView};
use crate::state::AppState;

use super::shared::CmdResult;

#[derive(Debug, Serialize)]
pub struct RemoteIface {
    pub name: String,
    pub ip: String,
    pub url: String,
    pub qr_svg: String,
    #[serde(rename = "isVirtual")]
    pub is_virtual: bool,
}

#[derive(Debug, Serialize)]
pub struct RemoteInfo {
    pub running: bool,
    pub port: u16,
    pub interfaces: Vec<RemoteIface>,
    pub clients: u32,
    pub devices: Vec<DeviceView>,
}

#[tauri::command]
pub async fn remote_info(state: State<'_, Arc<AppState>>) -> CmdResult<RemoteInfo> {
    let cfg = state.settings.read();
    let port = if cfg.remote_port == 0 { 9871 } else { cfg.remote_port };
    let interfaces = if state.remote.is_running() {
        local_ipv4_interfaces()
            .into_iter()
            .map(|(name, ip, is_virtual)| {
                let url = format!("http://{ip}:{port}/");
                let qr_svg = make_qr_svg(&url).unwrap_or_default();
                RemoteIface {
                    name,
                    ip,
                    url,
                    qr_svg,
                    is_virtual,
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(RemoteInfo {
        running: state.remote.is_running(),
        port,
        interfaces,
        clients: state.remote.client_count(),
        devices: state.remote.devices(),
    })
}

#[tauri::command]
pub async fn remote_push_state(
    state: State<'_, Arc<AppState>>,
    payload: Value,
) -> CmdResult<()> {
    if !state.remote.is_running() {
        return Ok(());
    }
    let mut obj = match payload {
        Value::Object(m) => m,
        _ => return Ok(()),
    };
    obj.insert("type".to_string(), Value::String("state".to_string()));
    let json = match serde_json::to_string(&Value::Object(obj)) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    state.remote.broadcast_state(json);
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ApprovalArgs {
    pub id: u64,
    pub approved: bool,
}

#[tauri::command]
pub async fn remote_set_approval(
    state: State<'_, Arc<AppState>>,
    args: ApprovalArgs,
) -> CmdResult<()> {
    state.remote.set_approval(args.id, args.approved);
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct RememberArgs {
    pub id: u64,
}

/// Pairs a connected, approved device so it can reconnect without asking.
#[tauri::command]
pub async fn remote_remember_device(
    state: State<'_, Arc<AppState>>,
    args: RememberArgs,
) -> CmdResult<()> {
    state.remote.remember_device(args.id);
    Ok(())
}

/// A connected device is addressed by `id`, an offline remembered one by
/// the `key` shown in the device list.
#[derive(Debug, Deserialize)]
pub struct ForgetArgs {
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(default)]
    pub key: Option<String>,
}

/// Drops a stored pairing: the device will have to be approved again the
/// next time it connects.
#[tauri::command]
pub async fn remote_forget_device(
    state: State<'_, Arc<AppState>>,
    args: ForgetArgs,
) -> CmdResult<()> {
    state.remote.forget_device(args.id, args.key.as_deref());
    Ok(())
}
