# Double-click launcher backend for Play-SeriesS-RTX3050.cmd.
# Locates game_scarlett_master.exe (via victory.toml, common folders, or a
# File Explorer picker), checks the shader cache, builds if needed, and
# launches VIctory in steady Xbox Series S Replica mode for RTX 3050 laptops.
param(
    [string]$Exe
)

$ErrorActionPreference = 'Continue'
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $RepoRoot

$TomlPath = Join-Path $RepoRoot 'victory.toml'
$LogDir = Join-Path $env:LOCALAPPDATA 'victory\logs'
$LogFile = Join-Path $LogDir 'series-s-last.log'
New-Item -ItemType Directory -Path $LogDir -Force | Out-Null

function Get-TomlPathSetting {
    param([string]$Key, [string]$DefaultValue)
    if (-not (Test-Path $TomlPath)) { return $DefaultValue }
    foreach ($line in (Get-Content $TomlPath -Encoding UTF8)) {
        if ($line -match "^\s*$([regex]::Escape($Key))\s*=\s*'([^']*)'") {
            return $Matches[1]
        }
        if ($line -match "^\s*$([regex]::Escape($Key))\s*=\s*`"([^`"]*)`"") {
            return ($Matches[1] -replace '\\\\', '\')
        }
    }
    return $DefaultValue
}

function Save-GameDirToml {
    param([string]$GameDir)
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    $escaped = $GameDir.Replace('\', '\\').Replace('"', '\"')
    $newLine = "game_dir = `"$escaped`""
    if (-not (Test-Path $TomlPath)) {
        $body = "[paths]`r`n$newLine`r`ntest_titles = []`r`n"
        [System.IO.File]::WriteAllText($TomlPath, $body, $utf8NoBom)
        return
    }
    $lines = [System.Collections.Generic.List[string]]::new()
    foreach ($l in (Get-Content $TomlPath -Encoding UTF8)) { $lines.Add($l) }
    $replaced = $false
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^\s*game_dir\s*=') {
            $lines[$i] = $newLine
            $replaced = $true
            break
        }
    }
    if (-not $replaced) {
        $pathsIdx = -1
        for ($i = 0; $i -lt $lines.Count; $i++) {
            if ($lines[$i] -match '^\s*\[paths\]\s*$') { $pathsIdx = $i; break }
        }
        if ($pathsIdx -ge 0) {
            $lines.Insert($pathsIdx + 1, $newLine)
        } else {
            $lines.Add('[paths]')
            $lines.Add($newLine)
        }
    }
    [System.IO.File]::WriteAllText($TomlPath, (($lines -join "`r`n") + "`r`n"), $utf8NoBom)
}

function Select-GameExeDialog {
    param([string]$StartDir)
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.Application]::EnableVisualStyles()
    $dlg = New-Object System.Windows.Forms.OpenFileDialog
    $dlg.Title = 'Select Xbox Series game executable (game_scarlett_master.exe)'
    $dlg.Filter = 'Title Executable (game_scarlett_master.exe)|game_scarlett_master.exe|Executables (*.exe)|*.exe'
    if ($StartDir -and (Test-Path $StartDir)) {
        $dlg.InitialDirectory = $StartDir
    } elseif (Test-Path 'C:\VIctory\Games') {
        $dlg.InitialDirectory = 'C:\VIctory\Games'
    } else {
        $dlg.InitialDirectory = 'C:\'
    }
    $owner = New-Object System.Windows.Forms.Form
    $owner.TopMost = $true
    $owner.StartPosition = 'CenterScreen'
    $owner.ShowInTaskbar = $false
    try {
        $res = $dlg.ShowDialog($owner)
        if ($res -eq [System.Windows.Forms.DialogResult]::OK -and (Test-Path $dlg.FileName)) {
            return $dlg.FileName
        }
        return $null
    } finally {
        $dlg.Dispose()
        $owner.Dispose()
    }
}

function Find-GameExe {
    param([string]$Hint)
    if ($Hint -and (Test-Path $Hint -PathType Leaf)) {
        return (Resolve-Path $Hint).Path
    }
    $cfgDir = Get-TomlPathSetting -Key 'game_dir' -DefaultValue ''
    if ($cfgDir) {
        $candidate = Join-Path $cfgDir 'game_scarlett_master.exe'
        if (Test-Path $candidate -PathType Leaf) { return (Resolve-Path $candidate).Path }
    }
    $known = @(
        'C:\VIctory\Games\YourGame\GTA6\game_scarlett_master.exe',
        'C:\VIctory\Games\GTA6\game_scarlett_master.exe'
    )
    foreach ($p in $known) {
        if (Test-Path $p -PathType Leaf) { return (Resolve-Path $p).Path }
    }
    if (Test-Path 'C:\VIctory\Games') {
        $found = Get-ChildItem -Path 'C:\VIctory\Games' -Filter 'game_scarlett_master.exe' -File -Recurse -Depth 3 -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($found) { return $found.FullName }
    }
    return $null
}

function Ensure-Built {
    $victoryExe = Join-Path $RepoRoot 'target\diag\victory.exe'
    if (Test-Path $victoryExe -PathType Leaf) { return $victoryExe }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "target\diag\victory.exe not found and 'cargo' is not on PATH. Install Rust or run from a shell with cargo."
    }
    Write-Host ''
    Write-Host '[*] target\diag\victory.exe not found. Building VIctory (diag + diagnostics)...' -ForegroundColor Cyan
    & cargo build --locked --profile diag -p victory --features diagnostics
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $victoryExe -PathType Leaf)) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
    return $victoryExe
}

function Get-ShaderCacheCount {
    $cacheRoot = Get-TomlPathSetting -Key 'cache_dir' -DefaultValue (Join-Path $env:LOCALAPPDATA 'victory\cache')
    $shaderDir = Join-Path $cacheRoot 'shaders'
    if (-not (Test-Path $shaderDir)) { return 0 }
    return @(Get-ChildItem -Path $shaderDir -File -Recurse -ErrorAction SilentlyContinue | Select-Object -First 501).Count
}

function Invoke-ShaderPrecompile {
    param([string]$VictoryExe, [string]$GameDir)
    Write-Host ''
    Write-Host '======================================================================' -ForegroundColor Cyan
    Write-Host '  Pre-translating ALL shaders for NVIDIA RTX 3050 (one-time setup)' -ForegroundColor Cyan
    Write-Host '  This avoids frozen/unresponsive UI and first-run shader stalls.' -ForegroundColor Cyan
    Write-Host '  Takes a few minutes once; safe to close and resume later.' -ForegroundColor Cyan
    Write-Host '======================================================================' -ForegroundColor Cyan
    $prevAdapter = $env:VICTORY_D3D12_ADAPTER
    $env:VICTORY_D3D12_ADAPTER = 'NVIDIA'
    try {
        & $VictoryExe shaders $GameDir 2>&1 | Tee-Object -FilePath $LogFile
        if ($LASTEXITCODE -ne 0) {
            Write-Host "[!] Shader precompile exited with code $LASTEXITCODE" -ForegroundColor Yellow
        }
    } finally {
        if ($null -eq $prevAdapter) { Remove-Item Env:VICTORY_D3D12_ADAPTER -ErrorAction SilentlyContinue }
        else { $env:VICTORY_D3D12_ADAPTER = $prevAdapter }
    }
}

function Show-ControlsHelp {
    Write-Host ''
    Write-Host '----------------------------------------------------------------------' -ForegroundColor DarkGray
    Write-Host '  IMPORTANT FOR MENU / UI NAVIGATION:' -ForegroundColor Yellow
    Write-Host '  1) Click ONCE inside the VIctory game window so it has focus.' -ForegroundColor White
    Write-Host '  2) Mouse clicks do NOT click Xbox console menus! Use keyboard/pad:' -ForegroundColor White
    Write-Host '       Arrow Keys / WASD = Move     Enter = A (Select/Confirm)' -ForegroundColor Green
    Write-Host '       Backspace = B (Back)         Esc / P = Menu / Pause' -ForegroundColor Green
    Write-Host '       F2 or ` (~) = Toggle Mouse-Look once inside the 3D world' -ForegroundColor Green
    Write-Host '       F3 = Toggle FPS / VRAM / RAM overlay panel' -ForegroundColor Green
    Write-Host "  Log saved to: $LogFile" -ForegroundColor Gray
    Write-Host '----------------------------------------------------------------------' -ForegroundColor DarkGray
    Write-Host ''
}

$resolvedExe = Find-GameExe -Hint $Exe
if (-not $resolvedExe) {
    Write-Host '[*] Could not auto-locate game_scarlett_master.exe. Opening File Explorer...' -ForegroundColor Cyan
    $picked = Select-GameExeDialog -StartDir 'C:\VIctory\Games'
    if (-not $picked) {
        Write-Host '[!] No executable selected. Exiting.' -ForegroundColor Yellow
        exit 1
    }
    $resolvedExe = $picked
    Save-GameDirToml -GameDir (Split-Path $resolvedExe -Parent)
    Write-Host "[+] Saved game_dir to victory.toml: $(Split-Path $resolvedExe -Parent)" -ForegroundColor Green
} else {
    $gameDir = Split-Path $resolvedExe -Parent
    if (-not (Get-TomlPathSetting -Key 'game_dir' -DefaultValue '')) {
        Save-GameDirToml -GameDir $gameDir
    }
}

$cachedShaders = Get-ShaderCacheCount
$dataDir = Get-TomlPathSetting -Key 'data_dir' -DefaultValue (Join-Path $env:LOCALAPPDATA 'victory\data')

Write-Host ''
Write-Host '======================================================================' -ForegroundColor Cyan
Write-Host '  VIctory - Xbox Series S Replica (RTX 3050 Laptop / LOQ 15IAX9)' -ForegroundColor Cyan
Write-Host '======================================================================' -ForegroundColor Cyan
Write-Host "  Executable  : $resolvedExe"
Write-Host "  ShaderCache : $cachedShaders+ cached shaders"
Write-Host "  Runtime Log : $LogFile"
Write-Host "  Crash Logs  : $dataDir\<TitleID>\crashes.log"
Write-Host ''
Write-Host '  [1] Play Xbox Series S Replica (1080p, LowSpec, RTX 3050, KBM+Pad)'
Write-Host '  [2] Play Safe Mode Series S    (use if troubleshooting black screen/crash)'
Write-Host '  [3] Pre-translate ALL shaders  (run once first so UI/world do not freeze!)'
Write-Host '  [4] Browse File Explorer to pick a different .exe'
Write-Host '  [5] Open logs & cache folder in File Explorer'
Write-Host ''

$choice = (Read-Host 'Choose [1-5] (press Enter for 1)').Trim()
if ([string]::IsNullOrEmpty($choice)) { $choice = '1' }

switch ($choice) {
    '4' {
        $picked = Select-GameExeDialog -StartDir (Split-Path $resolvedExe -Parent)
        if ($picked) {
            $resolvedExe = $picked
            Save-GameDirToml -GameDir (Split-Path $resolvedExe -Parent)
            Write-Host "[+] Selected and saved: $resolvedExe" -ForegroundColor Green
        } else {
            Write-Host '[!] Selection cancelled.' -ForegroundColor Yellow
            exit 0
        }
        $choice = '1'
    }
    '5' {
        $victoryAppData = Join-Path $env:LOCALAPPDATA 'victory'
        New-Item -ItemType Directory -Path $victoryAppData -Force | Out-Null
        Start-Process explorer.exe $victoryAppData
        exit 0
    }
}

$victoryExe = Ensure-Built
$gameDir = Split-Path $resolvedExe -Parent

if ($choice -eq '3') {
    Invoke-ShaderPrecompile -VictoryExe $victoryExe -GameDir $gameDir
    $ans = (Read-Host 'Shader pre-translation finished. Launch Series S Replica now? [Y/n]').Trim()
    if ($ans -and $ans -notmatch '^[Yy]') { exit 0 }
    $choice = '1'
} elseif ($cachedShaders -lt 500) {
    Write-Host ''
    Write-Host "[!] Shader cache only has $cachedShaders entries." -ForegroundColor Yellow
    Write-Host '    Without pre-translating shaders first, the initial UI/boot compiles live' -ForegroundColor Yellow
    Write-Host '    and can look frozen (audio plays while video/UI stalls).' -ForegroundColor Yellow
    $ans = (Read-Host 'Pre-translate shaders now (recommended, one-time)? [Y/n, Enter = Y]').Trim()
    if ([string]::IsNullOrEmpty($ans) -or $ans -match '^[Yy]') {
        Invoke-ShaderPrecompile -VictoryExe $victoryExe -GameDir $gameDir
    }
}

$playScript = Join-Path $PSScriptRoot 'play.ps1'
Show-ControlsHelp

$playArgs = @('-Exe', $resolvedExe, '-SeriesSReplica')
if ($choice -eq '2') {
    $playArgs += '-Safe'
}

& $playScript @playArgs *>&1 | Tee-Object -FilePath $LogFile
exit $LASTEXITCODE
