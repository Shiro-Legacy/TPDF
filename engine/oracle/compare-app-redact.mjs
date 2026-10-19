// SPDX-License-Identifier: AGPL-3.0-or-later
// Runs the same redactions (redact-search or redact-add, then redact-apply)
// on engine.ts (MuPDF) and on the Rust app engine and inspects both copies
// with MuPDF alone: the text MuPDF extracts from every page, every object
// of the file (strings, names, decoded streams) for the redacted text, the
// revisions, and MuPDF's drawing of each marked page outside the marks
// (identical pixels between the source and the copy). Usage, from the app
// root after `npm run build`, `npm run build:oracle` and engine/tools/build-workflow.sh:
//   node engine/oracle/compare-app-redact.mjs [summary.json]
import { readFileSync, writeFileSync } from 'node:fs';
import mupdf from 'mupdf';
import { createEngine as createMupdf, REDACT_SANITIZE_KEYS } from '../../build-oracle/engine/oracle/app-engine/engine.js';
import { createEngine as createRust } from '../../tests/rust-engine.mjs';

const engines = { mupdf: createMupdf(mupdf), rust: createRust(mupdf) };
const LATIN = 'latin1';

function rawPdf(objects) {
  const parts = [Buffer.from('%PDF-1.4\n', LATIN)];
  const offsets = [];
  let size = parts[0].length;
  for (const [index, body] of objects.entries()) {
    const bytes = Buffer.from(`${index + 1} 0 obj\n${body}\nendobj\n`, LATIN);
    offsets.push(size);
    parts.push(bytes);
    size += bytes.length;
  }
  let tail = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) tail += `${String(offset).padStart(10, '0')} 00000 n \n`;
  tail += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${size}\n%%EOF\n`;
  parts.push(Buffer.from(tail, LATIN));
  return Buffer.concat(parts);
}
const stream = (body, dict = '') => `<< ${dict} /Length ${Buffer.byteLength(body, LATIN)} >>\nstream\n${body}\nendstream`;
const HELVETICA = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>';
const gray = Buffer.alloc(16 * 16, 180).toString(LATIN);
// Kerned and split text, text in a Form, an image and line art under one mark.
const mixedPdf = rawPdf([
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Fm 6 0 R /Im 7 0 R >> >> /Contents 5 0 R >>',
  HELVETICA,
  stream([
    'BT /F1 14 Tf 72 700 Td [(Account: ) -20 (SEC) 40 (RET-4242) -30 ( due today)] TJ ET',
    'q 1 0 0 1 72 640 cm /Fm Do Q',
    'q 160 0 0 40 72 560 cm /Im Do Q',
    '0 0 1 RG 3 w 60 540 m 400 540 l S 0.9 g 60 500 300 30 re f',
    'BT /F1 12 Tf 72 505 Td (fill note SECRET-4242 end) Tj ET',
  ].join('\n')),
  stream('BT /F1 12 Tf 0 0 Td (form says SECRET-4242 here) Tj ET', '/Type /XObject /Subtype /Form /BBox [0 0 400 20] /Resources << /Font << /F1 4 0 R >> >>'),
  stream(gray, '/Type /XObject /Subtype /Image /Width 16 /Height 16 /ColorSpace /DeviceGray /BitsPerComponent 8'),
]);

const SCENARIOS = [
  { name: 'redact.pdf: search, sanitize all', file: 'fixtures/redact.pdf', search: 'P0B-SECRET-7731', sanitize: true },
  { name: 'redact.pdf: search, sanitize none', file: 'fixtures/redact.pdf', search: 'P0B-SECRET-7731', sanitize: false, contentOnly: true },
  { name: 'direct3: area mark over Alpha', file: 'fixtures/p6/direct3.pdf', marks: [[0, [70, 80, 100, 96]]], secret: 'Alpha', sanitize: true },
  { name: 'mixed: kerned, Form, image, line art', bytes: mixedPdf, search: 'SECRET-4242', marks: [[0, [60, 150, 300, 240]]], sanitize: true },
  { name: 'quartz: glyph lines, search', file: 'fixtures/p6/quartz.pdf', search: 'owner', sanitize: true },
  { name: 'paragraph: search', file: 'fixtures/p6/paragraph.pdf', search: 'paragraph', sanitize: true },
  { name: 'large: search across 500 pages', file: 'fixtures/large.pdf', search: 'the', sanitize: true },
  { name: 'nav: area mark, outlines kept', file: 'fixtures/nav.pdf', marks: [[0, [40, 40, 140, 90]]], sanitize: true, keep: ['outlines'] },
  { name: 'cjk: search', file: 'fixtures/cjk.pdf', search: '日本', sanitize: true },
  { name: 'rotated: search a turned word', file: 'fixtures/rotated.pdf', search: 'rotated', sanitize: true },
  { name: 'encrypted aes-256: user unlock, area mark', file: 'fixtures/encrypted-aes256.pdf', password: 'pdf4all-user', marks: [[0, [30, 30, 300, 120]]], sanitize: true },
  // Redaction redesign (plan §2.5): boxes sent as redact-apply `regions` to the Rust engine
  // (engine.ts gets the same boxes as redact-add marks). One box over the whole of page 1:
  // the Rust engine changes what the box covers in place (fourth review), so the Form page 2
  // also draws loses the secret there too and the reply says `overRedacted`; the secret is in
  // no object of the copy. engine.ts refuses (shared-content).
  { name: 'redact.pdf: one box over page 1 (regions)', file: 'fixtures/redact.pdf', regions: [[0, [0, 0, 612, 792]]], secret: 'P0B-SECRET-7731', sanitize: true },
  { name: 'redact.pdf: one box over each page (regions)', file: 'fixtures/redact.pdf', regions: [[0, [0, 0, 612, 792]], [1, [0, 0, 612, 792]]], secret: 'P0B-SECRET-7731', sanitize: true },
];

const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
let nextId = 1;
const call = (engine, session, request) => engine.dispatch({ id: nextId++, ...request }, session).reply;

function run(engine, scenario, engineName) {
  const session = engine.createSession();
  const bytes = scenario.bytes ?? readFileSync(scenario.file);
  const opened = call(engine, session, { op: 'open', name: 'x.pdf', pdf: buffer(bytes) });
  if (!opened.ok) return { error: `open: ${opened.error}` };
  if (scenario.password) {
    const auth = call(engine, session, { op: 'authenticate', password: scenario.password });
    if (!auth.ok) return { error: `authenticate: ${auth.error}` };
  }
  for (const [page, rect] of scenario.marks ?? []) {
    const added = call(engine, session, { op: 'redact-add', page, rect });
    if (!added.ok) return { error: `redact-add: ${added.error}` };
  }
  if (scenario.search) {
    const found = call(engine, session, { op: 'redact-search', text: scenario.search });
    if (!found.ok) return { error: `redact-search: ${found.error}` };
  }
  const marks = [];
  for (let page = 0; page < opened.pageCount; page++) {
    const listed = call(engine, session, { op: 'annots', page });
    for (const item of listed.items ?? []) if (item.type === 'Redact') marks.push({ page, rect: item.rect });
  }
  const regions = (scenario.regions ?? []).map(([page, rect]) => ({ page, rect }));
  if (engineName !== 'rust') {
    for (const { page, rect } of regions) {
      const added = call(engine, session, { op: 'redact-add', page, rect });
      if (!added.ok) return { error: `redact-add: ${added.error}` };
    }
  }
  if (engineName === 'rust') marks.push(...regions);
  const sanitize = Object.fromEntries(REDACT_SANITIZE_KEYS.map(key => [key, scenario.sanitize && !(scenario.keep ?? []).includes(key)]));
  const request = { op: 'redact-apply', sanitize };
  if (engineName === 'rust' && regions.length) request.regions = regions;
  const applied = call(engine, session, request);
  if (!applied.ok) return { error: `redact-apply: ${applied.reason} ${applied.error}`, marks };
  return { bytes: new Uint8Array(applied.bytes), reply: applied, marks, source: bytes };
}

function open(bytes, password) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  if (doc.needsPassword()) doc.authenticatePassword(password ?? '');
  return doc;
}

function allText(doc) {
  const pages = [];
  for (let page = 0; page < doc.countPages(); page++) pages.push(doc.loadPage(page).toStructuredText().asText());
  return pages.join('\n');
}

/** Every object of the file: strings and names, and streams decoded. */
function holdsAnywhere(doc, needle) {
  const pdf = doc.asPDF();
  const wanted = Buffer.from(needle, 'utf8');
  const wide = Buffer.from([0xfe, 0xff, ...Buffer.from(needle, 'utf16le').swap16()]);
  let found = false;
  const seen = new Set();
  const visit = object => {
    if (found || !object || object.isNull()) return;
    if (object.isIndirect()) {
      const number = object.asIndirect();
      if (seen.has(number)) return;
      seen.add(number);
    }
    if (object.isString()) {
      const text = object.asString() ?? '';
      const bytes = Buffer.from(text, LATIN);
      if (bytes.includes(wanted) || bytes.includes(wide) || text.includes(needle)) found = true;
      return;
    }
    if (object.isName()) {
      if (object.asName().includes(needle)) found = true;
      return;
    }
    if (object.isStream()) {
      try {
        const data = Buffer.from(object.readStream().asUint8Array());
        if (data.includes(wanted) || data.includes(wide)) found = true;
      } catch {}
    }
    if (object.isArray() || object.isDictionary() || object.isStream()) object.forEach(value => visit(value));
  };
  const count = pdf.countObjects();
  for (let number = 1; number < count && !found; number++) visit(pdf.newIndirect(number));
  visit(pdf.getTrailer());
  return found;
}

/** How many indirect objects themselves hold `needle` (their strings and names, references not
 * followed, and their decoded stream). */
function holders(doc, needle) {
  const pdf = doc.asPDF();
  const wanted = Buffer.from(needle, 'utf8');
  const wide = Buffer.from([0xfe, 0xff, ...Buffer.from(needle, 'utf16le').swap16()]);
  const inside = (object, top) => {
    if (!object || object.isNull()) return false;
    if (!top && object.isIndirect()) return false;
    if (object.isString()) {
      const text = object.asString() ?? '';
      const bytes = Buffer.from(text, LATIN);
      return bytes.includes(wanted) || bytes.includes(wide) || text.includes(needle);
    }
    if (object.isName()) return object.asName().includes(needle);
    let found = false;
    if (object.isStream()) {
      try {
        const data = Buffer.from(object.readStream().asUint8Array());
        found = data.includes(wanted) || data.includes(wide);
      } catch {}
    }
    if (!found && (object.isArray() || object.isDictionary() || object.isStream())) {
      object.forEach(value => { if (!found && inside(value, false)) found = true; });
    }
    return found;
  };
  let count = 0;
  for (let number = 1; number < pdf.countObjects(); number++) if (inside(pdf.newIndirect(number), true)) count++;
  return count;
}

function pixels(doc, page) {
  const pixmap = doc.loadPage(page).toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false, true);
  return { width: pixmap.getWidth(), height: pixmap.getHeight(), samples: pixmap.getPixels().slice() };
}

/** Pixels of `copy` that differ from `source` outside the marks grown by 3 points. */
function changedOutside(source, copy, marks, page) {
  const [a, b] = [pixels(source, page), pixels(copy, page)];
  if (a.width !== b.width || a.height !== b.height) return -1;
  let changed = 0;
  for (let y = 0; y < a.height; y++) {
    for (let x = 0; x < a.width; x++) {
      const inside = marks.some(m => m.page === page && x + 0.5 > m.rect[0] - 3 && x + 0.5 < m.rect[2] + 3 && y + 0.5 > m.rect[1] - 3 && y + 0.5 < m.rect[3] + 3);
      if (inside) continue;
      const at = (y * a.width + x) * 3;
      if (a.samples[at] !== b.samples[at] || a.samples[at + 1] !== b.samples[at + 1] || a.samples[at + 2] !== b.samples[at + 2]) changed++;
    }
  }
  return changed;
}

const summary = [];
for (const scenario of SCENARIOS) {
  const row = { name: scenario.name };
  const secret = scenario.secret ?? scenario.search;
  for (const [name, engine] of Object.entries(engines)) {
    const result = run(engine, scenario, name);
    if (result.error) {
      row[name] = { error: result.error };
      continue;
    }
    const copy = open(result.bytes, scenario.password);
    const source = open(result.source, scenario.password);
    const text = allText(copy);
    const marked = [...new Set(result.marks.map(m => m.page))];
    row[name] = {
      marks: result.reply.marks,
      verified: result.reply.verified,
      overRedacted: result.reply.overRedacted,
      kept: result.reply.kept,
      bytes: result.bytes.byteLength,
      revisions: copy.asPDF().countVersions(),
      repaired: copy.asPDF().wasRepaired(),
      encrypted: copy.needsPassword(),
      textHoldsSecret: secret ? text.includes(secret) : null,
      fileHoldsSecret: scenario.contentOnly || !secret ? null : holdsAnywhere(copy, secret),
      page1TextHoldsSecret: secret ? copy.loadPage(0).toStructuredText().asText().includes(secret) : null,
      objectsHoldingSecret: scenario.regions && secret ? holders(copy, secret) : undefined,
      words: text.split(/\s+/).filter(Boolean).length,
      changedOutsideMarks: marked.slice(0, 6).map(page => changedOutside(source, copy, result.marks, page)),
    };
  }
  summary.push(row);
  console.log(JSON.stringify(row));
}
if (process.argv[2]) writeFileSync(process.argv[2], `${JSON.stringify(summary, null, 2)}\n`);
