//! The app signed in to an account on a SIIISHUB server
//! (`server/accounts.rs`): its settings and user data sync with the
//! account's profile there (`ops::sync`). The app keeps the server's
//! address, the username and a token for this device, never the password.
//! The sync runs at start-up, a few seconds after a change here (every 20
//! seconds at most while changes keep coming), and as soon as the server
//! tells of a change in the browser or on another device: the app keeps a
//! request open for that news. Away from the server, it tries every minute.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ops::sync::{self, Applied, Change, Stores};
use crate::state::AppState;

/// Between two tries while the server cannot be reached.
const EVERY: Duration = Duration::from_secs(60);
/// Longest wait for news: the server answers within 50 seconds.
const NEWS_TIMEOUT: Duration = Duration::from_secs(70);
/// After a change here: a burst of them goes in one sync.
const SETTLE: Duration = Duration::from_secs(3);
/// Between two syncs while changes keep coming (the resume point, saved
/// every few seconds of playback).
const MIN_GAP: Duration = Duration::from_secs(20);

#[derive(Clone, Default, Serialize, Deserialize)]
struct Link {
    server: String,
    username: String,
    token: String,
    /// The place in the account's order of changes the app has everything
    /// up to.
    cursor: u64,
    /// The place in this device's order up to which its changes were sent.
    pushed: u64,
    #[serde(default)]
    last_sync: u64,
}

/// What the settings show.
#[derive(Debug, Clone, Serialize)]
pub struct AccountStatus {
    pub signed_in: bool,
    pub server: String,
    pub username: String,
    /// Seconds since 1970 of the last sync that went through.
    pub last_sync: u64,
    /// The last sync's failure, as a code the page words.
    pub error: Option<String>,
}

pub struct Account {
    path: PathBuf,
    link: Mutex<Option<Link>>,
    error: Mutex<Option<String>>,
    http: reqwest::Client,
    running: tokio::sync::Mutex<()>,
    /// Signed in or out: the sync in the background starts over.
    wake: tokio::sync::Notify,
}

#[derive(Deserialize)]
struct TokenReply {
    token: String,
    username: String,
}

#[derive(Deserialize)]
struct SyncReply {
    cursor: u64,
    #[serde(default)]
    changes: Vec<Change>,
}

fn now() -> u64 {
    crate::sync::now_ms() / 1000
}

/// `host:port`, with or without `http(s)://` and a trailing slash, as the
/// base of the server's API; none for what cannot be an address.
fn server_base(input: &str) -> Option<String> {
    let input = input.trim().trim_end_matches('/');
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("http://{input}") };
    let url = url::Url::parse(&with_scheme).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    Some(with_scheme)
}

impl Account {
    pub fn load(path: PathBuf) -> Self {
        let link = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let http = reqwest::Client::builder()
            .user_agent("siiishub/0.1")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self {
            path,
            link: Mutex::new(link),
            error: Mutex::new(None),
            http,
            running: tokio::sync::Mutex::new(()),
            wake: tokio::sync::Notify::new(),
        }
    }

    pub fn signed_in(&self) -> bool {
        self.link.lock().is_some()
    }

    pub fn status(&self) -> AccountStatus {
        let link = self.link.lock().clone().unwrap_or_default();
        AccountStatus {
            signed_in: !link.token.is_empty(),
            server: link.server,
            username: link.username,
            last_sync: link.last_sync,
            error: self.error.lock().clone(),
        }
    }

    fn save(&self, link: Option<&Link>) {
        let written = match link {
            Some(link) => serde_json::to_vec_pretty(link)
                .map_err(std::io::Error::other)
                .and_then(|bytes| std::fs::write(&self.path, bytes)),
            None => match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        };
        if let Err(e) = written {
            tracing::warn!("[account] saving {} failed: {e}", self.path.display());
        }
    }

    /// Signs in with the account's username and password, and syncs: what
    /// the account has comes in, what this device has on top goes to it.
    pub async fn sign_in(
        &self,
        stores: Stores<'_>,
        server: &str,
        username: &str,
        password: &str,
        device: &str,
    ) -> Result<Applied, String> {
        let base = server_base(server).ok_or("bad-address")?;
        let reply = self
            .http
            .post(format!("{base}/api/account/token"))
            .json(&json!({ "username": username.trim(), "password": password, "device": device }))
            .send()
            .await
            .map_err(|_| "unreachable")?;
        match reply.status().as_u16() {
            200 => {}
            401 => return Err("wrong-credentials".into()),
            429 => return Err("too-many-attempts".into()),
            _ => return Err("not-siiishub".into()),
        }
        let TokenReply { token, username } = reply.json().await.map_err(|_| "not-siiishub")?;
        // Another account, or the same after its server started over: what
        // this device has goes to it too, not only what changed here since
        // the last sync (the rest came from the previous account).
        sync::claim(stores).await;
        let link = Link { server: base, username, token, ..Link::default() };
        self.save(Some(&link));
        *self.link.lock() = Some(link);
        *self.error.lock() = None;
        let applied = self.sync(stores).await;
        self.wake.notify_one();
        applied
    }

    /// Signs out: the token stops working on the server, and this device
    /// keeps its settings and user data as they are.
    pub async fn sign_out(&self) {
        let link = self.link.lock().take();
        self.save(None);
        *self.error.lock() = None;
        self.wake.notify_one();
        if let Some(link) = link {
            let _ = self
                .http
                .post(format!("{}/api/account/revoke", link.server))
                .bearer_auth(&link.token)
                .send()
                .await;
        }
    }

    /// Sends this device's changes since the last sync and takes the
    /// account's.
    pub async fn sync(&self, stores: Stores<'_>) -> Result<Applied, String> {
        let _running = self.running.lock().await;
        let Some(link) = self.link.lock().clone() else {
            return Ok(Applied::default());
        };
        let result = self.exchange(stores, link).await;
        *self.error.lock() = result.as_ref().err().cloned();
        result
    }

    async fn exchange(&self, stores: Stores<'_>, mut link: Link) -> Result<Applied, String> {
        let (changes, reached) = sync::changes_since(stores, link.pushed, true);
        let reply = self
            .http
            .post(format!("{}/api/sync", link.server))
            .bearer_auth(&link.token)
            .json(&json!({ "since": link.cursor, "changes": changes }))
            .send()
            .await
            .map_err(|_| "unreachable")?;
        match reply.status().as_u16() {
            200 => {}
            // The token was revoked or the account deleted: signed out.
            401 => {
                tracing::warn!("[account] the server no longer knows this device: signed out");
                *self.link.lock() = None;
                self.save(None);
                return Err("signed-out".into());
            }
            _ => return Err("not-siiishub".into()),
        }
        let SyncReply { cursor, changes } = reply.json().await.map_err(|_| "not-siiishub")?;
        let applied = sync::apply(stores, changes, true).await?;
        link.pushed = reached;
        link.cursor = cursor;
        link.last_sync = now();
        // Signed out meanwhile: nothing to keep.
        let mut current = self.link.lock();
        if current.as_ref().is_some_and(|l| l.token == link.token) {
            self.save(Some(&link));
            *current = Some(link);
        }
        Ok(applied)
    }
}

impl Account {
    /// Waits for news of the account: true when the server tells of a change
    /// (in the browser, or on another device), false when it had none for a
    /// while. Away from the server, or with a server from before this, it
    /// waits a minute and says true, to try again.
    async fn news(&self) -> bool {
        let asked = std::time::Instant::now();
        let Some(link) = self.link.lock().clone() else {
            tokio::time::sleep(EVERY).await;
            return false;
        };
        let reply = self
            .http
            .get(format!("{}/api/sync/wait?since={}", link.server, link.cursor))
            .bearer_auth(&link.token)
            .timeout(NEWS_TIMEOUT)
            .send()
            .await;
        let news = match reply {
            Ok(reply) if reply.status().is_success() => reply
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|body| body["changed"].as_bool()),
            _ => None,
        };
        match news {
            Some(true) => true,
            // A server that answers at once with nothing is not waiting.
            Some(false) if asked.elapsed() >= Duration::from_secs(5) => false,
            _ => {
                tokio::time::sleep(EVERY).await;
                true
            }
        }
    }
}

/// The sync in the background, for as long as the app runs; `changed` tells
/// the interface what came in.
pub fn start(state: Arc<AppState>, account: Arc<Account>, changed: impl Fn(Applied) + Send + 'static) {
    tauri::async_runtime::spawn(async move {
        let mut due = true;
        loop {
            if due && account.signed_in() {
                match account.sync(state.as_ref().into()).await {
                    Ok(applied) if applied.userdata || applied.settings => changed(applied),
                    Ok(_) => {}
                    Err(e) => tracing::debug!("[account] sync failed: {e}"),
                }
            }
            let last = std::time::Instant::now();
            // A change here, news from the server, or a sign-in or -out.
            due = tokio::select! {
                _ = state.sync.changed() => {
                    tokio::time::sleep(SETTLE.max(MIN_GAP.saturating_sub(last.elapsed()))).await;
                    true
                }
                news = account.news() => news,
                _ = account.wake.notified() => false,
            };
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingsStore;
    use crate::sync::SyncLog;
    use crate::userdata::UserDataStore;

    struct Device {
        settings: SettingsStore,
        userdata: UserDataStore,
        log: SyncLog,
        account: Account,
    }

    impl Device {
        /// A device with `before` in its user data from before it ever
        /// synced.
        async fn open(dir: PathBuf, before: &[(&str, &str)]) -> Self {
            std::fs::create_dir_all(&dir).unwrap();
            let settings = SettingsStore::open(dir.join("settings.json")).await.unwrap();
            let userdata = UserDataStore::open(dir.join("userdata.json")).await.unwrap();
            for (k, v) in before {
                userdata.set(k.to_string(), v.to_string()).await.unwrap();
            }
            let existing = crate::sync::existing_keys(&settings.read(), &userdata.snapshot());
            let log = SyncLog::open(dir.join("sync.json"), existing).await;
            let account = Account::load(dir.join("account.json"));
            Self { settings, userdata, log, account }
        }

        fn stores(&self) -> Stores<'_> {
            Stores { settings: &self.settings, userdata: &self.userdata, log: &self.log }
        }

        /// A change made on the device, as ops::userdata does it.
        async fn set(&self, key: &str, value: Option<&str>) {
            match value {
                Some(v) => self.userdata.set(key.to_string(), v.to_string()).await.unwrap(),
                None => self.userdata.remove(key).await.unwrap(),
            }
            self.log.touch(&[key.to_string()]).await;
        }

        async fn sync(&self) -> Applied {
            self.account.sync(self.stores()).await.unwrap()
        }
    }

    /// Two devices signed in to the same account keep each other's changes.
    /// Needs a running server and one of its accounts:
    ///   SIIISHUB_TEST_SERVER=127.0.0.1:8081 SIIISHUB_TEST_USER=... \
    ///   SIIISHUB_TEST_PASSWORD=... cargo test --lib account -- --ignored
    #[tokio::test]
    #[ignore]
    async fn two_devices() {
        let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} not set"));
        let (server, user, password) = (var("SIIISHUB_TEST_SERVER"), var("SIIISHUB_TEST_USER"), var("SIIISHUB_TEST_PASSWORD"));
        let dir = std::env::temp_dir().join(format!("siiishub-sync-test-{}", crate::sync::now_ms()));
        let a = Device::open(dir.join("a"), &[("siiis:favorite:movie:1", "from A"), ("siiis:dl:seen", "A only")]).await;
        let b = Device::open(dir.join("b"), &[("siiishub-theme", "light")]).await;

        // What each had before signing in reaches the other; what belongs to
        // the device stays.
        a.account.sign_in(a.stores(), &server, &user, &password, "Test A").await.unwrap();
        b.account.sign_in(b.stores(), &server, &user, &password, "Test B").await.unwrap();
        a.sync().await;
        assert_eq!(b.userdata.get("siiis:favorite:movie:1").as_deref(), Some("from A"));
        assert_eq!(a.userdata.get("siiishub-theme").as_deref(), Some("light"));
        assert_eq!(b.userdata.get("siiis:dl:seen"), None);

        // A change on B reaches A; a deletion on A reaches B.
        b.set("siiis:resume:movie:2", Some("{\"t\":60}")).await;
        b.sync().await;
        assert!(a.sync().await.userdata);
        assert_eq!(a.userdata.get("siiis:resume:movie:2").as_deref(), Some("{\"t\":60}"));
        a.set("siiis:favorite:movie:1", None).await;
        a.sync().await;
        b.sync().await;
        assert_eq!(b.userdata.get("siiis:favorite:movie:1"), None);

        // A setting.
        let mut next = a.settings.read();
        next.tmdb_key = "KEY-FROM-A".into();
        a.settings.write(next).await.unwrap();
        a.log.touch(&[crate::sync::setting_key("tmdb_key")]).await;
        a.sync().await;
        assert!(b.sync().await.settings);
        assert_eq!(b.settings.read().tmdb_key, "KEY-FROM-A");

        // Nothing new: nothing applied.
        let quiet = b.sync().await;
        assert!(!quiet.userdata && !quiet.settings);

        // B, waiting for news, hears of A's change as soon as A sends it.
        let asked = std::time::Instant::now();
        let (news, _) = tokio::join!(b.account.news(), async {
            tokio::time::sleep(Duration::from_millis(500)).await;
            a.set("siiis:favorite:movie:3", Some("from A")).await;
            a.sync().await;
        });
        assert!(news);
        assert!(asked.elapsed() < Duration::from_secs(5), "news after {:?}", asked.elapsed());
        assert!(b.sync().await.userdata);
        assert_eq!(b.userdata.get("siiis:favorite:movie:3").as_deref(), Some("from A"));

        a.account.sign_out().await;
        b.account.sign_out().await;
        assert!(!a.account.signed_in());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A device that synced with an account has its values marked as taken
    /// from the server; signing in to another account offers them all again,
    /// with the keys the log never stamped.
    #[tokio::test]
    async fn claimed_for_a_new_account() {
        let dir = std::env::temp_dir().join(format!("siiishub-claim-test-{}", crate::sync::now_ms()));
        let d = Device::open(dir.clone(), &[("siiis:favorite:movie:1", "mine")]).await;
        d.userdata.set("siiis:favorite:movie:2".into(), "from the account".into()).await.unwrap();
        d.log.record(&[("siiis:favorite:movie:1".into(), 0), ("siiis:favorite:movie:2".into(), 5)], true).await;
        d.userdata.set("siiis:resume:movie:3".into(), "{}".into()).await.unwrap();
        assert!(sync::changes_since(d.stores(), 0, true).0.is_empty());

        sync::claim(d.stores()).await;
        let (sent, _) = sync::changes_since(d.stores(), 0, true);
        let mut keys: Vec<_> = sent.iter().map(|c| c.key.as_str()).collect();
        keys.sort();
        assert_eq!(keys, ["siiis:favorite:movie:1", "siiis:favorite:movie:2", "siiis:resume:movie:3"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A device moves to another account (or to its server started over):
    /// the new account gets what the device had, though it came from the
    /// first one. Needs a second account on the test server, besides the one
    /// of `two_devices`: SIIISHUB_TEST_USER2, SIIISHUB_TEST_PASSWORD2.
    #[tokio::test]
    #[ignore]
    async fn new_account() {
        let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} not set"));
        let server = var("SIIISHUB_TEST_SERVER");
        let dir = std::env::temp_dir().join(format!("siiishub-move-test-{}", crate::sync::now_ms()));
        let a = Device::open(dir.join("a"), &[("siiis:favorite:movie:7", "from A")]).await;
        a.account.sign_in(a.stores(), &server, &var("SIIISHUB_TEST_USER"), &var("SIIISHUB_TEST_PASSWORD"), "Test A").await.unwrap();
        a.sync().await;
        a.account.sign_out().await;

        a.account.sign_in(a.stores(), &server, &var("SIIISHUB_TEST_USER2"), &var("SIIISHUB_TEST_PASSWORD2"), "Test A").await.unwrap();
        let c = Device::open(dir.join("c"), &[]).await;
        c.account.sign_in(c.stores(), &server, &var("SIIISHUB_TEST_USER2"), &var("SIIISHUB_TEST_PASSWORD2"), "Test C").await.unwrap();
        assert_eq!(c.userdata.get("siiis:favorite:movie:7").as_deref(), Some("from A"));

        a.account.sign_out().await;
        c.account.sign_out().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn addresses() {
        assert_eq!(server_base("192.168.1.5:30808").as_deref(), Some("http://192.168.1.5:30808"));
        assert_eq!(server_base(" https://nas.lan/ ").as_deref(), Some("https://nas.lan"));
        assert_eq!(server_base("ftp://x"), None);
        assert_eq!(server_base(""), None);
    }
}
