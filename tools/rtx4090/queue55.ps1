param([string]$Label = 'r121-save70-look', [int]$Seconds = 420, [string]$Keys = '', [string[]]$Extra = @('-Dlss', 'quality', '-Fps', '1000'), [int]$ShotEvery = 10, [string]$LiveFile = '', [int]$KeysUntil = 200)
$fox = 'C:\Users\hinze\foxhound'
$data = "$env:LOCALAPPDATA\victory\data\7952987A"
$bench = 'C:\Users\hinze\vbench\bench-save-70\7952987A\saves'
$mk = ''; if ($Keys) { $mk = (Get-Content $Keys) -join ',' }
Move-Item "$fox\victory-settings.txt" "$fox\victory-settings.user.txt" -Force
Rename-Item "$data\saves" 'saves.user'
Copy-Item $bench "$data\saves" -Recurse
try {
  & 'C:\Users\hinze\gx-workers\bench.ps1' -Label $Label -Repo $fox -Launcher 'scripts\play-foxhound.ps1' -Seconds $Seconds -NoTurns -KeysUntil $KeysUntil -ShotEvery $ShotEvery -More $(if ($LiveFile) { @{ VICTORY_KEYS_FILE = $LiveFile } } else { @{} }) -MoreKeys $mk -Extra $Extra
} finally {
  Get-Process victory, victory_sm -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep 3
  if (Test-Path "$data\saves") { Remove-Item "$data\saves" -Recurse -Force }
  Rename-Item "$data\saves.user" 'saves'
  Move-Item "$fox\victory-settings.user.txt" "$fox\victory-settings.txt" -Force
}
