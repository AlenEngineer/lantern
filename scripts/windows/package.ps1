# Puts the Windows download together in a folder, then zips it:
#
#   lantern.exe, and GStreamer's runtime beside it (its DLLs, and the
#   plugins in lib\gstreamer-1.0, where GStreamer looks relative to its
#   own DLL), so videos play without installing anything;
#   poppler\, whose pdftoppm turns PDFs into pictures (in a folder of its
#   own: it brings DLLs of its own that GStreamer's would clash with).
#
# Run after `cargo build --release -p lantern-app`, with GStreamer and
# poppler installed as .github/actions/windows-deps does.
#
#   scripts\windows\package.ps1 -Name lantern-v0.9.0-x86_64-windows
param([Parameter(Mandatory)] [string] $Name)
$ErrorActionPreference = "Stop"

$gst = $env:GSTREAMER_1_0_ROOT_MSVC_X86_64
if (-not $gst -or -not (Test-Path "$gst\bin")) { throw "GSTREAMER_1_0_ROOT_MSVC_X86_64 isn't GStreamer's folder: '$gst'" }
$pdftoppm = (Get-Command pdftoppm.exe).Source

if (Test-Path $Name) { Remove-Item -Recurse -Force $Name }
New-Item -ItemType Directory $Name | Out-Null
Copy-Item target\release\lantern.exe, README.md, CHANGELOG.md, LICENSE-MIT, LICENSE-APACHE, crates\app\fonts\Inter-LICENSE.txt $Name

Copy-Item "$gst\bin\*.dll" $Name
foreach ($sub in "lib\gstreamer-1.0", "lib\gio\modules") {
  if (Test-Path "$gst\$sub") {
    New-Item -ItemType Directory -Force "$Name\$sub" | Out-Null
    Copy-Item "$gst\$sub\*.dll" "$Name\$sub"
  }
}
# The scanner GStreamer runs to read its plugins, and the certificates
# HTTPS streams are checked against.
foreach ($sub in "libexec\gstreamer-1.0", "etc\ssl") {
  if (Test-Path "$gst\$sub") {
    New-Item -ItemType Directory -Force "$Name\$sub" | Out-Null
    Copy-Item -Recurse "$gst\$sub\*" "$Name\$sub"
  }
}

New-Item -ItemType Directory "$Name\poppler" | Out-Null
Copy-Item "$(Split-Path $pdftoppm)\*" "$Name\poppler"

Compress-Archive -Path $Name -DestinationPath "$Name.zip" -Force
$hash = (Get-FileHash -Algorithm SHA256 "$Name.zip").Hash.ToLower()
"$hash  $Name.zip" | Out-File -Encoding ascii "$Name.zip.sha256"
"{0}: {1:N0} MB" -f "$Name.zip", ((Get-Item "$Name.zip").Length / 1MB)
