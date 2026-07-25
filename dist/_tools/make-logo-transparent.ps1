# Removes the opaque black background from images/logo.png.
# The artwork was flattened onto black, so dark pixels are un-multiplied:
#   m = max(R,G,B);  m >= T -> untouched;  m < T -> alpha = m/T, colour scaled by T/m.
param(
  [string]$Source = (Join-Path $PSScriptRoot '..\images\logo.png'),
  [int]$Threshold = 56
)

Add-Type -AssemblyName System.Drawing

$Source = (Resolve-Path $Source).Path
$backup = Join-Path ([IO.Path]::GetDirectoryName($Source)) ([IO.Path]::GetFileNameWithoutExtension($Source) + '-original.png')
if (-not (Test-Path $backup)) { Copy-Item $Source $backup }

$src = [System.Drawing.Bitmap]::new($backup)
$w = $src.Width; $h = $src.Height
$out = [System.Drawing.Bitmap]::new($w, $h, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)

$rect = [System.Drawing.Rectangle]::new(0, 0, $w, $h)
$sd = $src.LockBits($rect, 'ReadOnly', 'Format32bppArgb')
$dd = $out.LockBits($rect, 'WriteOnly', 'Format32bppArgb')

$len = [Math]::Abs($sd.Stride) * $h
$buf = [byte[]]::new($len)
[Runtime.InteropServices.Marshal]::Copy($sd.Scan0, $buf, 0, $len)

$T = [double]$Threshold
for ($i = 0; $i -lt $len; $i += 4) {
  # BGRA order
  $b = [int]$buf[$i]; $g = [int]$buf[$i + 1]; $r = [int]$buf[$i + 2]
  $m = [Math]::Max($r, [Math]::Max($g, $b))
  if ($m -eq 0) {
    $buf[$i] = 0; $buf[$i + 1] = 0; $buf[$i + 2] = 0; $buf[$i + 3] = 0
  }
  elseif ($m -lt $Threshold) {
    $k = $T / $m
    $buf[$i]     = [byte][Math]::Min(255, [Math]::Round($b * $k))
    $buf[$i + 1] = [byte][Math]::Min(255, [Math]::Round($g * $k))
    $buf[$i + 2] = [byte][Math]::Min(255, [Math]::Round($r * $k))
    $buf[$i + 3] = [byte][Math]::Round(255 * $m / $T)
  }
  else {
    $buf[$i + 3] = 255
  }
}

[Runtime.InteropServices.Marshal]::Copy($buf, 0, $dd.Scan0, $len)
$src.UnlockBits($sd)
$out.UnlockBits($dd)

$out.Save($Source, [System.Drawing.Imaging.ImageFormat]::Png)
$src.Dispose(); $out.Dispose()

Write-Host "Wrote $Source (backup: $backup)"
