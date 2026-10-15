# Building from source

This is for people who want to build Rusty MCLA themselves. If you only want to play, use the launcher from the Releases page.
Windows 10 or 11 (x64) is the supported build machine. Linux and macOS are planned; CI only checks that the platform-independent crates and the launcher compile there.

There are two cargo workspaces, with two different toolchains **on purpose**:

| Folder | What | Rust toolchain |
|---|---|---|
| `mcla-rs/` | the game (`mcla.exe`) and `mcla-tool` | `stable-x86_64-pc-windows-gnu` with a mingw-w64 **UCRT** C toolchain (the vehicle physics was checked bit for bit with it; do not switch it to MSVC without comparing results) |
| `launcher/` | the launcher (`rusty-mcla.exe`, Tauri 2) | `stable-x86_64-pc-windows-msvc` with a static C runtime, so it is one exe that needs nothing installed |

CI builds with Rust 1.99.0 (`RUST_VERSION` in `.github/workflows/ci.yml`); a current stable toolchain should work.

## 1. Prerequisites

Run `tools\check-prerequisites.ps1` (it only checks and prints the command that fixes what is missing; it installs nothing). What it looks for:

| Need | For | Install |
|---|---|---|
| Git | cloning | `winget install --id Git.Git -e` |
| Rust (rustup) with both toolchains | everything | `winget install --id Rustlang.Rustup -e`, then `rustup toolchain install stable-x86_64-pc-windows-gnu stable-x86_64-pc-windows-msvc --profile minimal` |
| mingw-w64, UCRT, with `gcc` and `dlltool` on `PATH` | the game | `winget install --id BrechtSanders.WinLibs.POSIX.UCRT -e` (GCC 16.2 is what the project uses), or MSYS2's `ucrt64` environment |
| Visual Studio 2022 Build Tools, workload "Desktop development with C++" | the launcher | `winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` |
| WebView2 runtime | running the launcher | already part of Windows 11 and current Windows 10; else `winget install --id Microsoft.EdgeWebView2Runtime -e` |

Open a **new** terminal after installing. WinLibs from winget is not put on `PATH` by the installer; for the game build add it in the terminal you build in:

```powershell
$mingw = (Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Directory -Filter "BrechtSanders.WinLibs*" | Select-Object -First 1).FullName + "\mingw64\bin"
$env:PATH = "$mingw;$env:PATH"
```

## 2. Get the source

```powershell
git clone https://github.com/holdmysocks/rusty-mcla.git
cd rusty-mcla
```

## 3. The game and the tools

```powershell
cd mcla-rs
cargo build --release -p mcla -p mcla-tool       # -> target\release\mcla.exe, target\release\mcla-tool.exe
cargo test --release --workspace                 # needs no game data: tests that need the disc or extracted data skip themselves
```

Use a private target folder if you build several copies at once: `$env:CARGO_TARGET_DIR = "C:\build\mcla"`.

## 4. The launcher

```powershell
cd launcher
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo +stable-x86_64-pc-windows-msvc build --release        # -> target\release\rusty-mcla.exe  (about 7 MB)
cargo +stable-x86_64-pc-windows-msvc test --release --workspace
```

The window's page (`launcher/ui`) is embedded at compile time, so rebuild after changing it. Details: `launcher/README.md`, `mcla-rs/README.md`.

## 5. Running what you built

You need your own disc image of the Xbox 360 *Midnight Club: Los Angeles, Complete Edition (USA, Europe)*; nothing of the game is in this repository.

**Build the game data once** (about two minutes, about 14 GB of disk; it downloads a pinned `ffmpeg` if there is none):

```powershell
mcla-rs\target\release\mcla-tool.exe install "C:\path\to\the-disc.iso" C:\RustyMCLA
mcla-rs\target\release\mcla.exe --data C:\RustyMCLA\data\cache       # the game alone, no launcher
```

**The launcher against that folder.** The launcher is portable: it looks for everything beside itself. Put the exes into the folder that holds `data\` and `manifest.local.json`:

```powershell
Copy-Item launcher\target\release\rusty-mcla.exe C:\RustyMCLA\RustyMCLA.exe
New-Item -ItemType Directory C:\RustyMCLA\game | Out-Null
Copy-Item mcla-rs\target\release\mcla.exe C:\RustyMCLA\game\mcla.exe
$env:RUSTY_MCLA_USER_DIR = "C:\RustyMCLA-user"       # optional: settings, logs and saves of this test, apart from your normal ones
C:\RustyMCLA\RustyMCLA.exe
```

PLAY then starts `game\mcla.exe` with `--data` set. Without `manifest.local.json` the launcher shows its first-run page and offers to build the data itself. The update check looks at the repository named in `launcher\release.json` (compiled in); set `RUSTY_MCLA_RELEASE_REPO=owner/name` to look at a fork.

## Dependency licences

`deny.toml` lists the licences the project accepts. `cargo install cargo-deny --locked`, then `cargo deny --manifest-path mcla-rs/Cargo.toml check licenses sources` (and the same for `launcher/Cargo.toml`).

## Releases

Maintainers: the release is built by `.github/workflows/release.yml` and signed on the maintainer's own machine with `tools\sign-release.ps1`. Only the launcher exe carries a signature; the launcher checks the game package against the SHA-256 in the release manifest.
