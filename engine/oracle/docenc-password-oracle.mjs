// SPDX-License-Identifier: AGPL-3.0-or-later
// Measures how MuPDF turns an R2-R4 password into bytes (security.rs
// pdf_doc_password): for each character c it saves fixtures/hello.pdf with
// encrypt=rc4-128 and user password "a" c "z", then finds the bytes the /U
// entry was computed from (Algorithms 2 and 5).
//   node oracle/docenc-password-oracle.mjs <app root> <out.json>
// Output: {"<hex code point>": "<hex byte>" | "dropped"}.
import {readFile, writeFile} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';

const [appRoot, outPath] = process.argv.slice(2);
const mupdf = await import(pathToFileURL(`${appRoot}/node_modules/mupdf/dist/mupdf.js`).href);
const source = await readFile(`${appRoot}/fixtures/hello.pdf`);
const PAD = Buffer.from('28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A', 'hex');
const md5 = (...parts) => parts.reduce((hash, part) => hash.update(part), createHash('md5')).digest();
function rc4(key, data) {
  const s = [...Array(256).keys()];
  let j = 0;
  for (let i = 0; i < 256; i++) {
    j = (j + s[i] + key[i % key.length]) & 255;
    [s[i], s[j]] = [s[j], s[i]];
  }
  const out = Buffer.alloc(data.length);
  for (let k = 0, i = 0, m = 0; k < data.length; k++) {
    i = (i + 1) & 255;
    m = (m + s[i]) & 255;
    [s[i], s[m]] = [s[m], s[i]];
    out[k] = data[k] ^ s[(s[i] + s[m]) & 255];
  }
  return out;
}
const padded = password => Buffer.concat([password, PAD]).subarray(0, 32);
function isUser(password, {O, P, id, U}) {
  const p = Buffer.alloc(4);
  p.writeInt32LE(P);
  let key = md5(padded(password), O.subarray(0, 32), p, id);
  for (let i = 0; i < 50; i++) key = md5(key);
  let value = md5(PAD, id);
  for (let i = 0; i < 20; i++) value = rc4(key.map(b => b ^ i), value);
  return value.equals(U.subarray(0, 16));
}

const PDFDOC = [0x2d8, 0x2c7, 0x2c6, 0x2d9, 0x2dd, 0x2db, 0x2da, 0x2dc, 0x2022, 0x2020, 0x2021, 0x2026,
  0x2014, 0x2013, 0x192, 0x2044, 0x2039, 0x203a, 0x2212, 0x2030, 0x201e, 0x201c, 0x201d, 0x2018, 0x2019,
  0x201a, 0x2122, 0xfb01, 0xfb02, 0x141, 0x152, 0x160, 0x178, 0x17d, 0x131, 0x142, 0x153, 0x161, 0x17e, 0x20ac];
const result = {};
for (const c of [...Array(255).keys()].map(i => i + 1).concat(PDFDOC, [0x100, 0x4e2d, 0x1f600, 0xfffd])) {
  const ch = String.fromCodePoint(c);
  if (ch === ',') continue; // MuPDF's option separator
  const doc = mupdf.Document.openDocument(source, 'application/pdf');
  const out = doc.saveToBuffer(`encrypt=rc4-128,user-password=a${ch}z,owner-password=own`).asUint8Array();
  doc.destroy();
  const check = mupdf.Document.openDocument(out, 'application/pdf');
  const trailer = check.asPDF().getTrailer();
  const encrypt = trailer.get('Encrypt');
  const handler = {
    O: Buffer.from(encrypt.get('O').asByteString()),
    U: Buffer.from(encrypt.get('U').asByteString()),
    P: Number(encrypt.get('P').toString()),
    id: Buffer.from(trailer.get('ID').get(0).asByteString()),
  };
  if (check.authenticatePassword(`a${ch}z`) !== 2) throw new Error(`MuPDF refused U+${c.toString(16)}`);
  check.destroy();
  let found = isUser(Buffer.from('az'), handler) ? 'dropped' : null;
  for (let b = 0; b < 256 && !found; b++) {
    if (isUser(Buffer.from([0x61, b, 0x7a]), handler)) found = b.toString(16).padStart(2, '0');
  }
  result[c.toString(16).padStart(4, '0')] = found ?? 'unknown';
}
const {version} = JSON.parse(await readFile(`${appRoot}/node_modules/mupdf/package.json`, 'utf8'));
await writeFile(outPath, `${JSON.stringify({mupdf: version, passwords: result}, null, 1)}\n`);
