// SPDX-License-Identifier: AGPL-3.0-or-later
// MuPDF check of the Rust writer's encrypted output (tests/encrypt_write.rs).
//   TPDF_ENCRYPT_ORACLE_DIR=<dir> cargo test --release --test encrypt_write oracle_cases
//   node tools/encrypt-oracle.mjs <app root> <dir> [summary.json]
// Every output must need a password, authenticate the user password as 2 and
// the owner password as 4 (wrong and empty as 0), report the source's
// algorithm and permissions (Keep) or V5 R6 with the requested bits (AES-256),
// and give the source's page text, title, Metadata and signature /Contents
// (and the /Contents of a /Type /Sig without /ByteRange, which is encrypted).
// Passwords are the manifest's UTF-8 strings; MuPDF converts them to
// PDFDocEncoding for R2-R4.
import {readFile, writeFile} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';

const [appRoot, dir, summaryPath] = process.argv.slice(2);
const mupdf = await import(pathToFileURL(`${appRoot}/node_modules/mupdf/dist/mupdf.js`).href);
const PERMISSIONS = {print: 4, edit: 8, copy: 16, annotate: 32, form: 256, accessibility: 512, assemble: 1024, 'print-hq': 2048};

const open = bytes => {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  doc.asPDF().disableJS();
  return doc;
};
const level = (bytes, password) => {
  const doc = open(bytes);
  try { return doc.authenticatePassword(password); } finally { doc.destroy(); }
};
function facts(bytes, password) {
  const doc = open(bytes);
  try {
    const needsPassword = doc.needsPassword();
    if (needsPassword) assert.ok(doc.authenticatePassword(password) > 0, 'password');
    const pdf = doc.asPDF();
    const pages = [];
    for (let i = 0; i < doc.countPages(); i++) {
      const page = doc.loadPage(i);
      const text = page.toStructuredText('preserve-whitespace');
      pages.push(text.asText());
      text.destroy();
      page.destroy();
    }
    const root = pdf.getTrailer().get('Root');
    const metadata = root.get('Metadata');
    const sig = root.get('Sig');
    const unsigned = root.get('Unsigned');
    const encrypt = pdf.getTrailer().get('Encrypt');
    return {
      needsPassword,
      repaired: pdf.wasRepaired(),
      encryption: doc.getMetaData('encryption'),
      title: doc.getMetaData('info:Title') ?? null,
      permissions: Object.fromEntries(Object.keys(PERMISSIONS).map(name => [name, doc.hasPermission(name)])),
      p: encrypt.isNull() ? null : Number(encrypt.get('P').toString()),
      pages,
      metadata: metadata.isStream() ? new TextDecoder('latin1').decode(metadata.readStream().asUint8Array()) : null,
      signature: sig.isDictionary() ? new TextDecoder('latin1').decode(sig.get('Contents').asByteString()) : null,
      unsigned: unsigned.isDictionary() ? new TextDecoder('latin1').decode(unsigned.get('Contents').asByteString()) : null,
    };
  } finally {
    doc.destroy();
  }
}

const manifest = JSON.parse(await readFile(`${dir}/manifest.json`, 'utf8'));
const results = [];
const failures = [];
for (const entry of manifest) {
  const bytes = await readFile(`${dir}/${entry.file}`);
  const source = await readFile(`${dir}/${entry.source}`);
  const result = {file: entry.file, mode: entry.mode, sha256: createHash('sha256').update(bytes).digest('hex')};
  try {
    result.levels = {user: level(bytes, entry.user), owner: level(bytes, entry.owner), wrong: level(bytes, 'wrong'), empty: level(bytes, '')};
    assert.deepEqual(result.levels, {user: 2, owner: 4, wrong: 0, empty: 0});
    const out = facts(bytes, entry.user);
    const expected = facts(source, entry.mode === 'keep' ? entry.user : entry.sourceUser);
    result.encryption = out.encryption;
    result.p = out.p;
    result.permissions = out.permissions;
    assert.equal(out.needsPassword, true);
    assert.equal(out.repaired, false);
    if (entry.mode === 'keep') {
      assert.equal(out.encryption, expected.encryption);
      assert.deepEqual(out.permissions, expected.permissions);
      assert.equal(out.p, expected.p);
    } else {
      assert.equal(out.encryption, 'Standard V5 R6 256-bit AES');
      const granted = entry.permissions >>> 0;
      assert.equal(out.p, ((granted & 0xF3C) | 0x200 | 0xFFFFF0C0) | 0);
      for (const [name, bit] of Object.entries(PERMISSIONS)) {
        assert.equal(out.permissions[name], name === 'accessibility' || (granted & bit) !== 0, name);
      }
      const owner = facts(bytes, entry.owner);
      assert.ok(Object.values(owner.permissions).every(Boolean), 'owner has every permission');
    }
    assert.deepEqual(out.pages, expected.pages);
    assert.equal(out.title, expected.title);
    // A source with /EncryptMetadata false keeps Metadata in plaintext, which
    // MuPDF decrypts anyway; such entries name the XML the output must hold.
    assert.equal(out.metadata, entry.metadata ?? expected.metadata);
    assert.equal(out.signature, expected.signature);
    assert.equal(out.unsigned, expected.unsigned);
    result.pages = out.pages.length;
    result.ok = true;
  } catch (error) {
    result.ok = false;
    result.error = error.message;
    failures.push(entry.file);
  }
  results.push(result);
}
const {version} = JSON.parse(await readFile(`${appRoot}/node_modules/mupdf/package.json`, 'utf8'));
const summary = {
  oracle: `MuPDF ${version} WASM (node_modules/mupdf, test only)`,
  notes: [
    'Keep and AES-256 outputs of every encrypted fixture and generated family (two with non-ASCII passwords, which MuPDF hashes in PDFDocEncoding for R2-R4), and AES-256 outputs of hello.pdf and a generated document for six permission sets.',
    'MuPDF decrypts the Metadata stream of an /EncryptMetadata false document although the writer left it in plaintext (ISO 32000, qpdf), so it reads that stream as empty in the source and in its Keep outputs; AES-256 outputs encrypt Metadata and are checked against the XML.',
  ],
  cases: results.length,
  passed: results.filter(r => r.ok).length,
  failures,
  results,
};
if (summaryPath) await writeFile(summaryPath, JSON.stringify(summary, null, 2) + '\n');
console.log(JSON.stringify({cases: summary.cases, passed: summary.passed, failures}, null, 2));
for (const r of results.filter(r => !r.ok)) console.log(r.file, r.error);
process.exitCode = failures.length ? 1 : 0;
