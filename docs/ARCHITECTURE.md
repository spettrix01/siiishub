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
  - `remote.rs`, `remote_page.html`: the phone remote control, an axum HTTP
    and WebSocket server with QR pairing and device approval, run by the
    desktop app and the TV app on a port of their own and by the web server
    at `/remote/`.
  - `ops/`: what the interface asks of the backend, shared by the Tauri
    commands (`commands/`) and the web server (`server/`, see
    [WEB.md](WEB.md)).
  - `settings.rs`, `userdata.rs`: settings and library in the app data folder.
  - `sync.rs`, `ops/sync.rs`, `account.rs`: the app signed in to an account
    of a SIIISHUB server syncs its settings and library with it, key by key,
    the later change winning (see [WEB.md](WEB.md), Accounts and sync).
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

## Navigation with a remote

The phone remote (`remote.rs` passes its keys to the interface as
`remote://cmd`, handled in `player.js`) and the Android TV remote
(`tv-nav.js`) drive the interface through `spatial-nav.js`, which moves a
selection (`.snav-focus`) the way a TV interface does:

- The open layer (the page, a details page, the settings, a dialog, a menu,
  the player) is divided into zones, listed in `ZONES`. In a row (the tabs,
  a rail of posters, the buttons of a stream) left and right go along it; a
  box (the grid of posters, the list of streams, a section of the settings)
  is made of rows, and up and down go to the next row keeping the column. At
  the edge of a zone the move carries on in the zone around it.
- Rows are measured as laid out, as if nothing were scrolled vertically, so
  they keep their order however far the page or a list has scrolled. Only
  real scrollers scroll to show the selection, centred along the move.
- Coming back into a zone, the selection returns to what it had selected
  there (the topbar, the rails, the grid, the parts of a details page), or
  goes to its current choice (the section shown, the track in use, the
  button that confirms), or to its first item (a section of the settings,
  a form).
- The settings show a section as soon as the selection reaches it in their
  menu; right goes into it, left back to the menu.
- What opens starts on its current choice or first action (a details page,
  the settings, a dropdown, a dialog, the player's settings), and what
  closes gives the selection back to what opened it. OK on the time bar
  plays or pauses, on the volume mutes.
- Back closes the innermost thing open (a menu, a dialog, which is
  cancelled), goes from a section of the settings back to their menu, and on
  the page brings the selection up to the tabs and the page to its top.
  Home closes everything and selects the Movies tab.

## The Android interface

The same `dist/` runs on phones and on Android TV, built into two APKs.
`js/platform.js` adds the `is-android` class to `<html>`, plus `is-phone` or
`is-tv`: the TV APK says it is one (`window.__SIIISHUB_TV__`, set by the
Cargo feature `tv`), and the phone APK takes the TV look on a device without
a touchscreen (`navigator.maxTouchPoints` is 0). What phones and TVs share,
the platform, depends on `is-android` and `IS_ANDROID`; the phone's own
interface, made for touch, on `is-phone` and `IS_PHONE`:

- `bottom-nav.js`: bottom navigation on phones, with the settings shown as a
  page.
- `picker.js` in popup mode, `android-inputs.js` for text fields edited in a
  centred popup, `android-genres.js` for the genre popup: phones only.
- `android-back.js`: the Back button closes popups and overlays first, through
  history entries.
- `android-player.js`: landscape player on both; the settings popup, pinch to
  fill the screen (`panscan`) and tap to show or hide the controls on phones.
- `remote-client.js`: Settings → Remote makes the phone the remote of
  SIIISHUB on a PC. It frames the QR code of the PC with the camera
  (`getUserMedia` and `BarcodeDetector`; without them the address is typed),
  then shows the phone page served by the PC (`remote_page.html`) full screen
  in a frame, so approval and pairing work as in the phone's browser. For it
  the release APK allows cleartext HTTP (`android-build.sh`), the Android CSP
  allows `http:` frames and connections (`tauri.android.conf.json`) and the
  plugin declares the camera, not required.

### Android TV

The TV has the interface of the desktop app, its dropdowns, text fields
edited in place and player included, without window controls and with
margins inside the TV safe area, and is driven with its remote (`tv-nav.js`):

- The D-pad moves the selection with `spatial-nav.js`, the navigation of the
  phone remote (see Navigation with a remote), and OK activates it. In a
  text field, left, right and OK stay with the field and its keyboard; up
  and down leave it.
- Back is the system Back button (`android-back.js`). It goes from a section
  of the settings to their menu, keeping their history entry; while the
  selection is in the page, below the topbar, the page holds an entry of its
  own, so Back first brings the selection up to the tabs and only the next
  one leaves the app. The details keep their entry while they hide under
  the player.
- In the player, with nothing selected, left and right seek by 10 s and OK
  plays or pauses; up and down show the controls and select the time bar. The
  media keys play, pause and seek by 30 s.
- Settings → Remote works as on a PC: the TV APK runs the remote server, so
  a phone scans its QR code and drives it. The TV's address comes from its
  network interfaces or, when Android keeps them from apps, from its default
  route.
- `MpvPlugin.kt` leaves the orientation to the TV, which a 1080p screen of
  540 dp would otherwise lock like a phone. The manifest carries the 320×180
  launcher banner, and `android-build.sh` adds the `LEANBACK_LAUNCHER`
  category to the TV APK.
- A TV stick sets what the TV interface may cost (a Fire TV Stick has four
  slow cores and a weak graphics chip). Nothing is blurred behind anything,
  as a blurred backdrop is redrawn on every frame; the selected card only
  lifts. Scrollers glide in 220 ms (`spatial-nav.js`) instead of the
  WebView's smooth scroll, which starts slowly and takes up to half a
  second. A details page lists its streams 20 at a time, one batch a frame.

Android WebView details worth knowing:

- `scrollbar-width` is not supported. A full-screen fixed scroller becomes the
  root scroller, whose scrollbar is drawn by the native view, so the plugin
  turns it off.
- The shell plugin intercepts `target="_blank"` links, and its opener fails on
  Android. `dom.js` handles those links in the capture phase and opens them
  with the `open_url` command.
- R8 minification stays off: the wry ProGuard template does not keep
  `WryActivity.getId()`, which tao calls through JNI at startup.
- A rule under `:has()` whose last part can match almost anything (such as
  `:not(.winctl)`) makes Chromium restyle the whole page after any change
  anywhere in it, a counter or the player's clock: about 170 ms each time on a
  Fire TV Stick. Such rules name the elements they style.

## Build configuration

- Cargo feature `real-mpv`, on by default, embeds libmpv on desktop. Without it,
  `stub-mpv` builds the desktop interface with no video.
- Cargo feature `devtools` keeps WebView remote debugging in release builds,
  for testing only.
- Cargo feature `tv` builds the Android TV APK; `server`, without the default
  features, builds `siiishub-server` (see [WEB.md](WEB.md)).
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
- Leaving the Android app with Back can end the process with a native crash
  instead of a clean exit. The app starts normally the next time.
