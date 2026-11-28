# Wechselt alle $Every s zwischen pin an/aus, sobald die Welt läuft
param([int]$From = 200, [int]$Until = 680, [int]$Every = 40)
$p = $null; while (-not $p) { $p = Get-Process victory -EA SilentlyContinue | Select-Object -First 1; Start-Sleep 1 }
$t0 = $p.StartTime
while (((Get-Date) - $t0).TotalSeconds -lt $From) { Start-Sleep 1 }
$on = $false
while (((Get-Date) - $t0).TotalSeconds -lt $Until -and (Get-Process victory -EA SilentlyContinue)) {
  if ($on) { & C:\Users\hinze\gx-workers\pin.ps1 -Off } else { & C:\Users\hinze\gx-workers\pin.ps1 -On }
  $on = -not $on; Start-Sleep $Every
}
if (Get-Process victory -EA SilentlyContinue) { & C:\Users\hinze\gx-workers\pin.ps1 -Off }
