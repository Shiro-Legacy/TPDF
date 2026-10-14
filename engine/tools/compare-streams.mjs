// SPDX-License-Identifier: AGPL-3.0-or-later
// Compare actual decoded PDF streams to pinned MuPDF; never modify fixtures.
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const [moduleArg, fixtureArg, rustArg, reportArg] = process.argv.slice(2);
if (!moduleArg || !fixtureArg || !rustArg || !reportArg) throw new Error('usage: node compare-streams.mjs /path/to/mupdf/dist/mupdf.js fixtures rust-binary report.json');
const modulePath = resolve(moduleArg), root = resolve(fixtureArg), rust = resolve(rustArg);
const meta = JSON.parse(await readFile(join(dirname(modulePath), '..', 'package.json'), 'utf8'));
if (meta.name !== 'mupdf' || meta.version !== '1.28.1') throw new Error('requires pinned mupdf 1.28.1');
const mupdf = await import(pathToFileURL(modulePath).href);
const inventory = JSON.parse(await readFile(new URL('../reference/fixture-inventory.json', import.meta.url), 'utf8'));
const rows = [];
let matched = 0, unsupported = 0, failures = 0, decodedBytes = 0;
for (const expected of inventory.documents) {
  const path = join(root, expected.file);
  const bytes = await readFile(path);
  if (createHash('sha256').update(bytes).digest('hex') !== expected.sha256) throw new Error(`fixture changed ${expected.file}`);
  const run = spawnSync(rust, ['--document', path], { timeout: 10000, maxBuffer: 8 * 1024 * 1024 });
  if (run.error || run.signal) throw run.error ?? new Error(`Rust reader signal ${run.signal}`);
  if (run.status !== 0) {
    const reason = run.stderr.toString().trim();
    if (run.status !== 2 || !reason.includes('unsupported')) { failures++; rows.push({file:expected.file,status:'failure',error:reason}); }
    continue;
  }
  const info = JSON.parse(run.stdout.toString());
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf').asPDF();
  try {
    doc.disableJS();
    for (const stream of info.streams) {
      const id = `${stream.object}:${stream.generation}`;
      const row = { file: expected.file, id };
      const decoded = spawnSync(rust, [`--stream=${id}`, path], {timeout:10000,maxBuffer:65 * 1024 * 1024});
      if (decoded.error || decoded.signal) throw decoded.error ?? new Error(`Rust decoder signal ${decoded.signal}`);
      if (decoded.status !== 0) {
        const reason = decoded.stderr.toString().trim();
        const status = decoded.status === 2 && (reason.includes('unsupported') || reason.includes('unknown filter')) ? 'unsupported' : 'failure';
        if (status === 'unsupported') unsupported++; else failures++;
        rows.push({...row,status,error:reason});
        continue;
      }
      const object = doc.newIndirect(stream.object);
      const native = object.readStream();
      try {
        const original = Buffer.from(native.asUint8Array());
        if (!original.equals(decoded.stdout)) { failures++; rows.push({...row,status:'failure',expected:original.length,actual:decoded.stdout.length}); }
        else {
          matched++; decodedBytes += original.length;
          rows.push({...row,status:'matched',bytes:original.length,sha256:createHash('sha256').update(original).digest('hex')});
        }
      } finally { native.destroy(); }
    }
  } finally { doc.destroy(); }
}
const result = {reference:'MuPDF 1.28.1 WASM',matched,unsupported,failures,decodedBytes,passed:failures===0 && matched>0,results:rows};
await writeFile(reportArg, JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify({...result,results:undefined},null,2));
for (const row of rows.filter(row=>row.status==='failure')) console.error(JSON.stringify(row));
process.exitCode = result.passed ? 0 : 1;
