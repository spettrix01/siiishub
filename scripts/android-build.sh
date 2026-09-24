#!/bin/bash
# Builds the SIIISHUB Android APKs, one per ABI (arm64 and armv7), for the
# phone and for the TV.
# Usage: scripts/android-build.sh [apk|check] [phone|tv|all]
#        (default: apk all)
# Requires scripts/android-setup.sh to have run once.
#
# - phone: SIIISHUB_<version>_<abi>.apk, the mobile layout; the phone is the
#   remote of a PC or of the server version.
# - tv: SIIISHUB-TV_<version>_<abi>.apk (feature `tv`), the interface of the
#   PC driven with the TV's remote, a phone as its remote too, and listed in
#   the TV's launcher.
# Both are dev.siiis.siiishub: either installs over the other, and over 1.1.0
# (one APK for both), keeping the settings and the library.
#
# Environment:
#   OUT_DIR   where the APKs are copied (default: <repo>/installers/android)
#   DEVTOOLS  1 keeps WebView remote debugging in the release APK, to inspect
#             the UI of a test device; never for APKs meant for users
#
# Building on a Windows drive seen from WSL (/mnt/...) is very slow, so the
# sources are first mirrored to ~/siiishub-apk.
set -euo pipefail
. "$HOME/.cargo/env"
. "$HOME/.siiishub-android.env"
MODE=${1:-apk}
VARIANTS=${2:-all}
case "$VARIANTS" in
  all) VARIANTS="phone tv" ;;
  phone|tv) ;;
  *) echo "unknown variant: $VARIANTS (phone, tv or all)"; exit 1 ;;
esac
ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT_DIR=${OUT_DIR:-$ROOT/installers/android}
BASE_FEATURES=()
if [ "${DEVTOOLS:-0}" = "1" ]; then BASE_FEATURES=(devtools); fi
case "$ROOT" in
  /mnt/*)
    WORK=$HOME/siiishub-apk
    mkdir -p "$WORK"
    rsync -a --delete --exclude '/.git' --exclude '/installers' \
      --exclude '/src-tauri/target' --exclude '/src-tauri/binaries' \
      --exclude '/src-tauri/gen/android' \
      --exclude '/src-tauri/plugins/android-player/android/.tauri' \
      --exclude '/src-tauri/plugins/android-player/android/build' \
      "$ROOT/" "$WORK/"
    ;;
  *) WORK=$ROOT ;;
esac
cd "$WORK/src-tauri"

# First run: generate the Gradle project in src-tauri/gen/android.
if [ ! -f gen/android/app/build.gradle.kts ]; then
  cargo tauri android init --skip-targets-install
fi

# Signing: a local keystore, created once, makes the APK installable by
# sideloading. Keep it safe: an APK signed with another key does not install
# over the previous version.
KEYSTORE=$HOME/siiishub-android.keystore
KS_PROPS=$HOME/siiishub-keystore.properties
if [ ! -f "$KEYSTORE" ]; then
  PASS=$(head -c 24 /dev/urandom | base64 | tr -d '/+=' | head -c 24)
  keytool -genkeypair -v -keystore "$KEYSTORE" -alias siiishub -keyalg RSA -keysize 2048 \
    -validity 10000 -storepass "$PASS" -keypass "$PASS" \
    -dname "CN=SIIISHUB" >/dev/null 2>&1
  cat > "$KS_PROPS" <<EOF
storeFile=$KEYSTORE
storePassword=$PASS
keyAlias=siiishub
keyPassword=$PASS
EOF
  chmod 600 "$KS_PROPS"
  echo "keystore created in $KEYSTORE (credentials in $KS_PROPS)"
fi
cp "$KS_PROPS" gen/android/keystore.properties

# Signing config injected into the generated build.gradle.kts (idempotent).
GRADLE=gen/android/app/build.gradle.kts
if ! grep -q 'keystore.properties' "$GRADLE"; then
  python3 - "$GRADLE" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p).read()
signing = '''
    signingConfigs {
        create("release") {
            val props = Properties()
            val f = rootProject.file("keystore.properties")
            if (f.exists()) {
                props.load(FileInputStream(f))
                storeFile = file(props.getProperty("storeFile"))
                storePassword = props.getProperty("storePassword")
                keyAlias = props.getProperty("keyAlias")
                keyPassword = props.getProperty("keyPassword")
            }
        }
    }
'''
for imp in ('import java.util.Properties', 'import java.io.FileInputStream'):
    if imp not in s:
        s = imp + '\n' + s
# signingConfigs block right after the opening of android { ... }
s = re.sub(r'(\nandroid \{\n)', r'\1' + signing, s, count=1)
# the release build type uses it
s = re.sub(r'(getByName\("release"\) \{\n)', r'\1            signingConfig = signingConfigs.getByName("release")\n', s, count=1)
open(p, 'w').write(s)
print('signing config injected')
PY
fi

# minSdk: the generated project keeps the value of the first `android init`;
# libmpv needs Android 8.0 (26).
sed -i -E 's/minSdk = 2[0-5]$/minSdk = 26/' gen/android/app/build.gradle.kts

# Cleartext HTTP: the phone can be the remote of SIIISHUB on a PC, whose
# remote page and WebSocket are plain HTTP on the local network (remote.rs).
# The template allows cleartext traffic in debug builds only.
sed -i 's/manifestPlaceholders\["usesCleartextTraffic"\] = "false"/manifestPlaceholders["usesCleartextTraffic"] = "true"/' gen/android/app/build.gradle.kts

# R8: the wry ProGuard template does not keep WryActivity.getId(), which the
# tao JNI glue calls at startup (NoSuchMethodError, then abort). Minification
# saves almost nothing here (the size is native code), so it stays off; the
# keep rules are added anyway in case it gets turned back on.
sed -i "s/isMinifyEnabled = true/isMinifyEnabled = false/" gen/android/app/build.gradle.kts
if ! grep -q "WryActivity { *; }" gen/android/app/proguard-rules.pro; then
  printf "
# tao/wry JNI glue looks these up by name at startup.
-keep class dev.siiis.siiishub.WryActivity { *; }
-keep class dev.siiis.siiishub.TauriActivity { *; }
-keep class dev.siiis.siiishub.MainActivity { *; }
" >> gen/android/app/proguard-rules.pro
fi

# App icons: `android init` ships the default Tauri launcher icons; ours live
# in icons/android (generated by `tauri icon`).
if [ -d icons/android ]; then
  cp -r icons/android/. gen/android/app/src/main/res/
fi

# Android TV: the Leanback launcher only lists activities with this category,
# which only the TV APK has.
MANIFEST=gen/android/app/src/main/AndroidManifest.xml
leanback() {
  python3 - "$MANIFEST" "$1" <<'PY'
import re, sys
p, on = sys.argv[1], sys.argv[2] == 'on'
s = open(p).read()
line = '\n                <category android:name="android.intent.category.LEANBACK_LAUNCHER" />'
s = s.replace(line, '')
if on:
    s = re.sub(r'(<category android:name="android.intent.category.LAUNCHER" ?/>)', r'\1' + line, s, count=1)
open(p, 'w').write(s)
PY
}

VERSION=$(grep -m1 '"version"' tauri.conf.json | sed -E 's/.*"([0-9.]+)".*/\1/')
for VARIANT in $VARIANTS; do
  FEATURES=("${BASE_FEATURES[@]}")
  PREFIX=SIIISHUB
  if [ "$VARIANT" = tv ]; then
    FEATURES+=(tv)
    PREFIX=SIIISHUB-TV
    leanback on
  else
    leanback off
  fi
  FEATURE_ARGS=()
  if [ ${#FEATURES[@]} -gt 0 ]; then FEATURE_ARGS=(--features "$(IFS=,; echo "${FEATURES[*]}")"); fi
  echo "== $VARIANT (${FEATURE_ARGS[*]:-no extra features})"
  case "$MODE" in
    check)
      # Plain cargo needs the NDK compiler and linker that `cargo tauri android`
      # otherwise sets up (ring and a few other crates build C code).
      NDK_BIN=$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin
      export CC_aarch64_linux_android=$NDK_BIN/aarch64-linux-android26-clang
      export AR_aarch64_linux_android=$NDK_BIN/llvm-ar
      export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$NDK_BIN/aarch64-linux-android26-clang
      cargo check --target aarch64-linux-android "${FEATURE_ARGS[@]}" 2>&1 | tail -40
      ;;
    apk)
      # One APK per ABI: libmpv weighs tens of MB per ABI. The previous
      # variant's APKs go first, so only this one's are copied.
      rm -rf gen/android/app/build/outputs/apk
      cargo tauri android build --apk --split-per-abi --target aarch64 --target armv7 "${FEATURE_ARGS[@]}"
      mkdir -p "$OUT_DIR"
      find gen/android/app/build/outputs/apk -name '*release*.apk' | while read -r apk; do
        case "$apk" in
          *arm64*) abi=arm64 ;;
          *x86_64*) abi=x86_64 ;;
          *x86*) abi=x86 ;;
          *arm*) abi=armv7 ;;
          *) abi=$(basename "$apk" .apk) ;;
        esac
        cp -v "$apk" "$OUT_DIR/${PREFIX}_${VERSION}_${abi}.apk"
      done
      ;;
    *) echo "unknown mode: $MODE"; exit 1 ;;
  esac
done
# The Gradle project is left as the phone's.
leanback off
[ "$MODE" = apk ] && echo "APKs copied to $OUT_DIR"
echo "ANDROID BUILD $MODE OK"
