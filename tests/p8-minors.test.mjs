/**
 * Acceptance review round 3, minor findings on accepted ops: app-level checks
 * that pass on both engines. Runs against src/renderer/engine.ts as the worker
 * does; tests/run-rust.mjs runs it against the Rust engine
 * (engine/design/app-accept-forms.md and the other notes' "Round 3 minors").
 */
import assert from 'node:assert/strict';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';

const engine = createEngine(mupdf);
let nextId = 1;

function op(session, request) {
  return engine.dispatch({ id: nextId++, ...request }, session).reply;
}

/** A classic-xref PDF from object bodies numbered from 1, as an ArrayBuffer. */
function classic(objects) {
  let text = '%PDF-1.7\n';
  const offsets = [];
  for (const [index, body] of objects.entries()) {
    offsets.push(text.length);
    text += `${index + 1} 0 obj\n${body}\nendobj\n`;
  }
  const xref = text.length;
  text += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) text += `${String(offset).padStart(10, '0')} 00000 n \n`;
  text += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  const bytes = Uint8Array.from(text, c => c.charCodeAt(0));
  return bytes.buffer;
}

function stream(dictionary, data) {
  return `<< ${dictionary} /Length ${data.length} >>\nstream\n${data}\nendstream`;
}

function openPdf(objects) {
  const session = engine.createSession();
  const reply = op(session, { op: 'open', name: 'minors.pdf', pdf: classic(objects) });
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function fields(session, page = 0) {
  const reply = op(session, { op: 'fields', page });
  assert.equal(reply.ok, true, reply.error);
  return reply.items;
}

function saved(session) {
  const reply = op(session, { op: 'save' });
  assert.equal(reply.ok, true, reply.error);
  return reply.bytes;
}

function reopen(bytes) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  doc.asPDF().disableJS();
  return doc.asPDF();
}

/** The /AS of each object `numbers`, read back from saved bytes. */
function states(bytes, numbers) {
  const pdf = reopen(bytes);
  return numbers.map(n => {
    const as = pdf.newIndirect(n, 0).resolve().get('AS');
    return as.isNull() ? null : as.asName();
  });
}

// ------------------------------------------------------------------- forms

const appearance = stream('/Type /XObject /Subtype /Form /BBox [0 0 16 16]', 'q Q');

test('a radio kid without /AP stays off when a kid with /AP states is chosen', () => {
  // Kid 6 has no /AP; kids 7 and 8 have /B and /C states (forms round 1's
  // radio fix, which had native tests only).
  const kid = (extra, y) => `<< /Type /Annot /Subtype /Widget /Parent 5 0 R /P 3 0 R /Rect [72 ${y} 88 ${y + 16}] `
    + `/MK << /BC [0] /BG [1] >> ${extra} >>`;
  const session = openPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 7 0 R 8 0 R] >>',
    '<< /Fields [5 0 R] /DA (/ZaDb 0 Tf 0 g) >>',
    '<< /FT /Btn /Ff 49152 /T (choice) /V /Off /Kids [6 0 R 7 0 R 8 0 R] >>',
    kid('/AS /Off', 700),
    kid('/AS /Off /AP << /N << /B 9 0 R /Off 9 0 R >> >>', 670),
    kid('/AS /Off /AP << /N << /C 9 0 R /Off 9 0 R >> >>', 640),
    appearance,
  ]);
  const kids = fields(session);
  assert.deepEqual(kids.map(item => item.kind), ['radio', 'radio', 'radio']);
  assert.equal(op(session, { op: 'field-set', page: 0, widgetId: kids[1].id, value: true }).ok, true);
  assert.deepEqual(fields(session).map(item => item.value), [false, true, false]);
  assert.deepEqual(states(saved(session), [6, 7, 8]), ['Off', 'B', 'Off']);
  assert.equal(op(session, { op: 'field-set', page: 0, widgetId: kids[2].id, value: true }).ok, true);
  assert.deepEqual(fields(session).map(item => item.value), [false, false, true]);
  assert.deepEqual(states(saved(session), [6, 7, 8]), ['Off', 'Off', 'C']);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual(fields(session).map(item => item.value), [false, true, false]);
});

test('a radio kid without /AP on another page is turned off by its group', () => {
  const session = openPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [7 0 R] >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [8 0 R] >>',
    '<< /Fields [6 0 R] /DA (/ZaDb 0 Tf 0 g) >>',
    '<< /FT /Btn /Ff 49152 /T (r) /V /A /Kids [7 0 R 8 0 R] >>',
    '<< /Type /Annot /Subtype /Widget /Parent 6 0 R /P 3 0 R /Rect [72 600 88 616] /AS /A >>',
    '<< /Type /Annot /Subtype /Widget /Parent 6 0 R /P 4 0 R /Rect [72 600 88 616] /AS /Off '
      + '/AP << /N << /B 9 0 R /Off 9 0 R >> >> >>',
    appearance,
  ]);
  assert.deepEqual(fields(session, 0).map(item => item.value), [true]);
  const other = fields(session, 1)[0];
  assert.equal(op(session, { op: 'field-set', page: 1, widgetId: other.id, value: true }).ok, true);
  assert.deepEqual(fields(session, 0).map(item => item.value), [false]);
  assert.deepEqual(fields(session, 1).map(item => item.value), [true]);
  assert.deepEqual(states(saved(session), [7, 8]), ['Off', 'B']);
});

test('a Polish, Czech or Romanian fill stores an appearance and no /NeedAppearances', () => {
  for (const value of ['Zażółć gęślą jaźń', 'Příliš žluťoučký', 'Țară și']) {
    const session = openPdf([
      '<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>',
      '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
      '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [6 0 R] >>',
      stream('', 'q Q'),
      '<< /Fields [6 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 7 0 R >> >> >>',
      '<< /Type /Annot /Subtype /Widget /F 4 /P 3 0 R /FT /Tx /T (t) /V () /Rect [72 700 252 724] '
        + '/DA (/Helv 12 Tf 0 g) >>',
      '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    ]);
    const widget = fields(session)[0];
    assert.equal(op(session, { op: 'field-set', page: 0, widgetId: widget.id, value }).ok, true);
    assert.equal(fields(session)[0].value, value);
    const pdf = reopen(saved(session));
    const acro = pdf.getTrailer().get('Root').get('AcroForm');
    assert.ok(acro.get('NeedAppearances').isNull(), `${value}: no document-wide redraw flag`);
    const normal = pdf.newIndirect(6, 0).resolve().get('AP').get('N');
    assert.ok(normal.isStream(), `${value}: the widget keeps a stored appearance`);
  }
});

// -------------------------------------------------------------------- edit

test('an empty /BaseFont lists its font name as null', () => {
  const session = openPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
    stream('', 'BT /F1 24 Tf 72 700 Td (Hello) Tj ET'),
    '<< /Type /Font /Subtype /TrueType /BaseFont / /Encoding /WinAnsiEncoding >>',
  ]);
  const reply = op(session, { op: 'edit-targets', page: 0 });
  assert.equal(reply.ok, true, reply.error);
  const item = reply.items.find(entry => entry.text === 'Hello');
  assert.ok(item, JSON.stringify(reply.items));
  assert.equal(item.font.name, null);
  assert.equal(item.font.subtype, 'TrueType');
});
