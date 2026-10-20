// SPDX-License-Identifier: AGPL-3.0-or-later
// Object-by-object comparison of the app save (examples/app_save_probe.rs)
// with MuPDF saveToBuffer('incremental=no,garbage=yes'). Encrypted inputs are
// written decrypted by both (MuPDF encrypt=none). Trailers are compared
// separately: /Size and the second /ID element differ (APP-ENGINE-DESIGN.md).
//   cargo build --release --example app_save_probe
//   node oracle/compare-app-save.mjs ../node_modules/mupdf/dist/mupdf.js ../fixtures \
//     target/release/examples/app_save_probe [report.json]
import { readFile, readdir, writeFile, mkdtemp, rm } from 'node:fs/promises';
import { basename, join, relative, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

// One private user document is never read, wherever it is. It is matched by the SHA-256 of its
// file name, so the name itself is not in the source.
const FORBIDDEN_NAME_SHA256 = '79c4929919f1d1f0eea98be823f0ccd3ecdcb5bd20707473cfaf177c3c5395c7';

const [moduleArg, fixtureArg, probeArg, reportArg] = process.argv.slice(2);
if (!probeArg) throw new Error('usage: node compare-app-save.mjs mupdf.js fixtures app_save_probe [report.json]');
const mupdf = await import(pathToFileURL(resolve(moduleArg)).href);
const root = resolve(fixtureArg);
const PASSWORD = 'pdf4all-user';

async function pdfs(dir) {
  const out = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...await pdfs(path));
    else if (entry.name.endsWith('.pdf')) out.push(path);
  }
  return out.sort();
}

// Object bodies by number, cut at the offsets of the last classic xref.
function objects(bytes) {
  const text = Buffer.from(bytes).toString('latin1');
  const at = text.lastIndexOf('startxref');
  const xref = parseInt(text.slice(at + 9).trim(), 10);
  const lines = text.slice(xref).split('\n');
  const starts = [];
  let i = 1;
  while (i < lines.length && !lines[i].startsWith('trailer')) {
    const [first, count] = lines[i].trim().split(/\s+/).map(Number);
    for (let k = 0; k < count; k++) {
      const row = lines[i + 1 + k];
      if (row.trim().endsWith('n')) starts.push([first + k, parseInt(row, 10)]);
    }
    i += count + 1;
  }
  starts.sort((a, b) => a[1] - b[1]);
  const map = new Map();
  starts.forEach(([number, offset], k) => {
    const end = k + 1 < starts.length ? starts[k + 1][1] : xref;
    const chunk = text.slice(offset, end).replace(/\s+$/, '');
    map.set(number, chunk.slice(chunk.indexOf('obj') + 3).replace(/^\s+/, '').replace(/endobj$/, '').replace(/\s+$/, ''));
  });
  return map;
}

// The last trailer dictionary.
function trailer(bytes) {
  const text = Buffer.from(bytes).toString('latin1');
  return text.slice(text.lastIndexOf('trailer'), text.lastIndexOf('startxref')).trim();
}

const dir = await mkdtemp(join(tmpdir(), 'app-save-'));
const rows = [];
try {
  for (const file of await pdfs(root)) {
    const name = relative(root, file);
    if (createHash('sha256').update(basename(file)).digest('hex') === FORBIDDEN_NAME_SHA256) continue;
    const bytes = await readFile(file);
    const row = { file: name };
    try {
      const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
      const encrypted = doc.needsPassword();
      if (encrypted) doc.authenticatePassword(PASSWORD);
      const options = 'incremental=no,garbage=yes' + (encrypted ? ',encrypt=none' : '');
      const saved = doc.asPDF().saveToBuffer(options).asUint8Array();
      const expected = objects(saved);
      const out = join(dir, 'out.pdf');
      const run = spawnSync(resolve(probeArg), [file, out], {
        env: { ...process.env, TPDF_PDF_PASSWORD: encrypted ? PASSWORD : '' },
        timeout: 60000,
      });
      if (run.status !== 0) throw new Error(`probe: ${run.stderr.toString().trim()}`);
      const written = await readFile(out);
      const actual = objects(written);
      let same = 0;
      const differing = [];
      for (const [number, body] of expected) {
        if (actual.get(number) === body) same++;
        else differing.push(number);
      }
      Object.assign(row, {
        mupdf: expected.size,
        rust: actual.size,
        same,
        missing: [...expected.keys()].filter((n) => !actual.has(n)).length,
        extra: [...actual.keys()].filter((n) => !expected.has(n)).length,
        differing: differing.slice(0, 12),
        trailer: trailer(saved) === trailer(written),
      });
    } catch (error) {
      row.error = String(error.message ?? error).slice(0, 200);
    }
    rows.push(row);
    console.log(JSON.stringify(row));
  }
} finally {
  await rm(dir, { recursive: true, force: true });
}
const ok = rows.filter((r) => !r.error);
const summary = {
  files: rows.length,
  compared: ok.length,
  identical: ok.filter((r) => r.same === r.mupdf && r.rust === r.mupdf).length,
  objects: ok.reduce((n, r) => n + r.mupdf, 0),
  same: ok.reduce((n, r) => n + r.same, 0),
  trailers: ok.filter((r) => r.trailer).length,
};
console.log(JSON.stringify(summary));
if (reportArg) await writeFile(reportArg, JSON.stringify({ summary, rows }, null, 1) + '\n');
