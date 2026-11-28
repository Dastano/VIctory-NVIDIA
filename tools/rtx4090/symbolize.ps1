param([string]$Raw, [string]$Exe = 'C:\Users\hinze\foxhound\target\release\victory.exe', [string]$Module = 'victory.exe', [int]$Top = 40, [string]$CallersOf = '', [int]$Depth = 2)
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public class Sym {
  [DllImport("dbghelp.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern bool SymInitializeW(IntPtr p, string path, bool inv);
  [DllImport("dbghelp.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern ulong SymLoadModuleExW(IntPtr p, IntPtr f, string img, string mod, ulong b, uint sz, IntPtr d, uint fl);
  [DllImport("dbghelp.dll", SetLastError=true)] public static extern uint SymSetOptions(uint o);
  [DllImport("dbghelp.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern bool SymFromAddrW(IntPtr p, ulong a, out ulong disp, IntPtr info);
  static IntPtr h = new IntPtr(0x5151);
  public static bool Init(string img) { if (!SymInitializeW(h, System.IO.Path.GetDirectoryName(img), false)) return false; return SymLoadModuleExW(h, IntPtr.Zero, img, null, 0x10000000, 0, IntPtr.Zero, 0) != 0; }
  public static string Name(ulong off) {
    int sz = 88 + 2 * 2000; IntPtr info = Marshal.AllocHGlobal(sz);
    for (int i = 0; i < sz; i++) Marshal.WriteByte(info, i, 0);
    Marshal.WriteInt32(info, 0, 88); Marshal.WriteInt32(info, 80, 2000);
    ulong d; string r = SymFromAddrW(h, 0x10000000 + off, out d, info) ? Marshal.PtrToStringUni(new IntPtr(info.ToInt64() + 84)) : "?";
    Marshal.FreeHGlobal(info); return r;
  }
}
"@
if (-not [Sym]::Init($Exe)) { "init failed"; exit 1 }
if ($CallersOf) {
  $rows = Get-Content $Raw | Where-Object { $_ -like "$CallersOf *" }; $n = @{}
  foreach ($r in $rows) { $parts = $r -split ' '; $cs = if ($parts.Count -gt 2 -and $parts[2]) { $parts[2] -split ',' | Select-Object -First $Depth } else { @() }
    $k = (@($parts[1]) + ($cs | ForEach-Object { ([Sym]::Name([Convert]::ToUInt64($_, 16)) -replace '::h[0-9a-f]{16}$','' -replace '<.*','<..>') })) -join ' <- '; $n[$k] = 1 + [int]$n[$k] }
  "$($rows.Count) samples in $CallersOf"; $n.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First $Top | ForEach-Object { "{0,5} {1}" -f $_.Value, $_.Key }; exit }
$rows = Get-Content $Raw | Where-Object { $_ -like "$Module *" }
$n = @{}
foreach ($r in $rows) { $off = [Convert]::ToUInt64(($r -split ' ')[1], 16); $f = [Sym]::Name($off); $f = $f -replace '::h[0-9a-f]{16}$',''; $n[$f] = 1 + [int]$n[$f] }
"$($rows.Count) samples in $Module"
$n.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First $Top | ForEach-Object { "{0,5} {1}" -f $_.Value, $_.Key }
