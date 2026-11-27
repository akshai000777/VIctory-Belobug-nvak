# Double-click launcher backend for Play-SeriesS-RTX3050.cmd.
# Flow:
#   1. Open File Explorer to pick game_scarlett_master.exe (and save victory.toml)
#   2. Compile target\diag\victory.exe with cargo before launching anything
#   3. Pre-translate shaders once for RTX 3050 if not yet done for that game folder
#   4. Launch Xbox Series S Replica (1080p, LowSpec, NVIDIA adapter, KBM + Pad)
param(
    [string]$Exe
)

$ErrorActionPreference = 'Continue'
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $RepoRoot

$TomlPath = Join-Path $RepoRoot 'victory.toml'
$LogDir = Join-Path $env:LOCALAPPDATA 'victory\logs'
$LogFile = Join-Path $LogDir 'series-s-last.log'
$ShaderMarker = Join-Path $LogDir 'shaders-done.txt'
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

function Get-InitialBrowseDir {
    $cfgDir = Get-TomlPathSetting -Key 'game_dir' -DefaultValue ''
    if ($cfgDir -and (Test-Path $cfgDir -PathType Container)) { return $cfgDir }
    foreach ($p in @('C:\VIctory\Games\YourGame\GTA6', 'C:\VIctory\Games')) {
        if (Test-Path $p -PathType Container) { return $p }
    }
    return 'C:\'
}

function Select-GameExeDialog {
    param([string]$StartDir)
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    [System.Windows.Forms.Application]::EnableVisualStyles()

    $dlg = New-Object System.Windows.Forms.OpenFileDialog
    $dlg.Title = 'Step 1: Select game_scarlett_master.exe to compile & run'
    $dlg.Filter = 'Title Executable (game_scarlett_master.exe)|game_scarlett_master.exe|Executables (*.exe)|*.exe'
    if ($StartDir -and (Test-Path $StartDir -PathType Container)) {
        $dlg.InitialDirectory = $StartDir
        if (Test-Path (Join-Path $StartDir 'game_scarlett_master.exe') -PathType Leaf) {
            $dlg.FileName = 'game_scarlett_master.exe'
        }
    }

    $owner = New-Object System.Windows.Forms.Form
    $owner.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::None
    $owner.Size = New-Object System.Drawing.Size(1, 1)
    $owner.StartPosition = [System.Windows.Forms.FormStartPosition]::CenterScreen
    $owner.ShowInTaskbar = $false
    $owner.Opacity = 0
    $owner.TopMost = $true
    try {
        $owner.Show()
        $owner.Activate()
        $res = $dlg.ShowDialog($owner)
        if ($res -eq [System.Windows.Forms.DialogResult]::OK -and (Test-Path $dlg.FileName -PathType Leaf)) {
            return (Resolve-Path $dlg.FileName).Path
        }
        return $null
    } finally {
        $dlg.Dispose()
        $owner.Close()
        $owner.Dispose()
    }
}

function Invoke-CompileVictory {
    Write-Host ''
    Write-Host '[Step 2/4] Compiling VIctory (cargo build --locked --profile diag)...' -ForegroundColor Cyan
    $victoryExe = Join-Path $RepoRoot 'target\diag\victory.exe'
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        if (Test-Path $victoryExe -PathType Leaf) {
            Write-Host "[!] 'cargo' not found on PATH; using existing target\diag\victory.exe." -ForegroundColor Yellow
            return $victoryExe
        }
        Write-Host "[!] 'cargo' was not found on PATH. Install Rust or run from a shell with cargo." -ForegroundColor Red
        exit 1
    }
    & cargo build --locked --profile diag -p victory --features diagnostics
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $victoryExe -PathType Leaf)) {
        Write-Host "[!] cargo build failed with exit code $LASTEXITCODE." -ForegroundColor Red
        exit $LASTEXITCODE
    }
    return $victoryExe
}

function Test-ShadersDone {
    param([string]$GameDir)
    if (-not (Test-Path $ShaderMarker -PathType Leaf)) { return $false }
    $norm = $GameDir.TrimEnd('\').ToLowerInvariant()
    foreach ($line in (Get-Content $ShaderMarker -Encoding UTF8 -ErrorAction SilentlyContinue)) {
        if ($line.Trim().TrimEnd('\').ToLowerInvariant() -eq $norm) { return $true }
    }
    return $false
}

function Mark-ShadersDone {
    param([string]$GameDir)
    if (-not (Test-ShadersDone -GameDir $GameDir)) {
        Add-Content -Path $ShaderMarker -Value $GameDir -Encoding UTF8
    }
}

function Invoke-ShaderPrecompile {
    param([string]$VictoryExe, [string]$GameDir)
    Write-Host ''
    Write-Host '[Step 3/4] Pre-translating shaders for NVIDIA RTX 3050 (one-time per game)...' -ForegroundColor Cyan
    Write-Host '          This prevents audio-only / frozen UI stalls on boot.' -ForegroundColor Gray
    $prevAdapter = $env:VICTORY_D3D12_ADAPTER
    $env:VICTORY_D3D12_ADAPTER = 'NVIDIA'
    try {
        & $VictoryExe shaders $GameDir 2>&1 | Tee-Object -FilePath $LogFile
        if ($LASTEXITCODE -eq 0) {
            Mark-ShadersDone -GameDir $GameDir
        } else {
            Write-Host "[!] Shader precompile exited with code $LASTEXITCODE (continuing to launch)." -ForegroundColor Yellow
        }
    } finally {
        if ($null -eq $prevAdapter) { Remove-Item Env:VICTORY_D3D12_ADAPTER -ErrorAction SilentlyContinue }
        else { $env:VICTORY_D3D12_ADAPTER = $prevAdapter }
    }
}

Write-Host '======================================================================' -ForegroundColor Cyan
Write-Host '  VIctory - Xbox Series S Replica (RTX 3050 Laptop / LOQ 15IAX9)' -ForegroundColor Cyan
Write-Host '======================================================================' -ForegroundColor Cyan

$resolvedExe = $null
if ($Exe -and (Test-Path $Exe -PathType Leaf)) {
    $resolvedExe = (Resolve-Path $Exe).Path
} else {
    Write-Host '[Step 1/4] Opening File Explorer to pick game_scarlett_master.exe...' -ForegroundColor Cyan
    $resolvedExe = Select-GameExeDialog -StartDir (Get-InitialBrowseDir)
    if (-not $resolvedExe) {
        Write-Host '[!] No .exe selected. Cancelled.' -ForegroundColor Yellow
        exit 0
    }
}

$gameDir = Split-Path $resolvedExe -Parent
Save-GameDirToml -GameDir $gameDir
Write-Host "  Selected EXE : $resolvedExe" -ForegroundColor Green
Write-Host "  Saved config : $TomlPath" -ForegroundColor Gray

$victoryExe = Invoke-CompileVictory

if (Test-ShadersDone -GameDir $gameDir) {
    Write-Host ''
    Write-Host '[Step 3/4] Shaders already pre-translated for this folder (skipping).' -ForegroundColor Green
} else {
    Invoke-ShaderPrecompile -VictoryExe $victoryExe -GameDir $gameDir
}

Write-Host ''
Write-Host '[Step 4/4] Launching Xbox Series S Replica (1080p, LowSpec, RTX 3050)...' -ForegroundColor Cyan
Write-Host '----------------------------------------------------------------------' -ForegroundColor DarkGray
Write-Host '  IMPORTANT FOR MENUS / UI:' -ForegroundColor Yellow
Write-Host '  1) Click ONCE inside the game window so it has focus.' -ForegroundColor White
Write-Host '  2) Mouse clicks do NOT click Xbox console menus! Use keyboard:' -ForegroundColor White
Write-Host '       Arrows / WASD = Move     Enter = A (Confirm)   Backspace = B' -ForegroundColor Green
Write-Host '       Esc / P = Menu / Pause   F2 or ` = 3D Mouse Look   F3 = Stats' -ForegroundColor Green
Write-Host "  Log file : $LogFile" -ForegroundColor Gray
Write-Host '----------------------------------------------------------------------' -ForegroundColor DarkGray
Write-Host ''

$playScript = Join-Path $PSScriptRoot 'play.ps1'
& $playScript -Exe $resolvedExe -SeriesSReplica *>&1 | Tee-Object -FilePath $LogFile
exit $LASTEXITCODE
