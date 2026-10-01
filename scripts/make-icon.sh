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
# Windows: assets/icon.ico from the same art, one PNG entry per size (Windows Vista and later read PNG entries).
for s in 16 24 32 48 64 128 256; do sips -z $s $s $T/1024.png --out $T/ico-$s.png >/dev/null; done
node -e '
const fs = require("fs"), [dir, out] = process.argv.slice(1), sizes = [16, 24, 32, 48, 64, 128, 256];
const pngs = sizes.map(s => fs.readFileSync(`${dir}/ico-${s}.png`));
const head = Buffer.alloc(6 + 16 * sizes.length); head.writeUInt16LE(1, 2); head.writeUInt16LE(sizes.length, 4);
let offset = head.length;
sizes.forEach((s, i) => { const at = 6 + 16 * i; head[at] = head[at + 1] = s % 256; head.writeUInt16LE(1, at + 4); head.writeUInt16LE(32, at + 6); head.writeUInt32LE(pngs[i].length, at + 8); head.writeUInt32LE(offset, at + 12); offset += pngs[i].length; });
fs.writeFileSync(out, Buffer.concat([head, ...pngs]));
' $T assets/icon.ico
rm -rf $T
