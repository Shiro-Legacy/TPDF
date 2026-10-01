/**
 * Coded simple fonts (contract-coded.md D1–D6): a symbolic simple TrueType with no
 * /Encoding, a 1-byte /ToUnicode and /Widths goes through the P3 U3 repertoire gate.
 * Engine direct. Fixture: fixtures/p0b/edit-coded.pdf (make-edit-coded.mjs), codes in
 * first-use order L1 o2 r3 e4 m5 ' '6 i7 p8 s9 u10 D11 l12 x13 x14 (13 and 14 both `x`).
 * Helpers are copies of tests/p3-edit.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';

const engine = createEngine(mupdf);
const FIXTURE = 'fixtures/p0b/edit-coded.pdf';
/** Code order of the page repertoire: every drawn character except the ambiguous `x`. */
const REPERTOIRE = 'Lorem ipsuDl';

function toArrayBuffer(bytes) {
  if (bytes instanceof ArrayBuffer) return bytes;
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

let nextId = 900;
async function openFile(path, session = engine.createSession()) {
  const reply = engine.dispatch({ id: nextId++, op: 'open', name: path, pdf: toArrayBuffer(await readFile(path)) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function openBytes(bytes, session = engine.createSession(), name = 'scratch.pdf') {
  const reply = engine.dispatch({ id: nextId++, op: 'open', name, pdf: toArrayBuffer(bytes) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function op(session, request) {
  return engine.dispatch({ id: nextId++, ...request }, session).reply;
}

function scan(session, page = 0) {
  const reply = op(session, { op: 'edit-targets', page });
  assert.equal(reply.ok, true, reply.error);
  return reply;
}

function itemOf(session, text) {
  const reply = scan(session);
  const item = reply.items.find(entry => entry.text === text);
  assert.ok(item, `no run ${JSON.stringify(text)} in ${JSON.stringify(reply.items.map(entry => [entry.text, entry.support, entry.reason]))}`);
  return { item, generation: reply.generation, revision: reply.revision, page: 0 };
}

function candidate(session, text) {
  const target = itemOf(session, text);
  assert.equal(target.item.support, 'candidate', `${text}: ${target.item.reason}`);
  return target;
}

function edit(session, target, text) {
  return op(session, {
    op: 'edit-text', page: target.page, targetId: target.item.targetId,
    generation: target.generation, revision: target.revision, text,
  });
}

function contentOf(session, page = 0) {
  const target = session.doc.loadPage(page);
  try {
    const buffer = target.getObject().get('Contents').readStream();
    try {
      return Buffer.from(buffer.asUint8Array()).toString('latin1');
    } finally {
      buffer.destroy();
    }
  } finally {
    target.destroy();
  }
}

function savedBytes(session) {
  const reply = op(session, { op: 'save' });
  assert.equal(reply.ok, true, reply.error);
  return new Uint8Array(reply.bytes);
}

function lines(bytes) {
  const document = mupdf.PDFDocument.openDocument(bytes, 'application/pdf');
  const page = document.loadPage(0);
  try {
    const structured = page.toStructuredText();
    try {
      return structured.asText().split('\n').filter(Boolean);
    } finally {
      structured.destroy();
    }
  } finally {
    page.destroy();
  }
}

/** edit-coded.pdf with its /F2 simple TrueType dictionary changed by `patch(document, font)`. */
async function codedPdf(patch) {
  const source = mupdf.PDFDocument.openDocument(new Uint8Array(await readFile(FIXTURE)), 'application/pdf');
  const font = source.loadPage(0).getObject().get('Resources').get('Font').get('F2').resolve();
  patch(source, font);
  return openBytes(source.saveToBuffer({ compress: true, garbage: true }).asUint8Array(), engine.createSession(), 'coded-scratch.pdf');
}

test('coded fixture: bytes match the provenance pin', async () => {
  const provenance = JSON.parse(await readFile('fixtures/p0b/edit-coded.provenance.json', 'utf8'));
  const bytes = await readFile(FIXTURE);
  assert.equal(bytes.length, provenance.bytes);
  assert.equal(createHash('sha256').update(bytes).digest('hex'), provenance.sha256);
  assert.equal(provenance.sha256, '8f6b3bfaf4eedf39003e107dbbabbea48f834c3be4b8ab489aa0e29be48191bf');
});

test('coded (a): every /F2 run is a candidate with the page repertoire (x is ambiguous), PDF4All stays a candidate', async () => {
  const session = await openFile(FIXTURE);
  for (const text of ['Lorem ipsum', 'Dolor', 'xx']) {
    const { item } = candidate(session, text);
    assert.equal(item.repertoire, REPERTOIRE, text);
    assert.equal(item.font.subtype, 'TrueType', text);
    assert.equal(item.font.subset, true, text);
    assert.equal(item.font.embedded, true, text);
    assert.ok(Array.isArray(item.bounds) && item.bounds.length === 4, `${text} has stext bounds`);
  }
  const control = candidate(session, 'PDF4All');
  assert.equal(control.item.repertoire, undefined);
});

test('coded (b): Lorem ipsum -> ipsum Lorem writes 2 hex digits per code; undo restores every byte', async () => {
  const session = await openFile(FIXTURE);
  const before = contentOf(session);
  assert.ok(before.includes('<0102030405060708090a05> Tj'), before);
  const reply = edit(session, candidate(session, 'Lorem ipsum'), 'ipsum Lorem');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const after = contentOf(session);
  assert.ok(after.includes('<0708090a05060102030405> Tj'), after);
  assert.equal(after.includes('<0102030405060708090a05>'), false, 'the old operand is gone');
  assert.equal(after.includes('ActualText'), false, 'no tag is invented: the /ToUnicode map already extracts the new text');
  assert.ok(after.includes('(PDF4All) Tj'), 'the control run is untouched');
  const text = lines(savedBytes(session));
  assert.equal(text[1], 'ipsum Lorem', JSON.stringify(text));
  assert.ok(text.includes('PDF4All') && text.includes('Dolor') && text.includes('xx'), JSON.stringify(text));
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentOf(session), before);
});

/** edit-coded.pdf with its page content stream patched by `transform` (the font is untouched). */
async function codedContent(transform) {
  const source = mupdf.PDFDocument.openDocument(new Uint8Array(await readFile(FIXTURE)), 'application/pdf');
  const page = source.loadPage(0);
  try {
    const contents = page.getObject().get('Contents');
    const buffer = contents.readStream();
    let text;
    try {
      text = Buffer.from(buffer.asUint8Array()).toString('latin1');
    } finally {
      buffer.destroy();
    }
    contents.writeStream(Buffer.from(transform(text), 'latin1'));
  } finally {
    page.destroy();
  }
  return openBytes(source.saveToBuffer({ compress: true, garbage: true }).asUint8Array(), engine.createSession(), 'coded-tagged.pdf');
}

test('coded (b2): an inline ActualText equal to the decoded text travels with the rewrite (as U3)', async () => {
  const session = await codedContent(text => text.replace('BT /F2 28 Tf 72 660 Td <0102030405060708090a05> Tj ET',
    '/Span <</ActualText (Lorem ipsum)>> BDC BT /F2 28 Tf 72 660 Td <0102030405060708090a05> Tj ET EMC'));
  assert.equal(edit(session, candidate(session, 'Lorem ipsum'), 'ipsum Lorem').ok, true);
  const after = contentOf(session);
  assert.ok(after.includes('/ActualText (ipsum Lorem)'), after);
  assert.ok(after.includes('<0708090a05060102030405> Tj'), after);
  assert.equal(lines(savedBytes(session))[1], 'ipsum Lorem');
});

test('coded (c): a character outside the page repertoire is refused with the missing list, nothing written', async () => {
  const session = await openFile(FIXTURE);
  const before = contentOf(session);
  const lorez = edit(session, candidate(session, 'Lorem ipsum'), 'ipsum Lorez');
  assert.equal(lorez.ok, false);
  assert.equal(lorez.detail, 'unsupported-encoding');
  assert.deepEqual(lorez.missing, ['z']);
  const dolx = edit(session, candidate(session, 'Dolor'), 'Dolx');
  assert.equal(dolx.ok, false);
  assert.equal(dolx.detail, 'unsupported-encoding');
  assert.deepEqual(dolx.missing, ['x']);
  assert.equal(contentOf(session), before);
});

test('coded (d): a replacement wider than the free space is refused as overflow', async () => {
  const session = await openFile(FIXTURE);
  const before = contentOf(session);
  const reply = edit(session, candidate(session, 'Dolor'), 'Dolor Dolor Dolor Dolor Dolor Dolor Dolor');
  assert.equal(reply.ok, false);
  assert.equal(reply.detail, 'overflow', JSON.stringify(reply));
  assert.equal(contentOf(session), before);
});

test('coded (e): the coded path never fires when one D1 condition fails', async () => {
  const cases = {
    'no /ToUnicode': (document, font) => font.delete('ToUnicode'),
    '/Flags 32': (document, font) => font.get('FontDescriptor').put('Flags', 32),
    '/Encoding /WinAnsiEncoding': (document, font) => font.put('Encoding', document.newName('WinAnsiEncoding')),
    'no /Widths': (document, font) => font.delete('Widths'),
    'no /FirstChar': (document, font) => font.delete('FirstChar'),
    'no font program': (document, font) => font.get('FontDescriptor').delete('FontFile2'),
  };
  for (const [label, patch] of Object.entries(cases)) {
    const session = await codedPdf(patch);
    const runs = scan(session).items.filter(entry => entry.kind === 'text' && entry.font?.name === 'NotoSansHebrew');
    assert.equal(runs.length, 3, label);
    for (const run of runs) {
      assert.equal(run.support, 'unsupported', `${label}: ${run.text}`);
      assert.equal(run.reason, 'unsupported-encoding', `${label}: ${run.text}`);
      assert.equal(run.repertoire, undefined, label);
    }
    assert.equal(itemOf(session, 'PDF4All').item.support, 'candidate', label);
  }
});

test('coded (f): other fixtures scan exactly as before the unit', async () => {
  // Numbers from the pre-unit build (.quad/shared/p6/coded-build.md).
  const before = {
    'fixtures/p0b/edit-subset.pdf': { items: 3, reasons: { 'text:candidate:-': 3 } },
    'fixtures/p0b/edit-transformed.pdf': { items: 3, reasons: { 'image:candidate:unsupported-transform': 1, 'text:candidate:-': 2 } },
    'fixtures/p6/paragraph.pdf': { items: 9, reasons: { 'text:candidate:-': 8, 'image:candidate:-': 1 } },
  };
  for (const [path, want] of Object.entries(before)) {
    const items = scan(await openFile(path)).items;
    const reasons = {};
    for (const item of items) {
      const key = `${item.kind}:${item.support}:${item.reason ?? '-'}`;
      reasons[key] = (reasons[key] ?? 0) + 1;
    }
    assert.deepEqual({ items: items.length, reasons }, want, path);
  }
});

test('coded (g): Tw widens the single-byte code 32 (review): a space-adding edit under a large Tw overflows', async () => {
  // Space moves from code 6 to code 32 (ToUnicode, /Widths extended to 32 entries, content bytes); the
  // Dolor line gets 200 Tw. PDF: Tw applies to every single-byte code 32 of a simple font.
  const source = mupdf.PDFDocument.openDocument(new Uint8Array(await readFile(FIXTURE)), 'application/pdf');
  const page = source.loadPage(0).getObject();
  const font = page.get('Resources').get('Font').get('F2').resolve();
  const widths = font.get('Widths');
  const next = source.newArray();
  for (let index = 0; index < 32; index++) next.push(index === 31 ? widths.get(5).asNumber() : index < widths.length && index !== 5 ? widths.get(index).asNumber() : 0);
  font.put('Widths', next);
  font.put('LastChar', 32);
  const cmap = font.get('ToUnicode');
  cmap.writeStream(Buffer.from(Buffer.from(cmap.readStream().asUint8Array()).toString('latin1').replace('<06> <0020>', '<20> <0020>'), 'latin1'));
  const contents = page.get('Contents');
  contents.writeStream(Buffer.from(Buffer.from(contents.readStream().asUint8Array()).toString('latin1')
    .replace('<0102030405060708090a05>', '<0102030405200708090a05>')
    .replace('BT /F2 28 Tf 72 600 Td', 'BT /F2 28 Tf 200 Tw 72 600 Td'), 'latin1'));
  const session = openBytes(source.saveToBuffer({ compress: true, garbage: true }).asUint8Array(), engine.createSession(), 'coded-tw.pdf');
  const target = candidate(session, 'Dolor');
  assert.equal(target.item.repertoire, 'LoremipsuDl ');
  const before = contentOf(session);
  const reply = edit(session, target, 'Do lo');
  assert.equal(reply.ok, false, JSON.stringify(reply));
  assert.equal(reply.detail, 'overflow');
  // D o ' ' l o at 28 pt from /Widths, plus 200 Tw for the one code 32.
  const glyphs = [11, 2, 32, 12, 2].reduce((total, code) => total + next.get(code - 1).asNumber() * 28 / 1000, 0);
  assert.equal(reply.newWidth, Math.round((glyphs + 200) * 100) / 100);
  assert.equal(contentOf(session), before);
});
