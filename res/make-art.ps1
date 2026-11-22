# Draws Trimly's artwork with Windows' built-in System.Drawing (no extra tools):
#   res\app.ico                        app/tray/desktop icon, every size drawn natively
#   res\installer\wizard-*.png         installer side banner (welcome and finish pages)
#   res\installer\small-*.png          installer header logo (other pages)
# Run from anywhere:  powershell -ExecutionPolicy Bypass -File res\make-art.ps1
Add-Type -AssemblyName System.Drawing
$res = $PSScriptRoot
New-Item -ItemType Directory -Force (Join-Path $res "installer") | Out-Null

function Color($a, $r, $g, $b) { [System.Drawing.Color]::FromArgb($a, $r, $g, $b) }
function P($x, $y) { [System.Drawing.PointF]::new($x, $y) }

function RoundedPath($x, $y, $w, $r) {
  $p = New-Object System.Drawing.Drawing2D.GraphicsPath
  $p.AddArc($x, $y, 2 * $r, 2 * $r, 180, 90)
  $p.AddArc($x + $w - 2 * $r, $y, 2 * $r, 2 * $r, 270, 90)
  $p.AddArc($x + $w - 2 * $r, $y + $w - 2 * $r, 2 * $r, 2 * $r, 0, 90)
  $p.AddArc($x, $y + $w - 2 * $r, 2 * $r, 2 * $r, 90, 90)
  $p.CloseFigure()
  $p
}

# The Trimly tile: dark slate rounded square with a mint-to-cyan bolt, at ($x,$y), $s pixels wide.
function DrawTile($g, $x, $y, $s) {
  $inset = if ($s -le 24) { 0.5 } else { $s * 0.04 }
  $w = $s - 2 * $inset
  $path = RoundedPath ($x + $inset) ($y + $inset) $w ($w * 0.26)
  $bg = New-Object System.Drawing.Drawing2D.LinearGradientBrush (P 0 $y), (P 0 ($y + $s)), (Color 255 30 42 50), (Color 255 11 17 21)
  $g.FillPath($bg, $path)
  if ($s -ge 24) {
    $rim = New-Object System.Drawing.Pen (Color 40 255 255 255), ([Math]::Max(1, $s / 64))
    $g.DrawPath($rim, $path)
  }
  # Bolt from a 24-unit grid; bigger at small sizes so it stays legible.
  $pad = if ($s -le 20) { 0.08 } elseif ($s -le 32) { 0.13 } else { 0.18 }
  $k = ($s * (1 - 2 * $pad)) / 20.0
  $ox = $x + ($s - 15 * $k) / 2 - 4 * $k
  $oy = $y + ($s - 20 * $k) / 2 - 2 * $k
  $pts = [System.Drawing.PointF[]](@(@(13, 2), @(4, 14), @(11, 14), @(10, 22), @(19, 10), @(12, 10)) | ForEach-Object { P ($ox + $_[0] * $k) ($oy + $_[1] * $k) })
  $bolt = New-Object System.Drawing.Drawing2D.LinearGradientBrush (P ($x + $s * 0.25) ($y + $s * 0.1)), (P ($x + $s * 0.75) ($y + $s * 0.9)), (Color 255 74 222 128), (Color 255 34 211 238)
  $g.FillPolygon($bolt, $pts)
}

function NewCanvas($w, $h) {
  $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = 'AntiAlias'; $g.PixelOffsetMode = 'HighQuality'; $g.TextRenderingHint = 'AntiAliasGridFit'
  $g.Clear([System.Drawing.Color]::Transparent)
  @($bmp, $g)
}

# ---------- App icon ----------
$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$pngs = @()
foreach ($s in $sizes) {
  $bmp, $g = NewCanvas $s $s
  DrawTile $g 0 0 $s
  $g.Dispose()
  $ms = New-Object System.IO.MemoryStream
  $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  $pngs += , $ms.ToArray()
}
# ICO container: 6-byte header, a 16-byte entry per image, then the PNG data.
$ico = Join-Path $res "app.ico"
$bw = New-Object System.IO.BinaryWriter ([System.IO.File]::Create($ico))
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
  $dim = if ($sizes[$i] -ge 256) { 0 } else { $sizes[$i] }
  $bw.Write([byte]$dim); $bw.Write([byte]$dim); $bw.Write([byte]0); $bw.Write([byte]0)
  $bw.Write([uint16]1); $bw.Write([uint16]32); $bw.Write([uint32]$pngs[$i].Length); $bw.Write([uint32]$offset)
  $offset += $pngs[$i].Length
}
foreach ($p in $pngs) { $bw.Write($p) }
$bw.Close()

# ---------- Installer side banner (164 x 314 at 100%) ----------
foreach ($scale in 100, 150, 200) {
  $f = $scale / 100.0
  $w = [int](164 * $f); $h = [int](314 * $f)
  $bmp, $g = NewCanvas $w $h
  $bg = New-Object System.Drawing.Drawing2D.LinearGradientBrush (P 0 0), (P 0 $h), (Color 255 20 32 40), (Color 255 10 15 19)
  $g.FillRectangle($bg, 0, 0, $w, $h)
  # Soft mint glow behind the tile.
  $glow = New-Object System.Drawing.Drawing2D.GraphicsPath
  $glow.AddEllipse(-0.35 * $w, 0.05 * $h, 1.7 * $w, 0.55 * $h)
  $pgb = New-Object System.Drawing.Drawing2D.PathGradientBrush $glow
  $pgb.CenterColor = Color 70 52 211 153
  $pgb.SurroundColors = [System.Drawing.Color[]]@((Color 0 52 211 153))
  $g.FillPath($pgb, $glow)
  # Faint diagonal lines for texture.
  $line = New-Object System.Drawing.Pen (Color 10 255 255 255), (1 * $f)
  for ($i = -$h; $i -lt $w; $i += [int](18 * $f)) { $g.DrawLine($line, $i, $h, $i + $h, 0) }

  $tile = 72 * $f
  DrawTile $g (($w - $tile) / 2) (86 * $f) $tile
  $sf = New-Object System.Drawing.StringFormat; $sf.Alignment = 'Center'
  $title = New-Object System.Drawing.Font ("Segoe UI Semibold", (22 * $f), [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel)
  $sub = New-Object System.Drawing.Font ("Segoe UI", (11.5 * $f), [System.Drawing.FontStyle]::Regular, [System.Drawing.GraphicsUnit]::Pixel)
  $g.DrawString("Trimly", $title, (New-Object System.Drawing.SolidBrush (Color 255 236 242 246)), (New-Object System.Drawing.RectangleF 0, (172 * $f), $w, (32 * $f)), $sf)
  $g.DrawString("Keep your apps lean", $sub, (New-Object System.Drawing.SolidBrush (Color 255 134 150 163)), (New-Object System.Drawing.RectangleF 0, (204 * $f), $w, (20 * $f)), $sf)
  # Accent bar.
  $bar = New-Object System.Drawing.Drawing2D.LinearGradientBrush (P ($w * 0.38) 0), (P ($w * 0.62) 0), (Color 255 52 211 153), (Color 255 34 211 238)
  $g.FillRectangle($bar, $w * 0.38, 234 * $f, $w * 0.24, 3 * $f)
  $g.Dispose()
  $bmp.Save((Join-Path $res "installer\wizard-$scale.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
}

# ---------- Installer header logo (55 x 55 at 100%) ----------
foreach ($scale in 100, 150, 200) {
  $s = [int](55 * $scale / 100.0)
  $bmp, $g = NewCanvas $s $s
  DrawTile $g ($s * 0.06) ($s * 0.06) ($s * 0.88)
  $g.Dispose()
  $bmp.Save((Join-Path $res "installer\small-$scale.png"), [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
}
"Artwork written to $res"
