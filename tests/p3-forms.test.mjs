/**
 * P3 U6 — bounded form designer: create / rename / move / properties / delete
 * text, checkbox, dropdown and radio fields, fill them in the same session, and
 * prove refusals never dirty the document.
 *
 * Runs against src/renderer/engine.ts exactly as the worker does. Every rect the
 * engine reports is `getRect()` in page points, y down from the page top; the
 * contract's save-side checks are re-run independently with pypdf (grok's
 * `scripts/verify-roundtrip.py`) and are recorded in the unit evidence.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { performance } from 'node:perf_hooks';
import test from 'node:test';
import mupdf from 'mupdf';
import {
  createEngine, MIN_FIELD_RECT_PT, MAX_FIELD_KIDS, MAX_FIELD_LABEL, MAX_FIELD_NAME_CHARS,
} from '../build/renderer/engine.js';

const engine = createEngine(mupdf);

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

async function openBytes(bytes, name = 'scratch.pdf', session = engine.createSession()) {
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: bytes }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

async function open(id, session = engine.createSession()) {
  return openBytes(toArrayBuffer(await readFile(`fixtures/${id}.pdf`)), `${id}.pdf`, session);
}

function op(session, request) {
  return engine.dispatch({ id: 900 + (request.id ?? 0), ...request }, session).reply;
}

function byName(session, name, page = 0) {
  return fields(session, page).filter(item => item.name === name);
}

function fields(session, page = 0) {
  const reply = op(session, { op: 'fields', page });
  assert.equal(reply.ok, true, reply.error);
  return reply.items;
}

function set(session, widgetId, value, page = 0) {
  return op(session, { op: 'field-set', page, widgetId, value });
}

function add(session, spec, page = 0) {
  return op(session, { op: 'field-add', page, ...spec });
}

function props(session, spec, page = 0) {
  return op(session, { op: 'field-props', page, ...spec });
}

function remove(session, widgetId, page = 0) {
  return op(session, { op: 'field-delete', page, widgetId });
}

function saved(session) {
  const reply = op(session, { op: 'save' });
  assert.equal(reply.ok, true, reply.error);
  return reply.bytes;
}

function reopen(bytes) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  doc.asPDF().disableJS();
  return doc;
}

/** The raw `/Fields` and page-1 `/Annots` counts plus the widget list, from the bytes. */
function rawFacts(bytes, page = 0) {
  const doc = reopen(bytes);
  const pdf = doc.asPDF();
  const root = pdf.getTrailer().get('Root');
  const acro = root.get('AcroForm');
  const fieldArray = acro && !acro.isNull() ? acro.get('Fields') : null;
  const annots = pdf.findPage(page).get('Annots');
  const loaded = doc.loadPage(page);
  let widgetCount = 0;
  let names = [];
  try {
    widgetCount = loaded.getWidgets().length;
    names = loaded.getWidgets().map(widget => widget.getName());
  } finally {
    loaded.destroy();
  }
  return {
    hasAcroForm: !!acro && !acro.isNull(),
    fieldCount: fieldArray && fieldArray.isArray() ? fieldArray.length : 0,
    annotCount: annots && annots.isArray() ? annots.length : 0,
    widgetCount,
    names,
  };
}

/** A field dict's `/AP /N` facts, from the bytes: a stream or a state dictionary. */
function appearanceFacts(bytes, name, page = 0) {
  const doc = reopen(bytes);
  const loaded = doc.loadPage(page);
  try {
    const widget = loaded.getWidgets().find(item => item.getName() === name);
    if (!widget) return null;
    const dict = widget.getObject();
    const ap = dict.get('AP');
    const normal = ap && !ap.isNull() ? ap.get('N') : null;
    if (!normal || normal.isNull()) return { kind: 'none' };
    if (normal.isStream()) {
      const bbox = normal.get('BBox');
      return { kind: 'stream', bbox: bbox && !bbox.isNull() ? [...Array(4)].map((_x, i) => bbox.get(i).asNumber()) : null };
    }
    if (normal.isDictionary()) {
      const states = [];
      normal.forEach((_value, key) => states.push(String(key)));
      return { kind: 'states', states: states.sort() };
    }
    return { kind: 'other' };
  } finally {
    loaded.destroy();
  }
}

/** Non-white pixels inside a y-down page rect, at 1 pt = 1 px, from the saved bytes. */
function darkPixels(bytes, rect, page = 0) {
  const doc = reopen(bytes);
  const loaded = doc.loadPage(page);
  try {
    const pixmap = loaded.toPixmap(mupdf.Matrix.scale(1, 1), mupdf.ColorSpace.DeviceRGB, false);
    try {
      const width = pixmap.getWidth();
      const height = pixmap.getHeight();
      const components = pixmap.getNumberOfComponents();
      const stride = pixmap.getStride();
      const pixels = pixmap.getPixels();
      const x0 = Math.max(0, Math.floor(rect[0]));
      const y0 = Math.max(0, Math.floor(rect[1]));
      const x1 = Math.min(width, Math.ceil(rect[2]));
      const y1 = Math.min(height, Math.ceil(rect[3]));
      let dark = 0;
      for (let y = y0; y < y1; y++) {
        for (let x = x0; x < x1; x++) {
          const at = y * stride + x * components;
          if (pixels[at] + pixels[at + 1] + pixels[at + 2] < 600) dark++;
        }
      }
      return dark;
    } finally {
      pixmap.destroy();
    }
  } finally {
    loaded.destroy();
  }
}

/** Object-numbering-safe PDF from a 1-based list of indirect object bodies. */
function buildPdf(objects) {
  const parts = ['%PDF-1.4\n'];
  const offsets = [];
  objects.forEach((body, index) => {
    offsets.push(parts.join('').length);
    parts.push(`${index + 1} 0 obj\n${body}\nendobj\n`);
  });
  const xref = parts.join('').length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const offset of offsets) parts.push(`${String(offset).padStart(10, '0')} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  return toArrayBuffer(Buffer.from(parts.join(''), 'latin1'));
}

/** A widget whose `/Parent` is a plain field (not a radio group): an existing hierarchy. */
function hierarchyPdf() {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [7 0 R] >>',
    '<< /Length 2 >>\nstream\nq \nendstream',
    '<< /Fields [6 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 8 0 R >> >> >>',
    '<< /T (outer) /Kids [7 0 R] >>',
    '<< /Type /Annot /Subtype /Widget /FT /Tx /T (inner) /Parent 6 0 R /Rect [72 650 252 680] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
  ]);
}

// ------------------------------------------------------------ create, list, fill

test('text: add on a blank page lists it with /AP, fill works in the same session, save keeps it', async () => {
  const session = await open('hello');
  // Below the fixture's own heading, so the pixel counts are the field's alone.
  const rect = [300, 500, 480, 524];
  const added = add(session, { kind: 'text', name: 'n1', rect });
  assert.equal(added.ok, true, added.error);
  assert.equal(typeof added.widgetId, 'number');

  const listed = byName(session, 'n1');
  assert.equal(listed.length, 1);
  assert.equal(listed[0].kind, 'text');
  assert.deepEqual(listed[0].rect, rect);
  assert.equal(listed[0].value, '');

  // The gate: an authored field gets an /AP /N at create time, before any fill.
  const bytesBefore = saved(session);
  assert.deepEqual(appearanceFacts(bytesBefore, 'n1'), { kind: 'stream', bbox: [0, 0, 180, 24] });
  const empty = darkPixels(bytesBefore, rect);
  // review U6: /MK /BC gives an empty field a visible box, so it can be found before a fill.
  assert.ok(empty > 0, 'an empty authored field draws its box');

  // §7.7: field-set on the authored field grows the dark pixels inside its rect.
  assert.equal(set(session, listed[0].id, 'abc').ok, true);
  assert.equal(byName(session, 'n1')[0].value, 'abc');
  const bytesAfter = saved(session);
  assert.ok(darkPixels(bytesAfter, rect) > empty + 20, `the filled text is drawn (${empty} -> ${darkPixels(bytesAfter, rect)})`);

  const raw = rawFacts(bytesAfter);
  assert.equal(raw.fieldCount, 1);
  assert.equal(raw.annotCount, 1);
  assert.deepEqual(raw.names, ['n1']);
});

test('checkbox: explicit /AP states at create, fill true, resize keeps the state dictionary', async () => {
  const session = await open('hello');
  const rect = [72, 110, 88, 126];
  assert.equal(add(session, { kind: 'checkbox', name: 'c1', rect }).ok, true);
  const before = appearanceFacts(saved(session), 'c1');
  assert.deepEqual(before, { kind: 'states', states: ['Off', 'Yes'] });

  const item = byName(session, 'c1')[0];
  assert.equal(item.on, 'Yes');
  assert.equal(item.value, false);
  assert.equal(set(session, item.id, true).ok, true);
  assert.equal(byName(session, 'c1')[0].value, true);

  const moved = props(session, { widgetId: item.id, rect: [200, 110, 232, 142] });
  assert.equal(moved.ok, true, moved.error);
  assert.deepEqual(byName(session, 'c1')[0].rect, [200, 110, 232, 142]);
  assert.deepEqual(appearanceFacts(saved(session), 'c1'), { kind: 'states', states: ['Off', 'Yes'] });
});

test('dropdown: options land in /Opt, /V starts at the first option, fill picks another', async () => {
  const session = await open('hello');
  const rect = [72, 140, 252, 164];
  assert.equal(add(session, { kind: 'combo', name: 'd1', options: ['one', 'two', 'three'], rect }).ok, true);
  const item = byName(session, 'd1')[0];
  assert.equal(item.kind, 'combo');
  assert.deepEqual(item.options, ['one', 'two', 'three']);
  assert.equal(item.value, 'one');
  assert.equal(set(session, item.id, 'two').ok, true);
  assert.equal(byName(session, 'd1')[0].value, 'two');

  // §7.8: options without the current /V reset it to the first option.
  const changed = props(session, { widgetId: item.id, options: ['alpha', 'beta'] });
  assert.equal(changed.ok, true, changed.error);
  const after = byName(session, 'd1')[0];
  assert.deepEqual(after.options, ['alpha', 'beta']);
  assert.equal(after.value, 'alpha');
});

test('radio: kids share the name, are exclusive before and after save, and are deleted one by one', async () => {
  const session = await open('hello');
  const kids = [
    { on: 'A', rect: [72, 180, 88, 196] },
    { on: 'B', rect: [110, 180, 126, 196] },
  ];
  const added = add(session, { kind: 'radio', name: 'g', kids });
  assert.equal(added.ok, true, added.error);
  assert.deepEqual(added.widgetIds, byName(session, 'g').map(item => item.id));

  const items = byName(session, 'g');
  assert.equal(items.length, 2);
  assert.deepEqual(items.map(item => item.on), ['A', 'B']);
  assert.deepEqual(items.map(item => item.rect), [kids[0].rect, kids[1].rect]);

  const kidB = items.find(item => item.on === 'B');
  assert.equal(set(session, kidB.id, true).ok, true);
  assert.deepEqual(byName(session, 'g').map(item => item.value), [false, true]);

  const afterSave = reopen(saved(session));
  const loaded = afterSave.loadPage(0);
  try {
    const widgets = loaded.getWidgets();
    assert.deepEqual(widgets.map(widget => widget.getName()), ['g', 'g']);
    assert.deepEqual(widgets.map(widget => widget.getObject().get('AS').asName()), ['Off', 'B']);
  } finally {
    loaded.destroy();
  }

  // Deleting a kid leaves the group; deleting the last one takes the group from /Fields.
  assert.equal(remove(session, kidB.id).ok, true);
  assert.equal(byName(session, 'g').length, 1);
  const remaining = byName(session, 'g')[0];
  assert.equal(remove(session, remaining.id).ok, true);
  assert.equal(byName(session, 'g').length, 0);
  const raw = rawFacts(saved(session));
  assert.equal(raw.fieldCount, 0);
  assert.equal(raw.annotCount, 0);
  assert.equal(raw.hasAcroForm, true, '/AcroForm stays even when /Fields empties (§7.9)');
});

// ------------------------------------------------------------ properties

test('rename and move: /T and /Rect change, the reported rect is the requested one, kids re-key', async () => {
  const session = await open('hello');
  assert.equal(add(session, { kind: 'text', name: 'a', rect: [72, 72, 252, 96] }).ok, true);
  const item = byName(session, 'a')[0];
  const renamed = props(session, { widgetId: item.id, name: 'b' });
  assert.equal(renamed.ok, true, renamed.error);
  assert.equal(byName(session, 'b').length, 1);
  assert.equal(byName(session, 'a').length, 0);

  const moved = props(session, { widgetId: byName(session, 'b')[0].id, rect: [100, 200, 280, 260] });
  assert.equal(moved.ok, true, moved.error);
  assert.deepEqual(byName(session, 'b')[0].rect, [100, 200, 280, 260]);

  // A radio group rename re-keys every kid in the reported list.
  assert.equal(add(session, { kind: 'radio', name: 'g', kids: [
    { on: 'A', rect: [72, 300, 88, 316] }, { on: 'B', rect: [110, 300, 126, 316] },
  ] }).ok, true);
  const group = props(session, { widgetId: byName(session, 'g')[0].id, name: 'h' });
  assert.equal(group.ok, true, group.error);
  assert.equal(byName(session, 'g').length, 0);
  assert.deepEqual(byName(session, 'h').map(item => item.on), ['A', 'B']);
});

test('text properties: maxLen and multiline round-trip, and maxLen below the value is refused', async () => {
  const session = await open('hello');
  assert.equal(add(session, { kind: 'text', name: 't', rect: [72, 72, 252, 96], maxLen: 10 }).ok, true);
  const item = byName(session, 't')[0];
  assert.equal(item.maxLen, 10);

  const changed = props(session, { widgetId: item.id, multiline: true });
  assert.equal(changed.ok, true, changed.error);
  assert.equal(byName(session, 't')[0].multiline, true);

  assert.equal(set(session, item.id, 'abcdef').ok, true);
  const tooSmall = props(session, { widgetId: item.id, maxLen: 3 });
  assert.equal(tooSmall.ok, false);
  assert.equal(tooSmall.detail, 'invalid');
  assert.equal(byName(session, 't')[0].value, 'abcdef', 'the value is never truncated');
});

// ------------------------------------------------------------ undo/redo

test('every designer op is one undo step: /Fields, /Annots and widgets all come back', async () => {
  const session = await open('hello');
  const baseline = rawFacts(saved(session));
  const added = add(session, { kind: 'text', name: 'u1', rect: [72, 72, 252, 96] });
  assert.equal(added.ok, true, added.error);
  const withField = rawFacts(saved(session));
  assert.equal(withField.fieldCount, baseline.fieldCount + 1);
  assert.equal(withField.annotCount, baseline.annotCount + 1);

  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual(rawFacts(saved(session)), baseline);

  assert.equal(op(session, { op: 'redo' }).ok, true);
  assert.deepEqual(rawFacts(saved(session)), withField);

  // A props step and a delete step each undo on their own.
  const item = byName(session, 'u1')[0];
  assert.equal(props(session, { widgetId: item.id, rect: [200, 200, 380, 260] }).ok, true);
  assert.deepEqual(byName(session, 'u1')[0].rect, [200, 200, 380, 260]);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual(byName(session, 'u1')[0].rect, [72, 72, 252, 96]);

  assert.equal(remove(session, byName(session, 'u1')[0].id).ok, true);
  assert.equal(byName(session, 'u1').length, 0);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(byName(session, 'u1').length, 1);
  assert.deepEqual(rawFacts(saved(session)), withField, 'undo restores both arrays exactly');
});

// ------------------------------------------------------------ refusals, never dirty

test('refusals never dirty: duplicate and invalid names, XFA, scripted, signature', async () => {
  const session = await open('hello');
  assert.equal(add(session, { kind: 'text', name: 'dup', rect: [72, 72, 252, 96] }).ok, true);
  const clean = rawFacts(saved(session));

  const duplicate = add(session, { kind: 'text', name: 'dup', rect: [72, 110, 252, 134] });
  assert.equal(duplicate.ok, false);
  assert.equal(duplicate.detail, 'duplicate-name');

  for (const name of ['a.b', ' lead', 'trail ', 'x'.repeat(MAX_FIELD_NAME_CHARS + 1)]) {
    const bad = add(session, { kind: 'text', name, rect: [72, 110, 252, 134] });
    assert.equal(bad.ok, false, `${JSON.stringify(name)} must be refused`);
    assert.equal(bad.detail, 'invalid-name');
  }

  const tiny = add(session, { kind: 'text', name: 'small', rect: [72, 110, 72 + MIN_FIELD_RECT_PT - 1, 134] });
  assert.equal(tiny.ok, false);
  assert.equal(tiny.detail, 'invalid');
  const tooManyKids = add(session, { kind: 'radio', name: 'many', kids: Array.from({ length: MAX_FIELD_KIDS + 1 }, (_v, i) => ({ on: `k${i}`, rect: [72 + i, 400, 88 + i, 416] })) });
  assert.equal(tooManyKids.ok, false);
  assert.equal(tooManyKids.detail, 'invalid');

  assert.deepEqual(rawFacts(saved(session)), clean, 'every refusal left the document alone');

  const xfa = await open('p3/form-xfa');
  const xfaAdd = add(xfa, { kind: 'text', name: 'n9', rect: [360, 650, 540, 680] });
  assert.equal(xfaAdd.ok, false);
  assert.equal(xfaAdd.detail, 'xfa');

  const form2 = await open('form2');
  const amount = byName(form2, 'amount')[0];
  const scripted = props(form2, { widgetId: amount.id, name: 'total' });
  assert.equal(scripted.ok, false);
  assert.equal(scripted.detail, 'scripted');
  const sig = byName(form2, 'sig')[0];
  assert.equal(sig.kind, 'signature');
  // P5 U3 §9.1: an EMPTY signature field is deleted like any field, and undo puts it back.
  assert.equal(remove(form2, sig.id).ok, true);
  assert.equal(byName(form2, 'sig').length, 0);
  assert.equal(op(form2, { op: 'undo' }).ok, true);
  assert.equal(byName(form2, 'sig')[0]?.id, sig.id);
  // An existing, non-scripted field of a normal form may still be renamed.
  const notes = byName(form2, 'notes')[0];
  assert.equal(props(form2, { widgetId: notes.id, name: 'notes2' }).ok, true);
});

test('a widget inside an existing hierarchy cannot be renamed, moved or deleted', async () => {
  const session = await openBytes(hierarchyPdf(), 'hierarchy.pdf');
  const inner = byName(session, 'outer.inner')[0];
  assert.ok(inner, 'the nested widget lists under its dotted name');
  for (const reply of [
    props(session, { widgetId: inner.id, name: 'renamed' }),
    props(session, { widgetId: inner.id, rect: [72, 400, 252, 430] }),
    remove(session, inner.id),
  ]) {
    assert.equal(reply.ok, false, JSON.stringify(reply));
    assert.equal(reply.detail, 'invalid');
  }
});

test('permissions: authoring needs edit *and* annotate while filling still needs only form', async () => {
  const session = await open('p3/encrypted-edit-noannot');
  assert.equal(op(session, { op: 'authenticate', password: 'pdf4all-user' }).ok, true);
  const fillable = byName(session, 'fillable')[0];
  assert.ok(fillable, 'the encrypted fixture lists its field by name');
  assert.equal(fillable.kind, 'text');
  assert.equal(set(session, fillable.id, 'still fills').ok, true, 'bit 9 fill keeps working');
  const refused = add(session, { kind: 'text', name: 'n1', rect: [72, 72, 252, 96] });
  assert.equal(refused.ok, false);
  assert.equal(refused.reason, 'read-only');
});

test('signed and certified documents refuse authoring before any COS write', async () => {
  const certified = await openBytes(toArrayBuffer(await readFile('fixtures/p0b/certified-p2.pdf')), 'certified.pdf');
  const refused = add(certified, { kind: 'text', name: 'n1', rect: [72, 72, 252, 96] });
  assert.equal(refused.ok, false);
  assert.equal(refused.reason, 'read-only');
});

// ------------------------------------------------------------ saves stay script-free

test('a saved designer document gains no /AA, /JS, /CO or /NeedAppearances', async () => {
  const session = await open('hello');
  assert.equal(add(session, { kind: 'text', name: 'n1', rect: [72, 72, 252, 96] }).ok, true);
  assert.equal(add(session, { kind: 'checkbox', name: 'c1', rect: [72, 110, 88, 126] }).ok, true);
  assert.equal(add(session, { kind: 'combo', name: 'd1', options: ['one', 'two'], rect: [72, 140, 252, 164] }).ok, true);
  assert.equal(add(session, { kind: 'radio', name: 'g', kids: [
    { on: 'A', rect: [72, 180, 88, 196] }, { on: 'B', rect: [110, 180, 126, 196] },
  ] }).ok, true);
  const doc = reopen(saved(session));
  const pdf = doc.asPDF();
  const acro = pdf.getTrailer().get('Root').get('AcroForm');
  assert.ok(!acro.get('NeedAppearances') || acro.get('NeedAppearances').isNull());
  assert.ok(!acro.get('XFA') || acro.get('XFA').isNull());
  assert.ok(!acro.get('CO') || acro.get('CO').isNull() || acro.get('CO').length === 0);
  const names = pdf.getTrailer().get('Root').get('Names');
  if (names && !names.isNull()) {
    const js = names.get('JavaScript');
    assert.ok(!js || js.isNull(), 'no document-level JavaScript');
  }
  // Every new widget carries /DA and /F 4.
  const loaded = doc.loadPage(0);
  try {
    for (const widget of loaded.getWidgets()) {
      const dict = widget.getObject();
      assert.ok(!dict.get('DA').isNull());
      assert.equal(dict.get('F').asNumber(), 4);
    }
  } finally {
    loaded.destroy();
  }
});

// ------------------------------------------------------------ the awkward pages

test('§7.1 rect: the reported rect is the requested one on a CropBox page and a rotated page', async () => {
  for (const page of [0, 1]) {
    const session = await open('p3/form-design');
    const rect = [72, 72, 152, 92];
    const added = add(session, { kind: 'text', name: 'n1', rect }, page);
    assert.equal(added.ok, true, `${added.error} (page ${page})`);
    const listed = byName(session, 'n1', page)[0];
    for (let index = 0; index < 4; index++) {
      assert.ok(Math.abs(listed.rect[index] - rect[index]) <= 0.5,
        `page ${page} rect ${JSON.stringify(listed.rect)} != ${JSON.stringify(rect)}`);
    }
  }
});

test('§7.6 shared indirect /Annots: adding on page 2 leaves page 3 untouched', async () => {
  const session = await open('p3/form-design');
  const added = add(session, { kind: 'text', name: 'n1', rect: [72, 72, 252, 96] }, 2);
  assert.equal(added.ok, true, added.error);
  const bytes = saved(session);
  const doc = reopen(bytes);
  const pdf = doc.asPDF();
  const page2 = pdf.findPage(2).get('Annots');
  const page3 = pdf.findPage(3).get('Annots');
  assert.equal(page2.length, 1, 'page 2 has the new widget');
  assert.equal(page3.length, 0, 'page 3 kept the shared array as it was');
});

test('§7.5 resize: a filled text field redraws in the new size, ink stays inside the new rect', async () => {
  const session = await open('hello');
  const rect = [300, 500, 480, 524];
  assert.equal(add(session, { kind: 'text', name: 'n1', rect }).ok, true);
  const item = byName(session, 'n1')[0];
  assert.equal(set(session, item.id, 'ABC').ok, true);

  const moved = [300, 300, 420, 340];
  assert.equal(props(session, { widgetId: item.id, rect: moved }).ok, true);
  const bytes = saved(session);
  const appearance = appearanceFacts(bytes, 'n1');
  assert.equal(appearance.kind, 'stream');
  assert.deepEqual(appearance.bbox, [0, 0, 120, 40], 'the /AP BBox is the new size');

  assert.ok(darkPixels(bytes, moved) > 20, 'the value is drawn in the new rect');
  assert.equal(darkPixels(bytes, rect), 0, 'nothing is drawn in the old rect');
});

test('§7.10 a 500-page form: the name-uniqueness walk stays bounded and finishes', async () => {
  const session = await open('large');
  const started = performance.now();
  const added = add(session, { kind: 'text', name: 'n1', rect: [72, 72, 252, 96] });
  const ms = performance.now() - started;
  assert.equal(added.ok, true, added.error);
  assert.ok(ms < 5000, `the add took ${ms.toFixed(0)} ms`);
});

test('review U6: a dropdown options change keeps a value that is still an option', async () => {
  const session = await open('hello');
  const added = add(session, { kind: 'combo', name: 'd1', rect: [72, 72, 252, 96], options: ['one', 'two', 'three'] });
  assert.equal(added.ok, true, JSON.stringify(added));
  assert.equal(set(session, added.widgetId, 'two').ok, true);
  assert.equal(props(session, { widgetId: added.widgetId, options: ['one', 'two', 'four'] }).ok, true);
  assert.equal(byName(session, 'd1')[0].value, 'two');
  assert.equal(props(session, { widgetId: added.widgetId, options: ['five', 'six'] }).ok, true);
  assert.equal(byName(session, 'd1')[0].value, 'five');
});

test('review U6: authored widgets carry /MK border, background and the page /Rotate', async () => {
  const session = await open('p3/form-design');
  for (const page of [0, 1]) {
    const text = add(session, { kind: 'text', name: `t${page}`, rect: [200, 100, 380, 124] }, page);
    const radio = add(session, { kind: 'radio', name: `g${page}`, kids: [{ on: 'A', rect: [100, 100, 116, 116] }, { on: 'B', rect: [130, 100, 146, 116] }] }, page);
    assert.equal(text.ok && radio.ok, true);
    const target = session.doc.loadPage(page);
    try {
      const widgets = target.getWidgets();
      assert.equal(widgets.length, 3);
      for (const widget of widgets) {
        const mk = widget.getObject().get('MK');
        assert.equal(String(mk.get('BC')), '[0]');
        assert.equal(String(mk.get('BG')), '[1]');
        assert.equal(mk.get('R').isNull() ? 0 : mk.get('R').asNumber(), page === 1 ? 90 : 0);
      }
    } finally {
      target.destroy();
    }
  }
});

test('review U6: after a fill, an off radio kid and an unchecked checkbox still draw inside their rects', async () => {
  const session = await open('hello');
  const radio = add(session, { kind: 'radio', name: 'g', kids: [{ on: 'A', rect: [300, 500, 316, 516] }, { on: 'B', rect: [330, 500, 346, 516] }] });
  const check = add(session, { kind: 'checkbox', name: 'c1', rect: [300, 540, 316, 556] });
  assert.equal(radio.ok && check.ok, true);
  assert.equal(set(session, radio.widgetIds[1], true).ok, true);
  assert.equal(set(session, check.widgetId, true).ok, true);
  assert.equal(set(session, check.widgetId, false).ok, true);
  const bytes = saved(session);
  assert.ok(darkPixels(bytes, [300, 500, 316, 516]) > 0, 'the off kid A is visible');
  assert.ok(darkPixels(bytes, [330, 500, 346, 516]) > darkPixels(bytes, [300, 500, 316, 516]), 'the on kid B draws more');
  assert.ok(darkPixels(bytes, [300, 540, 316, 556]) > 0, 'the unchecked checkbox is visible');
});

// ------------------------------------------------------- U7: label, flags and default

/** The field dict's `/TU`, `/Ff` and `/DV` from the bytes (the parent's dict for a radio kid). */
function fieldFacts(bytes, name, page = 0) {
  const doc = reopen(bytes);
  const loaded = doc.loadPage(page);
  try {
    const widget = loaded.getWidgets().find(item => item.getName() === name);
    if (!widget) return null;
    let dict = widget.getObject();
    const parent = dict.get('Parent');
    if (parent && !parent.isNull()) dict = parent;
    const ff = dict.get('Ff');
    const dv = dict.get('DV');
    const tu = dict.get('TU');
    return {
      hasFf: !!ff && !ff.isNull(),
      ff: ff && !ff.isNull() ? ff.asNumber() : null,
      dv: !dv || dv.isNull() ? null : dv.isName() ? dv.asName() : dv.asString(),
      tu: !tu || tu.isNull() ? null : tu.asString(),
    };
  } finally {
    loaded.destroy();
  }
}

/** Every radio kid's `/AS` in widget order, from the bytes. */
function kidStates(bytes, name, page = 0) {
  const doc = reopen(bytes);
  const loaded = doc.loadPage(page);
  try {
    return loaded.getWidgets().filter(widget => widget.getName() === name).map(widget => {
      const as = widget.getObject().get('AS');
      return as && !as.isNull() ? as.asName() : null;
    });
  } finally {
    loaded.destroy();
  }
}

/** A radio group whose first kid may carry its own `/Ff` (the §8.2 shadowing fixture). */
function ownFlagRadioPdf(kidFf) {
  const kid = (rect, on) => `<< /Type /Annot /Subtype /Widget /Parent 9 0 R /Rect ${rect} /F 4 /P 3 0 R`
    + ' /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [1] >> /AS /Off'
    + ` /AP << /N << /Off 6 0 R /${on} 6 0 R >> >>${kidFf === undefined ? '' : ` /Ff ${kidFf}`} >>`;
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [7 0 R 10 0 R] >>',
    '<< /Length 3 >>\nstream\nq \nendstream',
    '<< /Fields [9 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 8 0 R >> >> >>',
    '<< /Type /XObject /Subtype /Form /BBox [0 0 16 16] /Length 31 >>\nstream\nq 1 g 0 0 16 16 re f Q\nendstream',
    kid('[72 700 88 716]', 'A'),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    '<< /FT /Btn /T (g) /Ff 32768 /Kids [7 0 R 10 0 R] /V /Off /DA (/Helv 12 Tf 0 g) >>',
    kid('[110 700 126 716]', 'B'),
  ]);
}

test('U7 text: /TU, /Ff bit 2 and /DV land on add and survive a save and reopen', async () => {
  const session = await open('hello');
  const rect = [72, 72, 252, 96];
  const added = add(session, { kind: 'text', name: 'u7t', rect, label: 'Full name', required: true, default: 'Ann' });
  assert.equal(added.ok, true, added.error);
  const item = byName(session, 'u7t')[0];
  assert.equal(item.label, 'Full name');
  assert.equal(item.tu, 'Full name', '§8.4: tu is the raw /TU');
  assert.equal(item.required, true);
  assert.equal(item.readOnly, false);
  assert.equal(item.default, 'Ann');
  assert.equal(item.value, 'Ann', '§2.1: on add /V follows default');
  assert.deepEqual(fieldFacts(saved(session), 'u7t'), { hasFf: true, ff: 2, dv: 'Ann', tu: 'Full name' });

  const reopened = await openBytes(saved(session), 'u7.pdf');
  const after = byName(reopened, 'u7t')[0];
  assert.equal(after.label, 'Full name');
  assert.equal(after.required, true);
  assert.equal(after.default, 'Ann');
  assert.equal(after.value, 'Ann');

  // §8.4: a field with no /TU reports label = name and no tu at all, and an empty label on
  // add writes no /TU either (U7 review follow-up 1).
  const plain = await open('hello');
  assert.equal(add(plain, { kind: 'text', name: 'plain', rect, label: '' }).ok, true);
  const bare = byName(plain, 'plain')[0];
  assert.equal(bare.label, 'plain');
  assert.equal(bare.tu, undefined);
  assert.equal(bare.default, undefined);
  assert.equal(fieldFacts(saved(plain), 'plain').tu, null, 'an empty add label writes no /TU');
});

test('U7 defaults: checkbox front /AS, radio parent /DV with the chosen kid, combo /DV', async () => {
  const session = await open('hello');
  assert.equal(add(session, { kind: 'checkbox', name: 'c1', rect: [72, 110, 88, 126], default: true }).ok, true);
  assert.deepEqual(fieldFacts(saved(session), 'c1'), { hasFf: false, ff: null, dv: 'Yes', tu: null });
  assert.equal(byName(session, 'c1')[0].default, true, '§8.5: /DV /Yes reads back as true');
  assert.equal(byName(session, 'c1')[0].value, true);

  const kids = [{ on: 'A', rect: [72, 180, 88, 196] }, { on: 'B', rect: [110, 180, 126, 196] }];
  assert.equal(add(session, { kind: 'radio', name: 'g', kids, default: 'B' }).ok, true);
  assert.deepEqual(fieldFacts(saved(session), 'g'), { hasFf: true, ff: 32768, dv: 'B', tu: null });
  assert.deepEqual(kidStates(saved(session), 'g'), ['Off', 'B'], '§8.6: the chosen kid carries /AS');
  assert.equal(byName(session, 'g').find(entry => entry.on === 'B').default, 'B');

  assert.equal(add(session, { kind: 'combo', name: 'd1', options: ['one', 'two'], rect: [72, 220, 252, 244], default: 'two' }).ok, true);
  assert.deepEqual(fieldFacts(saved(session), 'd1'), { hasFf: true, ff: 131072, dv: 'two', tu: null });
  assert.equal(byName(session, 'd1')[0].value, 'two', 'the default replaces the first-option rule');
});

test('U7 props: read-only blocks fill but not design, required moves bit 2 only, label round-trips', async () => {
  const session = await open('hello');
  assert.equal(add(session, { kind: 'text', name: 'p1', rect: [72, 72, 252, 96], multiline: true, default: 'Ann' }).ok, true);
  const item = byName(session, 'p1')[0];
  const multilineBits = mupdf.PDFWidget.TX_FIELD_IS_MULTILINE;
  assert.equal(fieldFacts(saved(session), 'p1').ff, multilineBits);

  assert.equal(props(session, { widgetId: item.id, readOnly: true }).ok, true);
  assert.equal(byName(session, 'p1')[0].readOnly, true);
  const beforeRefusal = rawFacts(saved(session));
  const refused = set(session, item.id, 'Bob');
  assert.equal(refused.ok, false);
  assert.equal(refused.detail, 'field-read-only', '§8.1: the field token, not the permission one');
  assert.deepEqual(rawFacts(saved(session)), beforeRefusal, 'a refused fill changes nothing');
  // §2.1: read-only never blocks design, or it could never be cleared.
  assert.equal(props(session, { widgetId: item.id, label: 'Name' }).ok, true);
  assert.equal(props(session, { widgetId: item.id, readOnly: false }).ok, true);
  assert.equal(byName(session, 'p1')[0].readOnly, false);
  assert.equal(set(session, item.id, 'Bob').ok, true);

  const base = fieldFacts(saved(session), 'p1').ff;
  assert.equal(base, multilineBits, 'the read-only round trip left no other bit behind');
  assert.equal(props(session, { widgetId: item.id, required: true }).ok, true);
  assert.equal(fieldFacts(saved(session), 'p1').ff, base | 2, 'bit 2 only');
  assert.equal(props(session, { widgetId: item.id, required: false }).ok, true);
  assert.equal(fieldFacts(saved(session), 'p1').ff, base, 'the multiline bit 13 is untouched');

  // §8.7: a property that is not a flag never writes /Ff at all, so an absent key stays
  // absent; once a flag write has created it, clearing the bit writes 0 rather than dropping it.
  assert.equal(add(session, { kind: 'text', name: 'p2', rect: [300, 72, 480, 96] }).ok, true);
  const bare = byName(session, 'p2')[0];
  assert.equal(fieldFacts(saved(session), 'p2').hasFf, false);
  assert.equal(props(session, { widgetId: bare.id, label: 'Bare' }).ok, true);
  assert.equal(fieldFacts(saved(session), 'p2').hasFf, false, 'absent stays absent');
  assert.equal(props(session, { widgetId: bare.id, required: true }).ok, true);
  assert.deepEqual(fieldFacts(saved(session), 'p2'), { hasFf: true, ff: 2, dv: null, tu: 'Bare' });
  assert.equal(props(session, { widgetId: bare.id, required: false }).ok, true);
  assert.deepEqual(fieldFacts(saved(session), 'p2'), { hasFf: true, ff: 0, dv: null, tu: 'Bare' });

  // A radio group's flags land on the parent and are inherited by every kid.
  const kids = [{ on: 'A', rect: [72, 300, 88, 316] }, { on: 'B', rect: [110, 300, 126, 316] }];
  assert.equal(add(session, { kind: 'radio', name: 'g', kids }).ok, true);
  const kidB = byName(session, 'g').find(entry => entry.on === 'B');
  assert.equal(props(session, { widgetId: kidB.id, required: true }).ok, true);
  assert.equal(fieldFacts(saved(session), 'g').ff, 32770, 'the radio bit plus bit 2');
  assert.equal(byName(session, 'g').find(entry => entry.on === 'A').required, true);
  assert.equal(props(session, { widgetId: kidB.id, required: false }).ok, true);
  assert.equal(fieldFacts(saved(session), 'g').ff, 32768);

  // §2.1: a non-ASCII label round-trips through getLabel() and the raw key.
  assert.equal(props(session, { widgetId: item.id, label: 'Nom (é)' }).ok, true);
  assert.equal(byName(session, 'p1')[0].label, 'Nom (é)');
  assert.equal(byName(session, 'p1')[0].tu, 'Nom (é)');
  assert.equal(props(session, { widgetId: item.id, label: '' }).ok, true);
  assert.equal(fieldFacts(saved(session), 'p1').tu, null, 'the empty label deletes /TU');
  assert.equal(byName(session, 'p1')[0].label, 'p1', 'label falls back to the name');

  // §2.1: props never moves /V, whatever the default says.
  assert.equal(props(session, { widgetId: item.id, default: 'Cara' }).ok, true);
  assert.equal(byName(session, 'p1')[0].default, 'Cara');
  assert.equal(byName(session, 'p1')[0].value, 'Bob', 'the filled value stays');
});

test('U7 refusals never dirty: label bounds, control characters, bad defaults, bad flags', async () => {
  const session = await open('hello');
  assert.equal(add(session, { kind: 'text', name: 't1', rect: [72, 72, 252, 96], maxLen: 3 }).ok, true);
  assert.equal(add(session, { kind: 'combo', name: 'd1', options: ['one', 'two'], rect: [72, 110, 252, 134] }).ok, true);
  assert.equal(add(session, { kind: 'checkbox', name: 'c1', rect: [72, 150, 88, 166] }).ok, true);
  assert.equal(add(session, { kind: 'radio', name: 'g', kids: [
    { on: 'A', rect: [72, 200, 88, 216] }, { on: 'B', rect: [110, 200, 126, 216] },
  ] }).ok, true);
  const clean = rawFacts(saved(session));
  const text = byName(session, 't1')[0];
  const combo = byName(session, 'd1')[0];
  const check = byName(session, 'c1')[0];
  const radio = byName(session, 'g')[0];
  const cases = [
    [add(session, { kind: 'text', name: 'x1', rect: [300, 72, 480, 96], label: 'x'.repeat(MAX_FIELD_LABEL + 1) }), 'a label holds at most'],
    [add(session, { kind: 'text', name: 'x2', rect: [300, 72, 480, 96], label: 'a\u0007b' }), 'no control characters'],
    [add(session, { kind: 'text', name: 'x3', rect: [300, 72, 480, 96], maxLen: 3, default: 'abcd' }), 'longer than maxLen 3'],
    [props(session, { widgetId: text.id, default: 'abcd' }), 'longer than maxLen 3'],
    [add(session, { kind: 'combo', name: 'x4', options: ['a'], rect: [300, 72, 480, 96], default: 'b' }), 'not one of this field'],
    [props(session, { widgetId: combo.id, default: 'three' }), 'not one of this field'],
    [add(session, { kind: 'checkbox', name: 'x5', rect: [300, 72, 316, 88], default: 'yes' }), 'true or false'],
    [props(session, { widgetId: check.id, default: 'yes' }), 'true or false'],
    [add(session, { kind: 'radio', name: 'x6', kids: [
      { on: 'A', rect: [300, 72, 316, 88] }, { on: 'B', rect: [330, 72, 346, 88] },
    ], default: 'C' }), 'not one of this group'],
    [props(session, { widgetId: radio.id, default: 'C' }), 'not one of this group'],
    [props(session, { widgetId: text.id, required: 'yes' }), 'required is true or false'],
    [props(session, { widgetId: text.id, readOnly: 1 }), 'readOnly is true or false'],
    [props(session, { widgetId: text.id, label: 5 }), 'a label holds at most'],
  ];
  for (const [reply, expected] of cases) {
    assert.equal(reply.ok, false, `${expected} was accepted`);
    assert.equal(reply.detail, 'invalid');
    assert.ok(reply.error.includes(expected), `${expected} missing from ${reply.error}`);
  }
  assert.deepEqual(rawFacts(saved(session)), clean, 'every refusal left the document alone');
  assert.equal(byName(session, 't1')[0].default, undefined);
});

test('U7 undo/redo: one step restores /TU, /Ff and /DV exactly, absent keys staying absent', async () => {
  const session = await open('hello');
  const opened = rawFacts(saved(session));
  assert.equal(add(session, { kind: 'text', name: 'u7', rect: [72, 72, 252, 96] }).ok, true);
  const listed = rawFacts(saved(session));
  assert.deepEqual(fieldFacts(saved(session), 'u7'), { hasFf: false, ff: null, dv: null, tu: null });
  const item = byName(session, 'u7')[0];
  const changed = props(session, { widgetId: item.id, label: 'Full name', readOnly: true, required: true, default: 'Ann' });
  assert.equal(changed.ok, true, changed.error);
  assert.deepEqual(fieldFacts(saved(session), 'u7'), { hasFf: true, ff: 3, dv: 'Ann', tu: 'Full name' });

  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual(rawFacts(saved(session)), listed, 'the field itself is still there');
  assert.deepEqual(fieldFacts(saved(session), 'u7'), { hasFf: false, ff: null, dv: null, tu: null });
  // The U5 rule: a save on either side of the undo does not change what redo brings back.
  const undone = rawFacts(saved(session));
  assert.equal(op(session, { op: 'undo' }).ok, true, 'the add is its own step');
  assert.deepEqual(rawFacts(saved(session)), opened);
  assert.equal(op(session, { op: 'redo' }).ok, true);
  assert.deepEqual(rawFacts(saved(session)), undone);
  assert.equal(op(session, { op: 'redo' }).ok, true);
  assert.deepEqual(fieldFacts(saved(session), 'u7'), { hasFf: true, ff: 3, dv: 'Ann', tu: 'Full name' });
});

test('U7 §8.2: a group whose kid shadows the field properties refuses all four', async () => {
  const clean = await openBytes(ownFlagRadioPdf(undefined), 'radio-clean.pdf');
  assert.deepEqual(byName(clean, 'g').map(entry => entry.kind), ['radio', 'radio'], 'the group lists');
  const kid = byName(clean, 'g')[0];
  assert.equal(props(clean, { widgetId: kid.id, label: 'Group' }).ok, true, 'no own flag: the write is allowed');
  assert.equal(fieldFacts(saved(clean), 'g').tu, 'Group', 'and it lands on the parent');

  const shadowed = await openBytes(ownFlagRadioPdf(0), 'radio-ff0.pdf');
  const dirty = byName(shadowed, 'g');
  assert.equal(dirty.length, 2, 'the group still lists as radio');
  const cleanBytes = rawFacts(saved(shadowed));
  for (const spec of [{ label: 'Group' }, { readOnly: true }, { required: true }, { default: 'B' }]) {
    const reply = props(shadowed, { widgetId: dirty[0].id, ...spec });
    assert.equal(reply.ok, false, `${JSON.stringify(spec)} was accepted`);
    assert.equal(reply.detail, 'invalid');
    assert.ok(reply.error.includes('hierarchy'), reply.error);
  }
  assert.deepEqual(rawFacts(saved(shadowed)), cleanBytes, 'refusals never dirtied it');
});

test('U7 §8.3: a stale /DV goes with the options, with maxLen and with the kid it named', async () => {
  const session = await open('hello');
  // Options without the current /DV drop it.
  assert.equal(add(session, { kind: 'combo', name: 'd1', options: ['one', 'two'], rect: [72, 72, 252, 96], default: 'two' }).ok, true);
  const combo = byName(session, 'd1')[0];
  assert.equal(props(session, { widgetId: combo.id, options: ['alpha', 'two'] }).ok, true);
  assert.equal(fieldFacts(saved(session), 'd1').dv, 'two', 'still an option, still the default');
  assert.equal(props(session, { widgetId: combo.id, options: ['alpha', 'beta'] }).ok, true);
  assert.equal(fieldFacts(saved(session), 'd1').dv, null, 'a default that is no longer an option goes');

  // A maxLen that would cut the existing /DV is refused, and never dirties.
  assert.equal(add(session, { kind: 'text', name: 't1', rect: [300, 72, 480, 96], default: 'abcdef' }).ok, true);
  const text = byName(session, 't1')[0];
  assert.equal(set(session, text.id, 'ab').ok, true, 'the value is short enough for maxLen 3');
  const clean = rawFacts(saved(session));
  const tooSmall = props(session, { widgetId: text.id, maxLen: 3 });
  assert.equal(tooSmall.ok, false);
  assert.equal(tooSmall.detail, 'invalid');
  assert.ok(tooSmall.error.includes('default'), tooSmall.error);
  assert.deepEqual(rawFacts(saved(session)), clean);
  assert.equal(props(session, { widgetId: text.id, maxLen: 6 }).ok, true, 'exactly the default length is fine');
  assert.equal(fieldFacts(saved(session), 't1').dv, 'abcdef');

  // Deleting the kid a parent /DV names takes the /DV with it.
  const kids = [{ on: 'A', rect: [72, 200, 88, 216] }, { on: 'B', rect: [110, 200, 126, 216] }];
  assert.equal(add(session, { kind: 'radio', name: 'g', kids, default: 'B' }).ok, true);
  assert.equal(fieldFacts(saved(session), 'g').dv, 'B');
  const kidB = byName(session, 'g').find(entry => entry.on === 'B');
  assert.equal(remove(session, kidB.id).ok, true);
  assert.equal(fieldFacts(saved(session), 'g').dv, null, 'the default pointed at the kid that left');
});

test('U7 the new keys are refused on hierarchies, scripted fields, XFA and permission-limited files', async () => {
  const nested = await openBytes(hierarchyPdf(), 'hierarchy.pdf');
  const inner = byName(nested, 'outer.inner')[0];
  for (const spec of [{ label: 'x' }, { readOnly: true }, { required: true }, { default: 'x' }]) {
    const reply = props(nested, { widgetId: inner.id, ...spec });
    assert.equal(reply.ok, false, JSON.stringify(spec));
    assert.equal(reply.detail, 'invalid');
  }

  const form2 = await open('form2');
  const amount = byName(form2, 'amount')[0];
  assert.equal(props(form2, { widgetId: amount.id, label: 'Total' }).detail, 'scripted');

  const xfa = await open('p3/form-xfa');
  assert.equal(add(xfa, { kind: 'text', name: 'n9', rect: [360, 650, 540, 680], label: 'x' }).detail, 'xfa');

  const locked = await open('p3/encrypted-edit-noannot');
  assert.equal(op(locked, { op: 'authenticate', password: 'pdf4all-user' }).ok, true);
  const fillable = byName(locked, 'fillable')[0];
  assert.ok(fillable, 'the encrypted fixture lists its field');
  const refused = props(locked, { widgetId: fillable.id, label: 'Name' });
  assert.equal(refused.ok, false);
  assert.equal(refused.reason, 'read-only', 'the permission gate, before any COS write');
});
