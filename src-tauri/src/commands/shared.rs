pub type CmdResult<T> = std::result::Result<T, String>;

pub fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

pub fn rd_err(e: crate::realdebrid::RdError) -> String {
    use crate::realdebrid::RdError;
    match e {
        RdError::NoToken => crate::util::loc("error.debrid.invalidToken"),
        RdError::NoPlayableFile => crate::util::loc("error.debrid.noPlayableFile"),
        RdError::Timeout => crate::util::loc("error.debrid.timeout"),
        RdError::Cancelled => crate::util::loc("details.playback.cancelled"),
        RdError::Http { status, .. } => {
            crate::util::loc_p("error.debrid.http", serde_json::json!({ "status": status }))
        }
        RdError::Other(inner) => err(inner),
    }
}

/// Shows a folder to the user: the platform file manager on desktop; on
/// Android a file manager app opened on the shared-storage folder through the
/// Kotlin plugin (Samsung My Files, the system Files app or the downloads
/// screen, whichever exists).
pub fn open_folder(app: &tauri::AppHandle, path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let plugin = app.state::<siiishub_android_player::AndroidMpv<tauri::Wry>>();
        plugin
            .open_folder(&path.to_string_lossy())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        open_in_file_manager(path)
    }
}

/// Opens a web page outside the app. On Android the shell plugin's open
/// spawns a desktop opener that does not exist on the phone, so the URL goes
/// to the system through the Kotlin plugin; desktop hands it to the platform
/// opener, which starts the default browser.
pub fn open_url(app: &tauri::AppHandle, url: &str) -> std::io::Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "indirizzo non supportato",
        ));
    }
    #[cfg(target_os = "android")]
    {
        use tauri::Manager;
        let plugin = app.state::<siiishub_android_player::AndroidMpv<tauri::Wry>>();
        plugin
            .open_url(url)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        open_in_file_manager(std::path::Path::new(url))
    }
}

/// Reveals a folder in the platform file manager (Explorer / xdg-open / Finder).
#[cfg(not(target_os = "android"))]
pub fn open_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    #[cfg(windows)]
    let mut cmd = Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = Command::new("xdg-open");
    cmd.arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}
