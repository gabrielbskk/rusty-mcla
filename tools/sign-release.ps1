<#
.SYNOPSIS
  Signs the launcher of a DRAFT release made by the release workflow and finishes the release manifest.
  Run this on YOUR machine; the private signing key never leaves it and is never read by this script (minisign reads it and asks for its password).

.DESCRIPTION
  The release workflow cannot sign: it builds RustyMCLA.exe and game-<ver>.zip and puts a draft release on GitHub with an UNSIGNED manifest
  (manifest.unsigned.json). This script then, step by step:
    1. finds the draft release for -Tag and downloads RustyMCLA.exe and manifest.unsigned.json from it (with the GitHub CLI, `gh`);
    2. checks that the exe is the one the unsigned manifest describes (SHA-256 and size) and shows you both, then asks you to confirm;
    3. signs the exe with minisign using your offline key (minisign asks for the key's password; this script never sees or stores it),
       with the trusted comment "RustyMCLA <version>" that the launcher requires;
    4. runs `mk-manifest --finish`: it verifies the signature with the public key in launcher/update-key.pub, embeds it, checks the result with
       the launcher's own validator and writes manifest.json; then it verifies the signature once more with the minisign program itself;
    5. uploads manifest.json to the draft (replacing an older one) and tells you to review and publish the release.
  Nothing is published by this script. You publish the draft yourself on GitHub (or with `gh release edit <tag> --draft=false`).

.PARAMETER Tag
  The release tag, for example v0.1.0.
.PARAMETER KeyFile
  Your minisign SECRET key file. Default: $env:MINISIGN_KEY, else ~\.minisign\minisign.key.
.PARAMETER Minisign
  Path of minisign.exe. Default: the first minisign on PATH, else the WinGet package folder.
.PARAMETER Repo
  owner/name. Default: "repo" in launcher\release.json.
.PARAMETER PubKey
  The public key to verify with. Default: launcher\update-key.pub (the one compiled into the launcher). Only change it for tests.
.PARAMETER MkManifest
  A ready mk-manifest.exe. Default: it is built from this checkout (needs the Rust MSVC toolchain, see BUILDING.md).
.PARAMETER WorkDir
  Where the files are put. Default: a new folder under %TEMP%.
.PARAMETER DraftDir
  TEST MODE: use this local folder as the "draft release" (it must hold RustyMCLA.exe and manifest.unsigned.json) instead of GitHub.
.PARAMETER Yes
  Do not ask for confirmation before signing.
.PARAMETER NoUpload
  Make manifest.json but do not upload it.
.PARAMETER AllowPublished
  Also work on a release that is already published (normally refused: a published release should not change under its users).
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Tag,
  [string]$KeyFile = "",
  [string]$Minisign = "",
  [string]$Repo = "",
  [string]$PubKey = "",
  [string]$MkManifest = "",
  [string]$WorkDir = "",
  [string]$DraftDir = "",
  [switch]$Yes,
  [switch]$NoUpload,
  [switch]$AllowPublished
)
$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot

function Fail([string]$msg) { Write-Host ""; Write-Host "STOPPED: $msg" -ForegroundColor Red; exit 1 }
function Step([string]$msg) { Write-Host ""; Write-Host "== $msg" -ForegroundColor Cyan }
function Run-Native([string]$exe, [string[]]$arguments) {
  & $exe @arguments
  if ($LASTEXITCODE -ne 0) { Fail "$exe failed with exit code $LASTEXITCODE" }
}

if ($Tag -notmatch '^v\d+\.\d+\.\d+([-+][0-9A-Za-z.\-+]+)?$') { Fail "-Tag must look like v0.1.0 (got '$Tag')" }
$Version = $Tag.Substring(1)

# ---- what is needed -------------------------------------------------------------------------------------------------------------------
Step "Checking what is needed"
if ($PubKey -eq "") { $PubKey = Join-Path $RepoRoot "launcher\update-key.pub" }
if (-not (Test-Path $PubKey)) { Fail "public key file not found: $PubKey" }
if ((Get-Content $PubKey -Raw) -match 'NOT SET') {
  Fail "launcher\update-key.pub is still the placeholder. Make your key pair (docs: 'Signing key'), put the PUBLIC key into that file, commit it, and build the release again so the launcher in it contains the key."
}
if ($Repo -eq "") { $Repo = (Get-Content (Join-Path $RepoRoot "launcher\release.json") -Raw | ConvertFrom-Json).repo }
if ($Repo -notmatch '^[A-Za-z0-9_.\-]+/[A-Za-z0-9_.\-]+$') { Fail "repository '$Repo' is not owner/name" }

if ($Minisign -eq "") {
  $cmd = Get-Command minisign -ErrorAction SilentlyContinue
  if ($cmd) { $Minisign = $cmd.Source }
  else {
    $pkg = Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Packages"
    if (Test-Path $pkg) {
      $hit = Get-ChildItem $pkg -Recurse -Filter minisign.exe -ErrorAction SilentlyContinue | Where-Object { $_.FullName -match 'x86_64' } | Select-Object -First 1
      if ($hit) { $Minisign = $hit.FullName }
    }
  }
}
if ($Minisign -eq "" -or -not (Test-Path $Minisign)) { Fail "minisign.exe not found. Install it (winget install jedisct1.minisign) or pass -Minisign <path>." }

if ($KeyFile -eq "") { if ($env:MINISIGN_KEY) { $KeyFile = $env:MINISIGN_KEY } else { $KeyFile = Join-Path $HOME ".minisign\minisign.key" } }
if (-not (Test-Path $KeyFile)) { Fail "secret key file not found: $KeyFile (pass -KeyFile <path>; keep that file offline)" }
# a safety net, not a check of the key: the secret key must not be inside a git checkout that could publish it
# ProviderPath, not Path: for a key on a network share Path is "Microsoft.PowerShell.Core\FileSystem::\\server\share\..." which minisign cannot open
$keyFull = (Resolve-Path $KeyFile).ProviderPath
if ($keyFull.StartsWith($RepoRoot, [System.StringComparison]::OrdinalIgnoreCase)) { Fail "the secret key is inside the repository folder ($keyFull). Move it out of the checkout first." }

if ($DraftDir -eq "") {
  if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Fail "the GitHub CLI (gh) is not installed. winget install GitHub.cli, then gh auth login." }
  & gh auth status 2>&1 | Out-Null
  if ($LASTEXITCODE -ne 0) { Fail "gh is not logged in. Run: gh auth login" }
}

if ($WorkDir -eq "") { $WorkDir = Join-Path $env:TEMP ("rusty-mcla-sign-" + $Tag + "-" + (Get-Date -Format "yyyyMMdd-HHmmss")) }
New-Item -ItemType Directory -Force -Path $WorkDir | Out-Null
Write-Host "repository : $Repo"
Write-Host "release    : $Tag"
Write-Host "minisign   : $Minisign"
Write-Host "secret key : $KeyFile"
Write-Host "public key : $PubKey"
Write-Host "work folder: $WorkDir"

# ---- mk-manifest -----------------------------------------------------------------------------------------------------------------------
Step "Preparing mk-manifest"
if ($MkManifest -eq "") {
  $env:RUSTFLAGS = "-C target-feature=+crt-static"
  Push-Location (Join-Path $RepoRoot "launcher")
  try { Run-Native "cargo" @("+stable-x86_64-pc-windows-msvc", "build", "--release", "-p", "mcla-updater", "--bin", "mk-manifest") } finally { Pop-Location }
  $MkManifest = Join-Path $RepoRoot "launcher\target\release\mk-manifest.exe"
}
if (-not (Test-Path $MkManifest)) { Fail "mk-manifest not found: $MkManifest" }

# ---- get the draft -----------------------------------------------------------------------------------------------------------------------
$assetsDir = Join-Path $WorkDir "assets"
New-Item -ItemType Directory -Force -Path $assetsDir | Out-Null
$releaseId = $null

if ($DraftDir -ne "") {
  Step "TEST MODE: using the local folder $DraftDir as the draft release"
  foreach ($n in @("RustyMCLA.exe", "manifest.unsigned.json")) {
    if (-not (Test-Path (Join-Path $DraftDir $n))) { Fail "$n is not in $DraftDir" }
    Copy-Item (Join-Path $DraftDir $n) (Join-Path $assetsDir $n) -Force
  }
} else {
  Step "Finding the draft release $Tag on GitHub"
  # drafts are not returned by the "release by tag" lookups, so list the releases and pick the one with this tag
  $json = & gh api "repos/$Repo/releases?per_page=100" 2>&1
  if ($LASTEXITCODE -ne 0) { Fail "could not list the releases of $Repo : $json" }
  $matches = @((($json -join "`n") | ConvertFrom-Json) | Where-Object { $_.tag_name -eq $Tag })
  if ($matches.Count -eq 0) { Fail "no release with tag $Tag in $Repo (has the release workflow finished?)" }
  if ($matches.Count -gt 1) { Fail "more than one release with tag $Tag; delete the extra drafts on GitHub first" }
  $rel = $matches[0]
  if (-not $rel.draft -and -not $AllowPublished) { Fail "release $Tag is already published. A published release is not changed by this script (use -AllowPublished only if you are sure)." }
  $releaseId = $rel.id
  Write-Host "found release id $releaseId, draft=$($rel.draft), $($rel.assets.Count) assets"
  $sizes = @{}
  foreach ($n in @("RustyMCLA.exe", "manifest.unsigned.json")) {
    $a = @($rel.assets | Where-Object { $_.name -eq $n })
    if ($a.Count -ne 1) { Fail "the draft has no asset named $n" }
    $sizes[$n] = $a[0].size
  }
  # `gh release download` copes with drafts (it falls back to listing them) and writes the bytes as they are
  Run-Native "gh" @("release", "download", $Tag, "-R", $Repo, "-p", "RustyMCLA.exe", "-p", "manifest.unsigned.json", "-D", $assetsDir, "--clobber")
  foreach ($n in $sizes.Keys) {
    if ((Get-Item (Join-Path $assetsDir $n)).Length -ne $sizes[$n]) { Fail "$n downloaded with the wrong size" }
  }
}

# ---- check what will be signed -------------------------------------------------------------------------------------------------------
Step "Checking the files"
$exe = Join-Path $assetsDir "RustyMCLA.exe"
$unsigned = Join-Path $assetsDir "manifest.unsigned.json"
$m = Get-Content $unsigned -Raw | ConvertFrom-Json
if ($m.launcher.version -ne $Version) { Fail "the manifest says launcher version $($m.launcher.version) but the tag is $Tag" }
if ($m.version -ne $Version) { Fail "the manifest says release $($m.version) but the tag is $Tag" }
if ($m.launcher.signature) { Fail "manifest.unsigned.json already contains a signature" }
$sha = (Get-FileHash -Algorithm SHA256 $exe).Hash.ToLower()
$size = (Get-Item $exe).Length
Write-Host "RustyMCLA.exe  $size bytes"
Write-Host "  sha256 (file)     $sha"
Write-Host "  sha256 (manifest) $($m.launcher.sha256)"
if ($sha -ne $m.launcher.sha256 -or $size -ne $m.launcher.size) { Fail "RustyMCLA.exe is not the file the unsigned manifest describes. Do not sign it." }
Write-Host "The file matches the manifest. Compare the sha256 above with the one in the release notes / the workflow run before you sign."
if (-not $Yes) {
  $ans = Read-Host "Sign RustyMCLA.exe $Version with your key? (type yes)"
  if ($ans -ne "yes") { Fail "not confirmed; nothing was signed" }
}

# ---- sign ------------------------------------------------------------------------------------------------------------------------------
Step "Signing (minisign will ask for the key's password)"
$sig = Join-Path $assetsDir "RustyMCLA.exe.minisig"
if (Test-Path $sig) { Remove-Item $sig -Force }
Run-Native $Minisign @("-S", "-s", $keyFull, "-m", $exe, "-x", $sig, "-t", "RustyMCLA $Version")
if (-not (Test-Path $sig)) { Fail "minisign did not write $sig" }

# ---- finish the manifest ---------------------------------------------------------------------------------------------------------------
Step "Making manifest.json and verifying it"
$manifest = Join-Path $assetsDir "manifest.json"
Run-Native $MkManifest @("--finish", $unsigned, "--assets", $assetsDir, "--signature", $sig, "--pubkey", $PubKey, "--out", $manifest)
Run-Native $Minisign @("-V", "-p", $PubKey, "-x", $sig, "-m", $exe)
Write-Host "Both the launcher's own validator and the minisign program accept the signature." -ForegroundColor Green

# ---- upload ---------------------------------------------------------------------------------------------------------------------------
if ($NoUpload -or $DraftDir -ne "") {
  if ($DraftDir -ne "") { Copy-Item $manifest (Join-Path $DraftDir "manifest.json") -Force; Copy-Item $sig (Join-Path $DraftDir "RustyMCLA.exe.minisig") -Force }
  Step "Not uploading ($(if ($DraftDir -ne '') { 'test mode: files copied to the draft folder' } else { '-NoUpload' }))"
  Write-Host "manifest.json: $manifest"
  exit 0
}
Step "Uploading manifest.json to the draft"
Run-Native "gh" @("release", "upload", $Tag, $manifest, "-R", $Repo, "--clobber")

Write-Host ""
Write-Host "Done. manifest.json is on the DRAFT release $Tag. Nothing is published yet." -ForegroundColor Green
Write-Host "Now: open https://github.com/$Repo/releases , check the draft (assets: RustyMCLA.exe, game-$Version.zip, manifest.json, manifest.unsigned.json), fill in the release notes, and press Publish."
Write-Host "After publishing, launchers older than $Version will offer the update. Delete manifest.unsigned.json from the release if you like; the launcher ignores it."
