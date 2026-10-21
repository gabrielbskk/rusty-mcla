#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
api="${ANDROID_MIN_SDK_VERSION:-30}"
if [[ -n "$sdk" ]]; then
  ndk="${ANDROID_NDK_HOME:-$sdk/ndk/27.2.12479018}"
  toolchain="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
  tools="${ANDROID_BUILD_TOOLS_DIR:-$sdk/build-tools/35.0.0}"
  android_jar="${ANDROID_JAR:-$sdk/platforms/android-35/android.jar}"
  libcxx="$ndk/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so"
  export PATH="$toolchain:$PATH"
  export CC="$toolchain/aarch64-linux-android${api}-clang"
  export CXX="$toolchain/aarch64-linux-android${api}-clang++"
  export AR="$toolchain/llvm-ar"
else
  prefix="${PREFIX:-/data/data/com.termux/files/usr}"
  tools="${ANDROID_BUILD_TOOLS_DIR:-$prefix/bin}"
  android_jar="${ANDROID_JAR:-$prefix/share/java/android.jar}"
  libcxx="${ANDROID_LIBCXX_SHARED:-$prefix/lib/libc++_shared.so}"
  export CC="${CC:-$prefix/bin/aarch64-linux-android-clang}"
  export CXX="${CXX:-$prefix/bin/aarch64-linux-android-clang++}"
  export AR="${AR:-$prefix/bin/llvm-ar}"
fi
if [[ -n "${ANDROID_FRAMEWORK_RES:-}" ]]; then
  framework_resources="$ANDROID_FRAMEWORK_RES"
elif [[ -n "$sdk" ]]; then
  framework_resources="$android_jar"
else
  # Termux's android.jar is enough for javac/D8 but has no resources.arsc for
  # AAPT2. On-device, link against the system framework resource table instead.
  framework_resources=/system/framework/framework-res.apk
fi
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER:-$CC}"
if [[ "${SKIP_RUST_BUILD:-0}" != 1 ]]; then
  (cd ../mcla-rs && cargo build -p mcla --lib --target aarch64-linux-android --release --features android-fps-counter)
fi
library=../mcla-rs/target/aarch64-linux-android/release/libmcla_android.so
if [[ ! -f "$library" ]]; then echo "Missing $library" >&2; exit 1; fi
if [[ ! -s "$android_jar" ]]; then echo "Missing Android API jar: $android_jar" >&2; exit 1; fi
if [[ ! -s "$framework_resources" ]]; then echo "Missing Android framework resource table: $framework_resources (set ANDROID_FRAMEWORK_RES)" >&2; exit 1; fi
mkdir -p build/classes build/package/lib/arm64-v8a
index="${ANDROID_WORLD_INDEX:-}"
if [[ ! -s "$index" ]]; then echo 'Set ANDROID_WORLD_INDEX to the generated data/cache/world_index.json.' >&2; exit 1; fi
mkdir -p build/package/assets
cp "$index" build/package/assets/world_index.json
if command -v javac >/dev/null 2>&1; then
  javac --release 8 -encoding UTF-8 -classpath "$android_jar" -d build/classes src/org/rustymcla/game/*.java
elif [[ -n "${ECJ_JAR:-}" ]]; then
  java -jar "$ECJ_JAR" -1.8 -encoding UTF-8 -classpath "$android_jar" -d build/classes src/org/rustymcla/game/*.java
else
  echo 'Install a JDK (javac), or set ECJ_JAR to the Eclipse Java compiler jar.' >&2
  exit 1
fi
if [[ -x "$tools/d8" ]]; then
  "$tools/d8" --min-api "$api" --lib "$android_jar" --output build/package build/classes/org/rustymcla/game/*.class
elif [[ -n "${R8_JAR:-}" && -f "${R8_JAR}" ]]; then
  java -cp "$R8_JAR" com.android.tools.r8.D8 --min-api "$api" --lib "$android_jar" --output build/package build/classes/org/rustymcla/game/*.class
elif [[ -f build/r8.jar ]]; then
  java -cp build/r8.jar com.android.tools.r8.D8 --min-api "$api" --lib "$android_jar" --output build/package build/classes/org/rustymcla/game/*.class
else
  echo 'D8 not found; set ANDROID_BUILD_TOOLS_DIR or R8_JAR.' >&2; exit 1
fi
cp "$library" build/package/lib/arm64-v8a/libmcla_android.so
if [[ ! -s "$libcxx" ]]; then echo "Missing libc++_shared.so: $libcxx" >&2; exit 1; fi
cp "$libcxx" build/package/lib/arm64-v8a/libc++_shared.so
package_files=(classes.dex lib/arm64-v8a/libmcla_android.so lib/arm64-v8a/libc++_shared.so assets/world_index.json)
mkdir -p build/res-compiled
resource_files=(res/drawable/*)
"$tools/aapt2" compile "${resource_files[@]}" -o build/res-compiled
resource_args=()
for flat in build/res-compiled/*.flat; do resource_args+=(-R "$flat"); done
android_package="${ANDROID_PACKAGE_NAME:-org.rustymcla.game}"
sed -e "s/package=\"org.rustymcla.game\"/package=\"$android_package\"/" \
  -e "s/android:authorities=\"org.rustymcla.game.diagnostics\"/android:authorities=\"$android_package.diagnostics\"/" \
  AndroidManifest.xml > build/AndroidManifest.generated.xml
"$tools/aapt2" link -o build/unsigned.apk -I "$framework_resources" --manifest build/AndroidManifest.generated.xml --min-sdk-version "$api" --target-sdk-version 30 "${resource_args[@]}"
(cd build/package && zip -q -u ../unsigned.apk "${package_files[@]}")
"$tools/zipalign" -f 4 build/unsigned.apk build/aligned.apk
if [[ ! -f build/debug.keystore ]]; then
  keytool -genkeypair -keystore build/debug.keystore -storepass android -keypass android -alias androiddebugkey -dname 'CN=Android Debug,O=Android,C=US' -keyalg RSA -keysize 2048 -validity 10000 >/dev/null 2>&1
fi
apk_name="${ANDROID_APK_NAME:-rusty-mcla-android-dev.apk}"
"$tools/apksigner" sign --ks build/debug.keystore --ks-pass pass:android --key-pass pass:android --out "build/$apk_name" build/aligned.apk
"$tools/apksigner" verify "build/$apk_name"
echo "APK: $(pwd)/build/$apk_name"
