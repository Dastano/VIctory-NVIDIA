# pin.ps1 -On / -Off : game main thread and our busiest submit thread each on a core of their own
param([switch]$On, [switch]$Off, [int]$TitleCpu = 0, [int]$SubmitCpu = 2, [string]$Log = 'C:\Users\hinze\vbench\pin.log')
Add-Type @"
using System; using System.Runtime.InteropServices;
public class PN {
  [DllImport("kernel32.dll")] public static extern IntPtr OpenThread(int a, bool i, int id);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll")] public static extern UIntPtr SetThreadAffinityMask(IntPtr h, UIntPtr m);
  [DllImport("ntdll.dll")] public static extern int NtQueryInformationThread(IntPtr h, int c, out IntPtr v, int l, IntPtr r);
  public static long Start(int id) { IntPtr h = OpenThread(0x0040, false, id); if (h == IntPtr.Zero) return -1; IntPtr v; int s = NtQueryInformationThread(h, 9, out v, IntPtr.Size, IntPtr.Zero); CloseHandle(h); return s == 0 ? v.ToInt64() : -2; }
  public static bool Mask(int id, ulong m) { IntPtr h = OpenThread(0x0060, false, id); if (h == IntPtr.Zero) return false; UIntPtr r = SetThreadAffinityMask(h, new UIntPtr(m)); CloseHandle(h); return r != UIntPtr.Zero; }
}
"@
$p = Get-Process victory | Select-Object -First 1
$all = [uint64]0xFFFFFFFF
if ($Off) { foreach ($t in $p.Threads) { [void][PN]::Mask($t.Id, $all) }; Add-Content $Log "$((Get-Date).ToUniversalTime().ToString("s")) off"; 'off'; return }
$vx = $p.Modules | Where-Object ModuleName -eq 'victory.exe'
$img = $p.Modules | Where-Object { $_.ModuleName -like 'victory-image-*' } | Sort-Object ModuleMemorySize -Descending | Select-Object -First 1
$a = @{}; foreach ($t in $p.Threads) { try { $a[$t.Id] = $t.TotalProcessorTime.TotalMilliseconds } catch {} }
Start-Sleep 3; $p.Refresh()
$rows = foreach ($t in $p.Threads) { try { $s = [PN]::Start($t.Id); [pscustomobject]@{ id = $t.Id; ms = $t.TotalProcessorTime.TotalMilliseconds - $a[$t.Id]; title = ($s -ge [int64]$img.BaseAddress -and $s -lt [int64]$img.BaseAddress + $img.ModuleMemorySize); ours = ($s -ge [int64]$vx.BaseAddress -and $s -lt [int64]$vx.BaseAddress + $vx.ModuleMemorySize) } } catch {} }
$title = $rows | Where-Object title | Sort-Object ms -Descending | Select-Object -First 1
$sub = $rows | Where-Object ours | Sort-Object ms -Descending | Select-Object -First 1
$excl = $all -bxor ([uint64]3 -shl $TitleCpu) -bxor ([uint64]3 -shl $SubmitCpu)
$n = 0; foreach ($r in $rows) { if ($r.id -ne $title.id -and $r.id -ne $sub.id) { if ([PN]::Mask($r.id, $excl)) { $n++ } } }
$ok1 = [PN]::Mask($title.id, [uint64]1 -shl $TitleCpu); $ok2 = [PN]::Mask($sub.id, [uint64]1 -shl $SubmitCpu)
$msg = "$((Get-Date).ToUniversalTime().ToString("s")) on: title $($title.id) ($([int]($title.ms/30))%) -> cpu $TitleCpu $ok1, submit $($sub.id) ($([int]($sub.ms/30))%) -> cpu $SubmitCpu $ok2, others $n masked 0x$('{0:X}' -f $excl)"
Add-Content $Log $msg; $msg
