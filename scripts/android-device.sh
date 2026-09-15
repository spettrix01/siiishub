#!/bin/bash
# Connects an Android device over Wireless debugging and drives it with adb.
# Usage:
#   scripts/android-device.sh pair IP:PORT CODE     once: address and code from "Pair device with pairing code"
#   scripts/android-device.sh connect IP:PORT       address shown on the Wireless debugging page
#   scripts/android-device.sh install [arm64|armv7] installs the APK built by android-build.sh
#   scripts/android-device.sh logcat [SECONDS]      app log: Rust, mpv, Java crashes
#   scripts/android-device.sh screenshot FILE.png
#   scripts/android-device.sh launch | stop | devices
# Environment: OUT_DIR, where android-build.sh copied the APKs
# (default: <repo>/installers/android).
set -euo pipefail
. "$HOME/.siiishub-android.env"
ADB=$ANDROID_HOME/platform-tools/adb
PKG=dev.siiis.siiishub
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT_DIR=${OUT_DIR:-$ROOT/installers/android}
case "${1:-}" in
  pair)    "$ADB" pair "$2" "$3" ;;
  connect) "$ADB" connect "$2"; "$ADB" devices ;;
  devices) "$ADB" devices -l ;;
  install)
    ABI=${2:-arm64}
    APK=$(ls -t "$OUT_DIR"/SIIISHUB_*_"$ABI".apk | head -1)
    "$ADB" install -r "$APK" ;;
  launch)  "$ADB" shell monkey -p "$PKG" -c android.intent.category.LAUNCHER 1 >/dev/null; echo "launched" ;;
  stop)    "$ADB" shell am force-stop "$PKG"; echo "stopped" ;;
  logcat)
    "$ADB" logcat -c
    "$ADB" shell monkey -p "$PKG" -c android.intent.category.LAUNCHER 1 >/dev/null
    sleep "${2:-8}"
    "$ADB" logcat -d -v time 2>/dev/null | grep -E "siiishub|mpv|AndroidRuntime|DEBUG|libc |FATAL|Tauri|RustStdout|RustStderr|chromium" | tail -150 ;;
  screenshot)
    "$ADB" exec-out screencap -p > "${2:-/tmp/screen.png}"; echo "saved ${2:-/tmp/screen.png}" ;;
  *) echo "usage: pair|connect|devices|install|launch|stop|logcat|screenshot"; exit 1 ;;
esac
