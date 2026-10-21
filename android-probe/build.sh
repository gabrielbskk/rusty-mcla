#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [[ -z "$sdk" ]]; then
  echo 'Set ANDROID_HOME or ANDROID_SDK_ROOT to the Android SDK directory.' >&2
  exit 1
fi
tools="$sdk/build-tools/35.0.0"
android_jar="$sdk/platforms/android-35/android.jar"
mkdir -p build/classes build/dex
if command -v javac >/dev/null 2>&1; then
  javac --release 8 -encoding UTF-8 -classpath "$android_jar" -d build/classes src/org/rustymcla/probe/MainActivity.java
elif [[ -n "${ECJ_JAR:-}" ]]; then
  java -jar "$ECJ_JAR" -1.8 -encoding UTF-8 -classpath "$android_jar" -d build/classes src/org/rustymcla/probe/MainActivity.java
else
  echo 'Install a JDK (javac), or set ECJ_JAR to the Eclipse Java compiler jar.' >&2
  exit 1
fi
"$tools/d8" --min-api 26 --lib "$android_jar" --output build/dex build/classes/org/rustymcla/probe/*.class
"$tools/aapt2" link -o build/unsigned.apk -I "$android_jar" --manifest AndroidManifest.xml --min-sdk-version 26 --target-sdk-version 28
cd build/dex
zip -q -u ../unsigned.apk classes.dex
cd ../..
"$tools/zipalign" -f 4 build/unsigned.apk build/aligned.apk
if [[ ! -f build/debug.keystore ]]; then
  keytool -genkeypair -keystore build/debug.keystore -storepass android -keypass android -alias androiddebugkey -dname 'CN=Android Debug,O=Android,C=US' -keyalg RSA -keysize 2048 -validity 10000 >/dev/null 2>&1
fi
"$tools/apksigner" sign --ks build/debug.keystore --ks-pass pass:android --key-pass pass:android --out build/mcla-android-probe.apk build/aligned.apk
"$tools/apksigner" verify build/mcla-android-probe.apk
echo "APK: $(pwd)/build/mcla-android-probe.apk"
