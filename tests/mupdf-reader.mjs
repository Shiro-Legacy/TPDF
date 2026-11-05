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

/**
 * The distinct names in the /Names arrays of an /EmbeddedFiles name tree and
 * its /Kids (a name that is not a string counts as the empty name); each
 * indirect node is visited once.
 */
function countEmbeddedNames(tree) {
  const names = new Set();
  const seen = new Set();
  const visit = (node, depth) => {
    if (!node || depth > 32 || !tryRead(() => node.isDictionary(), false)) return;
    if (node.isIndirect()) {
      if (seen.has(node.asIndirect())) return;
      seen.add(node.asIndirect());
    }
    const pairs = entry(node, 'Names');
    if (pairs && pairs.isArray()) {
      for (let index = 0; index < pairs.length; index += 2) {
        const name = pairs.get(index);
        names.add(tryRead(() => (name.isString() ? name.asString() : ''), ''));
      }
    }
    const kids = entry(node, 'Kids');
    if (kids && kids.isArray()) for (let index = 0; index < kids.length; index++) visit(kids.get(index), depth + 1);
  };
  visit(tree, 0);
  return names.size;
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

  /**
   * DocumentInfo as MuPDF reads `doc`, field by field as engine/src/app/info.rs
   * defines it. `needsPassword`, when a boolean, is the caller's answer (a copy
   * authenticated with its password still asks MuPDF for one).
   */
  function detect(doc, needsPassword) {
    const pdf = tryRead(() => doc.asPDF(), null);
    const trailer = pdf && tryRead(() => pdf.getTrailer(), null);
    const encrypt = entry(trailer, 'Encrypt');
    const root = entry(trailer, 'Root');
    const acroForm = entry(root, 'AcroForm');
    const permissions = {};
    for (const name of PERMISSIONS) permissions[name] = tryRead(() => doc.hasPermission(name), true);
    const locked = typeof needsPassword === 'boolean' ? needsPassword : tryRead(() => doc.needsPassword(), false);
    // info.rs counts the trailer chain (this section and every earlier one) less one.
    const sections = pdf ? tryRead(() => pdf.countVersions(), 0) : 0;
    const marked = entry(entry(root, 'MarkInfo'), 'Marked');
    return {
      encrypted: encrypt !== null,
      needsPassword: locked,
      algorithm: algorithmOf(encrypt),
      permissions,
      repaired: pdf ? tryRead(() => pdf.wasRepaired(), false) : false,
      signatures: countSignatures(acroForm),
      certified: entry(entry(root, 'Perms'), 'DocMDP') !== null,
      xfa: entry(acroForm, 'XFA') !== null,
      attachments: countEmbeddedNames(entry(entry(root, 'Names'), 'EmbeddedFiles')),
      openAction: ['OpenAction', 'AA'].some(key => entry(root, key) !== null),
      tagged: tryRead(() => marked?.isBoolean() === true && marked.asBoolean(), false),
      versions: sections > 1 ? sections - 1 : 0,
      printable: permissions.print,
    };
  }

  return { openDocument, closeDocument, detect };
}
