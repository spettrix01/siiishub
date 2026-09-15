# Builds the SIIISHUB Windows installer (NSIS) with the MSVC toolchain.
# Usage (PowerShell):  .\scripts\windows-build.ps1 [-DevTools]
#
# Requires Visual Studio Build Tools with the C++ workload, Rust (rustup),
# tauri-cli (cargo install tauri-cli --version "^2" --locked) and
# src-tauri\binaries\libmpv-2.dll (see docs/BUILDING.md). The import library
# mpv.lib is generated from the DLL when it is missing.
#
# -DevTools keeps WebView remote debugging in the release build (testing only).
# The installer is copied to installers\windows, or to $env:OUT_DIR when set.
# Run it from PowerShell or cmd, not from Git Bash: its link.exe shadows the
# MSVC linker.
param([switch]$DevTools)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$tauriDir = Join-Path $root 'src-tauri'
$binDir = Join-Path $tauriDir 'binaries'
$outDir = if ($env:OUT_DIR) { $env:OUT_DIR } else { Join-Path $root 'installers\windows' }

# MSVC environment (cl, link, lib, dumpbin) of the newest Visual Studio or Build Tools.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path $vswhere)) { throw 'vswhere.exe not found: install Visual Studio Build Tools with the C++ workload.' }
$vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vsPath) { throw 'No Visual Studio installation with the C++ tools was found.' }
$vcvars = Join-Path $vsPath 'VC\Auxiliary\Build\vcvars64.bat'
cmd /c "`"$vcvars`" >nul 2>&1 && set" | ForEach-Object {
  if ($_ -match '^([^=]+)=(.*)$') { Set-Item -Path "env:$($Matches[1])" -Value $Matches[2] }
}

# Release binaries embed the source paths of their dependencies (panic
# locations). Rewrite the Cargo home and the checkout folder, so the user name
# and the folder layout of the build machine stay out of the executable.
# CARGO_ENCODED_RUSTFLAGS keeps paths with spaces intact.
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$rustFlags = @()
if ($env:RUSTFLAGS) { $rustFlags += $env:RUSTFLAGS -split '\s+' | Where-Object { $_ } }
$rustFlags += "--remap-path-prefix=$cargoHome=/cargo"
$rustFlags += "--remap-path-prefix=$root=/siiishub"
$env:CARGO_ENCODED_RUSTFLAGS = $rustFlags -join [char]0x1f

$dll = Join-Path $binDir 'libmpv-2.dll'
if (-not (Test-Path $dll)) { throw "Missing $dll (see docs/BUILDING.md)." }
$importLib = Join-Path $binDir 'mpv.lib'
if (-not (Test-Path $importLib)) {
  Write-Host 'Generating mpv.lib from libmpv-2.dll'
  $def = Join-Path $binDir 'mpv.def'
  # Only the mpv client API. LIBRARY names the DLL that the import records point to.
  $exports = @(& dumpbin /nologo /exports $dll |
    Select-String -Pattern '^\s+\d+\s+[0-9A-Fa-f]+\s+[0-9A-Fa-f]{8}\s+(mpv_\S+)' |
    ForEach-Object { '    ' + $_.Matches[0].Groups[1].Value })
  if (-not $exports) { throw 'No mpv exports found in libmpv-2.dll.' }
  Set-Content -Path $def -Encoding ascii -Value (@('LIBRARY libmpv-2', 'EXPORTS') + $exports)
  & lib /nologo "/def:$def" /machine:x64 "/out:$importLib"
  if ($LASTEXITCODE -ne 0) { throw 'lib.exe failed.' }
}

Push-Location $tauriDir
try {
  $cargoArgs = @('tauri', 'build')
  if ($DevTools) { $cargoArgs += @('--features', 'devtools') }
  & cargo @cargoArgs
  if ($LASTEXITCODE -ne 0) { throw 'cargo tauri build failed.' }
} finally {
  Pop-Location
}

New-Item -ItemType Directory -Force -Path $outDir | Out-Null
Get-ChildItem (Join-Path $tauriDir 'target\release\bundle\nsis\*.exe') |
  Copy-Item -Destination $outDir -Force -PassThru |
  ForEach-Object { Write-Host "Installer: $($_.FullName)" }
