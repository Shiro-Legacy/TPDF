// SPDX-License-Identifier: MIT
// The few document-level reads the tests make with the npm `mupdf` package, an
// independent reader of the Rust engine's output (test-only; nothing of it
// ships). rust-engine.mjs hands them out as members of the test engine.
// `detect` reports the DocumentInfo facts (web/protocol.ts) as a second reader
// sees them in the bytes, read the way engine/src/app/info.rs defines them.
import { MAX_PDF_BYTES } from '../build-test/limits.js';

const PERMISSIONS = ['print', 'copy', 'edit', 'annotate', 'form'];

/** `read()`, or `fallback` when it throws. */
function tryRead(read, fallback) {
  try {
    return read();
  } catch {
    return fallback;
  }
}

/** `object[key]` when `object` is a dictionary and the value is not null, else null. */
function entry(object, key) {
  if (!object || !tryRead(() => object.isDictionary() || object.isStream(), false)) return null;
  const value = tryRead(() => object.get(key), null);
  return value && !value.isNull() ? value : null;
}

const nameOf = object => (object && tryRead(() => object.isName(), false) ? object.asName() : null);
const numberOf = object => (object && tryRead(() => object.isNumber(), false) ? object.asNumber() : null);

/**
 * The /Encrypt algorithm name (ISO 32000-2 7.6): V 5 is AES-256; from V 4 the
 * stream crypt filter's /CFM (the string filter when that is /Identity) names
 * the method, before V 4 it is RC4. RC4 is 40-bit for R 2 or a 40-bit /Length.
 */
function algorithmOf(encrypt) {
  if (!encrypt) return 'none';
  if (!tryRead(() => encrypt.isDictionary(), false)) return 'unknown';
  const v = numberOf(entry(encrypt, 'V')) ?? 0;
  const r = numberOf(entry(encrypt, 'R')) ?? 0;
  if (v >= 5) return 'aes-256';
  const methodOf = key => {
    const name = nameOf(entry(encrypt, key)) ?? 'Identity';
    if (name === 'Identity') return 'Identity';
    return nameOf(entry(entry(entry(encrypt, 'CF'), name), 'CFM')) ?? 'None';
  };
  let method = 'V2';
  let methods = [];
  if (v >= 4) {
    methods = [methodOf('StmF'), methodOf('StrF')];
    method = methods[0] === 'Identity' ? methods[1] : methods[0];
  }
  if (method === 'AESV3') return 'aes-256';
  if (method === 'AESV2') return 'aes-128';
  if (method !== 'V2') return 'unknown';
  const length = v === 1 ? 40 : methods.includes('AESV2') ? 128 : numberOf(entry(encrypt, 'Length')) ?? (v === 4 ? 128 : 40);
  return r <= 2 || length === 40 ? 'rc4-40' : 'rc4-128';
}

/** Fields under /Fields and their /Kids with their own /FT /Sig and /V; each object counted once. */
function countSignatures(acroForm) {
  const seen = new Set();
  let count = 0;
  const visit = (field, depth) => {
    if (!field || depth > 64 || !tryRead(() => field.isDictionary(), false)) return;
    if (field.isIndirect()) {
      const number = field.asIndirect();
      if (seen.has(number)) return;
      seen.add(number);
    }
    if (nameOf(entry(field, 'FT')) === 'Sig' && entry(field, 'V')) count++;
    walk(entry(field, 'Kids'), depth + 1);
  };
  const walk = (array, depth) => {
    if (!array || !tryRead(() => array.isArray(), false)) return;
    for (let index = 0; index < array.length; index++) visit(array.get(index), depth);
  };
  walk(entry(acroForm, 'Fields'), 0);
  return count;
}

export function createReader(mupdf) {
  /** Opens bytes with MuPDF, timed; the caller closes the document. */
  function openDocument(pdf) {
    if (!(pdf instanceof ArrayBuffer)) throw new TypeError('openDocument: expected an ArrayBuffer');
    if (pdf.byteLength > MAX_PDF_BYTES) throw new RangeError(`openDocument: over the ${MAX_PDF_BYTES} byte ceiling`);
    const started = performance.now();
    const doc = mupdf.Document.openDocument(pdf, 'application/pdf');
    return { doc, openMs: performance.now() - started };
  }

  /** Destroys a document; null and an already destroyed one are fine. */
  function closeDocument(doc) {
    tryRead(() => doc?.destroy(), undefined);
  }

  /** DocumentInfo as MuPDF reads `doc`; `needsPassword` overrides MuPDF's own answer. */
  function detect(doc, needsPassword) {
    const pdf = tryRead(() => doc.asPDF(), null);
    const trailer = pdf && tryRead(() => pdf.getTrailer(), null);
    const encrypt = entry(trailer, 'Encrypt');
    const root = entry(trailer, 'Root');
    const acroForm = entry(root, 'AcroForm');
    const encryption = tryRead(() => doc.getMetaData('encryption'), undefined);
    const permissions = Object.fromEntries(PERMISSIONS.map(name => [name, tryRead(() => doc.hasPermission(name), true)]));
    const marked = entry(entry(root, 'MarkInfo'), 'Marked');
    return {
      encrypted: (encryption !== undefined && encryption !== 'None') || !!encrypt,
      needsPassword: needsPassword ?? tryRead(() => doc.needsPassword(), false),
      algorithm: algorithmOf(encrypt),
      permissions,
      repaired: !!pdf && tryRead(() => pdf.wasRepaired(), false),
      signatures: countSignatures(acroForm),
      certified: !!entry(entry(root, 'Perms'), 'DocMDP'),
      xfa: !!entry(acroForm, 'XFA'),
      attachments: pdf ? tryRead(() => Object.keys(pdf.getEmbeddedFiles()).length, 0) : 0,
      openAction: !!entry(root, 'OpenAction') || !!entry(root, 'AA'),
      tagged: !!marked && tryRead(() => marked.asBoolean(), false),
      versions: pdf ? Math.max(0, tryRead(() => pdf.countVersions(), 1) - 1) : 0,
      printable: permissions.print,
    };
  }

  return { openDocument, closeDocument, detect };
}
