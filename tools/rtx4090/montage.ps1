param([string]$Dir, [int]$From = 0, [int]$Per = 16, [string]$Out)
Add-Type -AssemblyName System.Drawing
$files = Get-ChildItem $Dir -Filter 'shot-*.jpg' | Sort-Object Name | Select-Object -Skip $From -First $Per
$cols = 4; $w = 640; $h = 360; $rows = [math]::Ceiling($files.Count / $cols)
$b = New-Object Drawing.Bitmap ($cols * $w), ($rows * $h); $g = [Drawing.Graphics]::FromImage($b)
$font = New-Object Drawing.Font 'Arial', 14
for ($i = 0; $i -lt $files.Count; $i++) {
  $im = [Drawing.Image]::FromFile($files[$i].FullName)
  $x = ($i % $cols) * $w; $y = [math]::Floor($i / $cols) * $h
  $g.DrawImage($im, (New-Object Drawing.Rectangle $x, $y, $w, $h), (New-Object Drawing.Rectangle 8, 31, ($im.Width - 16), ($im.Height - 39)), [Drawing.GraphicsUnit]::Pixel)
  $g.FillRectangle([Drawing.Brushes]::Black, $x, $y, 90, 22); $g.DrawString($files[$i].BaseName.Substring(5), $font, [Drawing.Brushes]::Yellow, $x + 2, $y + 1)
  $im.Dispose()
}
$b.Save($Out, [Drawing.Imaging.ImageFormat]::Jpeg); $g.Dispose(); $b.Dispose(); "$($files.Count) -> $Out"
