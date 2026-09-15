#!/bin/bash
# Builds SIIISHUB for Linux and produces the packages (.deb, optionally AppImage).
# Usage: scripts/linux-build.sh [deb|appimage|all]   (default: deb)
#
# Environment:
#   OUT_DIR  where the packages are copied (default: <repo>/installers/linux)
#
# Under WSL with the sources on a Windows drive (/mnt/...), the sources are
# first mirrored to ~/siiishub-linux: building on /mnt/c is very slow.
set -euo pipefail
. "$HOME/.cargo/env"
MODE=${1:-deb}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT_DIR=${OUT_DIR:-$ROOT/installers/linux}
case "$ROOT" in
  /mnt/*)
    WORK=$HOME/siiishub-linux
    mkdir -p "$WORK"
    rsync -a --delete --exclude '/.git' --exclude '/installers' \
      --exclude '/src-tauri/target' --exclude '/src-tauri/binaries' \
      --exclude '/src-tauri/gen' "$ROOT/" "$WORK/"
    ;;
  *) WORK=$ROOT ;;
esac
cd "$WORK/src-tauri"
# AppImage: linuxdeploy cannot mount FUSE inside WSL or containers.
export APPIMAGE_EXTRACT_AND_RUN=1 NO_STRIP=true
case "$MODE" in
  deb)      cargo tauri build --bundles deb ;;
  appimage) cargo tauri build --bundles appimage ;;
  all)      cargo tauri build --bundles deb,appimage ;;
  *) echo "unknown mode: $MODE"; exit 1 ;;
esac
mkdir -p "$OUT_DIR"
cp -v target/release/bundle/deb/*.deb "$OUT_DIR/" 2>/dev/null || true
cp -v target/release/bundle/appimage/*.AppImage "$OUT_DIR/" 2>/dev/null || true
echo "Packages copied to $OUT_DIR"
