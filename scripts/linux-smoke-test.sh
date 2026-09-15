#!/bin/bash
# Smoke test of the Linux binary: starts the app, checks in the log that the
# video pipeline (GtkGLArea + libmpv) is up, plays a generated test video
# through the mpv IPC socket and prints the playback properties.
# Requires ffmpeg and socat.
# Usage: scripts/linux-smoke-test.sh [x11|wayland] [path to the binary]
set -uo pipefail
BACKEND=${1:-x11}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${2:-$ROOT/src-tauri/target/release/SIIISHUB}
OUT=${SMOKE_OUT:-/tmp/siiishub-smoke-$BACKEND}
SOCK=/tmp/siiishub-mpv-smoke.sock
mkdir -p "$OUT"
[ -x "$BIN" ] || { echo "binary not found: $BIN"; exit 1; }
[ -f /tmp/siiishub-test.mp4 ] || ffmpeg -loglevel error -y -f lavfi -i testsrc2=duration=60:size=1280x720:rate=30 \
  -f lavfi -i sine=frequency=440:duration=60 -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac /tmp/siiishub-test.mp4
export GDK_BACKEND=$BACKEND SIIISHUB_MPV_IPC=$SOCK RUST_LOG=info,siiishub_desktop_lib=debug
rm -f "$SOCK"
"$BIN" > "$OUT/app.log" 2>&1 &
APP=$!
sleep 8
if ! kill -0 $APP 2>/dev/null; then echo "FAIL: the app exited"; tail -20 "$OUT/app.log"; exit 1; fi
echo "--- video pipeline log ($BACKEND) ---"
grep -E '\[render\]|ERROR|WARN|panick' "$OUT/app.log"
ipc() { echo "$1" | socat -t 2 - UNIX-CONNECT:"$SOCK"; }
if [ -S "$SOCK" ]; then
  ipc '{"command":["loadfile","/tmp/siiishub-test.mp4"]}' >/dev/null
  sleep 6
  for p in time-pos video-params/w video-params/h hwdec-current vo-configured estimated-vf-fps; do
    printf '%s: ' "$p"; ipc "{\"command\":[\"get_property\",\"$p\"]}"
  done
else
  echo "FAIL: mpv IPC socket missing"
fi
kill $APP 2>/dev/null; sleep 2; kill -9 $APP 2>/dev/null; wait $APP 2>/dev/null
if grep -q 'first mpv frame rendered' "$OUT/app.log"; then echo "SMOKE $BACKEND: OK"; else echo "SMOKE $BACKEND: FAIL (no frame rendered)"; exit 1; fi
