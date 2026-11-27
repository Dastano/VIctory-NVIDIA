<#
.SYNOPSIS
Shared NVIDIA environment and settings configuration for Victory.
Used by play-nvidia.ps1 and warm-nvidia.ps1 to guarantee 100% environment parity.
#>

function Get-VictoryNvidiaSettings {
    [CmdletBinding()]
    param(
        [switch]$SeriesX,
        [switch]$Quality,
        [switch]$RayTracing,
        [ValidateSet('2160', '1440', '1080', '720', '540', '480')][string]$Resolution,
        [string]$Adapter,
        [ValidateSet('diag', 'release')][string]$Build = 'release',
        [hashtable]$More = @{}
    )

    $settings = @{
        # Record all pipelines the title creates into known-pipelines.bin and known-shaders.bin
        # so victory warm / warm-nvidia.ps1 can precompile them.
        VICTORY_PIPELINE_BOOK = '1'

        # This fork's play set (scripts\play.ps1, not -Safe): the speed set.
        VICTORY_TEXTURE_NO_WAIT = '1'; VICTORY_RELAY = '2'; VICTORY_CARD_STEPS = '2'
        VICTORY_WIDE_BACK = '1'; VICTORY_CARD_REHASH_RUNS = '1'
        VICTORY_JOIN_COPIES = '1'; VICTORY_INDIRECT_WINDOW = '1'; VICTORY_RELAY_COPY = '1'
        VICTORY_INDIRECT_BATCH = '2'; VICTORY_MAP_THREADS = '1'; VICTORY_TITLE_PRIORITIES = '0'; VICTORY_CARD_AWAKE = '1'
        VICTORY_SHOW_AFTER = '1'; VICTORY_AMBIENT = '0.8'; VICTORY_SKIP_PIPELINES = 'none'
        VICTORY_THIN = '1'; VICTORY_JOIN_PARTS = '8'; VICTORY_UPLOAD_AHEAD = '1'
        # Avoid 40-60 ms card_steps refusal stalls when a placed copy writes into the constant ring behind fills.
        VICTORY_PLACED_ORDER = '1'
        # Hybrid CPU (e.g. 6P+8E i5-13600K): pin only critical submit/relay/heap-sync threads to P-cores,
        # and spin briefly on wide barriers instead of sleeping on OS condvars.
        VICTORY_FAST_CORES = '1'; VICTORY_WIDE_SPIN = '80'
        # Cheap picture fixes, on in both sets:
        # - Overbright views & fog: bound volumetric fog and missed-ray room ambient (ambient.rs).
        VICTORY_AMBIENT_AIR = '1.5'; VICTORY_AMBIENT_MOST = '2'
        # - Black blobs: float NaN/Inf scrubbed to 0 (integer/R9G9B9E5 UAVs left untouched; emit/lower.rs).
        VICTORY_NAN_SCRUB = '1'
        # Wait for new shaders rather than drop water/character/lighting draws (each pipeline waited for once at most).
        VICTORY_PIPELINE_WAIT_MS = '10000'
        # NVIDIA caps the shader-visible view heap at 1,000,000: without more dynamic slots thousands of draws a frame
        # are dropped (descriptor/host.rs dynamic_slots).
        VICTORY_DYNAMIC_SLOTS = '524288'
        # Half render scale with the title's own upscaler (Series S needs it with a scale below 1; victory-loader hooks.rs).
        VICTORY_RENDER_SCALE = '0.5'; VICTORY_OWN_UPSCALER = '1'
        # Ray walks end at the root while the structure builder is off (emit/intersect.rs).
        VICTORY_BVH_MISS = '1'
        # Memory compares on the card instead of the CPU (down to 64 KiB) + ReBAR hot windows.
        VICTORY_CARD_BUFFERS_MIN_MB = '1'; VICTORY_CARD_BUFFERS_MIN_KB = '64'; VICTORY_CARD_BUFFERS_MB = '1536'
        VICTORY_WINDOWS_BAR_MB = '256'
        # Series S presents the interface as a second plane: laid over the game (compose.rs), else black before gameplay.
        VICTORY_COMPOSE = '1'
    }

    if ($Build -eq 'diag') {
        $settings.VICTORY_D3D12_DRED = 'light'
    }

    # Ray tracing mostly off. The structure builder cannot run on NVIDIA (its 64-lane programs are refused; upstream
    # Marvisak/VIctory pull request 2311), so every ray misses, and the passes that turn misses into light made rooms go
    # white depending on the view angle. rt_off_hashes.txt lists the 1,000 CS_RaytracingResolve and 34 traced-shadow
    # shaders, which are skipped. The screen-probe and visibility traces keep running: skipping those too left their
    # outputs as garbage (rainbow glitter everywhere).
    $rtList = Join-Path $PSScriptRoot 'rt_off_hashes.txt'
    if (-not $RayTracing -and (Test-Path $rtList)) {
        $settings.VICTORY_SKIP_PIPELINES = (Get-Content $rtList -Raw).Trim()
    }

    if ($Quality) {
        # Stretched/jumping bodies: relay stage 1 and no card steps (avatarmanz/VIctory-7900XTX issue 13,
        # fxceless/victory-fork issue 89). Rainbow skin: texture refreshes wait for the runtime's remapping.
        # Costs about 7 fps (the submitting thread waits about 30 ms a frame for the card).
        $settings.VICTORY_RELAY = '1'; $settings.VICTORY_CARD_STEPS = '0'; $settings.VICTORY_TEXTURE_NO_WAIT = '0'
    }

    # Series S VRAM budget: at <= 720p (scale <= 0.67) RT pools are ~1.6 GB; above 720p (e.g. native 1080p)
    # hooks.rs raises RT pools to 2.9 GB so reserve 2850 MB and cap card buffers at 1024 MB to prevent WDDM paging.
    if (-not $SeriesX) {
        $renderScale = [double]::Parse($settings.VICTORY_RENDER_SCALE, [Globalization.CultureInfo]::InvariantCulture)
        if ($renderScale -gt 0.67) {
            $settings.VICTORY_CARD_ROOM_FOR_TARGETS_MB = '2850'
            $settings.VICTORY_TEXTURE_ROOM_PERCENT = '88'
            $settings.VICTORY_CARD_BUFFERS_MB = '1024'
        } else {
            $settings.VICTORY_CARD_ROOM_FOR_TARGETS_MB = '1600'
            $settings.VICTORY_TEXTURE_ROOM_PERCENT = '94'
        }
    }

    # The render scale is the rendered height over the output height (Series S 1080, Series X 2160).
    $outHeight = if ($SeriesX) { 2160 } else { 1080 }
    $console = if ($SeriesX) { 'X' } else { 'S' }
    if ($Resolution) {
        if ([int]$Resolution -gt $outHeight) {
            Write-Host "Series $console (output ${outHeight}p): ${Resolution}p is above it."
            return $null
        }
        $settings.VICTORY_RENDER_SCALE = ([int]$Resolution / $outHeight).ToString('0.####', [Globalization.CultureInfo]::InvariantCulture)
    }

    if ($Adapter) {
        $settings.VICTORY_ADAPTER = $Adapter
    }

    foreach ($k in $More.Keys) {
        $settings[$k] = $More[$k]
    }

    return $settings
}

function Set-VictoryNvidiaEnv {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][hashtable]$Settings
    )

    # (Other settings left over in this window go; where your files are stays.)
    $paths = 'VICTORY_CONFIG', 'VICTORY_GAME_DIR', 'VICTORY_CACHE_DIR', 'VICTORY_DATA_DIR'
    Get-ChildItem Env:VICTORY_* | Where-Object { $_.Name -notin $paths } | ForEach-Object { Remove-Item "Env:$($_.Name)" }
    foreach ($k in $Settings.Keys) {
        Set-Item "Env:$k" $Settings[$k]
    }
}
