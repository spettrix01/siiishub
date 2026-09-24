# Building SIIISHUB

SIIISHUB is a [Tauri 2](https://tauri.app) app: a Rust backend in `src-tauri/`
and a plain JavaScript interface in `dist/`, with no Node build step. The same
code base builds all three platforms.

| Platform | Built on | Video | Output |
|---|---|---|---|
| Windows x64 | Windows with MSVC | libmpv-2.dll, composited under WebView2 | NSIS installer |
| Linux x64 | Debian 13, Ubuntu 24.04 or newer | system libmpv, OpenGL under WebKitGTK | .deb, AppImage optional |
| Android arm64 and armv7 | Debian, native or WSL 2 | libmpv-android through a Kotlin plugin | one APK per ABI |

The build scripts copy the finished packages to `installers/<platform>/`,
which git ignores. Set `OUT_DIR` to use another folder.

## Windows

Requirements:

- Visual Studio Build Tools 2022 or newer, with the **Desktop development
  with C++** workload.
- Rust from [rustup](https://rustup.rs) with the MSVC toolchain, and tauri-cli:
  `cargo install tauri-cli --version "^2" --locked`.
- The WebView2 Runtime, already present on Windows 11 and on an up-to-date
  Windows 10.
- libmpv: download an `mpv-dev-x86_64-*.7z` archive from the
  [mpv-winbuild-cmake releases](https://github.com/shinchiro/mpv-winbuild-cmake/releases)
  and put its `libmpv-2.dll` in `src-tauri/binaries/`.

Build from PowerShell or cmd. Git Bash does not work, because its `link.exe`
shadows the MSVC linker.

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\windows-build.ps1
```

The script loads the MSVC environment, generates the `mpv.lib` import library
from the DLL the first time, runs `cargo tauri build` and copies the installer
to `installers\windows\`. The installer ships the DLL next to the executable.
The script also rewrites the local paths that Rust embeds in the executable,
so the user name of the build machine does not end up in it.

Optional: with `ffprobe.exe` on the `PATH`, or in a `binaries` folder next to
the executable, the app reads the tracks of a file before playing it.

## Linux

On Debian 13, Ubuntu 24.04 or newer, run as root:

```bash
bash scripts/linux-deps.sh
bash scripts/linux-build.sh deb     # or: appimage, all
```

`linux-deps.sh` installs the build tools, WebKitGTK and libmpv with apt, then
Rust and tauri-cli into the home of the account that runs it. That suits a
dedicated build environment such as a WSL distribution or a container. On your
own desktop you can install the same apt packages, then Rust and tauri-cli as
your user, and run `linux-build.sh` without root.

The .deb depends on `libmpv2` and `ffmpeg`. Build on the oldest distribution
you want to support: the package uses the glibc and the libraries of the build
system.

`scripts/linux-smoke-test.sh [x11|wayland]` starts the built binary, plays a
generated clip through the mpv IPC socket and checks that frames are rendered.
It needs `ffmpeg` and `socat`.

## Android

The APKs are built on Debian, native or in WSL 2. Run the three scripts as
root, so that the Rust toolchain, the Android SDK and the signing key live in
the same home:

```bash
bash scripts/linux-deps.sh           # Rust and tauri-cli
bash scripts/android-setup.sh        # JDK, Android SDK and NDK, Rust targets
bash scripts/android-build.sh apk    # installers/android/SIIISHUB{,-TV}_<version>_{arm64,armv7}.apk
```

`android-setup.sh` accepts the Android SDK licenses on your behalf. The build
makes two APKs per ABI: `SIIISHUB` for phones and tablets, `SIIISHUB-TV` for
Android TV (Cargo feature `tv`, listed in the TV launcher). `apk phone` or
`apk tv` builds only one of them. The APKs need Android 8.0 or newer: `arm64`
fits almost every phone and tablet, `armv7` is for older 32-bit devices and
for the many TVs with a 32-bit system.

From Windows, with the sources on the Windows drive:

```powershell
wsl -d Debian -u root -- bash /mnt/c/path/to/siiishub/scripts/android-build.sh apk
```

Sources under `/mnt` are mirrored to `~/siiishub-apk` first, because building
on a Windows drive is very slow. From Git Bash, prefix the command with
`MSYS_NO_PATHCONV=1` so the `/mnt` path is left alone.

**Signing key.** The first build creates `~/siiishub-android.keystore`, with
its random password in `~/siiishub-keystore.properties`. Back up both files:
an update signed with a different key cannot be installed over the previous
version.

**Device.** `scripts/android-device.sh` wraps adb for a device paired over
Wireless debugging: `pair`, `connect`, `install`, `launch`, `logcat` and
`screenshot`.

## Debugging

- `DEVTOOLS=1 bash scripts/android-build.sh apk`, or `-DevTools` for the
  Windows script, keeps WebView remote debugging in a release build, so the
  interface can be inspected with `chrome://inspect`. Never distribute such
  a build.
- Desktop logs go to the console; set `RUST_LOG=debug` for more detail.
  Android logs go to logcat: `adb logcat -s siiishub mpv`.
- `SIIISHUB_MPV_IPC=/tmp/mpv.sock` exposes the mpv JSON IPC socket, or a named
  pipe on Windows, to drive playback from outside the app.
- `cargo tauri dev` in `src-tauri` runs a development build that reloads
  `dist/` on change. On Windows, run it from a shell with the MSVC environment.
- Without libmpv, `cargo check --no-default-features --features custom-protocol,stub-mpv`
  builds the desktop interface without video.
- `node scripts/check-i18n.mjs` checks that every language has the same keys
  as English.
