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
the Drive package in place on the first launch. Game saves are written beside
the converted data under `data/cache`. The original split ZIP files are not a converted
install; all 154 converted ZIPs must be extracted first.

The tester-supplied cover is displayed on the setup screen and used as the launcher
icon. Its third-party artwork rights have not been independently verified. On Android, the city streamer starts with a 210 m radius and at most
three sector decodes at once. The full city map is built only when requested.
The setup screen reports recent frame and sector progress. Version 0.8 can share
its diagnostic as a text file, with a short summary that prioritizes the native
`android_main` thread and an available ANR trace of up to 4 MiB. This improves
troubleshooting; it does not establish a fix for the ANRs.

## Current mobile input

While driving, the left half of the screen steers (left quarter = left,
second quarter = right). The upper right accelerates, lower right brakes, and
the top strip sends Back. Multitouch lets steering and pedals work together.
Menus receive touch events through egui. Android controllers are not yet
connected to the engine.

## Physical test status

The tester uses a Android device Android device, Android 16, with an Adreno 740
Vulkan GPU. Version 0.6 showed a vehicle and parts of the city, but the ground
was white or absent. Version 0.7 still produced black screens and input ANRs.
One trace showed the Java main thread waiting during `NativeActivity.onPause`;
the native game thread was not present in the excerpt. The cause remains under
investigation. RAM readings in the supplied traces do not establish an OOM.
Please attach a full version 0.8 text diagnostic when reporting Android issues.
See [known issues and useful contributions](KNOWN-ISSUES.md).

## Verification still needed

- Confirm a complete playable session on a physical ARM64 device with Vulkan support.
- Measure shader and texture compatibility, memory use and frame rate.
- Check audio, cutscenes, save/load, suspend/resume and all touch menus.
- Provide an Android or cloud conversion path for an original split archive.

The project does not distribute game data. The user must provide their own
copy. The APK uses a locally generated debug key and is not a release build.

Before building the APK, run `mcla-tool world-index /path/to/data/cache` on the
converted installation and set `ANDROID_WORLD_INDEX` to the resulting
`world_index.json`. The setup Activity installs the index to the shared game
folder on first launch. This skips scanning all city files on the phone.
