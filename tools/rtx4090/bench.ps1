# Unattended benchmark run: starts the title through scripts\play-nvidia.ps1, presses Enter
# through the start screens (VICTORY_KEYS), screenshots the game window only, measures fps in
# the world, stops. Output (logs, screenshots: title content!) goes OUTSIDE the repo, to -OutRoot.
#   tools\rtx4090\bench.ps1 -Label base -Seconds 420 [-SeriesX] [-More @{VICTORY_X='1'}] [-Extra @('-Quality')]
param(
    [Parameter(Mandatory)][string]$Label,
    [int]$Seconds = 420,
    [string]$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path,
    [string]$GameExe = (Join-Path $env:VICTORY_GAME_DIR 'game_scarlett_master.exe'),
    [string]$OutRoot = (Join-Path $env:LOCALAPPDATA 'victory\bench'),
    [switch]$SeriesX,
    [hashtable]$More = @{},
    [string[]]$Extra = @()
)
$ErrorActionPreference = 'Continue'
$out = Join-Path $OutRoot $Label
New-Item -ItemType Directory -Force $out | Out-Null
Get-Process victory, victory_sm -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep 2

# Enter alle 4 s von 15 bis 200 s (Startbildschirme, Kontoauswahl, Story starten)
$keys = (15..50 | ForEach-Object { "$($_ * 4):Enter" }) -join ','
$m = @{ VICTORY_KEYS = $keys; VICTORY_LOG = 'info' }
foreach ($k in $More.Keys) { $m[$k] = $More[$k] }
$moreText = ($m.GetEnumerator() | ForEach-Object { "'$($_.Key)'='$($_.Value)'" }) -join ';'
$series = if ($SeriesX) { '-SeriesX' } else { '' }
 $extraText = $Extra -join ' '
$cmd = "& '$Repo\scripts\play-nvidia.ps1' -Exe '$GameExe' -NoSmoothMotion $series $extraText -More @{$moreText}"
Set-Content "$out\cmd.txt" $cmd
Set-Content "$out\launch.ps1" $cmd -Encoding UTF8
$p = Start-Process cmd -ArgumentList '/c', "powershell -NoProfile -ExecutionPolicy Bypass -File `"$out\launch.ps1`" > `"$out\run.log`" 2>&1" -PassThru -WindowStyle Minimized

Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public class WinRect { [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); }
"@
$t0 = Get-Date
$shots = 0
while (((Get-Date) - $t0).TotalSeconds -lt $Seconds) {
    Start-Sleep 30
    if (-not (Get-Process victory, victory_sm -ErrorAction SilentlyContinue)) { Add-Content "$out\notes.txt" "exited early at $([int]((Get-Date)-$t0).TotalSeconds) s"; break }
    try {
        # nur das Spielfenster, nie den ganzen Bildschirm (Privatsphäre)
        $gp = Get-Process victory, victory_sm -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        if (-not $gp) { continue }
        $r = New-Object WinRect+RECT
        [void][WinRect]::GetWindowRect($gp.MainWindowHandle, [ref]$r)
        $b = New-Object System.Drawing.Rectangle $r.Left, $r.Top, ($r.Right - $r.Left), ($r.Bottom - $r.Top)
        if ($b.Width -lt 100) { continue }
        $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        [void][WinRect]::SetForegroundWindow($gp.MainWindowHandle); Start-Sleep -Milliseconds 300; $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
        $bmp.Save(("$out\shot-{0:000}.jpg" -f [int]((Get-Date) - $t0).TotalSeconds), [System.Drawing.Imaging.ImageFormat]::Jpeg)
        $g.Dispose(); $bmp.Dispose(); $shots++
    } catch {}
}
Get-Process victory, victory_sm -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep 2

# Auswertung: fps-Zeilen in der Welt (> 1000 draws/frame), ab 60 s nach dem ersten Weltframe
$rows = Select-String -Path "$out\run.log" -Pattern 'victory_gpu::host: ([0-9.]+) fps, ([0-9.]+) ms/frame \(([0-9.]+) preparing: copies ([0-9.]+) ms.*?([0-9.]+) on the GPU; ([0-9.]+) copying back.*?, ([0-9]+) draws/frame, dropped ([0-9]+)' |
    ForEach-Object { $g = $_.Matches[0].Groups; [pscustomobject]@{ fps=[double]$g[1].Value; ms=[double]$g[2].Value; prep=[double]$g[3].Value; copy=[double]$g[4].Value; gpu=[double]$g[5].Value; back=[double]$g[6].Value; draws=[int]$g[7].Value; dropped=[int]$g[8].Value } }
$world = @($rows | Where-Object { $_.draws -gt 1000 })
$steady = if ($world.Count -gt 12) { $world[12..($world.Count - 1)] } else { $world }
$sum = if ($steady.Count) {
    $f = $steady.fps | Sort-Object
    "{0}: n={1} fps mean={2:N1} min={3:N1} p10={4:N1} | ms prep={5:N1} copy={6:N1} gpu={7:N1} back={8:N1} | draws={9:N0} dropped={10:N0}" -f $Label, $steady.Count,
        ($steady.fps | Measure-Object -Average).Average, $f[0], $f[[int]($f.Count * 0.1)],
        ($steady.prep | Measure-Object -Average).Average, ($steady.copy | Measure-Object -Average).Average,
        ($steady.gpu | Measure-Object -Average).Average, ($steady.back | Measure-Object -Average).Average,
        ($steady.draws | Measure-Object -Average).Average, ($steady.dropped | Measure-Object -Average).Average
} else { "${Label}: keine Weltframes erreicht (rows=$($rows.Count))" }
if ($world.Count -gt 24) {
    $tail = $world[($world.Count - 24)..($world.Count - 1)]; $tf = $tail.fps | Sort-Object
    $sum += (" || last2min fps mean={0:N1} min={1:N1} prep={2:N1} back={3:N1} dropped={4:N0}" -f ($tail.fps | Measure-Object -Average).Average, $tf[0], ($tail.prep | Measure-Object -Average).Average, ($tail.back | Measure-Object -Average).Average, ($tail.dropped | Measure-Object -Average).Average)
}
$sum | Tee-Object -FilePath "$out\summary.txt"
Add-Content (Join-Path $OutRoot 'results.txt') "$(Get-Date -Format s) $sum | shots=$shots"
