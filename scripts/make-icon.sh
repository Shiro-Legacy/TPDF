#!/bin/zsh
# Rebuild assets/icon.icns from assets/icon.svg (macOS only: sips + iconutil). Run from the repo root.
set -eu
T=$(mktemp -d); I=$T/icon.iconset; mkdir $I
sips -s format png assets/icon.svg --out $T/1024.png >/dev/null
for s in 16 32 128 256 512; do
  sips -z $s $s $T/1024.png --out $I/icon_${s}x${s}.png >/dev/null
  sips -z $((s*2)) $((s*2)) $T/1024.png --out $I/icon_${s}x${s}@2x.png >/dev/null
done
iconutil -c icns $I -o assets/icon.icns
rm -rf $T
