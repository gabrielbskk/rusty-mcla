# MCLA Android Probe

This is an **installable diagnostic APK**, not a playable game build. It uses
Android's folder picker to inspect a Rusty MCLA data directory created by the
desktop `mcla-tool install` command. It contains no original game assets or
engine binary.

## Build

Install Android SDK platform 35 and build tools 35.0.0. Set `ANDROID_HOME` to
the SDK directory and run `bash build.sh`. A JDK with `javac` is normally
required; alternatively set `ECJ_JAR` to an Eclipse Java compiler jar.

The resulting `build/mcla-android-probe.apk` is signed with a local debug key
generated on the first build. That key is never committed. Debug signatures
are for sideload testing and are not suitable for a production release.

## Use

This diagnostic is optional. It does not require selecting a folder to open.
If a converted Rusty MCLA install already exists, copy it to the device and
choose it. The app checks the local manifest, cache resources, tunes,
converted shaders, audio and video. Split ZIP files and a raw ISO are not
accepted. The Android engine has not been ported yet.

The folder picker does not grant the future Rust engine direct filesystem
access. That requires an Android data import or file descriptor bridge.
The full Android engine work is tracked in `../ANDROID.md`.
