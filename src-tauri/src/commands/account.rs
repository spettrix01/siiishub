use std::sync::Arc;

use tauri::State;

use crate::account::{Account, AccountStatus};
use crate::ops::sync::{Applied, Stores};
use crate::state::AppState;

use super::shared::CmdResult;

#[tauri::command]
pub async fn sync_status(account: State<'_, Arc<Account>>) -> CmdResult<AccountStatus> {
    Ok(account.status())
}

#[tauri::command]
pub async fn sync_sign_in(
    state: State<'_, Arc<AppState>>,
    account: State<'_, Arc<Account>>,
    server: String,
    username: String,
    password: String,
    device: String,
) -> CmdResult<Applied> {
    account.sign_in(Stores::from(&**state), &server, &username, &password, &device).await
}

/// Syncs at once: Settings → Account does when it opens, to tell whether
/// the server answers.
#[tauri::command]
pub async fn sync_now(
    state: State<'_, Arc<AppState>>,
    account: State<'_, Arc<Account>>,
) -> CmdResult<Applied> {
    account.sync(Stores::from(&**state)).await
}

#[tauri::command]
pub async fn sync_sign_out(account: State<'_, Arc<Account>>) -> CmdResult<()> {
    account.sign_out().await;
    Ok(())
}
