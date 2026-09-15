# How SIIISHUB works

## Layout

- `dist/` is the interface: plain ES modules, no bundler. `app.js` wires the
  modules together, `js/api.js` wraps the Tauri commands and `js/locales/`
  holds the 23 languages.
- `src-tauri/src/` is the Rust backend:
  - `stremio.rs`: client of the Stremio addon protocol (manifests, streams,
    subtitles).
  - `torrent.rs`, `torrent_file.rs`: the librqbit session. Its HTTP API serves
    `/torrents/{id}/stream/{file}`, which mpv plays while the pieces arrive.
  - `realdebrid.rs`, `alldebrid.rs`: debrid resolution of magnets and torrents.
  - `download.rs`, `commands/downloads.rs`: downloads over P2P, HTTP and local
    files, with the poster as folder icon.
  - `remote.rs`, `remote_page.html`: the desktop remote control, an axum HTTP
    and WebSocket server with QR pairing and device approval.
  - `settings.rs`, `userdata.rs`: settings and library in the app data folder.
  - `mpv.rs` and `mpv/`: one player interface with three backends. `real.rs`
    is libmpv on desktop, `android.rs` bridges to the Kotlin plugin and
    `stub.rs` is for development without libmpv.
  - `render.rs` (Windows) and `render_linux.rs` (Linux): video compositing.
  - `power.rs`: keeps the display awake during playback.
- `src-tauri/plugins/android-player/` is a Tauri plugin with the Kotlin side
  of the Android player.

## The player

The player controls are HTML drawn by the webview. The video is drawn by mpv
on a native surface below a transparent webview, so controls, menus and
panels sit on top of the picture without copying frames. The interface sends
the rectangle of the video area in physical pixels (`mpv_set_geometry`), and
mpv events come back to it as `mpv://event`.

### Windows

mpv renders into a D3D11 texture that DirectComposition places under the
WebView2 (`render.rs`).

HTTPS streams are verified. The Windows libmpv uses OpenSSL without a CA
store, so the Mozilla CA bundle in `resources/cacert.pem`, embedded in the
executable, is written to the app data folder and passed to mpv as
`tls-ca-file`. Refresh it from https://curl.se/docs/caextract.html before a
release.

### Linux

mpv draws through the libmpv OpenGL render API into a `GtkGLArea`. The
transparent WebKitGTK view sits above it in a `GtkOverlay` (`render_linux.rs`).
It works on X11 and Wayland, with hardware decoding through `hwdec=auto-safe`.
HTTPS streams are verified against the CA bundle of the distribution.

Details handled there:

- `libmpv2::render::RenderContext` 4.1 instantiates the `get_proc_address`
  trampoline with the wrong type, so the render context is driven through
  `libmpv2-sys` directly.
- libmpv refuses `mpv_create()` unless `LC_NUMERIC` is `C`. GTK sets it from
  the user locale, so it is forced back before the handle is created.
- The overlay must be the direct child of the `GtkWindow`: the resize handler
  of undecorated windows in tauri-runtime-wry expects
  `webview.parent().parent()` to be the window.
- The `realize`, `render` and `unrealize` handlers are connected before
  `show_all()`, because the window is already mapped.

Desktop integration uses `org.freedesktop.ScreenSaver` to inhibit the
screensaver, with `GtkApplication::inhibit` as fallback, `xdg-open` for
folders and the GIO `metadata::custom-icon` attribute for folder icons.

### Android

`MpvPlugin.kt` loads the prebuilt `dev.jdtech.mpv:libmpv` package (mpv, FFmpeg
and libass) and draws into a `SurfaceView` behind the WebView, with `vo=gpu`,
`gpu-context=android` and MediaCodec hardware decoding. `mpv/android.rs`
forwards the desktop player interface to the plugin. `track-list` and
`video-params` are rebuilt from their sub-properties, because the JNI binding
does not expose mpv nodes.

- HTTPS streams are verified: the bundled FFmpeg uses Mbed TLS without a CA
  store, so the plugin exports the system certificates to a PEM file and turns
  on `tls-verify`.
- The activity is edge-to-edge. The web content is padded away from the system
  bars and the keyboard, and the player goes immersive landscape as soon as it
  opens.
- Subtitles use the system fonts in `/system/fonts`.
- Downloads go to the shared `Download/SIIISHUB` folder when it is writable
  (Android 11 and newer), otherwise to the private app folder.

## The Android interface

The same `dist/` runs on the phone. `js/platform.js` adds the `is-android`
class to `<html>`, and Android-only behaviour depends on that class and on the
`IS_ANDROID` flag:

- `bottom-nav.js`: bottom navigation, with the settings shown as a page.
- `picker.js` in popup mode, `android-inputs.js` for text fields edited in a
  centred popup, `android-genres.js` for the genre popup.
- `android-back.js`: the Back button closes popups and overlays first, through
  history entries.
- `android-player.js`: landscape player, settings popup, pinch to fill the
  screen (`panscan`) and tap to show or hide the controls.

Android WebView details worth knowing:

- `scrollbar-width` is not supported. A full-screen fixed scroller becomes the
  root scroller, whose scrollbar is drawn by the native view, so the plugin
  turns it off.
- The shell plugin intercepts `target="_blank"` links, and its opener fails on
  Android. `dom.js` handles those links in the capture phase and opens them
  with the `open_url` command.
- R8 minification stays off: the wry ProGuard template does not keep
  `WryActivity.getId()`, which tao calls through JNI at startup.

## Build configuration

- Cargo feature `real-mpv`, on by default, embeds libmpv on desktop. Without it,
  `stub-mpv` builds the desktop interface with no video.
- Cargo feature `devtools` keeps WebView remote debugging in release builds,
  for testing only.
- `tauri.conf.json` holds the shared configuration. `tauri.windows.conf.json`,
  `tauri.linux.conf.json` and `tauri.android.conf.json` add the bundle
  settings of each platform.
- `vendor/librqbit-sha1-wrapper` replaces the SHA-1 wrapper of librqbit with
  the pure-Rust `sha1` crate, which uses the CPU SHA extensions on x86 and ARM.
  No target needs a native crypto library.

## Known limitations

- Android may suspend the app in the background. Torrent streaming continues
  only while the app is visible or just behind, as there is no foreground
  service yet.
- `ffprobe` does not exist on Android, so tracks are not analysed before
  playback. mpv still lists them in the player.
- The Android TV launcher shows the regular icon, since there is no TV banner.
- Leaving the Android app with Back can end the process with a native crash
  instead of a clean exit. The app starts normally the next time.
