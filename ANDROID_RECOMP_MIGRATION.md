# Android ARM64 Recompilation Track — Research Note

Date: 2026-10-21

## Goal

Explore a separate Xbox 360 PowerPC static-recompilation path for Android ARM64. Keep the current Rust Android port buildable as a fallback while the recompilation route is investigated. Do not bundle game files or generated game code in the public repository.

## Findings

- The current `rusty-mcla` Android branch is a Rust game-engine implementation compiled as `arm64-v8a`; it is not a static recompilation of the Xbox 360 executable.
- The closest same-game recompilation baseline found is [LARecomp](https://github.com/mzzvxm/LARecomp). Its README describes Xbox 360 PowerPC-to-C++ static recompilation with ReXGlue and reports 24,927 function hints and 112 hooks. Its checked-in build presets and manifest target Windows x86-64. The generated directory is ignored, and code generation requires the user's own `default.xex`.
- LARecomp's README identifies a custom ReXGlue build with rendering fixes that has not been published. Its CMake target also contains Windows and D3D12-specific application code that needs platform separation before Android can build.
- The linked [Dark Souls II ARM64 repository](https://github.com/alesson000/Dark-Souls-2-RECOMP-ARM64) describes an AOT route through ReXGlue, but its README labels the project pre-release/WIP and says the 3D renderer is still being debugged. Its device table says it is awaiting the 3D pipeline; later roadmap checkboxes conflict with that status. Treat its claims as an experimental reference, not a proven playable baseline.
- Android ReXGlue ports exist as community work, but GPU compatibility is a separate problem from ARM64 CPU code generation. For example, [LibertyRecompAndroid](https://github.com/vaduur/LibertyRecompAndroid) documents Adreno/Turnip support and explicitly excludes Mali and Xclipse devices. This does not meet the existing universal-GPU requirement.

## Reuse boundary

Potentially reusable from this Android fork: the Android launcher/activity and storage flow, APK packaging, diagnostics/FPS reporting, settings UI concepts, and Rust format-analysis tools. The Rust gameplay implementation and renderer would not substitute for the generated Xbox 360 guest code or ReXGlue's Xenos/runtime subsystems.

## Current blocker

No `default.xex` or `xarchive_*.rpf` files were found in the Codespace repository or the local Rust build-input folder. That folder contains the Rust port's small `world_index` cache, which is not enough to run ReXGlue code generation. A recompilation build needs the user's own extracted Xbox 360 game files, kept outside the repository.

## Safe milestones

1. Keep the existing Rust ARM64 build as fallback; do not replace it in place.
2. In an isolated worktree, prove the Android ARM64 ReXGlue toolchain and platform runtime using a small sample before bringing in MCLA.
3. With the user's own XEX available, run MCLA code generation and resolve missing function boundaries, imports, and hooks using the same-game LARecomp research.
4. Port Android lifecycle, storage, input, audio, and the Xenos Vulkan runtime. First validate on the S23; validate Mali/Xclipse separately before claiming universal GPU support.
5. Only package a test APK after the generated guest code and Android runtime both build. Do not send it until requested.
