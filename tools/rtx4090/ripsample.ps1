param([int]$Tid, [int]$Samples = 2000, [int]$SleepMs = 3, [string]$Out = 'C:\Users\hinze\vbench\prof\rip.txt')
Add-Type @"
using System; using System.Runtime.InteropServices;
public class RS {
  [DllImport("kernel32.dll")] public static extern IntPtr OpenThread(int a, bool i, int id);
  [DllImport("kernel32.dll")] public static extern uint SuspendThread(IntPtr h);
  [DllImport("kernel32.dll")] public static extern uint ResumeThread(IntPtr h);
  [DllImport("kernel32.dll")] public static extern bool GetThreadContext(IntPtr h, IntPtr ctx);
  [DllImport("kernel32.dll")] public static extern IntPtr OpenProcess(int a, bool i, int id);
  [DllImport("kernel32.dll")] public static extern bool ReadProcessMemory(IntPtr p, IntPtr a, byte[] b, IntPtr n, out IntPtr r);
  public static long[][] Stacks;
  public static long[] Sample(int pid, int tid, int n, int sleepMs) {
    IntPtr ph = OpenProcess(0x0010, false, pid); Stacks = new long[n][]; byte[] buf = new byte[1024];
    IntPtr h = OpenThread(0x0002 | 0x0008 | 0x0040, false, tid); if (h == IntPtr.Zero) return null;
    IntPtr raw = Marshal.AllocHGlobal(1232 + 16); IntPtr ctx = new IntPtr((raw.ToInt64() + 15) & ~15L);
    long[] rips = new long[n];
    for (int i = 0; i < n; i++) {
      for (int b = 0; b < 1232; b += 8) Marshal.WriteInt64(ctx, b, 0);
      Marshal.WriteInt32(ctx, 0x30, 0x00100001); // CONTEXT_AMD64 | CONTEXT_CONTROL
      SuspendThread(h);
      rips[i] = GetThreadContext(h, ctx) ? Marshal.ReadInt64(ctx, 0xF8) : -1;
      IntPtr rd; long rsp = Marshal.ReadInt64(ctx, 0x98); Stacks[i] = new long[128];
      if (rips[i] > 0 && ReadProcessMemory(ph, new IntPtr(rsp), buf, new IntPtr(1024), out rd)) for (int q = 0; q < 128; q++) Stacks[i][q] = BitConverter.ToInt64(buf, q * 8);
      ResumeThread(h);
      System.Threading.Thread.Sleep(sleepMs);
    }
    Marshal.FreeHGlobal(raw); return rips;
  }
}
"@
$p = Get-Process victory | Select-Object -First 1
$mods = $p.Modules | ForEach-Object { [pscustomobject]@{ n = $_.ModuleName; b = [int64]$_.BaseAddress; e = [int64]$_.BaseAddress + $_.ModuleMemorySize } } | Sort-Object b
$rips = [RS]::Sample($p.Id, $Tid, $Samples, $SleepMs)
$vx = $mods | Where-Object { $_.n -eq 'victory.exe' }
$tab = @{}
$raw = New-Object System.Collections.Generic.List[string]
$byMod = @{}
for ($si = 0; $si -lt $rips.Count; $si++) { $a = $rips[$si]
  $m = $mods | Where-Object { $a -ge $_.b -and $a -lt $_.e } | Select-Object -First 1
  $mn = if ($m) { $m.n } else { 'unknown' }
  $byMod[$mn] = 1 + [int]$byMod[$mn]
  $callers = @($RS_dummy); $callers = @(); foreach ($q in [RS]::Stacks[$si]) { if ($q -ge $vx.b -and $q -lt $vx.e) { $callers += ("{0:X}" -f ($q - $vx.b)); if ($callers.Count -ge 4) { break } } }
  if ($m) { $raw.Add(("{0} {1:X} {2}" -f $mn, ($a - $m.b), ($callers -join ','))) }
  $key = if ($m) { "{0}+0x{1:X}" -f $mn, (($a - $m.b) -band (-bnot 0xFF)) } else { "0x{0:X}" -f ($a -band (-bnot 0xFF)) }
  $tab[$key] = 1 + [int]$tab[$key]
}
$o = @("thread $Tid, $($rips.Count) samples", "== by module")
$o += $byMod.GetEnumerator() | Sort-Object Value -Descending | ForEach-Object { "{0,6} {1}" -f $_.Value, $_.Key }
$o += "== top 256-byte blocks"
$o += $tab.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 40 | ForEach-Object { "{0,6} {1}" -f $_.Value, $_.Key }
$raw | Set-Content ($Out + ".raw"); $o | Set-Content $Out; $o | Select-Object -First 60
