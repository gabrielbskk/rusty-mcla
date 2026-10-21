#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [[ -z "$sdk" ]]; then echo 'Set ANDROID_HOME to the Android SDK.' >&2; exit 1; fi
ndk="${ANDROID_NDK_HOME:-$sdk/ndk/27.2.12479018}"
toolchain="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
tools="$sdk/build-tools/35.0.0"
android_jar="$sdk/platforms/android-35/android.jar"
export PATH="$toolchain:$PATH"
export CC="$toolchain/aarch64-linux-android26-clang"
export CXX="$toolchain/aarch64-linux-android26-clang++"
export AR="$toolchain/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC"
if [[ "${SKIP_RUST_BUILD:-0}" != 1 ]]; then
  (cd ../mcla-rs && cargo build -p mcla --lib --target aarch64-linux-android --release)
fi
library=../mcla-rs/target/aarch64-linux-android/release/libmcla_android.so
if [[ ! -f "$library" ]]; then echo "Missing $library" >&2; exit 1; fi
mkdir -p build/classes build/package/lib/arm64-v8a
if command -v javac >/dev/null 2>&1; then
  javac --release 8 -encoding UTF-8 -classpath "$android_jar" -d build/classes src/org/rustymcla/game/MainActivity.java
elif [[ -n "${ECJ_JAR:-}" ]]; then
  java -jar "$ECJ_JAR" -1.8 -encoding UTF-8 -classpath "$android_jar" -d build/classes src/org/rustymcla/game/MainActivity.java
else
  echo 'Install a JDK (javac), or set ECJ_JAR to the Eclipse Java compiler jar.' >&2
  exit 1
fi
"$tools/d8" --min-api 26 --lib "$android_jar" --output build/package build/classes/org/rustymcla/game/*.class
cp "$library" build/package/lib/arm64-v8a/libmcla_android.so
"$tools/aapt2" link -o build/unsigned.apk -I "$android_jar" --manifest AndroidManifest.xml --min-sdk-version 30 --target-sdk-version 30
(cd build/package && zip -q -u ../unsigned.apk classes.dex lib/arm64-v8a/libmcla_android.so)
"$tools/zipalign" -f 4 build/unsigned.apk build/aligned.apk
if [[ ! -f build/debug.keystore ]]; then
  keytool -genkeypair -keystore build/debug.keystore -storepass android -keypass android -alias androiddebugkey -dname 'CN=Android Debug,O=Android,C=US' -keyalg RSA -keysize 2048 -validity 10000 >/dev/null 2>&1
fi
"$tools/apksigner" sign --ks build/debug.keystore --ks-pass pass:android --key-pass pass:android --out build/rusty-mcla-android-dev.apk build/aligned.apk
"$tools/apksigner" verify build/rusty-mcla-android-dev.apk
echo "APK: $(pwd)/build/rusty-mcla-android-dev.apk"
