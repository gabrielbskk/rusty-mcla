# Android port: known issues and useful contributions

This branch is experimental. The APK has not been verified as playable.

## Startup ANR and missing ground on the Galaxy S23

Physical test device: one Android 16 ARM64 phone,
using the system Vulkan driver and converted data from the tester's own copy.

- Version 0.6 displayed the vehicle and parts of the city, but the ground was
  white or absent. Another run showed a black screen.
- Version 0.7 still produced 10-second input dispatch ANRs for touch and key
  events. The last shared game log reached UI/map asset loading, before the
  `Android scene initialization complete` and first-frame messages.
- In one excerpt the Java main thread waited inside `android_activity` during
  `NativeActivity.onPause`. Another excerpt showed it idle in the Looper.
  Neither excerpt contained the native `android_main` stack, so the blocking
  operation is unknown. The reported memory values do not establish OOM.

`ApplicationHandler::resumed` in `mcla-rs/crates/game/src/main.rs` initializes
Vulkan, loads subsystems, and uploads scene assets synchronously before it
returns to the Android event loop. A pause arriving during this work may wait
for the native thread. This is a hypothesis to test, not a confirmed cause.

Version 0.8 can export a `.txt` report with the recent game log, a summary that
prioritizes `android_main`, and the available ANR trace up to 4 MiB. The native
engine is the same as in 0.7. Useful next work: analyze the complete trace;
measure initialization stages; make long startup work responsive to lifecycle
events; verify the first rendered frame and ground geometry; test pause/resume.

Please share technical logs without commercial game data, personal information,
or third-party driver files. Testing requires a legally obtained copy of the
game. Pull requests can target `android-experimental`.
