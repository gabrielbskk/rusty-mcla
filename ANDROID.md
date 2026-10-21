# Android port status

The original repository builds a Windows PC game and launcher. The
`android-game/` work now cross-compiles the Rust engine and packages an
experimental Android APK. Version 0.5 reached the Vulkan renderer and began
building the city index on an Android device; gameplay remains unverified.
Version 0.6 includes a precomputed index. Version 0.7 reduces initial sector
streaming work and defers the full city map on Android; device coverage remains
limited. The engine
reads converted data from a user-supplied Xbox 360 Complete Edition disc
image. The APK contains engine code and reads the converted data from the
shared `Rusty` directory on Android after the user grants all-files access.
During local testing, `mcla-tool install` converted a checksum-verified original
image into 13.5 GB of game data. `tools/pack-android-data.py` split that
install into independent ZIPs below the Drive upload limit. The import
Activity also rejoins the large intro video, which is split into two files.

## First milestone: a window on Android

1. An Android `cdylib` entry point uses winit's Android event loop extension
   and is packaged with a `NativeActivity`.
2. A `suspended()` path recreates the `App` to discard the invalid Android
   surface on pause. This needs device testing and save-state review.
3. Test the wgpu surface and full engine on a physical ARM64 device.

## Next milestones

- **Data:** The converted data is prepared outside the phone and extracted into
  the shared `Rusty` directory. The engine reads it directly; game saves are
  written in `data/cache`, while settings remain private to the app. The installer uses desktop tools such as
  ffmpeg; an on-device conversion workflow remains separate work.
- **Renderer:** Verify Vulkan support, render-target formats, texture limits,
  memory use and shader compilation on actual devices. The renderer already
  decodes BC textures to RGBA when BC compression is absent, but the memory
  cost needs measurement. The GPU device request now uses the adapter's own
  limits so a mobile adapter is not rejected by a desktop limit request.
- **Input:** Map Android touch events to steering, accelerator, brake and menu
  actions. SDL3 gamepad mappings currently load a Windows controller database;
  Android controller support needs separate testing.
- **Audio and video:** Check `rodio` output on Android and implement a platform
  audio backend if needed. Check cutscene playback and the converted media
  formats on ARM64.
- **Packaging:** Build for `aarch64-linux-android`, package the native library
  and UI assets in an APK, and keep disc-derived data outside the APK. Validate
  a cold start, suspend/resume, save/load, and a complete race on a device.

The ZIP parts of an original disc image are input material for the desktop
installer, not an Android executable. A complete image must be reconstructed
and validated before conversion; the port cannot be built by simply putting
those parts into an APK.
