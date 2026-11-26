#!/usr/bin/env bash
# Regenerates the media fixtures that scripts/e2e/media.script shows.
# Needs ImageMagick and ffmpeg. The outputs are committed, so the e2e run
# itself needs neither.
set -euo pipefail
out=$(cd "$(dirname "$0")" && pwd)/fixtures
mkdir -p "$out"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

magick -size 800x450 gradient:'#1e3a8a-#9333ea' -gravity center -fill white \
    -pointsize 140 -annotate 0 IMAGE "$out/image.png"

# As big as the e2e stage screen, so unlike image.png it doesn't look soft.
magick -size 1920x1080 gradient:'#0f766e-#facc15' -gravity center -fill white \
    -pointsize 300 -annotate 0 PHOTO -quality 85 "$out/photo.jpg"

# Taller than wide, as a phone's photos are, so the library's Shapes has
# a tall tile.
magick -size 1080x1350 gradient:'#9d174d-#fb923c' -gravity center -fill white \
    -pointsize 170 -annotate 0 PORTRAIT -quality 85 "$out/portrait.jpg"

# testsrc draws a seconds counter, so shots at different times differ.
ffmpeg -hide_banner -loglevel error -y -f lavfi -i testsrc=size=480x270:rate=30:duration=6 \
    -c:v libx264 -pix_fmt yuv420p -crf 32 -movflags +faststart "$out/video.mp4"

for n in 1 2 3; do
    magick -size 600x800 xc:white -gravity center -fill black -pointsize 420 -annotate 0 "$n" \
        -gravity north -pointsize 60 -annotate +0+40 PAGE "$work/$n.png"
done
magick "$work"/{1,2,3}.png -compress Zip "$out/pages.pdf"

printf '<!doctype html><title>web</title>\n' >"$out/page.html"
ls -la "$out"
