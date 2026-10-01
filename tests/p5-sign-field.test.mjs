/**
 * P5 U3 §1 — sign an EXISTING empty signature field in place, and author empty signature
 * fields in the designer. Runs against build/renderer/engine.js as the worker does.
 * Fixture: fixtures/p5/empty-sigfield.pdf (Signature1 obj 7, Signature2 obj 8 with a grey
 * /AP, Signature3 obj 10 on a /Rotate 90 page). pyHanko runs in its own skippable test (§9.32).
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';
import { parseP12 } from '../build/sign/pkcs12.js';
import { cmsDetached, patchSignature } from '../build/sign/cms.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const py = join(root, '.venv-qa/bin/python');
const FIXTURE = 'p5/empty-sigfield.pdf';
const FIXTURE_SHA = '07e46851060fc7aa179c0f89a7d08ebdac26f682f4322edf20c57dad0523a206';
const engine = createEngine(mupdf);
let id = 1;
const bytesOf = b => b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);
const fixture = async name => bytesOf(await readFile(new URL(`../fixtures/${name}`, import.meta.url)));
const dispatch = (s, request) => engine.dispatch({ id: id++, ...request }, s).reply;
const openBytes = (bytes, name = 'scratch.pdf') => {
  const s = engine.createSession();
  const r = dispatch(s, { op: 'open', name, pdf: bytes });
  assert.equal(r.ok, true, r.error);
  return { s, info: r.info };
};
const open = async name => openBytes(await fixture(name), name.split('/').pop()).s;
const request = (overrides = {}) => ({ op: 'sign-prepare', name: 'P0B Signer', reason: 'field test', page: 0, rect: null, certify: false, when: Date.now(), ...overrides });
const fields = (s, page = 0) => { const r = dispatch(s, { op: 'fields', page }); assert.equal(r.ok, true, r.error); return r.items; };
const byName = (s, name, page = 0) => fields(s, page).find(item => item.name === name);
const saveBytes = s => { const r = dispatch(s, { op: 'save' }); assert.equal(r.ok, true, r.error); return r.bytes; };
const text = bytes => new TextDecoder('latin1').decode(new Uint8Array(bytes));

let p12;
const sign = async prepared => {
  p12 ??= parseP12(await readFile(join(root, 'fixtures/p5/leaf.p12')), 'pdf4all');
  const patched = patchSignature(new Uint8Array(prepared.bytes), prepared, content => cmsDetached({ content, certs: p12.certs, key: p12.key, signingTime: new Date() }));
  assert.equal(patched.ok, true, patched.reason);
  return bytesOf(patched.bytes);
};
/** Exact-once placeholder, as tests/p5-sign.test.mjs:34-38. */
const exactOnce = prepared => {
  const bytes = new Uint8Array(prepared.bytes);
  assert.equal(text(bytes).slice(prepared.byteRangeAt, prepared.byteRangeAt + 36), '[0 1000000000 1000000000 1000000000]');
  assert.equal(bytes[prepared.contentsAt - 1], 60);
  assert.equal(bytes[prepared.contentsAt + prepared.contentsLen], 62);
};
/** Every signature widget in raw bytes: object number, `/V`, and its `/AP /N` stream text. */
const sigWidgets = bytes => {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf').asPDF();
  try {
    const out = {};
    for (let p = 0; p < doc.countPages(); p++) {
      const page = doc.loadPage(p);
      for (const w of page.getWidgets()) {
        if (w.getFieldType() !== 'signature') continue;
        const obj = w.getObject(), ap = obj.get('AP'), n = ap.isNull() ? null : ap.get('N');
        out[w.getName()] = { obj: obj.asIndirect(), page: p, hasV: !obj.get('V').isNull(), ap: !n || n.isNull() ? null : new TextDecoder('latin1').decode(n.readStream().asUint8Array()) };
      }
      page.destroy();
    }
    return out;
  } finally { doc.destroy(); }
};
/** A refusal: the reason and no journal change. */
const refuses = (s, req, reason) => {
  const before = engine.journalState(s);
  const r = dispatch(s, req);
  assert.equal(r.ok, false, `${JSON.stringify(req)} was accepted`);
  assert.equal(r.reason, reason, r.error);
  assert.deepEqual(engine.journalState(s), before);
  assert.equal(before.dirty, false);
  return r;
};
/** Outputs the pyHanko test checks: [file, expected signatures, [line regexes]]. */
const pyhankoCases = [];

let signed1; // the fixture with Signature1 signed (full rewrite)
let signed2; // signed1 plus Signature2 (one increment)

test('P5 U3 fixture: pinned bytes, three empty signature fields, writable', async () => {
  const bytes = await fixture(FIXTURE);
  assert.equal(createHash('sha256').update(new Uint8Array(bytes)).digest('hex'), FIXTURE_SHA);
  const provenance = JSON.parse(await readFile(join(root, 'fixtures/p5/empty-sigfield.provenance.json'), 'utf8'));
  assert.equal(JSON.stringify(provenance).includes(FIXTURE_SHA), true);
  const { s, info } = openBytes(bytes, 'empty-sigfield.pdf');
  assert.equal(info.signatures, 0);
  const shape = items => items.map(({ id, name, kind, rect, signed }) => ({ id, name, kind, rect, signed }));
  assert.deepEqual(shape(fields(s, 0)), [
    { id: 7, name: 'Signature1', kind: 'signature', rect: [72, 620, 340, 680], signed: false },
    { id: 8, name: 'Signature2', kind: 'signature', rect: [360, 620, 540, 680], signed: false },
  ]);
  assert.deepEqual(shape(fields(s, 1)), [{ id: 10, name: 'Signature3', kind: 'signature', rect: [500, 72, 560, 300], signed: false }]);
  saveBytes(s);
});

test('P5 U3 sign-prepare {field}: adopts Signature1 in place, then signs', async () => {
  const s = await open(FIXTURE);
  const before = engine.journalState(s);
  const prepared = dispatch(s, request({ field: 'Signature1' }));
  assert.equal(prepared.ok, true, prepared.error);
  assert.equal(prepared.revisions, 1);
  assert.equal(prepared.field, 'Signature1');
  assert.deepEqual(prepared.rect, [72, 620, 340, 680]);
  exactOnce(prepared);
  assert.deepEqual(engine.journalState(s), before, 'signing never touches the open document');
  const widgets = sigWidgets(prepared.bytes);
  assert.deepEqual(Object.keys(widgets), ['Signature1', 'Signature2', 'Signature3']);
  assert.deepEqual([widgets.Signature1.obj, widgets.Signature2.obj, widgets.Signature3.obj], [7, 8, 10]);
  assert.deepEqual([widgets.Signature1.hasV, widgets.Signature2.hasV, widgets.Signature3.hasV], [true, false, false]);
  assert.match(widgets.Signature1.ap, /Digitally signed by P0B Signer/);
  assert.ok(widgets.Signature2.ap && !widgets.Signature2.ap.includes('Digitally signed'), 'Signature2 keeps its grey AP');
  assert.equal(widgets.Signature3.ap, null);

  signed1 = await sign(prepared);
  const { s: reopened, info } = openBytes(signed1, 'signed1.pdf');
  assert.equal(info.signatures, 1);
  assert.equal(byName(reopened, 'Signature1').signed, true);
  assert.equal(byName(reopened, 'Signature2').signed, false);
  assert.equal(dispatch(reopened, { op: 'save' }).reason, 'read-only');
  const list = dispatch(reopened, { op: 'sign-list' });
  assert.equal(list.ok, true, list.error);
  assert.equal(list.signatures.find(e => e.field === 'Signature1').empty, false);
  pyhankoCases.push(['signed1.pdf', signed1, 1, [/^Signature1 INTACT ENTIRE_FILE trusted docmdp-ok$/m]]);
});

test('P5 U3 sign-prepare {field} on a signed source: refusals, then an incremental second field', async () => {
  assert.ok(signed1, 'needs the previous test');
  const { s } = openBytes(signed1, 'signed1.pdf');
  refuses(s, request({ field: 'Signature1' }), 'already-signed');
  const nope = refuses(s, request({ field: 'Nope' }), 'no-such-field');
  assert.equal(nope.error, 'sign-prepare: page 0 has no signature field Nope');
  refuses(s, request({ field: 'Signature2', page: 1 }), 'no-such-field');
  refuses(s, request({ field: 'Signature2', rect: [1, 1, 2, 2] }), 'invalid');
  const prepared = dispatch(s, request({ field: 'Signature2' }));
  assert.equal(prepared.ok, true, prepared.error);
  assert.equal(prepared.revisions, 2);
  assert.deepEqual(prepared.rect, [360, 620, 540, 680]);
  exactOnce(prepared);
  const source = new Uint8Array(signed1);
  assert.deepEqual(new Uint8Array(prepared.bytes).slice(0, source.byteLength), source);
  assert.match(text(prepared.bytes).slice(source.byteLength), /(^|\n)8 0 obj/, 'widget 8 rewritten in the increment');
  assert.equal(sigWidgets(prepared.bytes).Signature2.obj, 8);
  signed2 = await sign(prepared);
  assert.equal(openBytes(signed2, 'signed2.pdf').info.signatures, 2);
  pyhankoCases.push(['signed2.pdf', signed2, 2, [/^Signature1 INTACT ENTIRE_REVISION trusted docmdp-ok$/m, /^Signature2 INTACT ENTIRE_FILE trusted docmdp-ok$/m]]);
});

test('P5 U3 sign-prepare {field}: a third signature on a twice-signed source reports revisions 3', async () => {
  assert.ok(signed2, 'needs the previous test');
  const { s } = openBytes(signed2, 'signed2.pdf');
  const prepared = dispatch(s, request({ field: 'Signature3', page: 1 }));
  assert.equal(prepared.ok, true, prepared.error);
  assert.equal(prepared.revisions, 3);
  const signed3 = await sign(prepared);
  assert.equal(openBytes(signed3, 'signed3.pdf').info.signatures, 3);
  pyhankoCases.push(['signed3.pdf', signed3, 3, [/^Signature3 INTACT ENTIRE_FILE trusted docmdp-ok$/m]]);
});

test('P5 U3 sign-prepare {field} on the /Rotate 90 page: /Matrix and pixels inside the field', async () => {
  const s = await open(FIXTURE);
  const prepared = dispatch(s, request({ field: 'Signature3', page: 1 }));
  assert.equal(prepared.ok, true, prepared.error);
  assert.deepEqual(prepared.rect, [500, 72, 560, 300]);
  assert.match(sigWidgets(prepared.bytes).Signature3.ap, /Digitally signed/);
  const doc = mupdf.Document.openDocument(prepared.bytes, 'application/pdf');
  try {
    assert.match(text(prepared.bytes), /\/Matrix\[0 1 -1 0 0 0\]/);
    const page = doc.loadPage(1);
    const pixmap = page.toPixmap(mupdf.Matrix.scale(2, 2), mupdf.ColorSpace.DeviceRGB, false);
    try {
      const pixels = pixmap.getPixels(), n = pixmap.getNumberOfComponents(), width = pixmap.getWidth();
      let blue = 0, inside = 0;
      for (let i = 0; i < pixels.length; i += n) {
        if (pixels[i + 2] - pixels[i] > 8 && pixels[i + 2] - pixels[i + 1] > 8) {
          blue++;
          const x = (i / n) % width, y = Math.floor(i / n / width);
          if (x >= 1000 && x < 1120 && y >= 144 && y < 600) inside++;
        }
      }
      assert.ok(blue > 500, `rotated field AP rendered (${blue} blue pixels)`);
      assert.equal(inside, blue, 'every blue pixel is inside the field');
    } finally { pixmap.destroy(); page.destroy(); }
  } finally { doc.destroy(); }
  pyhankoCases.push(['signed3.pdf', await sign(prepared), 1, [/^Signature3 INTACT ENTIRE_FILE trusted docmdp-ok$/m]]);
});

test('P5 U3 a certified (P=2) document: an existing field signs, a new visible one stays refused', async () => {
  const s = await open(FIXTURE);
  const certify = dispatch(s, request({ certify: true }));
  assert.equal(certify.ok, true, certify.error);
  const certified = await sign(certify);
  const { s: cs } = openBytes(certified, 'certified.pdf');
  refuses(cs, request({ rect: [72, 300, 340, 360] }), 'certified');
  const prepared = dispatch(cs, request({ field: 'Signature1' }));
  assert.equal(prepared.ok, true, prepared.error);
  assert.equal(prepared.revisions, 2);
  pyhankoCases.push(['certified-filled.pdf', await sign(prepared), 2, [/^Signature4 INTACT ENTIRE_REVISION trusted docmdp-ok$/m, /^Signature1 INTACT ENTIRE_FILE trusted docmdp-ok$/m, /^DocMDP 2$/m]]);
});

test('P5 U3 field-add kind signature: free name, no /AP, survives save, signs in place', async () => {
  const s = await open('hello.pdf');
  const added = dispatch(s, { op: 'field-add', page: 0, kind: 'signature', rect: [72, 72, 272, 132] });
  assert.equal(added.ok, true, added.error);
  assert.equal(added.name, 'Signature1');
  const item = byName(s, 'Signature1');
  assert.equal(item.kind, 'signature');
  assert.equal(item.signed, false);
  assert.deepEqual(item.rect, [72, 72, 272, 132]);
  const saved = saveBytes(s);
  assert.equal(sigWidgets(saved).Signature1.ap, null, 'no /AP on an authored signature field');
  const { s: re, info } = openBytes(saved, 'hello-sig.pdf');
  assert.equal(info.signatures, 0);
  assert.equal(byName(re, 'Signature1').id, item.id, 'object number kept across garbage=yes');
  assert.deepEqual(byName(re, 'Signature1').rect, [72, 72, 272, 132]);
  const prepared = dispatch(re, request({ field: 'Signature1' }));
  assert.equal(prepared.ok, true, prepared.error);
  assert.deepEqual(prepared.rect, [72, 72, 272, 132]);
  pyhankoCases.push(['hello-field.pdf', await sign(prepared), 1, [/^Signature1 INTACT ENTIRE_FILE trusted docmdp-ok$/m]]);

  const fresh = await open('hello.pdf');
  const small = dispatch(fresh, { op: 'field-add', page: 0, kind: 'signature', name: 'S', rect: [72, 72, 100, 80] });
  assert.equal(small.detail, 'invalid');
  assert.equal(small.error, 'field-add: a signature field is at least 40x20 pt');
  assert.equal(dispatch(fresh, { op: 'field-add', page: 0, kind: 'signature', name: 'S', rect: [72, 72, 272, 132], default: 'x' }).error, 'field-add: default is not a signature property');
  assert.equal(dispatch(fresh, { op: 'field-add', page: 0, kind: 'signature', name: 'S', rect: [72, 72, 272, 132], options: ['a'] }).error, 'field-add: options are a dropdown property');
  assert.equal(dispatch(fresh, { op: 'field-add', page: 0, kind: 'signature', name: 'S', rect: [72, 72, 272, 132], kids: [] }).error, 'field-add: kids are a radio property');
  assert.equal(dispatch(fresh, { op: 'field-add', page: 0, kind: 'signature', name: 'S', rect: [72, 72, 272, 132], maxLen: 3 }).error, 'field-add: maxLen and multiline are text properties');
  assert.equal(dispatch(fresh, { op: 'field-add', page: 0, kind: 'signature', name: 'S' }).detail, 'invalid', 'rect is required');
  assert.equal(dispatch(fresh, { op: 'field-add', page: 0, kind: 'text', rect: [72, 72, 272, 132] }).detail, 'invalid-name', 'only a signature field may omit its name');
  assert.equal(engine.journalState(fresh).dirty, false);
  assert.equal(dispatch(fresh, { op: 'field-add', page: 0, kind: 'signature', name: 'S', rect: [72, 72, 272, 132] }).ok, true);
  assert.equal(dispatch(fresh, { op: 'field-add', page: 0, kind: 'signature', name: 'S', rect: [72, 200, 272, 260] }).detail, 'duplicate-name');

  const fx = await open(FIXTURE);
  const fourth = dispatch(fx, { op: 'field-add', page: 0, kind: 'signature', name: '', rect: [72, 300, 292, 360] });
  assert.equal(fourth.ok, true, fourth.error);
  assert.equal(fourth.name, 'Signature4');
  const fxSaved = openBytes(saveBytes(fx), 'fx.pdf').s;
  assert.deepEqual(byName(fxSaved, 'Signature4').rect, [72, 300, 292, 360]);
});

test('P5 U3 field-props / field-delete / field-set on an EMPTY signature field', async () => {
  const s = await open(FIXTURE);
  const sig = byName(s, 'Signature1');
  const moved = dispatch(s, { op: 'field-props', page: 0, widgetId: sig.id, rect: [72, 500, 340, 580] });
  assert.equal(moved.ok, true, moved.error);
  const reMoved = openBytes(saveBytes(s), 'moved.pdf').s;
  assert.deepEqual(byName(reMoved, 'Signature1').rect, [72, 500, 340, 580]);
  assert.equal(sigWidgets(saveBytes(s)).Signature1.ap, null);
  assert.equal(dispatch(s, { op: 'undo' }).ok, true);
  assert.deepEqual(byName(s, 'Signature1').rect, [72, 620, 340, 680]);

  // Signature2 carries a grey /AP; a move drops it (§9.4).
  const sig2 = byName(s, 'Signature2');
  assert.equal(dispatch(s, { op: 'field-props', page: 0, widgetId: sig2.id, rect: [360, 500, 540, 560], label: 'Approver', required: true }).ok, true);
  const props2 = byName(openBytes(saveBytes(s), 'p2.pdf').s, 'Signature2');
  assert.deepEqual([props2.rect, props2.label, props2.required], [[360, 500, 540, 560], 'Approver', true]);
  assert.equal(sigWidgets(saveBytes(s)).Signature2.ap, null);
  assert.equal(dispatch(s, { op: 'undo' }).ok, true);

  const clean = engine.journalState(s);
  const refusal = (req, detail, message) => {
    const r = dispatch(s, req);
    assert.equal(r.ok, false);
    assert.equal(r.detail, detail);
    assert.equal(r.error, message);
    assert.deepEqual(engine.journalState(s), clean);
  };
  refusal({ op: 'field-props', page: 0, widgetId: sig.id, name: 'Other' }, 'signature', 'field-props: a signature field keeps its name');
  refusal({ op: 'field-props', page: 0, widgetId: sig.id, rect: [72, 620, 100, 630] }, 'invalid', 'field-props: a signature field is at least 40x20 pt');
  refusal({ op: 'field-props', page: 0, widgetId: sig.id, default: 'x' }, 'invalid', 'field-props: default is not a signature property');
  refusal({ op: 'field-props', page: 0, widgetId: sig.id, options: ['a'] }, 'invalid', 'field-props: options are a dropdown property');
  refusal({ op: 'field-set', page: 0, widgetId: sig.id, value: 'x' }, 'signature', 'field-set: click the signature field to sign it');

  const deleted = dispatch(s, { op: 'field-delete', page: 0, widgetId: sig.id });
  assert.equal(deleted.ok, true, deleted.error);
  const reDeleted = openBytes(saveBytes(s), 'deleted.pdf').s;
  assert.deepEqual(fields(reDeleted).map(i => i.name), ['Signature2']);
  assert.equal(dispatch(s, { op: 'undo' }).ok, true);
  assert.deepEqual(fields(s).map(i => [i.id, i.name]), [[7, 'Signature1'], [8, 'Signature2']]);
});

test('P5 U3 sign-prepare {field} refuses a read-only field (§9.6)', async () => {
  const s = await open('hello.pdf');
  assert.equal(dispatch(s, { op: 'field-add', page: 0, kind: 'signature', name: 'RO', rect: [72, 72, 272, 132] }).ok, true);
  const item = byName(s, 'RO');
  assert.equal(dispatch(s, { op: 'field-props', page: 0, widgetId: item.id, readOnly: true }).ok, true);
  const { s: re } = openBytes(saveBytes(s), 'ro.pdf');
  assert.equal(byName(re, 'RO').readOnly, true);
  const r = refuses(re, request({ field: 'RO' }), 'invalid');
  assert.equal(r.error, 'sign-prepare: RO is read-only');
});

/** A one-page scratch PDF from object bodies (1 = catalog, 2 = pages, 3 = page), as tests/p2-decorate.test.mjs buildPdf. */
const buildPdf = objects => {
  const parts = ['%PDF-1.4\n'], offsets = [];
  objects.forEach((body, index) => { offsets.push(parts.join('').length); parts.push(`${index + 1} 0 obj\n${body}\nendobj\n`); });
  const xref = parts.join('').length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const offset of offsets) parts.push(`${String(offset).padStart(10, '0')} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  return bytesOf(Buffer.from(parts.join(''), 'latin1'));
};
const sigDoc = (fields, annots, ...rest) => buildPdf([
  `<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [${fields}] >> >>`,
  '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
  `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [${annots}] >>`,
  ...rest,
]);

test('P5 U3 sign-prepare {field} refuses what it cannot sign in place (§9.3, §1.4)', () => {
  const cases = [
    // A widget on /Annots that /AcroForm /Fields does not list: no verifier would see it.
    [sigDoc('', '4 0 R', '<< /Type /Annot /Subtype /Widget /FT /Sig /T (Orphan) /Rect [72 600 272 660] /F 4 /P 3 0 R >>'),
      'Orphan', 'sign-prepare: Orphan is not in the document\'s /AcroForm /Fields'],
    // A /Sig parent with one /T kid widget.
    [sigDoc('4 0 R', '5 0 R', '<< /FT /Sig /T (Parent) /Kids [5 0 R] >>', '<< /Type /Annot /Subtype /Widget /Parent 4 0 R /T (Kid) /Rect [72 600 272 660] /F 4 /P 3 0 R >>'),
      'Parent.Kid', 'sign-prepare: Parent.Kid is part of a field hierarchy'],
    // A 30x10 field.
    [sigDoc('4 0 R', '4 0 R', '<< /Type /Annot /Subtype /Widget /FT /Sig /T (Tiny) /Rect [72 112 102 122] /F 4 /P 3 0 R >>'),
      'Tiny', 'sign-prepare: Tiny is too small for a signature appearance'],
  ];
  for (const [bytes, field, message] of cases) {
    const { s } = openBytes(bytes, 'refuse.pdf');
    assert.equal(byName(s, field)?.kind, 'signature', `${field} is listed as a signature field`);
    assert.equal(refuses(s, request({ field }), 'invalid').error, message);
  }
});

test('P5 U3 a signed signature field cannot be changed (scratch document, not in /Fields)', () => {
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [] >> >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>',
    '<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signed) /Rect [72 600 272 660] /F 4 /P 3 0 R /V << /Type /Sig >> >>',
  ];
  const parts = ['%PDF-1.4\n'], offsets = [];
  objects.forEach((body, index) => { offsets.push(parts.join('').length); parts.push(`${index + 1} 0 obj\n${body}\nendobj\n`); });
  const xref = parts.join('').length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const offset of offsets) parts.push(`${String(offset).padStart(10, '0')} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  const { s, info } = openBytes(bytesOf(Buffer.from(parts.join(''), 'latin1')), 'signed-scratch.pdf');
  assert.equal(info.signatures, 0, 'the document stays writable');
  const item = byName(s, 'Signed');
  assert.equal(item.signed, true);
  const before = engine.journalState(s);
  for (const req of [
    { op: 'field-props', page: 0, widgetId: item.id, rect: [72, 72, 272, 132] },
    { op: 'field-delete', page: 0, widgetId: item.id },
  ]) {
    const r = dispatch(s, req);
    assert.equal(r.detail, 'signature');
    assert.match(r.error, /a signed field cannot be changed$/);
  }
  assert.deepEqual(engine.journalState(s), before);
  assert.equal(before.dirty, false);
});

test('P5 U3 pyHanko accepts every field signature', { skip: existsSync(py) ? false : 'no .venv-qa' }, () => {
  assert.equal(pyhankoCases.length, 6, 'every signing test ran');
  const dir = mkdtempSync(join(tmpdir(), 'p5u3-field-'));
  try {
    for (const [name, bytes, n, lines] of pyhankoCases) {
      const abs = join(dir, name);
      writeFileSync(abs, new Uint8Array(bytes));
      const r = spawnSync(py, ['scripts/verify-sign.py', abs, '--ca', join(root, 'fixtures/p0b/.ca/ca.pem'), '--expect-signatures', String(n)], { cwd: root, encoding: 'utf8' });
      assert.equal(r.status, 0, `${name}: ${r.stdout}${r.stderr}`);
      for (const line of lines) assert.match(r.stdout, line, `${name}: ${r.stdout}`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('debt1 row 11: /V on the field /Parent reads signed and sign-prepare refuses already-signed', async () => {
  const doc = mupdf.Document.openDocument(await readFile(join(root, 'fixtures', FIXTURE)), 'application/pdf').asPDF();
  let bytes;
  try {
    const widget = doc.newIndirect(7);
    const parent = doc.addObject({ FT: 'Sig', T: doc.newString('P') });
    parent.put('V', doc.addObject({ Type: 'Sig', Filter: 'Adobe.PPKLite', SubFilter: 'adbe.pkcs7.detached' }));
    const kids = doc.newArray();
    kids.push(widget);
    parent.put('Kids', kids);
    widget.put('Parent', parent);
    const list = doc.getTrailer().get('Root').get('AcroForm').get('Fields');
    for (let i = 0; i < list.length; i++) if (list.get(i).asIndirect() === 7) list.put(i, parent);
    bytes = bytesOf(doc.saveToBuffer('incremental=no').asUint8Array());
  } finally { doc.destroy(); }
  const { s } = openBytes(bytes, 'parent-v.pdf');
  const item = byName(s, 'P.Signature1');
  assert.ok(item, JSON.stringify(fields(s).map(f => f.name)));
  assert.equal(item.signed, true);
  assert.equal(byName(s, 'Signature2').signed, false);
  refuses(s, request({ field: 'P.Signature1' }), 'already-signed');
});

test('debt1 row 12: /Rotate inherited from /Pages is the page rotation and turns the signature /AP', async () => {
  const doc = mupdf.Document.openDocument(await readFile(join(root, 'fixtures/hello.pdf')), 'application/pdf').asPDF();
  let bytes;
  try {
    assert.equal(doc.findPage(0).get('Rotate').isNull(), true);
    doc.getTrailer().get('Root').get('Pages').put('Rotate', 90);
    bytes = bytesOf(doc.saveToBuffer('incremental=no').asUint8Array());
  } finally { doc.destroy(); }
  const s = engine.createSession();
  const opened = dispatch(s, { op: 'open', name: 'inherit-rotate.pdf', pdf: bytes });
  assert.equal(opened.ok, true, opened.error);
  assert.deepEqual(opened.rotations, [90]);
  const added = dispatch(s, { op: 'text-add', page: 0, point: [100, 100], text: 'x', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.equal(added.reason, 'unsupported-transform', added.error);
  const prepared = dispatch(s, request({ rect: [100, 100, 300, 200] }));
  assert.equal(prepared.ok, true, prepared.error);
  assert.match(text(prepared.bytes), /\/Matrix\[0 1 -1 0 0 0\]/);
});
