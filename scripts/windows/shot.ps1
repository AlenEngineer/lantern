# LANTERN_SHOT on Windows: saves the whole screen as $env:SHOTS\NAME.png.
param([Parameter(Mandatory)] [string] $Name)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bitmap = New-Object System.Drawing.Bitmap $screen.Width, $screen.Height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($screen.Left, $screen.Top, 0, 0, $bitmap.Size)
New-Item -ItemType Directory -Force $env:SHOTS | Out-Null
$bitmap.Save("$env:SHOTS\$Name.png", [System.Drawing.Imaging.ImageFormat]::Png)
