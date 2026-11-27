# Builds standalone Windows DLLs (xmem.dll, d3d12_xs.dll, chad.dll, xg_xs.dll)
# backed by this fork's victory-hle / victory-gpu / victory-formats crates.
#
# Usage:
#   scripts\build-dlls.ps1
#   scripts\build-dlls.ps1 -OutDir C:\out
#   scripts\build-dlls.ps1 -Profile debug
param(
    [ValidateSet('release', 'debug')][string]$Profile = 'release',
    [string]$OutDir,
    [string]$CopyTo
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$dllsDir = Join-Path $root 'dlls'
$manifest = Join-Path $dllsDir 'Cargo.toml'
$rootLock = Join-Path $root 'Cargo.lock'
$dllLock = Join-Path $dllsDir 'Cargo.lock'

if (-not (Test-Path $manifest)) {
    throw "Missing $manifest"
}

if (-not (Test-Path $dllLock) -and (Test-Path $rootLock)) {
    Copy-Item $rootLock $dllLock
}

$cargoArgs = @('build', '--manifest-path', $manifest, '--workspace')
if ($Profile -eq 'release') {
    $cargoArgs += '--release'
}

Write-Host "Building standalone Xbox HLE DLLs ($Profile)..."
& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$targetSubdir = if ($Profile -eq 'release') { 'release' } else { 'debug' }
$builtDir = Join-Path $dllsDir "target\$targetSubdir"
if (-not $OutDir) {
    $OutDir = Join-Path $dllsDir 'target\export'
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$names = @('xmem.dll', 'd3d12_xs.dll', 'chad.dll', 'xg_xs.dll')
foreach ($name in $names) {
    $src = Join-Path $builtDir $name
    if (-not (Test-Path $src)) {
        throw "Expected built DLL missing: $src"
    }
    $dst = Join-Path $OutDir $name
    Copy-Item $src $dst -Force
    $size = (Get-Item $dst).Length
    Write-Host ("  {0,-14} -> {1} ({2:N0} bytes)" -f $name, $dst, $size)
}

if ($CopyTo) {
    New-Item -ItemType Directory -Force -Path $CopyTo | Out-Null
    foreach ($name in $names) {
        Copy-Item (Join-Path $OutDir $name) (Join-Path $CopyTo $name) -Force
    }
    Write-Host "Copied all 4 DLLs to $CopyTo"
}
