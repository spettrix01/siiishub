//! Keeps display/system awake while mpv is actively playing. With the libmpv
//! render API (`vo=libmpv`) mpv does not inhibit the screensaver itself — the
//! embedding application is responsible.

#[cfg(windows)]
#[cfg_attr(not(feature = "real-mpv"), allow(dead_code))]
mod imp {
    use std::sync::mpsc::{self, Sender};
    use std::sync::OnceLock;

    static TX: OnceLock<Sender<bool>> = OnceLock::new();

    pub fn init(_app: tauri::AppHandle) {}

    // SetThreadExecutionState is per-thread state: the request must be issued
    // and cleared from the same thread, so a dedicated worker owns it.
    pub fn set(on: bool) {
        let tx = TX.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<bool>();
            std::thread::spawn(move || {
                use windows::Win32::System::Power::{
                    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED,
                    ES_SYSTEM_REQUIRED,
                };
                let mut current = false;
                while let Ok(next) = rx.recv() {
                    if next == current {
                        continue;
                    }
                    current = next;
                    unsafe {
                        if next {
                            let _ = SetThreadExecutionState(
                                ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED,
                            );
                        } else {
                            let _ = SetThreadExecutionState(ES_CONTINUOUS);
                        }
                    }
                    tracing::info!("[power] keep display awake: {next}");
                }
                unsafe {
                    let _ = SetThreadExecutionState(ES_CONTINUOUS);
                }
            });
            tx
        });
        let _ = tx.send(on);
    }
}

/// Linux: inhibit the screensaver over D-Bus (`org.freedesktop.ScreenSaver`,
/// implemented by GNOME, KDE, Xfce, Cinnamon, MATE...). If no such service is
/// reachable, fall back to `GtkApplication::inhibit`, which talks to the GNOME
/// session manager / desktop portal. Both are cookie based; the inhibit is
/// tied to the shared session-bus connection, which lives as long as the
/// process.
#[cfg(target_os = "linux")]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;

    use anyhow::{anyhow, Context, Result};
    use gtk::glib;
    use gtk::prelude::*;
    use parking_lot::Mutex;
    use tauri::AppHandle;

    const APP_NAME: &str = "SIIISHUB";
    const REASON: &str = "Video playback";

    enum Cookie {
        DBus(u32),
        Gtk(u32),
    }

    static APP: OnceLock<AppHandle> = OnceLock::new();
    static WANTED: AtomicBool = AtomicBool::new(false);
    static COOKIE: Mutex<Option<Cookie>> = Mutex::new(None);

    pub fn init(app: AppHandle) {
        let _ = APP.set(app);
    }

    pub fn set(on: bool) {
        WANTED.store(on, Ordering::SeqCst);
        let current = COOKIE.lock().take();
        match (on, current) {
            (true, Some(c)) => *COOKIE.lock() = Some(c),
            (false, None) => {}
            (true, None) => match dbus_inhibit() {
                Ok(cookie) => {
                    tracing::info!("[power] screensaver inhibited via org.freedesktop.ScreenSaver (cookie {cookie})");
                    *COOKIE.lock() = Some(Cookie::DBus(cookie));
                }
                Err(e) => {
                    tracing::debug!("[power] D-Bus inhibit unavailable ({e:#}); trying GtkApplication");
                    gtk_inhibit_async();
                }
            },
            (false, Some(Cookie::DBus(cookie))) => {
                match dbus_uninhibit(cookie) {
                    Ok(()) => tracing::info!("[power] screensaver inhibit released (cookie {cookie})"),
                    Err(e) => tracing::debug!("[power] UnInhibit failed: {e:#}"),
                }
            }
            (false, Some(Cookie::Gtk(cookie))) => gtk_uninhibit_async(cookie),
        }
    }

    fn session_bus() -> Result<gtk::gio::DBusConnection> {
        gtk::gio::bus_get_sync(gtk::gio::BusType::Session, gtk::gio::Cancellable::NONE)
            .context("connecting to the session bus")
    }

    fn dbus_inhibit() -> Result<u32> {
        let conn = session_bus()?;
        let reply = conn
            .call_sync(
                Some("org.freedesktop.ScreenSaver"),
                "/org/freedesktop/ScreenSaver",
                "org.freedesktop.ScreenSaver",
                "Inhibit",
                Some(&(APP_NAME, REASON).to_variant()),
                Some(glib::VariantTy::new("(u)").map_err(|e| anyhow!("{e}"))?),
                gtk::gio::DBusCallFlags::NONE,
                2000,
                gtk::gio::Cancellable::NONE,
            )
            .context("org.freedesktop.ScreenSaver.Inhibit")?;
        let (cookie,): (u32,) = reply
            .get()
            .ok_or_else(|| anyhow!("unexpected Inhibit reply type {}", reply.type_()))?;
        Ok(cookie)
    }

    fn dbus_uninhibit(cookie: u32) -> Result<()> {
        let conn = session_bus()?;
        conn.call_sync(
            Some("org.freedesktop.ScreenSaver"),
            "/org/freedesktop/ScreenSaver",
            "org.freedesktop.ScreenSaver",
            "UnInhibit",
            Some(&(cookie,).to_variant()),
            None,
            gtk::gio::DBusCallFlags::NONE,
            2000,
            gtk::gio::Cancellable::NONE,
        )
        .context("org.freedesktop.ScreenSaver.UnInhibit")?;
        Ok(())
    }

    fn gtk_application() -> Option<gtk::Application> {
        gtk::gio::Application::default()?.downcast::<gtk::Application>().ok()
    }

    fn gtk_inhibit_async() {
        let Some(app) = APP.get() else {
            return;
        };
        let _ = app.run_on_main_thread(|| {
            let Some(gtk_app) = gtk_application() else {
                tracing::debug!("[power] no GtkApplication available for inhibit");
                return;
            };
            let flags = gtk::ApplicationInhibitFlags::IDLE | gtk::ApplicationInhibitFlags::SUSPEND;
            let cookie = gtk_app.inhibit(None::<&gtk::Window>, flags, Some(REASON));
            if cookie == 0 {
                tracing::debug!("[power] GtkApplication::inhibit refused (no session manager?)");
                return;
            }
            // Playback may already have stopped while we hopped threads.
            if WANTED.load(Ordering::SeqCst) {
                tracing::info!("[power] idle inhibited via GtkApplication (cookie {cookie})");
                *COOKIE.lock() = Some(Cookie::Gtk(cookie));
            } else {
                gtk_app.uninhibit(cookie);
            }
        });
    }

    fn gtk_uninhibit_async(cookie: u32) {
        let Some(app) = APP.get() else {
            return;
        };
        let _ = app.run_on_main_thread(move || {
            if let Some(gtk_app) = gtk_application() {
                gtk_app.uninhibit(cookie);
                tracing::info!("[power] GtkApplication inhibit released (cookie {cookie})");
            }
        });
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub fn init(_app: tauri::AppHandle) {}
    pub fn set(_on: bool) {}
}

/// Must be called once from `setup` so the Linux fallback can reach the GTK
/// main thread. No-op elsewhere.
pub fn init(app: tauri::AppHandle) {
    imp::init(app);
}

// Only the embedded desktop mpv pipeline drives this; on Android the Kotlin
// plugin keeps the screen on during playback.
#[cfg_attr(any(target_os = "android", not(feature = "real-mpv")), allow(dead_code))]
pub fn keep_display_awake(on: bool) {
    imp::set(on);
}
