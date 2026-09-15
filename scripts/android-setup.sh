#!/bin/bash
# Prepares Debian (native or WSL) to build the SIIISHUB APK with Tauri 2.
# Installs Debian's JDK (default-jdk, 21 on trixie), the Android command-line
# tools, platform, build-tools and NDK through sdkmanager, the Rust Android
# targets, and writes the environment to ~/.siiishub-android.env, which
# android-build.sh and android-device.sh read.
# Rust and tauri-cli must already be installed (scripts/linux-deps.sh does it).
# Running it accepts the Android SDK licenses.
# Run as root: bash scripts/android-setup.sh
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

SDK_ROOT=${ANDROID_SDK_ROOT:-$HOME/android-sdk}
CMDLINE_TOOLS_URL="https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip"
NDK_VERSION="27.2.12479018"
PLATFORM="android-36"
BUILD_TOOLS="36.0.0"
ENV_FILE=$HOME/.siiishub-android.env

echo "== system packages =="
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  default-jdk-headless unzip curl ca-certificates rsync build-essential pkg-config libssl-dev python3

JAVA_HOME=$(dirname "$(dirname "$(readlink -f "$(command -v javac)")")")
echo "JAVA_HOME=$JAVA_HOME"

echo "== Android command-line tools =="
mkdir -p "$SDK_ROOT/cmdline-tools"
if [ ! -x "$SDK_ROOT/cmdline-tools/latest/bin/sdkmanager" ]; then
  TMP=$(mktemp -d)
  curl -fsSL -o "$TMP/cmdline-tools.zip" "$CMDLINE_TOOLS_URL"
  unzip -q "$TMP/cmdline-tools.zip" -d "$TMP"
  rm -rf "$SDK_ROOT/cmdline-tools/latest"
  mv "$TMP/cmdline-tools" "$SDK_ROOT/cmdline-tools/latest"
  rm -rf "$TMP"
fi
SDKMANAGER="$SDK_ROOT/cmdline-tools/latest/bin/sdkmanager"

echo "== SDK licenses =="
# `yes` exits with SIGPIPE when sdkmanager ends: that must not stop the script (pipefail).
yes 2>/dev/null | "$SDKMANAGER" --sdk_root="$SDK_ROOT" --licenses >/dev/null || true

echo "== platform-tools, platform $PLATFORM, build-tools $BUILD_TOOLS, NDK $NDK_VERSION =="
"$SDKMANAGER" --sdk_root="$SDK_ROOT" --install \
  "platform-tools" "platforms;$PLATFORM" "build-tools;$BUILD_TOOLS" "ndk;$NDK_VERSION" >/dev/null

echo "== Rust Android targets =="
. "$HOME/.cargo/env"
rustup target add aarch64-linux-android armv7-linux-androideabi
cargo tauri --version

cat > "$ENV_FILE" <<EOF
export JAVA_HOME="$JAVA_HOME"
export ANDROID_HOME="$SDK_ROOT"
export ANDROID_SDK_ROOT="$SDK_ROOT"
export NDK_HOME="$SDK_ROOT/ndk/$NDK_VERSION"
export PATH="\$JAVA_HOME/bin:$SDK_ROOT/platform-tools:$SDK_ROOT/cmdline-tools/latest/bin:\$PATH"
EOF
echo "environment written to $ENV_FILE"
echo "ANDROID SETUP OK"
