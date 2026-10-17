// SPDX-License-Identifier: AGPL-3.0-or-later
// Read-only golden inventory using the existing pinned MuPDF JS engine.
import { readdir, readFile } from 'node:fs/promises';
import { dirname, join, relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';

const [moduleArg, fixtureArg] = process.argv.slice(2);
if (!moduleArg || !fixtureArg) throw new Error('usage: node fixture-inventory.mjs /path/to/mupdf/dist/mupdf.js /path/to/fixtures');
const modulePath = resolve(moduleArg);
const meta = JSON.parse(await readFile(join(dirname(modulePath), '..', 'package.json'), 'utf8'));
if (meta.name !== 'mupdf' || meta.version !== '1.28.1') throw new Error('requires pinned mupdf 1.28.1');
const mupdf = await import(pathToFileURL(modulePath).href);
const root = resolve(fixtureArg);
async function list(dir) {
  const result = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (entry.name.startsWith('.')) continue;
    const path = join(dir, entry.name);
    if (entry.isDirectory()) result.push(...await list(path));
    else if (entry.isFile() && entry.name.endsWith('.pdf')) result.push(path);
  }
  return result.sort();
}
const documents = [];
for (const path of await list(root)) {
  const data = await readFile(path);
  const row = { file: relative(root, path), bytes: data.length, sha256: createHash('sha256').update(data).digest('hex') };
  let doc;
  try {
    doc = mupdf.Document.openDocument(data, 'application/pdf');
    row.passwordRequired = !!doc.needsPassword();
    if (!row.passwordRequired) {
      doc.disableJS();
      row.pages = doc.countPages();
      row.objects = doc.asPDF().countObjects();
      row.pageDetails = [];
      for (let i = 0; i < row.pages; i++) {
        const page = doc.loadPage(i);
        try { row.pageDetails.push({ bounds: page.getBounds(), rotation: doc.asPDF().findPage(i).get('Rotate').asNumber() }); }
        finally { page.destroy(); }
      }
    }
  } catch (error) { row.error = String(error); }
  finally { doc?.destroy(); }
  documents.push(row);
}
console.log(JSON.stringify({ reference: `MuPDF ${meta.version} WASM`, documents }, null, 2));
