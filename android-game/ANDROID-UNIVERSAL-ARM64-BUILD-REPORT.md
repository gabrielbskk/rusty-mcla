# Historical build report — Rusty MCLA Android ARM64 (APK 0.9.5)

Date: 2026-10-21

This report covers build 0.9.5, not the current checkout. The latest runtime
results are recorded in `TESTING-STATUS.md`; they show performance below 1 FPS
and do not establish stability.

## Goal

Build the Rust engine for Android ARM64 using system graphics APIs, without GPU
vendor selection, custom drivers, or x86 instruction translation.

## Changes

- wgpu enables Vulkan and GLES simultaneously on Android. It chooses an adapter
  compatible with the surface; the selected backend is recorded in the game log.
- The Android wgpu build enables Vulkan and GLES. The desktop declaration remains
  separate; `sdl3-sys` is not included in the Android target.
- Relative mouse input and the right mouse button are desktop-only. Shared
  keyboard paths, touch, and Android controller input remain available.
- Android cpal uses Oboe and `libc++_shared`. winit continues to use
  `android-activity` through its NativeActivity backend.
- The build does not select Adreno/Qualcomm or import Turnip or another driver.
- The Termux packager uses `/system/framework/framework-res.apk` for AAPT2
  linking because Termux's `android.jar` supports javac/D8 but does not contain
  `resources.arsc`. `ANDROID_FRAMEWORK_RES` can override this path.

## Build

Target: `aarch64-linux-android` (`arm64-v8a`)

Rust command:

```sh
cargo build -p mcla --lib --target aarch64-linux-android --release --features android-fps-counter
```

Outputs:

- `mcla-rs/target/aarch64-linux-android/release/libmcla_android.so`
- `android-game/build/Rusty-MCLA-Android-0.9.5-Universal-ARM64.apk`

## Build verification

- The final `cargo build` completed successfully in 15m18s.
- The `.so` is an ELF64 shared object for AArch64, targeting Android API 24.
- Final `.so` size: 155,371,200 bytes.
- The Android Cargo tree confirmed that wgpu-core includes GLES and Vulkan
  backends; SDL3, DX12, Metal, WinAPI, Adreno, and Turnip were not dependencies
  of the target.
- AAPT2 packaged the APK and `apksigner verify` passed.
- The APK was installed with `adb install -r` and its launch activity was
  started.

## Known limits

- This confirms a native ARM64 build and backend selection without vendor
  hardcoding; it does not prove compatibility with every Android device.
- The APK declares minSdk 30 and GLES 3.0 as its minimum. Actual compatibility
  depends on system Vulkan/GLES features and driver quality.
- Testing was limited to one Android 16 ARM64 device. This build was not tested
  in a gameplay session, and Mali or Xclipse GPUs were not validated.
- The Rust build emitted 151 warnings; javac emitted 3 source/target 8 warnings.
  Both completed without errors.

## Stability note

Successful compilation and installation do not demonstrate stability, frame
rate, or freedom from ANRs. A new diagnostic after a gameplay session is needed
to verify those points.
