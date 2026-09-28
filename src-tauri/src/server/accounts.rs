//! The server's accounts. The first one, the administrator's, is made on the
//! login page from the home network; the administrator makes the others in
//! the settings. An account is used from the browser and from the apps,
//! which sync their settings and user data with its profile here
//! (`ops::sync`): the profile lives in `accounts/<id>/` of the data folder,
//! over the server's torrents and downloads. Whoever enters without an
//! account uses the server's own profile, the one of the data folder.
//!
//! Passwords are kept as Argon2 hashes. An app keeps a token of its own
//! instead of the password, one per device, kept here as its SHA-256 with
//! what the device's requests tell of it: its name, the app's version, the
//! address it came from and when. The administrator sees an account's
//! devices in the settings, and signs one out there.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::state::AppState;

const MIN_PASSWORD: usize = 6;
const MAX_USERNAME: usize = 32;

/// Who a request is from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Viewer {
    /// Without an account: the server's own profile.
    Guest,
    Account(String),
}

#[derive(Clone, Serialize, Deserialize)]
struct Account {
    id: String,
    username: String,
    hash: String,
    #[serde(default)]
    admin: bool,
    #[serde(default)]
    created: u64,
    #[serde(default)]
    tokens: Vec<DeviceToken>,
}

#[derive(Clone, Serialize, Deserialize)]
struct DeviceToken {
    /// SHA-256 of the token, in hex.
    hash: String,
    /// What the app said it is when it signed in: Windows, Linux, Android,
    /// Android TV.
    #[serde(default)]
    device: String,
    #[serde(default)]
    created: u64,
    /// The device's own name (a PC's, a phone's model) and the app's
    /// version, as its requests tell them from 1.3.8 on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    version: String,
    /// When and from where the device last reached the server.
    #[serde(default)]
    seen: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    address: String,
}

impl DeviceToken {
    /// How the settings name it: the start of the token's hash, enough to
    /// tell it from the others and nothing to sign in with.
    fn id(&self) -> &str {
        self.hash.get(..16).unwrap_or(&self.hash)
    }

    fn update(&mut self, visit: &Visit) -> bool {
        let mut changed = false;
        for (field, value) in [
            (&mut self.name, &visit.name),
            (&mut self.version, &visit.version),
            (&mut self.address, &visit.address),
        ] {
            if !value.is_empty() && field != value {
                field.clone_from(value);
                changed = true;
            }
        }
        changed
    }
}

/// What an app's request tells of its device (sync_api.rs).
#[derive(Debug, Clone, Default)]
pub struct Visit {
    pub name: String,
    pub version: String,
    pub address: String,
}

/// A device signed in to an account, as the administrator's list shows it.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceView {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub version: String,
    pub created: u64,
    /// 0 when it has not reached this server since the server knows when.
    pub seen: u64,
    pub address: String,
    /// Reached the server in the last two minutes: an open app keeps a
    /// request waiting for news, asked again every 50 seconds.
    pub online: bool,
}

/// Seen this recently, a device is online.
const ONLINE_SECS: u64 = 120;
/// A device seen again is written down at most this often: an open app asks
/// for news every 50 seconds.
const SEEN_SAVE_SECS: u64 = 300;

impl DeviceView {
    fn new(t: &DeviceToken, now: u64) -> Self {
        Self {
            id: t.id().to_string(),
            kind: t.device.clone(),
            name: t.name.clone(),
            version: t.version.clone(),
            created: t.created,
            seen: t.seen,
            address: t.address.clone(),
            online: t.seen > 0 && now.saturating_sub(t.seen) < ONLINE_SECS,
        }
    }
}

/// An account as the settings show it.
#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    pub id: String,
    pub username: String,
    pub admin: bool,
    pub created: u64,
    pub devices: usize,
}

impl From<&Account> for AccountView {
    fn from(a: &Account) -> Self {
        Self {
            id: a.id.clone(),
            username: a.username.clone(),
            admin: a.admin,
            created: a.created,
            devices: a.tokens.len(),
        }
    }
}

/// An account in the administrator's list, with its devices: the ones open
/// now first, then the ones seen last.
#[derive(Debug, Clone, Serialize)]
pub struct AccountEntry {
    pub id: String,
    pub username: String,
    pub admin: bool,
    pub created: u64,
    pub devices: Vec<DeviceView>,
}

impl AccountEntry {
    fn new(a: &Account, now: u64) -> Self {
        let mut devices: Vec<DeviceView> = a.tokens.iter().map(|t| DeviceView::new(t, now)).collect();
        devices.sort_by(|x, y| {
            y.online
                .cmp(&x.online)
                .then(y.seen.cmp(&x.seen))
                .then(y.created.cmp(&x.created))
        });
        Self {
            id: a.id.clone(),
            username: a.username.clone(),
            admin: a.admin,
            created: a.created,
            devices,
        }
    }
}

/// Why an account could not be made or changed, as the page words it
/// (`web.account.error.<code>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountError {
    UsernameInvalid,
    UsernameTaken,
    PasswordShort,
    WrongPassword,
    NotFound,
    LastAdmin,
}

impl AccountError {
    pub fn code(self) -> &'static str {
        match self {
            Self::UsernameInvalid => "username-invalid",
            Self::UsernameTaken => "username-taken",
            Self::PasswordShort => "password-short",
            Self::WrongPassword => "wrong-password",
            Self::NotFound => "not-found",
            Self::LastAdmin => "last-admin",
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn valid_username(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_USERNAME
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Argon2 takes tens of milliseconds by design: off the async threads.
async fn hash_password(password: String) -> Option<String> {
    tokio::task::spawn_blocking(move || {
        use rand::RngCore;
        let mut salt = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut salt);
        let salt = SaltString::encode_b64(&salt).ok()?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .ok()
            .map(|h| h.to_string())
    })
    .await
    .ok()
    .flatten()
}

async fn password_matches(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || {
        PasswordHash::new(&hash)
            .is_ok_and(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
    })
    .await
    .unwrap_or(false)
}

pub struct Accounts {
    path: PathBuf,
    list: Mutex<Vec<Account>>,
    /// When the file was last written.
    saved: Mutex<u64>,
    /// A device signed out by the administrator, or an account deleted: the
    /// apps' waits for news check they are still signed in.
    kicked: tokio::sync::Notify,
}

impl Accounts {
    pub fn load(path: PathBuf) -> Self {
        let list = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            list: Mutex::new(list),
            saved: Mutex::new(now()),
            kicked: tokio::sync::Notify::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.list.lock().is_empty()
    }

    pub fn get(&self, id: &str) -> Option<AccountView> {
        self.list.lock().iter().find(|a| a.id == id).map(AccountView::from)
    }

    pub fn list(&self) -> Vec<AccountEntry> {
        let now = now();
        self.list.lock().iter().map(|a| AccountEntry::new(a, now)).collect()
    }

    pub async fn create(&self, username: &str, password: &str, admin: bool) -> Result<AccountView, AccountError> {
        let username = username.trim();
        if !valid_username(username) {
            return Err(AccountError::UsernameInvalid);
        }
        if password.chars().count() < MIN_PASSWORD {
            return Err(AccountError::PasswordShort);
        }
        if self.find(username).is_some() {
            return Err(AccountError::UsernameTaken);
        }
        let hash = hash_password(password.to_string()).await.ok_or(AccountError::PasswordShort)?;
        let account = Account {
            id: super::random_hex(8),
            username: username.to_string(),
            hash,
            admin,
            created: now(),
            tokens: Vec::new(),
        };
        let view = AccountView::from(&account);
        let mut list = self.list.lock();
        // Another request may have taken the name while the hash was made.
        if list.iter().any(|a| a.username.eq_ignore_ascii_case(username)) {
            return Err(AccountError::UsernameTaken);
        }
        list.push(account);
        self.persist(&list);
        Ok(view)
    }

    fn find(&self, username: &str) -> Option<Account> {
        self.list
            .lock()
            .iter()
            .find(|a| a.username.eq_ignore_ascii_case(username.trim()))
            .cloned()
    }

    /// The account of `username`, when `password` is its password.
    pub async fn verify(&self, username: &str, password: &str) -> Option<AccountView> {
        let account = self.find(username)?;
        password_matches(password.to_string(), account.hash.clone())
            .await
            .then(|| AccountView::from(&account))
    }

    pub async fn set_password(&self, id: &str, current: &str, next: &str) -> Result<(), AccountError> {
        let hash = self
            .list
            .lock()
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.hash.clone())
            .ok_or(AccountError::NotFound)?;
        if !password_matches(current.to_string(), hash).await {
            return Err(AccountError::WrongPassword);
        }
        if next.chars().count() < MIN_PASSWORD {
            return Err(AccountError::PasswordShort);
        }
        let hash = hash_password(next.to_string()).await.ok_or(AccountError::PasswordShort)?;
        let mut list = self.list.lock();
        let account = list.iter_mut().find(|a| a.id == id).ok_or(AccountError::NotFound)?;
        account.hash = hash;
        self.persist(&list);
        Ok(())
    }

    /// Deletes an account; never the last administrator.
    pub fn delete(&self, id: &str) -> Result<(), AccountError> {
        let mut list = self.list.lock();
        let index = list.iter().position(|a| a.id == id).ok_or(AccountError::NotFound)?;
        if list[index].admin && list.iter().filter(|a| a.admin).count() == 1 {
            return Err(AccountError::LastAdmin);
        }
        list.remove(index);
        self.persist(&list);
        drop(list);
        self.kicked.notify_waiters();
        Ok(())
    }

    /// A new token for an app, to sign in with on `device`.
    pub fn issue_token(&self, id: &str, device: &str, visit: &Visit) -> Option<String> {
        let token = super::random_hex(32);
        let mut list = self.list.lock();
        let account = list.iter_mut().find(|a| a.id == id)?;
        let mut entry = DeviceToken {
            hash: token_hash(&token),
            device: device.chars().take(64).collect(),
            created: now(),
            name: String::new(),
            version: String::new(),
            seen: now(),
            address: String::new(),
        };
        entry.update(visit);
        account.tokens.push(entry);
        self.persist(&list);
        Some(token)
    }

    /// The account an app's token signs in to.
    pub fn token_account(&self, token: &str) -> Option<AccountView> {
        let hash = token_hash(token);
        self.list
            .lock()
            .iter()
            .find(|a| a.tokens.iter().any(|t| t.hash == hash))
            .map(AccountView::from)
    }

    /// The account an app's token signs in to, the device seen now. Its
    /// name, version or address changed go to the file at once; the time
    /// alone, every few minutes.
    pub fn visit(&self, token: &str, visit: &Visit) -> Option<AccountView> {
        let hash = token_hash(token);
        let now = now();
        let mut list = self.list.lock();
        let account = list.iter_mut().find(|a| a.tokens.iter().any(|t| t.hash == hash))?;
        let device = account.tokens.iter_mut().find(|t| t.hash == hash)?;
        let changed = device.update(visit);
        device.seen = now;
        let view = AccountView::from(&*account);
        if changed || now.saturating_sub(*self.saved.lock()) >= SEEN_SAVE_SECS {
            self.persist(&list);
        }
        Some(view)
    }

    /// Signs a device out of an account, from the settings: its token stops
    /// working, and its app hears it at once if it is waiting for news.
    pub fn revoke_device(&self, id: &str, device: &str) -> Result<(), AccountError> {
        let mut list = self.list.lock();
        let account = list.iter_mut().find(|a| a.id == id).ok_or(AccountError::NotFound)?;
        let before = account.tokens.len();
        account.tokens.retain(|t| t.id() != device);
        if account.tokens.len() == before {
            return Err(AccountError::NotFound);
        }
        tracing::info!("[accounts] a device of {} signed out from the settings", account.username);
        self.persist(&list);
        drop(list);
        self.kicked.notify_waiters();
        Ok(())
    }

    /// Resolves when a device is signed out or an account deleted; the ones
    /// made before the change resolve too, polled or not.
    pub fn kicked(&self) -> tokio::sync::futures::Notified<'_> {
        self.kicked.notified()
    }

    pub fn revoke_token(&self, token: &str) {
        let hash = token_hash(token);
        let mut list = self.list.lock();
        let mut changed = false;
        for account in list.iter_mut() {
            let before = account.tokens.len();
            account.tokens.retain(|t| t.hash != hash);
            changed |= account.tokens.len() != before;
        }
        if changed {
            self.persist(&list);
        }
    }

    /// Hashes and tokens open the accounts: only the server's user may read
    /// the file.
    fn persist(&self, list: &[Account]) {
        let Ok(bytes) = serde_json::to_vec_pretty(list) else {
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
        match written {
            Ok(()) => *self.saved.lock() = now(),
            Err(e) => tracing::warn!("[accounts] saving {} failed: {e}", self.path.display()),
        }
    }
}

/// The profiles: the server's own, and each account's, opened when first
/// used.
pub struct Profiles {
    dir: PathBuf,
    server: Arc<AppState>,
    open: tokio::sync::Mutex<HashMap<String, Arc<AppState>>>,
}

impl Profiles {
    pub fn new(dir: PathBuf, server: Arc<AppState>) -> Self {
        Self { dir, server, open: tokio::sync::Mutex::new(HashMap::new()) }
    }

    pub async fn get(&self, viewer: &Viewer) -> Result<Arc<AppState>, String> {
        let id = match viewer {
            Viewer::Guest => return Ok(self.server.clone()),
            Viewer::Account(id) => id,
        };
        let mut open = self.open.lock().await;
        if let Some(state) = open.get(id) {
            return Ok(state.clone());
        }
        let state = Arc::new(
            self.server
                .profile(self.dir.join(id))
                .await
                .map_err(|e| format!("{e:#}"))?,
        );
        open.insert(id.clone(), state.clone());
        Ok(state)
    }

    /// An account deleted: its profile goes too.
    pub async fn remove(&self, id: &str) {
        self.open.lock().await.remove(id);
        let dir = self.dir.join(id);
        if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("[accounts] removing {} failed: {e}", dir.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert!(valid_username("siiis"));
        assert!(valid_username("Mario.Rossi_2"));
        assert!(!valid_username(""));
        assert!(!valid_username("a b"));
        assert!(!valid_username("../x"));
        assert!(!valid_username(&"x".repeat(33)));
    }

    #[tokio::test]
    async fn passwords() {
        let hash = hash_password("segreto1".into()).await.unwrap();
        assert!(hash.starts_with("$argon2"));
        assert!(password_matches("segreto1".into(), hash.clone()).await);
        assert!(!password_matches("segreto2".into(), hash).await);
    }

    #[tokio::test]
    async fn devices() {
        let dir = std::env::temp_dir().join(format!("siiishub-accounts-test-{}", super::super::random_hex(4)));
        std::fs::create_dir_all(&dir).unwrap();
        let accounts = Accounts::load(dir.join("accounts.json"));
        let mario = accounts.create("mario", "password1", false).await.unwrap();
        let old = accounts.issue_token(&mario.id, "Windows", &Visit::default()).unwrap();
        let visit = Visit { name: "SALOTTO".into(), version: "1.3.8".into(), address: "192.168.1.20".into() };
        let tv = accounts.issue_token(&mario.id, "Android TV", &visit).unwrap();

        let list = accounts.list();
        let devices = &list[0].devices;
        assert_eq!(devices.len(), 2);
        assert!(devices.iter().all(|d| d.online && d.id.len() == 16));
        let shown = devices.iter().find(|d| d.kind == "Android TV").unwrap();
        assert_eq!((shown.name.as_str(), shown.version.as_str(), shown.address.as_str()), ("SALOTTO", "1.3.8", "192.168.1.20"));

        // A request of an app from before 1.3.8 keeps what is known.
        assert!(accounts.visit(&tv, &Visit { address: "10.0.0.5".into(), ..Visit::default() }).is_some());
        let list = accounts.list();
        let shown = list[0].devices.iter().find(|d| d.kind == "Android TV").unwrap();
        assert_eq!((shown.name.as_str(), shown.address.as_str()), ("SALOTTO", "10.0.0.5"));

        // Signed out from the settings: its token stops working, the other's
        // does not, and a wait made before hears of it.
        let kicked = accounts.kicked();
        let id = shown.id.clone();
        assert_eq!(accounts.revoke_device("nobody", &id), Err(AccountError::NotFound));
        accounts.revoke_device(&mario.id, &id).unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_secs(1), kicked).await.is_ok());
        assert!(accounts.token_account(&tv).is_none());
        assert!(accounts.token_account(&old).is_some());
        assert_eq!(accounts.revoke_device(&mario.id, &id), Err(AccountError::NotFound));

        // The file keeps what was seen.
        let again = Accounts::load(dir.join("accounts.json"));
        assert_eq!(again.list()[0].devices.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
