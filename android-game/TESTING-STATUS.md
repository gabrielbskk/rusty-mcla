# Android test status

**Status: experimental; stability work is ongoing.** This page summarizes
retained build records and a single-device test session as of 2026-10-21. It
contains aggregate results only; raw logs, storage paths, process IDs, account
details, and screenshots are not published.

## What has been verified

| Area | Result | Scope |
| --- | --- | --- |
| Native build | `cargo build -p mcla --lib --target aarch64-linux-android --release --features android-fps-counter` completed successfully | Recorded for the 0.9.5 universal ARM64 APK; 151 Rust warnings, no build errors |
| Packaging | AAPT2 packaging and `apksigner verify` succeeded; APK installed and launcher opened | Recorded for 0.9.5 |
| Later test packages | 0.9.6, 0.9.7, and 0.9.8 APK artifacts were produced; the 0.9.8 test package launched on the test phone | This does not verify a clean build from the current source tree |
| Engine startup | Logs show engine initialization, UI assets, and the world streaming path reached gameplay | One ARM64 phone running Android 16 |
| Graphics backend | Build record shows wgpu Vulkan and GLES enabled for Android | No Mali or Xclipse device has been tested |

The detailed [0.9.5 build report](ANDROID-UNIVERSAL-ARM64-BUILD-REPORT.txt) is
historical evidence for that APK only.

## Current device results

The latest retained gameplay trace recorded sustained frames around **1.7–1.9
seconds each** (roughly **0.5–0.6 FPS**) after the initial loading period. A
separate test capture displayed about **2 FPS**. These rates make the current
build impractical to play.

An earlier run recorded an Android input-dispatch ANR while the game activity
was active. Later logs show the engine reaching its render loop and streaming
city sectors, but they do not establish that the ANR is fixed. The user stopped
the latest run after testing; no full race, long play session, or stability
soak was recorded.

## Implemented in the current source changes

- Native Android ARM64 build configuration with wgpu Vulkan and GLES enabled.
- Android setup screen with a dark layout, original vector launcher logo, and
  graphics controls for resolution scale, frame cap, blur, effects, pedestrians,
  and draw distance.
- Android render path changes for resolution scaling, reduced effects, sector
  streaming, staged game loading, and performance diagnostics.
- Android gamepad event bridge and immersive full-screen game activity.
- Exportable diagnostics and Android-specific build packaging support.

These are code changes, not proof of reliable behavior. The current worktree
contains source modifications beyond the build report for 0.9.5; no fresh build
of the exact source snapshot in this repository update is claimed here.

## Release blockers

- Improve frame time substantially and confirm performance during actual play.
- Reproduce and resolve the input ANR and investigate the recorded wgpu fatal
  error path.
- Verify touch, physical gamepad, audio, save/load, suspend/resume, and a full
  race on hardware.
- Test at least one Mali or Xclipse device before making broad GPU compatibility
  claims.

Do not describe this build as stable or as a release candidate until those checks
pass. For issue reports, share a short redacted summary rather than raw device
logs or screenshots containing unrelated device information.
