//! Whether a newer SIIISHUB is out, next to the version in a corner of the
//! interface: the latest release on GitHub. The apps ask for themselves, the
//! web server for its pages; an answer holds an hour, as GitHub answers 60
//! requests an hour to an address without a key.

use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

const LATEST: &str = "https://api.github.com/repos/spettrix01/siiishub/releases/latest";
const RELEASES: &str = "https://github.com/spettrix01/siiishub/releases";
const FRESH: Duration = Duration::from_secs(3600);
/// After a failure (offline, GitHub down), sooner.
const RETRY: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Serialize)]
pub struct Update {
    /// This version.
    pub current: String,
    /// The latest release's.
    pub latest: String,
    /// The latest release is newer than this version.
    pub newer: bool,
    /// Its page.
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: String,
}

type Answer = Result<Update, String>;

/// The last answer, and when it came.
static LAST: Lazy<Mutex<Option<(Instant, Answer)>>> = Lazy::new(|| Mutex::new(None));

pub async fn check(http: &reqwest::Client) -> Result<Update, String> {
    if let Some((at, answer)) = LAST.lock().as_ref() {
        let holds = if answer.is_ok() { FRESH } else { RETRY };
        if at.elapsed() < holds {
            return answer.clone();
        }
    }
    let answer = ask(http).await;
    if let Err(e) = &answer {
        tracing::debug!("[update] the latest release is unknown: {e}");
    }
    *LAST.lock() = Some((Instant::now(), answer.clone()));
    answer
}

async fn ask(http: &reqwest::Client) -> Result<Update, String> {
    let reply = http
        .get(LATEST)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| "unreachable".to_string())?;
    if !reply.status().is_success() {
        return Err(format!("http-{}", reply.status().as_u16()));
    }
    let release: Release = reply.json().await.map_err(|_| "unreadable".to_string())?;
    let latest = release.tag_name.trim().trim_start_matches(['v', 'V']).to_string();
    let current = env!("CARGO_PKG_VERSION").to_string();
    // The interface opens GitHub's pages only.
    let url = if release.html_url.starts_with("https://github.com/") {
        release.html_url
    } else {
        RELEASES.to_string()
    };
    Ok(Update { newer: newer(&latest, &current), current, latest, url })
}

/// `a` comes after `b`, compared as dotted numbers; what is not such a
/// version (a pre-release's `1.4.0-beta`) never does.
fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| v.split('.').map(|part| part.parse::<u64>().ok()).collect::<Option<Vec<_>>>();
    matches!((parse(a), parse(b)), (Some(a), Some(b)) if a > b)
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn versions() {
        assert!(newer("1.3.7", "1.3.6"));
        assert!(newer("1.3.10", "1.3.9"));
        assert!(newer("1.4", "1.3.6"));
        assert!(newer("2.0.0", "1.9.9"));
        assert!(!newer("1.3.6", "1.3.6"));
        assert!(!newer("1.3.5", "1.3.6"));
        assert!(!newer("1.4.0-beta", "1.3.6"));
        assert!(!newer("", "1.3.6"));
    }
}
