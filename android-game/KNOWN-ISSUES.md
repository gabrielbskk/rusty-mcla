# Known Android issues

The Android port is experimental and is not verified as a playable release.
For the full evidence and release blockers, see
[TESTING-STATUS.md](TESTING-STATUS.md).

## Performance

Retained traces from one Android 16 ARM64 phone show sustained frame times of
roughly 1.7–1.9 seconds per frame after loading, or about 0.5–0.6 FPS. A test
screen showed about 2 FPS. The graphics preset, scene scale, draw distance,
effects, and sector streaming controls are implemented, but have not delivered
playable performance on that device.

## Input ANR

An earlier physical run recorded a 10-second input-dispatch ANR while the game
activity was active. Later logs reached the render loop and loaded city sectors,
but the available evidence does not confirm the ANR has been fixed. Full
play-through, suspend/resume, audio, save/load, and sustained-session checks are
still open.

## GPU compatibility

The Android build enables wgpu's system Vulkan and GLES backends. Only one
Adreno-based device has been tested. Compatibility with Mali, Xclipse, other
Adreno models, and older Android versions is unverified.

## Contributing useful reports

Please include the APK version, Android version, device/GPU family, the steps
that trigger the issue, and a short relevant log excerpt. Remove account data,
storage paths, serial numbers, unrelated notifications, and other personal
details before sharing. Do not attach converted game data, raw driver packages,
or full unreviewed diagnostic dumps.
