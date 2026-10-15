#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Render each FILE's pages with a view_probe build and with MuPDF (the app's
// node_modules copy) and report pixel agreement: over the page, and over
// "ink" (pixels either render left non-white), where text and line work are.
// usage: compare-app-view.mjs VIEW_PROBE DPI [--dump DIR] FILE...
// The probe reads TPDF_PDF_PASSWORD, TPDF_VIEW_APP=1 (render through the
// app's view, AppDocument::view) and TPDF_CJK_FONT (a font to install);
// PAGES=0,3 limits the pages. --dump writes Rust | MuPDF side by side as PNG.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const mupdf = await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"));
const args = process.argv.slice(2);
const cli = args.shift();
const dpi = Number(args.shift());
let dump = null;
const at = args.indexOf("--dump");
if (at >= 0) {
  dump = args[at + 1];
  args.splice(at, 2);
  fs.mkdirSync(dump, { recursive: true });
}
const only = process.env.PAGES ? process.env.PAGES.split(",").map(Number) : null;
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "tpdf-view-"));

function crc32(bytes) {
  let crc = 0xffffffff;
  for (const b of bytes) {
    crc ^= b;
    for (let k = 0; k < 8; k++) crc = crc & 1 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function png(w, h, rgb) {
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) rgb.copy(raw, y * (w * 3 + 1) + 1, y * w * 3, (y + 1) * w * 3);
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body));
    return Buffer.concat([len, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2;
  const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  return Buffer.concat([sig, chunk("IHDR", ihdr), chunk("IDAT", zlib.deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]);
}

const rows = [];
for (const file of args) {
  const doc = mupdf.Document.openDocument(fs.readFileSync(file), "application/pdf");
  if (doc.needsPassword()) doc.authenticatePassword(process.env.TPDF_PDF_PASSWORD ?? "");
  for (let page = 0; page < doc.countPages(); page++) {
    if (only && !only.includes(page)) continue;
    const out = path.join(scratch, `${rows.length}.ppm`);
    const row = { file: path.relative(root, path.resolve(file)), page };
    try {
      execFileSync(cli, ["render", file, String(page), String(dpi), out], { stdio: ["ignore", "ignore", "pipe"] });
    } catch (e) {
      rows.push({ ...row, rustError: String(e.stderr).trim() });
      continue;
    }
    const ppm = fs.readFileSync(out);
    const header = ppm.toString("latin1", 0, 64).split(/\s+/);
    const [w, h] = [Number(header[1]), Number(header[2])];
    const rust = ppm.subarray(ppm.length - w * h * 3);
    const scale = mupdf.Matrix.scale(dpi / 72, dpi / 72);
    const pix = doc.loadPage(page).toPixmap(scale, mupdf.ColorSpace.DeviceRGB, false, true);
    const ref = pix.getPixels();
    if (w !== pix.getWidth() || h !== pix.getHeight()) {
      rows.push({ ...row, size: [w, h], mupdfSize: [pix.getWidth(), pix.getHeight()] });
      continue;
    }
    let exact = 0, near = 0, far = 0, ink = 0, inkExact = 0, inkNear = 0, rustInk = 0, mupdfInk = 0;
    for (let i = 0; i < w * h; i++) {
      let d = 0, rustWhite = true, mupdfWhite = true;
      for (let c = 0; c < 3; c++) {
        d = Math.max(d, Math.abs(rust[i * 3 + c] - ref[i * 3 + c]));
        if (rust[i * 3 + c] < 250) rustWhite = false;
        if (ref[i * 3 + c] < 250) mupdfWhite = false;
      }
      if (d === 0) exact++;
      if (d <= 16) near++;
      if (d > 64) far++;
      if (!rustWhite) rustInk++;
      if (!mupdfWhite) mupdfInk++;
      if (!rustWhite || !mupdfWhite) {
        ink++;
        if (d === 0) inkExact++;
        if (d <= 16) inkNear++;
      }
    }
    const pct = (n, of) => (of ? Math.round((10000 * n) / of) / 100 : 100);
    rows.push({
      ...row, exactPct: pct(exact, w * h), within16Pct: pct(near, w * h), over64Px: far,
      inkPx: ink, inkExactPct: pct(inkExact, ink), inkWithin16Pct: pct(inkNear, ink), rustInk, mupdfInk,
    });
    if (dump) {
      const both = Buffer.alloc(w * 2 * h * 3);
      const mu = Buffer.from(ref.buffer, ref.byteOffset, ref.length);
      for (let y = 0; y < h; y++) {
        rust.copy(both, y * w * 6, y * w * 3, (y + 1) * w * 3);
        mu.copy(both, y * w * 6 + w * 3, y * w * 3, (y + 1) * w * 3);
      }
      fs.writeFileSync(path.join(dump, `${path.basename(file)}-${page}.png`), png(w * 2, h, both));
    }
  }
}
fs.rmSync(scratch, { recursive: true, force: true });
console.log(JSON.stringify(rows, null, 1));
