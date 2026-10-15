// SPDX-License-Identifier: AGPL-3.0-or-later
// Runs the same content edits on engine.ts (MuPDF) and on the Rust app
// engine, saves both, renders every edited page of both saves with MuPDF at
// scale 1 and reports differing pixels and whether the edited page's content
// streams decode to the same text. Usage, from the app root after
// `npm run build` and engine/tools/build-workflow.sh:
//   node engine/tools/compare-app-edit.mjs [summary.json]
import { readFileSync, writeFileSync } from 'node:fs';
import mupdf from 'mupdf';
import { createEngine as createMupdf } from '../../build/renderer/engine.js';
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
function onePage(content, extra = []) {
  return rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Fm1 6 0 R >> >> /Contents 5 0 R >>',
    HELVETICA,
    stream(content),
    ...extra,
  ]);
}
const formPdf = onePage('q 1 0 0 1 72 600 cm /Fm1 Do Q q 1 0 0 1 72 400 cm /Fm1 Do Q\n',
  [stream('BT /F1 14 Tf 0 10 Td (FORM TEXT) Tj ET', '/Type /XObject /Subtype /Form /BBox [0 0 300 100] /Resources << /Font << /F1 4 0 R >> >>')]);

// Each step finds a candidate by its text (or a paragraph by index) in a fresh scan.
const SCENARIOS = [
  { name: 'hello: replace', file: 'fixtures/hello.pdf', steps: [{ op: 'edit-text', find: 'PDF4All', text: 'PDF4' }] },
  { name: 'transformed: replace in a scaled cm', file: 'fixtures/p0b/edit-transformed.pdf', steps: [{ op: 'edit-text', find: 'ALPHA', text: 'BRAV' }] },
  { name: 'transformed: move and size', file: 'fixtures/p0b/edit-transformed.pdf', steps: [{ op: 'edit-text', find: 'NEIGHBOR', move: { dx: 40, dy: 30 }, size: 18 }] },
  { name: 'edit-tj: TJ becomes Tj', file: 'fixtures/edit-tj.pdf', steps: [{ op: 'edit-text', find: null, kerned: true, text: 'Kerned' }] },
  { name: 'subset: Identity-H rewrite', file: 'fixtures/p0b/edit-subset.pdf', steps: [{ op: 'edit-text', cid: true }] },
  { name: 'coded: one-byte coded rewrite', file: 'fixtures/p0b/edit-coded.pdf', steps: [{ op: 'edit-text', cid: true }] },
  { name: 'shared: copy-on-write element', file: 'fixtures/p0b/edit-shared.pdf', steps: [{ op: 'edit-text', cow: true, text: 'Shared' }] },
  { name: 'form: copy-on-write Form', bytes: formPdf, steps: [{ op: 'edit-text', find: 'FORM TEXT', text: 'FORM EDIT' }] },
  { name: 'tagged: ActualText follows', file: 'fixtures/p0b/edit-tagged.pdf', steps: [{ op: 'edit-text', any: true, text: 'Tag' }] },
  { name: 'paragraph: rewrap', file: 'fixtures/p6/paragraph.pdf', steps: [{ op: 'edit-paragraph', paragraph: 0, text: 'A new paragraph that wraps over more than one line of the old width, as typed.' }] },
  { name: 'paragraph: move, size, leading', file: 'fixtures/p6/paragraph.pdf', steps: [{ op: 'edit-paragraph', paragraph: 0, move: { dx: 10, dy: -20 }, size: 10, leading: 13 }] },
  { name: 'quartz: glyph line edit', file: 'fixtures/p6/quartz.pdf', steps: [{ op: 'edit-text', glyphs: true, text: 'Edited line' }] },
  { name: 'quartz: glyph line move', file: 'fixtures/p6/quartz.pdf', steps: [{ op: 'edit-text', glyphs: true, move: { dx: 20, dy: 15 } }] },
  { name: 'text-add', file: 'fixtures/hello.pdf', steps: [{ op: 'text-add', point: [100, 300], text: 'Added line\nSecond line', font: 'Times-Bold', size: 20, color: [0.2, 0.4, 0.6] }] },
  { name: 'replace-apply', file: 'fixtures/p3/edit-replace.pdf', steps: [{ op: 'replace', find: 'a', replace: 'o' }] },
];

function buffer(bytes) {
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

let nextId = 1;
const call = (engine, session, request) => engine.dispatch({ id: nextId++, ...request }, session).reply;

function pick(scan, step) {
  const candidates = scan.items.filter(item => item.kind === 'text' && item.support === 'candidate');
  if (step.glyphs) return candidates.find(item => item.glyphs > 3);
  if (step.cid) return candidates.find(item => item.repertoire);
  if (step.cow) return candidates.find(item => item.cowable);
  if (step.kerned) return candidates.find(item => item.kerned);
  if (step.any) return candidates[0];
  return candidates.find(item => item.text === step.find);
}

/** Runs one scenario on one engine; returns the saved bytes and the pages it edited. */
function run(engine, scenario) {
  const session = engine.createSession();
  const bytes = scenario.bytes ?? readFileSync(scenario.file);
  const opened = call(engine, session, { op: 'open', name: 'x.pdf', pdf: buffer(bytes) });
  if (!opened.ok) return { error: `open: ${opened.error}` };
  const pages = new Set();
  for (const step of scenario.steps) {
    const page = step.page ?? 0;
    pages.add(page);
    const scan = call(engine, session, { op: 'edit-targets', page });
    if (!scan.ok) return { error: `edit-targets: ${scan.error}` };
    const base = { page, generation: scan.generation, revision: scan.revision };
    let reply;
    if (step.op === 'edit-text') {
      const target = pick(scan, step);
      if (!target) return { error: 'no target' };
      const text = step.cid ? [...target.text].reverse().join('') : step.text;
      reply = call(engine, session, { op: 'edit-text', ...base, targetId: target.targetId, text, move: step.move, size: step.size });
    } else if (step.op === 'edit-paragraph') {
      const paragraph = scan.paragraphs[step.paragraph];
      if (!paragraph) return { error: 'no paragraph' };
      const text = step.text ?? paragraph.targetIds.map(id => scan.items.find(item => item.targetId === id).text).join(' ');
      reply = call(engine, session, { op: 'edit-paragraph', ...base, paragraphId: paragraph.paragraphId, text, move: step.move, size: step.size, leading: step.leading });
    } else if (step.op === 'text-add') {
      reply = call(engine, session, { op: 'text-add', page, point: step.point, text: step.text, font: step.font, size: step.size, color: step.color });
    } else if (step.op === 'replace') {
      const preview = call(engine, session, { op: 'edit-replace-preview', page, find: step.find, replace: step.replace });
      if (!preview.ok) return { error: `preview: ${preview.error}` };
      reply = call(engine, session, { op: 'edit-replace-apply', page, find: step.find, replace: step.replace, previewToken: preview.previewToken, targetIds: preview.matches.map(match => match.targetId) });
      reply.matches = preview.matches.length;
    }
    if (!reply.ok) return { error: `${step.op}: ${reply.error} ${reply.detail ?? ''}` };
  }
  const saved = call(engine, session, { op: 'save' });
  if (!saved.ok) return { error: `save: ${saved.error}` };
  return { bytes: new Uint8Array(saved.bytes), pages: [...pages] };
}

function contentText(doc, page) {
  const contents = doc.loadPage(page).getObject().get('Contents');
  const list = contents.isArray() ? Array.from({ length: contents.length }, (_, i) => contents.get(i)) : [contents];
  return list.map(item => Buffer.from(item.readStream().asUint8Array()).toString(LATIN)).join('\n');
}

function pixels(doc, page) {
  const pixmap = doc.loadPage(page).toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false, true);
  return { width: pixmap.getWidth(), height: pixmap.getHeight(), samples: pixmap.getPixels().slice() };
}

const summary = [];
for (const scenario of SCENARIOS) {
  const results = { mupdf: run(engines.mupdf, scenario), rust: run(engines.rust, scenario) };
  const row = { name: scenario.name };
  if (results.mupdf.error || results.rust.error) {
    Object.assign(row, { mupdf: results.mupdf.error ?? 'ok', rust: results.rust.error ?? 'ok' });
    summary.push(row);
    console.log(JSON.stringify(row));
    continue;
  }
  const docs = { mupdf: mupdf.Document.openDocument(results.mupdf.bytes, 'application/pdf'), rust: mupdf.Document.openDocument(results.rust.bytes, 'application/pdf') };
  row.pages = [];
  for (const page of results.mupdf.pages) {
    const [a, b] = [pixels(docs.mupdf, page), pixels(docs.rust, page)];
    let differing = 0, worst = 0;
    if (a.width === b.width && a.height === b.height) {
      for (let i = 0; i < a.samples.length; i += 3) {
        const d = Math.max(Math.abs(a.samples[i] - b.samples[i]), Math.abs(a.samples[i + 1] - b.samples[i + 1]), Math.abs(a.samples[i + 2] - b.samples[i + 2]));
        if (d) differing++;
        worst = Math.max(worst, d);
      }
    } else differing = -1;
    row.pages.push({ page, pixels: a.width * a.height, differing, worst, sameContent: contentText(docs.mupdf, page) === contentText(docs.rust, page) });
  }
  summary.push(row);
  console.log(JSON.stringify(row));
}
if (process.argv[2]) writeFileSync(process.argv[2], `${JSON.stringify(summary, null, 2)}\n`);
