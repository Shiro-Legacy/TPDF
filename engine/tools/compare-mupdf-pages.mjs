#!/usr/bin/env node
// Render each FILE's pages with the Rust workflow CLI and with MuPDF (the
// application's node_modules copy) at 72 DPI, and report pixel agreement.
// usage: compare-mupdf-pages.mjs RUST_CLI FILE... (TPDF_PDF_PASSWORD optional)
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const mupdf = await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"));
const [cli, ...files] = process.argv.slice(2);
const password = process.env.TPDF_PDF_PASSWORD ?? "";
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "tpdf-cmp-"));
const rows = [];
for (const file of files) {
  const doc = mupdf.Document.openDocument(fs.readFileSync(file), "application/pdf");
  if (doc.needsPassword()) doc.authenticatePassword(password);
  for (let page = 0; page < doc.countPages(); page++) {
    const out = path.join(scratch, `${rows.length}.ppm`);
    const row = { file: path.basename(file), page };
    try {
      execFileSync(cli, ["render", file, String(page), "72", out], { stdio: ["ignore", "ignore", "pipe"] });
    } catch (e) {
      rows.push({ ...row, rustError: String(e.stderr).trim() });
      continue;
    }
    const ppm = fs.readFileSync(out);
    const header = ppm.toString("latin1", 0, 64).split(/\s+/);
    const [w, h] = [Number(header[1]), Number(header[2])];
    const rust = ppm.subarray(ppm.length - w * h * 3);
    const pix = doc.loadPage(page).toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false, true);
    const ref = pix.getPixels();
    Object.assign(row, { size: [w, h], mupdfSize: [pix.getWidth(), pix.getHeight()] });
    if (w !== pix.getWidth() || h !== pix.getHeight()) { rows.push(row); continue; }
    let exact = 0, near = 0, far = 0;
    for (let i = 0; i < w * h; i++) {
      const d = Math.max(...[0, 1, 2].map((c) => Math.abs(rust[i * 3 + c] - ref[i * 3 + c])));
      if (d === 0) exact++;
      if (d <= 16) near++;
      if (d > 64) far++;
    }
    const pct = (n) => Math.round((10000 * n) / (w * h)) / 100;
    rows.push({ ...row, exactPct: pct(exact), within16Pct: pct(near), over64Px: far });
  }
}
fs.rmSync(scratch, { recursive: true, force: true });
console.log(JSON.stringify(rows, null, 1));
