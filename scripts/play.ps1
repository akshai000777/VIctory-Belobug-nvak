# Starts the title with the settings this fork is played with (README.md, "How to run it").
#   scripts\play.ps1 -Exe <path of your own copy's executable>
# -Console          series-x (default) or series-s (lighter 1080p / 7.6 GiB console profile)
# -SeriesSReplica   Xbox Series S replica preset for 6 GB RTX 3050 laptops (series-s + LowSpec + NVIDIA adapter + KBM GTA)
# -Adapter          D3D12 adapter name or DXGI index (for hybrid/MUX laptops, e.g. -Adapter NVIDIA)
# -KbmGta           GTA V PC-style keyboard & mouse bindings on the stand-in pad (F2 or ` toggles mouse look)
# -Fps              Frame pace target: 30 (default title pace), 60 (16.667 ms), or uncapped (0)
# -Safe             without the newer, faster ways (the first thing to try when something is wrong)
# -RayTracing       the title's own ray-traced indirect light: lit interiors, about a third slower
# -Fsr4K            PC FidelityFX at 3840x2160 (quality, balanced or performance): FSR 4.1.2 where AMD's SDK offers it for the card, FSR 3.1 otherwise
# -Fsr              PC FidelityFX at the selected -Output resolution (native, quality, balanced, performance, ultra_performance)
# -Dlss             NVIDIA DLSS Super Resolution (dlaa, quality, balanced, performance, ultra_performance) via nvngx_dlss.dll
# -Output           Output resolution (1280x720, 1664x936, 1920x1080, 2560x1440, 3840x2160)
# -Borderless       Borderless fullscreen window sized to the primary monitor
# -LosslessScaling  Borderless fullscreen + ALLOW_TEARING + tight sub-ms frame pacing for Lossless Scaling FG
# -LowSpec          Preset for 6 GB VRAM / 16 GB RAM PCs (1080p default output, VRAM texture cap, strict RAM residency)
# -Build            target\diag (the default, the build all measurements are of) or target\release
#                   Light DRED is requested by default for diag and requires --features diagnostics.
# -More             other settings: -More @{VICTORY_PANEL='1'}
# Run it from a normal window, not as administrator: the driver then keeps its shader cache elsewhere.
param(
    [Parameter(Mandatory)][string]$Exe,
    [ValidateSet('series-x', 'series-s')][string]$Console = 'series-x',
    [switch]$SeriesSReplica,
    [string]$Adapter,
    [switch]$KbmGta,
    [ValidateSet('30', '60', 'uncapped')][string]$Fps,
    [switch]$Safe,
    [switch]$RayTracing,
    [ValidateSet('quality', 'balanced', 'performance')][string]$Fsr4K,
    [ValidateSet('native', 'quality', 'balanced', 'performance', 'ultra_performance')][string]$Fsr,
    [ValidateSet('dlaa', 'native', 'quality', 'balanced', 'performance', 'ultra_performance')][string]$Dlss,
    [ValidateSet('1280x720', '1664x936', '1920x1080', '2560x1440', '3840x2160')][string]$Output,
    [switch]$Borderless,
    [switch]$LosslessScaling,
    [switch]$LowSpec,
    [ValidateSet('diag', 'release')][string]$Build = 'diag',
    [hashtable]$More = @{}
)
if ($SeriesSReplica) {
    $Console = 'series-s'
    $LowSpec = $true
    $KbmGta = $true
    if (-not $Adapter) { $Adapter = 'NVIDIA' }
}
$settings = @{
    VICTORY_TEXTURE_NO_WAIT = '1'; VICTORY_RELAY = '2'; VICTORY_WIDE_BACK = '1'; VICTORY_CARD_REHASH_RUNS = '1'
    VICTORY_JOIN_COPIES = '1'; VICTORY_INDIRECT_WINDOW = '1'; VICTORY_RELAY_COPY = '1'; VICTORY_CARD_STEPS = '2'
    VICTORY_INDIRECT_BATCH = '2'; VICTORY_MAP_THREADS = '1'; VICTORY_TITLE_PRIORITIES = '0'; VICTORY_CARD_AWAKE = '1'
    VICTORY_SHOW_AFTER = '1'; VICTORY_AMBIENT = '0.8'
}
if ($Build -eq 'diag') { $settings.VICTORY_D3D12_DRED = 'light' }
if ($Safe) {
    $settings.VICTORY_UNDER_WIDE = '0'; $settings.VICTORY_LAND_ECHO = '0'
} else {
    $settings.VICTORY_THIN = '1'; $settings.VICTORY_JOIN_PARTS = '8'; $settings.VICTORY_UPLOAD_AHEAD = '1'
}
if ($RayTracing) { $settings.VICTORY_BVH_BUILDER = '1' }
if ($LowSpec) {
    $settings.VICTORY_RESIDENCY = '1'
    $settings.VICTORY_RESIDENCY_LIMITED = '1'
    $settings.VICTORY_WINDOW_BUDGET_MB = '1024'
    if (-not $Output -and -not $Fsr4K) { $settings.VICTORY_OUTPUT = '1920x1080' }
}
if ($Output) { $settings.VICTORY_OUTPUT = $Output }
if ($Adapter) { $settings.VICTORY_D3D12_ADAPTER = $Adapter }
if ($KbmGta) { $settings.VICTORY_KBM_GTA = '1' }
if ($Fps -eq '60') { $settings.VICTORY_FRAME_INTERVAL_MS = '16.667' }
elseif ($Fps -eq 'uncapped') { $settings.VICTORY_FRAME_INTERVAL_MS = '0' }
if ($Borderless -or $LosslessScaling) { $settings.VICTORY_WINDOW_MODE = 'borderless' }
if ($LosslessScaling) {
    $settings.VICTORY_ALLOW_TEARING = '1'
    $settings.VICTORY_PACE_SPIN = '1'
}
if ($Fsr4K) { $settings.VICTORY_FSR_4K = $Fsr4K.ToLowerInvariant() }
if ($Fsr) { $settings.VICTORY_FSR = $Fsr.ToLowerInvariant() }
if ($Dlss) { $settings.VICTORY_DLSS = $Dlss.ToLowerInvariant() }
foreach ($k in $More.Keys) { $settings[$k] = $More[$k] }

$victory = Join-Path $PSScriptRoot "..\target\$Build\victory.exe"
if (-not (Test-Path $victory)) { Write-Host "No $victory. Build it first (README.md)."; exit 1 }
if (-not (Test-Path $Exe)) { Write-Host "No $Exe."; exit 1 }
# (Other settings left over in this window go; where your files are stays.)
$paths = 'VICTORY_CONFIG', 'VICTORY_GAME_DIR', 'VICTORY_CACHE_DIR', 'VICTORY_DATA_DIR', 'VICTORY_FIDELITYFX_DIR', 'VICTORY_DLSS_DIR'
$inherited = @(Get-ChildItem Env:VICTORY_* | Where-Object { $_.Name -notin $paths })
foreach ($entry in $inherited) { Remove-Item "Env:$($entry.Name)" }
foreach ($k in $settings.Keys) { Set-Item "Env:$k" $settings[$k] }
$discarded = $inherited | Where-Object {
    $_.Name -notin $More.Keys -and -not $_.Value.Equals([Environment]::GetEnvironmentVariable($_.Name))
} | Sort-Object Name
if ($discarded) {
    Write-Host "Discarded inherited values for $($discarded.Name -join ', '). Use -More to pass them explicitly."
}
$Exe = (Resolve-Path $Exe).Path
Push-Location (Join-Path $PSScriptRoot '..')   # victory.toml is looked for here
try { & $victory run $Exe --execute --console $Console } finally { Pop-Location }
exit $LASTEXITCODE
