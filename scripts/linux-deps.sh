#!/bin/bash
# Installs everything needed to build SIIISHUB on Debian 12/13 or Ubuntu 24.04+
# (native or WSL): build tools, WebKitGTK, libmpv, Rust (rustup) and tauri-cli.
# Run as root (or with sudo). Safe to run again.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  build-essential curl wget file ca-certificates pkg-config git rsync \
  libssl-dev libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  librsvg2-dev libxdo-dev libmpv-dev patchelf binutils \
  libgtk-3-bin libglib2.0-bin desktop-file-utils squashfs-tools xz-utils \
  xdg-utils ffmpeg dpkg-dev fakeroot
if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal -q
fi
. "$HOME/.cargo/env"
if [ ! -x "$HOME/.cargo/bin/cargo-tauri" ]; then
  TMP=$(mktemp -d)
  if curl -fsSL -o "$TMP/cli.tgz" https://github.com/tauri-apps/tauri/releases/download/tauri-cli-v2.11.0/cargo-tauri-x86_64-unknown-linux-gnu.tgz; then
    tar -xzf "$TMP/cli.tgz" -C "$TMP"
    install -m755 "$(find "$TMP" -name cargo-tauri -type f | head -1)" "$HOME/.cargo/bin/cargo-tauri"
  else
    cargo install tauri-cli --version "^2" --locked
  fi
fi
rustc --version; cargo tauri --version
echo "Dependencies ready."
