# Precompiles shaders and warms host pipelines for NVIDIA cards using the exact same environment as play-nvidia.ps1.
#   scripts\warm-nvidia.ps1 [-Used] [-Shaders] [-Again] [-Jobs <int>] [-Limit <int>] [-RayTracing] [-SeriesX] [-Adapter <string>] [-Build <diag|release>] [-More <hashtable>]
param(
    [switch]$Used,
    [switch]$Shaders,
    [switch]$Again,
    [int]$Jobs,
    [int]$Limit,
    [switch]$RayTracing,
    [switch]$SeriesX,
    [string]$Adapter,
    [ValidateSet('diag', 'release')][string]$Build = 'release',
    [hashtable]$More = @{}
)

$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if ($isAdmin) {
    Write-Warning "Running as Administrator: NVIDIA keeps a separate DXCache for elevated processes. For in-game caching, run from a normal (non-admin) prompt."
}

. (Join-Path $PSScriptRoot 'nvidia-env.ps1')

$settings = Get-VictoryNvidiaSettings `
    -SeriesX:$SeriesX `
    -RayTracing:$RayTracing `
    -Adapter $Adapter `
    -Build $Build `
    -More $More

if ($null -eq $settings) {
    return
}

Set-VictoryNvidiaEnv -Settings $settings

$victory = Join-Path $PSScriptRoot "..\target\$Build\victory.exe"
if (-not (Test-Path $victory)) {
    Write-Host "No $victory. Build it first (README.md)."
    return
}

Push-Location (Join-Path $PSScriptRoot '..')   # victory.toml is looked for here
try {
    if ($Shaders) {
        Write-Host "Pre-translating shaders into cache..."
        $shaderArgs = @('shaders')
        if ($Jobs -gt 0) {
            $shaderArgs += @('--jobs', $Jobs)
        }
        & $victory @shaderArgs
    }

    Write-Host "Warming pipelines..."
    $warmArgs = @('warm')
    if ($Used) { $warmArgs += '--used' }
    if ($Again) { $warmArgs += '--again' }
    if ($Jobs -gt 0) { $warmArgs += @('--jobs', $Jobs) }
    if ($Limit -gt 0) { $warmArgs += @('--limit', $Limit) }

    & $victory @warmArgs
} finally {
    Pop-Location
}
