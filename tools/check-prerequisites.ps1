<#
.SYNOPSIS
  Checks (and only checks; it never installs anything) what is needed to build Rusty MCLA on Windows, and prints the command that fixes
  whatever is missing. See BUILDING.md.
.PARAMETER Part
  What you want to build: all (default), game (the game and its tools) or launcher.
#>
param([ValidateSet("all", "game", "launcher")][string]$Part = "all")
$ErrorActionPreference = "Continue"
$missing = 0

function Ok([string]$what, [string]$detail) { Write-Host ("  OK       {0,-34} {1}" -f $what, $detail) -ForegroundColor Green }
function Miss([string]$what, [string]$fix) { Write-Host ("  MISSING  {0}" -f $what) -ForegroundColor Red; Write-Host ("           fix: {0}" -f $fix); $script:missing++ }
function Note([string]$what, [string]$detail, [string]$fix) { Write-Host ("  optional {0,-34} {1}" -f $what, $detail) -ForegroundColor Yellow; if ($fix) { Write-Host ("           {0}" -f $fix) } }

Write-Host "Rusty MCLA: build prerequisites ($Part)"
Write-Host ""

# ---- common ----------------------------------------------------------------------------------------------------------------------------
Write-Host "Always needed"
$git = Get-Command git -ErrorAction SilentlyContinue
if ($git) { Ok "git" ((& git --version) -join " ") } else { Miss "git" "winget install --id Git.Git -e" }

$rustup = Get-Command rustup -ErrorAction SilentlyContinue
$toolchains = @()
if ($rustup) {
  Ok "rustup" ((& rustup --version 2>$null | Select-Object -First 1) -join " ")
  $toolchains = @(& rustup toolchain list 2>$null)
} else {
  Miss "rustup (Rust)" "winget install --id Rustlang.Rustup -e     (then open a new terminal)"
}
function Has-Toolchain([string]$suffix) { return @($toolchains | Where-Object { $_ -match "^(stable|\d+\.\d+(\.\d+)?)-$suffix" }).Count -gt 0 }

# ---- game --------------------------------------------------------------------------------------------------------------------------------
if ($Part -ne "launcher") {
  Write-Host ""
  Write-Host "Game and tools (GNU toolchain)"
  if ($rustup) {
    if (Has-Toolchain "x86_64-pc-windows-gnu") { Ok "Rust toolchain, GNU" "x86_64-pc-windows-gnu" }
    else { Miss "Rust toolchain stable-x86_64-pc-windows-gnu" "rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal" }
  }
  # mingw-w64 with the UCRT runtime (the project is built and checked with WinLibs GCC 16.2, POSIX threads, UCRT)
  $gcc = Get-Command gcc -ErrorAction SilentlyContinue
  $winlibs = $null
  if (-not $gcc) {
    $pkg = Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Packages"
    if (Test-Path $pkg) {
      $winlibs = Get-ChildItem $pkg -Directory -Filter "BrechtSanders.WinLibs*" -ErrorAction SilentlyContinue | ForEach-Object { Join-Path $_.FullName "mingw64\bin" } | Where-Object { Test-Path (Join-Path $_ "gcc.exe") } | Select-Object -First 1
    }
  }
  if ($gcc) {
    $v = (& gcc --version | Select-Object -First 1)
    if ($v -match "ucrt") { Ok "mingw-w64 gcc (UCRT)" $v }
    else { Write-Host "  WARNING  gcc on PATH is not a UCRT build: $v" -ForegroundColor Yellow; Write-Host "           The project is built with WinLibs POSIX UCRT; another C runtime may give slightly different maths. winget install --id BrechtSanders.WinLibs.POSIX.UCRT -e" }
    if (-not (Get-Command dlltool -ErrorAction SilentlyContinue)) { Miss "dlltool (binutils) on PATH" "use a full mingw-w64 distribution such as WinLibs (winget install --id BrechtSanders.WinLibs.POSIX.UCRT -e)" }
  } elseif ($winlibs) {
    Miss "mingw-w64 bin folder on PATH" "WinLibs is installed but not on PATH. In this terminal:  `$env:PATH = `"$winlibs;`" + `$env:PATH"
  } else {
    Miss "mingw-w64 (gcc, dlltool)" "winget install --id BrechtSanders.WinLibs.POSIX.UCRT -e     (then put its mingw64\bin folder on PATH, see BUILDING.md)"
  }
}

# ---- launcher ---------------------------------------------------------------------------------------------------------------------------
if ($Part -ne "game") {
  Write-Host ""
  Write-Host "Launcher (MSVC toolchain, static runtime)"
  if ($rustup) {
    if (Has-Toolchain "x86_64-pc-windows-msvc") { Ok "Rust toolchain, MSVC" "x86_64-pc-windows-msvc" }
    else { Miss "Rust toolchain stable-x86_64-pc-windows-msvc" "rustup toolchain install stable-x86_64-pc-windows-msvc --profile minimal" }
  }
  $vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
  $vs = $null
  if (Test-Path $vswhere) {
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null | Select-Object -First 1
  }
  if ($vs) { Ok "Visual Studio C++ build tools" $vs }
  else { Miss "Visual Studio 2022 Build Tools, C++ workload" "winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override `"--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended`"" }

  $wv = $null
  foreach ($k in @("HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}", "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}", "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}")) {
    $p = (Get-ItemProperty -Path $k -ErrorAction SilentlyContinue).pv
    if ($p) { $wv = $p; break }
  }
  if ($wv) { Ok "WebView2 runtime (to run it)" $wv }
  else { Miss "WebView2 runtime (needed to RUN the launcher)" "winget install --id Microsoft.EdgeWebView2Runtime -e" }
}

# ---- optional -----------------------------------------------------------------------------------------------------------------------------
Write-Host ""
Write-Host "Only for releases and checks (optional)"
if (Get-Command gh -ErrorAction SilentlyContinue) { Ok "GitHub CLI (gh)" "" } else { Note "GitHub CLI (gh)" "needed by tools\sign-release.ps1" "winget install --id GitHub.cli -e" }
$ms = Get-Command minisign -ErrorAction SilentlyContinue
if (-not $ms) { $pkg = Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Packages"; if (Test-Path $pkg) { $ms = Get-ChildItem $pkg -Recurse -Filter minisign.exe -ErrorAction SilentlyContinue | Select-Object -First 1 } }
if ($ms) { Ok "minisign" "" } else { Note "minisign" "needed by tools\sign-release.ps1" "winget install --id jedisct1.minisign -e" }
if (Get-Command cargo-deny -ErrorAction SilentlyContinue) { Ok "cargo-deny" "" } else { Note "cargo-deny" "dependency licence check" "cargo install cargo-deny --locked" }

Write-Host ""
if ($missing -eq 0) { Write-Host "Nothing is missing for '$Part'." -ForegroundColor Green; exit 0 }
Write-Host "$missing thing(s) missing for '$Part'. Nothing was installed; run the commands above, open a new terminal and run this check again." -ForegroundColor Red
exit 1
