use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

pub fn loc(code: &str) -> String {
    serde_json::json!({ "code": code }).to_string()
}

/// Resolves when the token is cancelled; pends forever when no token is given,
/// so it can be dropped into `tokio::select!` without branching at call sites.
pub async fn cancelled_opt(cancel: Option<&tokio_util::sync::CancellationToken>) {
    match cancel {
        Some(t) => t.cancelled().await,
        None => std::future::pending::<()>().await,
    }
}

pub fn is_cancelled(cancel: Option<&tokio_util::sync::CancellationToken>) -> bool {
    cancel.map(|t| t.is_cancelled()).unwrap_or(false)
}

pub fn loc_p(code: &str, params: serde_json::Value) -> String {
    serde_json::json!({ "code": code, "params": params }).to_string()
}

pub fn filename_from_url(url: &str) -> String {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return "download.bin".to_string(),
    };
    let last = parsed
        .path_segments()
        .and_then(|mut s| s.next_back())
        .filter(|s| !s.is_empty())
        .unwrap_or("download.bin");
    let decoded = percent_encoding::percent_decode_str(last)
        .decode_utf8_lossy()
        .into_owned();
    safe_filename(&decoded)
}

/// Reduce an arbitrary (possibly attacker-controlled) name to a single safe
/// file-name component: drops any directory parts / `..` traversal and strips
/// path separators and reserved characters. Never returns a path that can
/// escape its parent folder.
pub fn safe_filename(name: &str) -> String {
    let base = std::path::Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "download.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_log_dir(dir: PathBuf) {
    let _ = LOG_DIR.set(dir);
}

/// App data folder: the one given to `set_log_dir` during setup, otherwise
/// the platform default.
#[cfg(windows)]
pub fn app_data_dir() -> Option<PathBuf> {
    LOG_DIR.get().cloned().or_else(default_app_data_dir)
}

#[cfg_attr(any(target_os = "android", not(feature = "real-mpv")), allow(dead_code))]
pub fn app_log_append(filename: &str, line: &str) {
    let dir = match LOG_DIR.get() {
        Some(d) => d.clone(),
        None => match default_app_data_dir() {
            Some(d) => d,
            None => return,
        },
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(filename))
        .and_then(|mut f| writeln!(f, "[{now}] {line}"));
}

/// Fallback used before `set_log_dir` runs: mirrors Tauri's `app_data_dir`
/// (`%APPDATA%` on Windows, `$XDG_DATA_HOME` or `~/.local/share` elsewhere).
#[cfg_attr(any(target_os = "android", not(feature = "real-mpv")), allow(dead_code))]
fn default_app_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|v| PathBuf::from(v).join("dev.siiis.siiishub"))
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })?;
        Some(base.join("dev.siiis.siiishub"))
    }
}
