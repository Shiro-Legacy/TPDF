// Rebuild the app icon files from assets/icon.svg, on any platform:  npx electron scripts/make-icon.mjs
//   assets/icon.png          512 px, Linux window/About icon and the .deb's hicolor entry (scripts/build.mjs, make-linux.sh)
//   assets/icon.ico          Windows, one PNG entry per size 16…256 (Vista and later read PNG entries); the exe and installer
//   assets/icon.icns         macOS, the iconutil set (16…512 at 1x and 2x) as PNG entries
//   assets/icon-preview.png  1024 px render for a visual check
// Electron rasterizes the SVG with Chromium in a hidden window (no sips/ImageMagick needed); the containers are written by hand.
import { app, BrowserWindow } from 'electron';
import { writeFile, readFile } from 'node:fs/promises';

// Electron emits 'ready' only after this module's top-level code has run, so the work waits for it in a callback.
app.whenReady().then(main).catch(e => { console.error(e); app.exit(1); });

async function main() {
const svg = await readFile('assets/icon.svg', 'utf8');
const sizes = [16, 24, 32, 48, 64, 128, 256, 512, 1024];
const win = new BrowserWindow({ show: false, webPreferences: { offscreen: true, sandbox: true } });
await win.loadURL('data:text/html,<img id=i>');
const url = `data:image/svg+xml;base64,${Buffer.from(svg).toString('base64')}`;
const dataUrls = await win.webContents.executeJavaScript(`(async () => {
  const img = document.getElementById('i'); img.src = ${JSON.stringify(url)}; await img.decode();
  return Object.fromEntries(${JSON.stringify(sizes)}.map(s => {
    const c = document.createElement('canvas'); c.width = c.height = s;
    c.getContext('2d').drawImage(img, 0, 0, s, s); return [s, c.toDataURL('image/png')];
  }));
})()`);
const png = Object.fromEntries(Object.entries(dataUrls).map(([s, u]) => [s, Buffer.from(u.split(',')[1], 'base64')]));

await writeFile('assets/icon.png', png[512]);
await writeFile('assets/icon-preview.png', png[1024]);

// ICO: header (reserved, type 1, count), one 16-byte directory entry per image, then the PNG payloads.
const ico = [16, 24, 32, 48, 64, 128, 256];
const head = Buffer.alloc(6 + 16 * ico.length); head.writeUInt16LE(1, 2); head.writeUInt16LE(ico.length, 4);
let offset = head.length;
ico.forEach((s, i) => {
  const at = 6 + 16 * i; head[at] = head[at + 1] = s % 256; head.writeUInt16LE(1, at + 4); head.writeUInt16LE(32, at + 6);
  head.writeUInt32LE(png[s].length, at + 8); head.writeUInt32LE(offset, at + 12); offset += png[s].length;
});
await writeFile('assets/icon.ico', Buffer.concat([head, ...ico.map(s => png[s])]));

// ICNS: 'icns' + total length, then (type, length incl. this 8-byte header, PNG) per entry; the types iconutil emits.
const icns = [['icp4', 16], ['ic11', 32], ['icp5', 32], ['ic12', 64], ['ic07', 128], ['ic13', 256], ['ic08', 256], ['ic14', 512], ['ic09', 512], ['ic10', 1024]];
const entries = icns.map(([type, s]) => { const h = Buffer.alloc(8); h.write(type, 0, 'ascii'); h.writeUInt32BE(8 + png[s].length, 4); return Buffer.concat([h, png[s]]); });
const total = Buffer.alloc(8); total.write('icns', 0, 'ascii'); total.writeUInt32BE(8 + entries.reduce((n, e) => n + e.length, 0), 4);
await writeFile('assets/icon.icns', Buffer.concat([total, ...entries]));
console.log('wrote assets/icon.png icon.ico icon.icns icon-preview.png');
app.exit(0);
}
