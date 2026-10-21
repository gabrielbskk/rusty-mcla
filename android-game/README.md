# Android game build (experimental)

This folder packages the **actual Rust game engine** as a 64-bit Android
`NativeActivity`. It is a development build, not a verified playable release.

## Build

Install Rust with the `aarch64-linux-android` target, Android SDK platform 35,
build tools 35.0.0, and NDK 27.2.12479018. Set `ANDROID_HOME` and run
`bash android-game/build.sh` from the repository root. A JDK with `javac` is
needed for the data import Activity; `ECJ_JAR` can point to the Eclipse Java
compiler instead. The output is `android-game/build/rusty-mcla-android-dev.apk`.

The APK contains engine code and a small index derived from the user's converted
city files. It does not contain the commercial game assets.
Version 0.2 reads a converted install directly from
`/storage/emulated/0/Rusty/RustyMCLA` (or directly from `Rusty`). Its setup
Activity requests Android's all-files access permission and checks the folder
before starting the native engine. The converted install is not copied into
app-private storage. The Activity joins the two `intro.mcv.part0x` files from
the Drive package in place on the first launch. Save data remains in the
app-private directory. The original split ZIP files are not a converted
install; all 154 converted ZIPs must be extracted first.

## Current mobile input

While driving, the left half of the screen steers (left quarter = left,
second quarter = right). The upper right accelerates, lower right brakes, and
the top strip sends Back. Multitouch lets steering and pedals work together.
Menus receive touch events through egui. Android controllers are not yet
connected to the engine.

## Verification still needed

- Install and launch on a physical ARM64 device with Vulkan support.
- Measure shader and texture compatibility, memory use and frame rate.
- Check audio, cutscenes, save/load, suspend/resume and all touch menus.
- Provide an Android or cloud conversion path for an original split archive.

The project does not distribute game data. The user must provide their own
copy. The APK uses a locally generated debug key and is not a release build.

Before building the APK, run `mcla-tool world-index /path/to/data/cache` on the
converted installation and set `ANDROID_WORLD_INDEX` to the resulting
`world_index.json`. The setup Activity installs the index to the shared game
folder on first launch. This skips scanning all city files on the phone.
