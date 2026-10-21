# Android build

This directory packages the Rust engine as an Android `arm64-v8a` app. It is
an experimental community port and is not a stable or release candidate build.
See [the test status](TESTING-STATUS.md) before trying it.

## Graphics and platform

The engine is compiled for `aarch64-linux-android` and renders through wgpu.
The Android build enables the system Vulkan and GLES backends so wgpu can use a
compatible adapter exposed by Android. The APK does not include a custom GPU
driver, and no GPU vendor is hard-coded. This is native ARM64 code; the Android
path does not execute the PC's DirectX renderer or translate x86 instructions.

This approach aims for broad Android compatibility, but it has only been tried
on one ARM64 Android 16 phone. It does not establish support for every Android
version or for Mali, Adreno, Xclipse, or other GPU families.

## Build on Termux

The build script uses the Android Rust target and local Android tools. You need
Rust with `aarch64-linux-android`, Clang/NDK support, an Android API jar, AAPT2,
D8, `zipalign`, `apksigner`, Java, and the converted game data prepared from your
own copy. Set `ANDROID_WORLD_INDEX` to the generated `world_index.json`, then
run from the repository root:

```sh
bash android-game/build.sh
```

For an SDK/NDK Linux host, set `ANDROID_HOME` and install the Android platform,
build tools, and NDK versions noted by the script. Environment overrides for
the API level, resource table, D8, package name, and output name are documented
in [`build.sh`](build.sh).

The signed development APK is written under `android-game/build/`. It uses a
local debug key and is not suitable for publishing as a release. Build outputs,
signing keys, converted game data, and full device diagnostics are intentionally
excluded from Git.

## Data and setup

The APK does not contain the commercial game. Prepare and convert your own
legally obtained copy using the project's tools. Place the converted files in
the expected `Rusty/RustyMCLA` shared-storage folder and grant the app the file
access permission it requests. The small world index is packaged separately to
avoid scanning all city files on the phone at startup. Saves are kept beside
the converted data under `data/cache`.

## Android controls and settings

The setup screen includes a default or performance preset and controls for
render scale, frame cap, blur, effects, pedestrian density, and draw distance.
The values are read when the game starts. The driving touch layout supports
steering and pedals at the same time. Android controller events are bridged to
the Rust input path, but physical controller mapping has not been fully tested.

The loading screen reports startup progress. The launcher can export a text
diagnostic with recent engine logs and an available Android ANR trace. Please
review and redact device-specific or personal content before sharing a report.

## Test status

See [TESTING-STATUS.md](TESTING-STATUS.md) for build evidence, the latest saved
frame-time results, known failures, and remaining checks. The current evidence
does not support calling this port stable or playable at normal frame rates.
