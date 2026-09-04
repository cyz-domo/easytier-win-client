# Local release build for EasyTier Windows Client.
# Checks the environment, optionally fixes missing pieces (-Fix), downloads
# the EasyTier core runtime, builds the GUI + service binaries, and assembles
# the NSIS installer and portable zip into dist/.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1              # x64, check env first
#   powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1 -Target aarch64
#   powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1 -Fix         # install missing deps where safe
#   powershell -ExecutionPolicy Bypass -File scripts/build-release.ps1 -SkipCoreDownload
#
# Requirements: Node.js >= 18, Rust (msvc), MSVC Build Tools + Windows SDK,
# WebView2 Runtime (Win10; Win11 ships it), Internet access for core download.
#
# Honors the project's standard toolchain locations when cargo is not on PATH:
#   RUSTUP_HOME / CARGO_HOME (e.g. E:\app\agent-worker\rustup | cargo)

param(
  [ValidateSet('x86_64', 'aarch64')]
  [string]$Target = 'x86_64',
  [switch]$Fix,
  [switch]$SkipCoreDownload,
  [string]$CoreVersion = '2.6.4',
  [string]$OutputDir = 'dist'
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

# Pick up the project's pinned toolchain if cargo is not globally available.
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  $rustupHome = $env:RUSTUP_HOME; $cargoHome = $env:CARGO_HOME
  if (-not $cargoHome -and (Test-Path 'E:\app\agent-worker\cargo\bin\cargo.exe')) {
    $rustupHome = 'E:\app\agent-worker\rustup'; $cargoHome = 'E:\app\agent-worker\cargo'
  }
  if ($cargoHome -and (Test-Path "$cargoHome\bin\cargo.exe")) {
    $env:RUSTUP_HOME = if ($rustupHome) { $rustupHome } else { Join-Path $cargoHome 'rustup' }
    $env:CARGO_HOME = $cargoHome
    $env:PATH = "$cargoHome\bin;$env:PATH"
    Write-Host "Using project toolchain: $cargoHome" -ForegroundColor DarkGray
  }
}

$failures = @()
$warnings = @()

function Test-CommandVersion([string]$Name, [scriptblock]$VersionOf, [Version]$Minimum) {
  try {
    $output = & $VersionOf 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return ($output | Select-Object -First 1)
  } catch { return $null }
}

function Add-Failure([string]$message, [string]$fixHint, [string]$fixCommand = '') {
  Write-Host "  [X] $message" -ForegroundColor Red
  if ($fixHint) { Write-Host "      -> $fixHint" -ForegroundColor Yellow }
  $script:failures += @{ Message = $message; Hint = $fixHint; Command = $fixCommand }
}

function Add-Warning([string]$message, [string]$hint) {
  Write-Host "  [!] $message" -ForegroundColor Yellow
  if ($hint) { Write-Host "      -> $hint" -ForegroundColor Yellow }
  $script:warnings += @{ Message = $message; Hint = $hint }
}

function Add-Pass([string]$message, [string]$detail = '') {
  $suffix = if ($detail) { " ($detail)" } else { '' }
  Write-Host "  [OK] $message$suffix" -ForegroundColor Green
}

function Get-PeMachine([string]$Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  if ($bytes.Length -lt 64) { return $null }
  $peOffset = [BitConverter]::ToInt32($bytes, 0x3c)
  if ($peOffset -lt 0 -or $peOffset + 6 -gt $bytes.Length -or $bytes[$peOffset] -ne 0x50 -or $bytes[$peOffset + 1] -ne 0x45) { return $null }
  return [BitConverter]::ToUInt16($bytes, $peOffset + 4)
}

function Assert-PeArchitecture([string]$Path, [int]$ExpectedMachine) {
  $machine = Get-PeMachine $Path
  if ($null -eq $machine -or $machine -ne $ExpectedMachine) {
    $actual = if ($null -eq $machine) { 'unknown' } else { '0x{0:X4}' -f $machine }
    throw "Architecture mismatch for $(Split-Path -Leaf $Path): machine $actual, expected 0x$('{0:X4}' -f $ExpectedMachine)"
  }
  Add-Pass "PE architecture $(Split-Path -Leaf $Path)" ("0x$('{0:X4}' -f $machine)")
}

Write-Host "== EasyTier Win Client local release build ==" -ForegroundColor Cyan
Write-Host "Target triple: $(if ($Target -eq 'aarch64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' })"
Write-Host ""

# --- 1. Node.js -------------------------------------------------------------
Write-Host '[1/5] Checking environment...' -ForegroundColor Cyan
$nodeVersionOutput = Test-CommandVersion 'node' { node --version }
if ($nodeVersionOutput -match '^v(\d+)') {
  $nodeMajor = [int]$Matches[1]
  if ($nodeMajor -ge 18) { Add-Pass 'Node.js' $nodeVersionOutput }
  else { Add-Failure "Node.js >= 18 required, found $nodeVersionOutput" 'Install from https://nodejs.org' 'winget install OpenJS.NodeJS.LTS' }
} else {
  Add-Failure 'Node.js not found' 'Install from https://nodejs.org' 'winget install OpenJS.NodeJS.LTS'
}

# --- 2. Rust toolchain ------------------------------------------------------
$cargoVersionOutput = Test-CommandVersion 'cargo' { cargo --version }
if ($cargoVersionOutput) {
  Add-Pass 'Cargo' $cargoVersionOutput
  $neededTarget = if ($Target -eq 'aarch64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
  $installedTargets = & rustup target list --installed 2>$null
  if ($LASTEXITCODE -eq 0 -and $installedTargets -contains $neededTarget) {
    Add-Pass "Rust target $neededTarget"
  } else {
    Add-Failure "Rust target $neededTarget missing" 'Run: rustup target add <target>' "rustup target add $neededTarget"
  }
} else {
  Add-Failure 'Rust/Cargo not found' 'Install from https://rustup.rs' 'winget install Rustlang.Rustup'
}

# --- 3. MSVC build tools ----------------------------------------------------
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$clOnPath = Get-Command cl.exe -ErrorAction SilentlyContinue
if ($clOnPath) {
  Add-Pass 'MSVC compiler' $clOnPath.Source
} elseif (Test-Path $vswhere) {
  $vsRoot = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
  if ($vsRoot) { Add-Pass 'MSVC Build Tools' $vsRoot }
  else { Add-Failure 'MSVC x64 tools component not found' 'Install Visual Studio Build Tools (C++ workload)' 'winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"' }
} else {
  Add-Failure 'MSVC Build Tools not found' 'Install Visual Studio Build Tools (C++ workload)' 'winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"'
}

# --- 4. WebView2 runtime ----------------------------------------------------
$webview2Keys = @(
  'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
  'HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
)
$webview2Found = $webview2Keys | Where-Object { Test-Path $_ }
if ($webview2Found) { Add-Pass 'WebView2 Runtime' }
else {
  $winBuild = [System.Environment]::OSVersion.Version.Build
  if ($winBuild -ge 22000) { Add-Warning 'WebView2 registry key not found (Win11 usually ships it; check Edge install)' '' }
  else { Add-Failure 'WebView2 Runtime not found' 'Install from https://developer.microsoft.com/microsoft-edge/webview2/' 'winget install Microsoft.EdgeWebView2Runtime' }
}

# --- Environment summary ----------------------------------------------------
if ($failures.Count -gt 0) {
  Write-Host ""
  Write-Host "Environment check failed with $($failures.Count) problem(s)." -ForegroundColor Red
  foreach ($f in $failures) {
    if ($f.Command -and $Fix) {
      Write-Host "Auto-fixing: $($f.Command)" -ForegroundColor Cyan
      try { Invoke-Expression $f.Command } catch { Write-Host "  auto-fix failed: $_" -ForegroundColor Red }
    }
  }
  if (-not $Fix) {
    Write-Host 'Re-run with -Fix to attempt automatic installation of missing pieces (winget-based).' -ForegroundColor Yellow
  }
  # Re-check after fixes; abort if cargo still missing because nothing downstream works.
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { exit 1 }
}

# --- 5. Frontend deps + codec test ------------------------------------------
Write-Host '[2/5] Installing frontend dependencies...' -ForegroundColor Cyan
npm ci
if ($LASTEXITCODE -ne 0) { Write-Host 'npm ci failed' -ForegroundColor Red; exit 1 }

Write-Host '[3/5] Running codec tests...' -ForegroundColor Cyan
npm run test:codec
if ($LASTEXITCODE -ne 0) { Write-Host 'codec tests failed' -ForegroundColor Red; exit 1 }

# --- Core download ------------------------------------------------------------
$targetTriple = if ($Target -eq 'aarch64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
$coreStage = 'src-tauri/core-stage'
$serviceStage = 'src-tauri/service-stage'

if ($SkipCoreDownload) {
  Write-Host '[4/5] Skipping core download (-SkipCoreDownload).' -ForegroundColor Yellow
  if (-not (Test-Path "$coreStage/easytier-core.exe")) {
    Write-Host "$coreStage/easytier-core.exe missing; cannot skip download." -ForegroundColor Red; exit 1
  }
} else {
  Write-Host '[4/5] Downloading EasyTier core runtime...' -ForegroundColor Cyan
  if (Test-Path $coreStage) { Remove-Item $coreStage -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $coreStage | Out-Null
  $asset = if ($Target -eq 'aarch64') { "easytier-windows-arm64-v$CoreVersion.zip" } else { "easytier-windows-x86_64-v$CoreVersion.zip" }
  $url = "https://github.com/EasyTier/EasyTier/releases/download/v$CoreVersion/$asset"
  Write-Host "  $url"
  Invoke-WebRequest -Uri $url -OutFile easytier-core.zip
  Expand-Archive -Path easytier-core.zip -DestinationPath core-expanded -Force
  $core = Get-ChildItem -Path core-expanded -Recurse -Filter easytier-core.exe | Select-Object -First 1
  if (-not $core) { Write-Host 'easytier-core.exe not found in archive' -ForegroundColor Red; exit 1 }
  New-Item -ItemType Directory -Force -Path $coreStage | Out-Null
  Copy-Item -Path ($core.DirectoryName + '/*') -Destination $coreStage -Recurse -Force
  Remove-Item -Recurse -Force core-expanded, easytier-core.zip
  foreach ($required in @('easytier-core.exe', 'easytier-cli.exe', 'wintun.dll', 'WinDivert64.sys', 'Packet.dll')) {
    if (-not (Test-Path "$coreStage/$required")) { Write-Host "Missing runtime file: $required" -ForegroundColor Red; exit 1 }
  }
}

# --- Build -------------------------------------------------------------------
Write-Host '[5/5] Building (this can take several minutes)...' -ForegroundColor Cyan
cargo build --release --target $targetTriple --manifest-path src-tauri/Cargo.toml --bin easytier-service
if ($LASTEXITCODE -ne 0) { Write-Host 'service build failed' -ForegroundColor Red; exit 1 }

New-Item -ItemType Directory -Force -Path $serviceStage | Out-Null
Copy-Item "src-tauri/target/$targetTriple/release/easytier-service.exe" "$serviceStage/easytier-service.exe" -Force

npm exec tauri -- build --target $targetTriple --config src-tauri/tauri.release.conf.json
if ($LASTEXITCODE -ne 0) {
  Write-Host 'tauri build failed' -ForegroundColor Red
  Write-Host 'If the failure is "failed to bundle project: timeout" while downloading NSIS, the network to github.com is unstable.' -ForegroundColor Yellow
  Write-Host 'Fix: download nsis-3.11.zip once via a mirror (e.g. https://ghfast.top/ prefix) and rerun;' -ForegroundColor Yellow
  Write-Host 'with bundle.useLocalToolsDir=true the NSIS cache lives under src-tauri/target/ so it is reused.' -ForegroundColor Yellow
  exit 1
}

# --- Assemble ----------------------------------------------------------------
$releaseDir = "src-tauri/target/$targetTriple/release"
$arch = if ($Target -eq 'aarch64') { 'arm64' } else { 'x64' }
$expectedMachine = if ($Target -eq 'aarch64') { 0xAA64 } else { 0x8664 }
Assert-PeArchitecture "$releaseDir/easytier-win-client.exe" $expectedMachine
Assert-PeArchitecture "$releaseDir/easytier-service.exe" $expectedMachine
Assert-PeArchitecture "$coreStage/easytier-core.exe" $expectedMachine
Assert-PeArchitecture "$coreStage/easytier-cli.exe" $expectedMachine
$portable = "easytier-win-client_${arch}_portable"

New-Item -ItemType Directory -Force -Path "$portable/core" | Out-Null
Copy-Item "$releaseDir/easytier-win-client.exe" "$portable/easytier-win-client.exe"
Copy-Item "$releaseDir/easytier-service.exe" "$portable/easytier-service.exe"
Copy-Item "$coreStage/*" "$portable/core" -Recurse -Force
Compress-Archive -Path "$portable/*" -DestinationPath "$portable.zip" -Force

$nsis = Get-ChildItem "$releaseDir/bundle/nsis/*.exe" | Select-Object -First 1
if (-not $nsis) { Write-Host 'NSIS installer not found after build' -ForegroundColor Red; exit 1 }

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
Copy-Item $nsis.FullName "$OutputDir/$(Split-Path -Leaf $nsis.FullName)" -Force
Copy-Item "$portable.zip" $OutputDir -Force
Remove-Item -Recurse -Force $portable, "$portable.zip" -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "== Build complete ==" -ForegroundColor Green
Get-ChildItem $OutputDir | Format-Table Name, Length -AutoSize
Write-Host "Artifacts are in: $RepoRoot/$OutputDir"
if ($warnings.Count -gt 0) {
  Write-Host 'Warnings from environment check (build succeeded anyway):' -ForegroundColor Yellow
  foreach ($w in $warnings) { Write-Host "  - $($w.Message)" -ForegroundColor Yellow }
}
