param([int[]]$Ids)
Add-Type @"
using System; using System.Runtime.InteropServices;
public class TS {
  [DllImport("kernel32.dll")] public static extern IntPtr OpenThread(int a, bool i, int id);
  [DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
  [DllImport("ntdll.dll")] public static extern int NtQueryInformationThread(IntPtr h, int c, out IntPtr v, int l, IntPtr r);
  public static long Start(int id) { IntPtr h = OpenThread(0x0040, false, id); if (h == IntPtr.Zero) return -1;
    IntPtr v; int s = NtQueryInformationThread(h, 9, out v, IntPtr.Size, IntPtr.Zero); CloseHandle(h); return s == 0 ? v.ToInt64() : -2; }
}
"@
$p = Get-Process victory | Select-Object -First 1
$mods = $p.Modules | ForEach-Object { [pscustomobject]@{ n = $_.ModuleName; b = [int64]$_.BaseAddress; e = [int64]$_.BaseAddress + $_.ModuleMemorySize } }
foreach ($id in $Ids) { $a = [TS]::Start($id); $m = $mods | Where-Object { $a -ge $_.b -and $a -lt $_.e } | Select-Object -First 1
  "{0} start 0x{1:X} {2}+0x{3:X}" -f $id, $a, $m.n, ($a - $m.b) }
