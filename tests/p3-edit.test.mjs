/**
 * P3 U2 — true text edit: `edit-text` on a U1 candidate. Engine direct.
 *
 * Expected values come from the fixture bytes and from the contract's width rule
 * (contract-p3-unit2.md §2, archived in .quad/backup/history-20260924.tgz): the replacement is refused unless it fits where the
 * old run ends, and a run with text following it on the same line must keep its
 * exact width. Helvetica advances are the font's own, so the numbers below are
 * measured in the test rather than guessed (ALPHA 39.35pt, BRAVO 42.01pt at 12pt).
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';

const engine = createEngine(mupdf);

function toArrayBuffer(bytes) {
  if (bytes instanceof ArrayBuffer) return bytes;
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

let nextId = 800;
async function openFile(path, session = engine.createSession()) {
  const reply = engine.dispatch(
    { id: 1, op: 'open', name: path, pdf: toArrayBuffer(await readFile(path)) },
    session,
  ).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

async function open(id, session = engine.createSession()) {
  return openFile(`fixtures/${id}.pdf`, session);
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

function candidate(session, text, page = 0) {
  const reply = scan(session, page);
  const item = reply.items.find(entry => entry.support === 'candidate' && entry.text === text);
  assert.ok(item, `no candidate ${JSON.stringify(text)} in ${JSON.stringify(reply.items.map(entry => entry.text))}`);
  return { item, generation: reply.generation, revision: reply.revision, page };
}

function edit(session, target, text) {
  return op(session, {
    op: 'edit-text', page: target.page, targetId: target.item.targetId,
    generation: target.generation, revision: target.revision, text,
  });
}

/** The decoded /Contents bytes of one page, straight from the session document. */
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

function savedText(session) {
  const reply = op(session, { op: 'save' });
  assert.equal(reply.ok, true, reply.error);
  return Buffer.from(new Uint8Array(reply.bytes)).toString('latin1');
}

// --------------------------------------------------------------- synthetic PDFs

const LATIN = 'latin1';
function stream(body) {
  const bytes = Buffer.isBuffer(body) ? body : Buffer.from(body, LATIN);
  return Buffer.concat([Buffer.from(`<< /Length ${bytes.length} >>\nstream\n`, LATIN), bytes, Buffer.from('endstream', LATIN)]);
}
function rawPdf(objects) {
  const head = Buffer.from('%PDF-1.4\n%\xe2\xe3\xcf\xd3\n', LATIN);
  const parts = [head];
  const offsets = [];
  let size = head.length;
  for (const [index, body] of objects.entries()) {
    const bytes = Buffer.isBuffer(body) ? body : Buffer.from(body, LATIN);
    const prefix = Buffer.from(`${index + 1} 0 obj\n`, LATIN);
    const suffix = Buffer.from('\nendobj\n', LATIN);
    offsets.push(size);
    parts.push(prefix, bytes, suffix);
    size += prefix.length + bytes.length + suffix.length;
  }
  const xref = size;
  let tail = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) tail += `${String(offset).padStart(10, '0')} 00000 n \n`;
  tail += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  parts.push(Buffer.from(tail, LATIN));
  return Buffer.concat(parts);
}
const HELVETICA = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>';
function onePage(content) {
  return rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
    HELVETICA,
    stream(content),
  ]);
}

// ------------------------------------------------------------------ happy path

test('replace: ALPHA -> BRAV rewrites the operand only, and undo restores the bytes', async () => {
  const session = await openFile('fixtures/p0b/edit-transformed.pdf');
  const before = contentOf(session);
  assert.equal(before.includes('(ALPHA) Tj'), true);

  const target = candidate(session, 'ALPHA');
  const reply = edit(session, target, 'BRAV');
  assert.equal(reply.ok, true, `${reply.error} ${reply.detail ?? ''}`);
  assert.equal(reply.revision, 1);
  assert.equal(reply.dirty, true);
  assert.equal(reply.canUndo, true);
  assert.equal(reply.page, 0);

  const after = contentOf(session);
  assert.equal(after.includes('(BRAV) Tj'), true);
  assert.equal(after.includes('(ALPHA)'), false, 'the old operator is gone from the stream');
  assert.equal(after.includes('(NEIGHBOR) Tj'), true, 'the neighbouring run is untouched');
  assert.equal(after.replace('(BRAV)', '(ALPHA)'), before, 'only the operand bytes changed');

  const rescan = scan(session);
  assert.deepEqual(rescan.items.filter(item => item.kind === 'text').map(item => item.text), ['BRAV', 'NEIGHBOR']);

  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentOf(session), before, 'undo restores the stream byte-for-byte');
});

test('delete: the operator keeps an empty string and the run stops being a candidate', async () => {
  const session = await openFile('fixtures/p0b/edit-transformed.pdf');
  const target = candidate(session, 'ALPHA');
  const reply = edit(session, target, '');
  assert.equal(reply.ok, true, reply.error);
  const after = contentOf(session);
  assert.equal(after.includes('() Tj'), true, 'the operator stays, with an empty string');
  assert.equal(after.includes('ALPHA'), false);
  assert.equal(savedText(session).includes('ALPHA'), false, 'the saved file carries no ALPHA');

  const rescan = scan(session);
  const empty = rescan.items.find(item => item.kind === 'text');
  assert.ok(empty, JSON.stringify(rescan.items));
  assert.equal(empty.support, 'unsupported');
  assert.equal(empty.reason, 'unsupported-text-operator');
});

// ----------------------------------------------------------------- width gate

test('width gate: the numbers are real and growth past the old run is refused', async () => {
  const session = await openFile('fixtures/p0b/edit-transformed.pdf');
  const target = candidate(session, 'ALPHA');
  const reply = edit(session, target, 'BRAVO');
  assert.equal(reply.ok, false);
  assert.equal(reply.detail, 'overflow');
  assert.equal(reply.reason, 'invalid');
  // The fixture draws this run at 12pt scaled by 2, so stext reports 24pt.
  assert.equal(Math.round(reply.oldWidth), 79, JSON.stringify(reply));
  assert.equal(Math.round(reply.newWidth), 84, JSON.stringify(reply));
  assert.equal(contentOf(session).includes('(ALPHA) Tj'), true, 'a refused edit writes nothing');

  // Same character count, slightly narrower: allowed and inside the old bounds.
  const same = edit(session, target, 'ALPHB');
  assert.equal(same.ok, true, `${same.error} ${same.detail ?? ''}`);
});

test('width gate: text following on the same line must keep the width exactly', async () => {
  // Line 1 `(ALPHA) Tj ( TAIL) Tj` (a tail follows); line 2 is repositioned first.
  const session = openBytes(onePage(
    'BT /F1 12 Tf 72 700 Td (ALPHA) Tj ( TAIL) Tj ET\n'
    + 'BT /F1 12 Tf 72 660 Td (BETA) Tj 120 0 Td (FAR) Tj ET\n',
  ));
  const reply = scan(session);
  const first = reply.items[0];
  const second = reply.items[3];
  assert.equal(first.tailFollows, true, JSON.stringify(reply.items));
  assert.equal(first.reason, undefined, 'ALPHA is unique on this page');
  assert.equal(second.tailFollows, false);

  const moved = edit(session, { item: first, generation: reply.generation, revision: reply.revision, page: 0 }, 'BRAVO');
  assert.equal(moved.ok, true, `${moved.error} ${moved.detail ?? ''}`);
  assert.ok(Math.abs(moved.shift - 2.66) < 0.01, 'the tail shifts by the measured width delta');

  const sameWidth = edit(session, candidate(session, 'BRAVO'), 'BRAVB');
  assert.equal(sameWidth.ok, true, `${sameWidth.error} ${sameWidth.detail ?? ''}`);
});

// ------------------------------------------------------------ actual text (BDC)

test('tagged text: a matching /ActualText is rewritten with the operand, a mismatched one is never a candidate', async () => {
  const matching = openBytes(onePage(
    '/Span << /MCID 3 /ActualText (ALPHA) >> BDC\nBT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\nEMC\n',
  ));
  const target = candidate(matching, 'ALPHA');
  const reply = edit(matching, target, 'ALPHB');
  assert.equal(reply.ok, true, `${reply.error} ${reply.detail ?? ''}`);
  const after = contentOf(matching);
  assert.equal(after.includes('(ALPHB) Tj'), true);
  assert.equal(after.includes('/ActualText (ALPHB)'), true, 'the tag travelled with the operand');
  assert.equal(after.includes('ALPHA'), false);
  assert.equal(after.includes('/MCID 3'), true, 'the MCID is left alone');

  const mismatched = openBytes(onePage(
    '/Span << /ActualText (Alpha) >> BDC\nBT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\nEMC\n',
  ));
  const scanReply = scan(mismatched);
  assert.equal(scanReply.items[0].support, 'unsupported');
  assert.equal(scanReply.items[0].reason, 'ambiguous-text-association');
  const refused = edit(mismatched, { item: scanReply.items[0], generation: scanReply.generation, revision: scanReply.revision, page: 0 }, 'ALPHB');
  assert.equal(refused.ok, false);
  assert.equal(refused.detail, 'not-candidate');
  assert.equal(refused.reason, 'ambiguous-text-association');
});

// ------------------------------------------------------------------- refusals

test('every refusal token is reachable and none of them writes', async () => {
  const session = await openFile('fixtures/p0b/edit-transformed.pdf');
  const before = contentOf(session);
  const target = candidate(session, 'ALPHA');

  const huge = edit(session, target, 'A'.repeat(4097));
  assert.equal(huge.detail, 'too-large');

  const wrongType = op(session, {
    op: 'edit-text', page: 0, targetId: target.item.targetId,
    generation: target.generation, revision: target.revision, text: 7,
  });
  assert.equal(wrongType.detail, 'unsupported-encoding');

  // Same width as ALPHA in Helvetica (A and \u00c4 are both 667 units), so the
  // edit fits whichever way the engine measures \u00c4; MuPDF measured it as 333.
  const nonAscii = edit(session, target, 'ALPH\u00c4');
  assert.equal(nonAscii.ok, true, nonAscii.error);
  const nonAsciiTarget = candidate(session, 'ALPH\u00c4');
  assert.ok(nonAsciiTarget);

  // A stale handle: any accepted mutation moves the revision on.
  const moved = edit(session, nonAsciiTarget, 'ALPHB');
  assert.equal(moved.ok, true, moved.error);
  const stale = edit(session, target, 'ALPHC');
  assert.equal(stale.ok, false);
  assert.equal(stale.detail, 'stale-target');
  assert.equal(contentOf(session).includes('(ALPHB) Tj'), true);

  // A locked document refuses through requireMutable before any detail.
  const locked = await openFile('fixtures/encrypted-noedit.pdf');
  const lockedReply = op(locked, { op: 'edit-text', page: 0, targetId: 'x', generation: 0, revision: 0, text: 'A' });
  assert.equal(lockedReply.ok, false);
  assert.equal(lockedReply.reason, 'read-only');
  assert.equal(lockedReply.detail, undefined, 'permission refusals keep the worker reason, not a detail');
  assert.equal(before.includes('(ALPHA)'), true);
});

test('unsupported targets refuse as not-candidate and carry the U1 reason', async () => {
  for (const [path, reason] of [
    ['fixtures/rtl.pdf', 'rtl-or-shaping'], // U3: the Hebrew run decodes, then refuses as read-only
    ['fixtures/p3/targets-streams.pdf', 'shared-or-unproven-sharing'], // the Fm1 nested inside Fm2 (depth 2)
    ['fixtures/layers.pdf', 'unsupported-graphics-state'],
  ]) {
    const session = await openFile(path);
    const reply = scan(session);
    const item = reply.items.find(entry => entry.kind === 'text' && entry.support === 'unsupported' && entry.reason === reason);
    assert.ok(item, path);
    const refused = edit(session, { item, generation: reply.generation, revision: reply.revision, page: 0 }, 'ALPHB');
    assert.equal(refused.ok, false, path);
    assert.equal(refused.detail, 'not-candidate', `${path}: ${JSON.stringify(refused)}`);
    assert.equal(refused.reason, reason, path);
  }
});

test('an image target is never editable as text', async () => {
  const session = await openFile('fixtures/p0b/edit-transformed.pdf');
  const reply = scan(session);
  const image = reply.items.find(item => item.kind === 'image');
  assert.ok(image);
  const refused = edit(session, { item: image, generation: reply.generation, revision: reply.revision, page: 0 }, 'X');
  assert.equal(refused.ok, false);
  assert.equal(refused.detail, 'not-candidate');
});

// ---------------------------------------------------------- inline images (U2)

test('an unfiltered inline image without /L is skipped by its own size; a filtered one still refuses', async () => {
  const unfiltered = onePage(Buffer.concat([
    Buffer.from('BT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\nq\nBI /W 1 /H 1 /CS /DeviceGray /BPC 8\nID ', LATIN),
    Buffer.from([0x40]),
    Buffer.from('\nEI\nQ\nBT /F1 12 Tf 72 650 Td (ALPHB) Tj ET\n', LATIN),
  ]));
  const reply = scan(openBytes(unfiltered));
  assert.deepEqual(reply.items.map(item => item.text), ['ALPHA', 'ALPHB'], 'the 1-byte image data is not text');

  const filtered = onePage('q BI /W 1 /H 1 /CS /DeviceGray /BPC 8 /F /ASCIIHexDecode ID 40\nEI Q\n');
  const refused = op(openBytes(filtered), { op: 'edit-targets', page: 0 });
  assert.equal(refused.ok, false);
  assert.equal(refused.reason, 'invalid');
  assert.match(refused.error, /no declared \/L/);
});

// -------------------------------------------------------------- saved output

test('the saved file loses the old text and keeps the neighbours, tags and link', async () => {
  const session = await openBytes(onePage(
    '/Span << /MCID 0 /ActualText (ALPHA) >> BDC\nBT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\nEMC\n'
    + 'BT /F1 12 Tf 72 650 Td (NEIGHBOR) Tj ET\n',
  ));
  const target = candidate(session, 'ALPHA');
  assert.equal(edit(session, target, 'ALPHB').ok, true);

  const saved = savedText(session);
  assert.equal(saved.includes('ALPHA'), false, 'no ALPHA anywhere in the saved bytes');
  assert.equal(saved.includes('ALPHB'), true);
  assert.equal(saved.includes('NEIGHBOR'), true);

  // The copy reopens with the same targets, and the engine's own validate agrees.
  const reopened = openBytes(toArrayBufferBytes(saved), engine.createSession(), 'saved.pdf');
  const valid = op(reopened, { op: 'validate', pdf: toArrayBufferBytes(saved) });
  assert.equal(valid.ok, true, valid.error);
  assert.deepEqual(scan(reopened).items.map(item => item.text), ['ALPHB', 'NEIGHBOR']);
});

function toArrayBufferBytes(text) {
  const bytes = Buffer.from(text, LATIN);
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

// ------------------------------------------------- struct-tree /ActualText (U2)

/**
 * A tagged page whose StructElem carries `/ActualText`, reachable the way
 * fixtures/p0b/edit-tagged.pdf does it: page `/StructParents 0` -> ParentTree
 * `/Nums [0 [8 0 R]]` -> elem 8. `structActual` is what that elem says.
 */
function taggedPdf(content, structActual = 'ALPHA', extraStruct = '') {
  return rawPdf([
    '<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 6 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R /StructParents 0 >>',
    HELVETICA,
    stream(content),
    '<< /Type /StructTreeRoot /ParentTree 7 0 R >>',
    '<< /Nums [0 [8 0 R]] >>',
    `<< /Type /StructElem /S /Span /P 6 0 R /Pg 3 0 R /K 0 /ActualText (${structActual}) ${extraStruct} >>`,
  ]);
}

test('struct-tree ActualText: rewritten in the same operation, and undo restores it', async () => {
  const content = '/Span << /MCID 0 >> BDC\nBT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\nEMC\n';
  const session = openBytes(taggedPdf(content));
  const target = candidate(session, 'ALPHA');
  const reply = edit(session, target, 'ALPHB');
  assert.equal(reply.ok, true, `${reply.error} ${reply.detail ?? ''}`);

  const saved = savedText(session);
  assert.equal(saved.includes('ActualText(ALPHB)'), true, 'the StructElem string changed');
  assert.equal(saved.includes('ActualText(ALPHA)'), false);
  assert.equal(saved.includes('/MCID 0'), true, 'the MCID and the marked content stay');

  // The saved copy's own text extraction must agree with the operand.
  const reopened = mupdf.Document.openDocument(toArrayBufferBytes(saved), 'application/pdf');
  try {
    const structured = reopened.loadPage(0).toStructuredText();
    try {
      assert.equal(structured.asText().trim(), 'ALPHB');
    } finally {
      structured.destroy();
    }
  } finally {
    reopened.destroy();
  }

  assert.equal(op(session, { op: 'undo' }).ok, true);
  const undone = savedText(session);
  assert.equal(undone.includes('ActualText(ALPHA)'), true, 'undo restores the StructElem string');
  assert.equal(undone.includes('ActualText(ALPHB)'), false);
});

test('struct-tree ActualText that differs, or an inline plus a struct one, is never a candidate', async () => {
  const plain = '/Span << /MCID 0 >> BDC\nBT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\nEMC\n';
  const differs = openBytes(taggedPdf(plain, 'Alpha'));
  assert.equal(scan(differs).items[0].reason, 'ambiguous-text-association');

  // Both an inline BDC ActualText and a StructElem one: which is authoritative is
  // not resolvable, so the run stays read-only.
  const both = '/Span << /MCID 0 /ActualText (ALPHA) >> BDC\nBT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\nEMC\n';
  assert.equal(scan(openBytes(taggedPdf(both))).items[0].reason, 'ambiguous-text-association');
});

test('a page with no StructParents still edits its inline tag and leaves /Alt alone', async () => {
  const content = '/Span << /ActualText (ALPHA) /Alt (alternate) >> BDC\nBT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\nEMC\n';
  const session = openBytes(onePage(content));
  const target = candidate(session, 'ALPHA');
  assert.equal(edit(session, target, 'ALPHB').ok, true);
  const saved = savedText(session);
  // The content stream keeps its original spacing; only the spliced span changes.
  assert.equal(saved.includes('/ActualText (ALPHB)'), true);
  assert.equal(saved.includes('/ActualText (ALPHA)'), false);
  assert.equal(saved.includes('/Alt (alternate)'), true, '/Alt is never touched');
});

test('fixtures/p3/edit-tail.pdf: the tail, tag and optional-content cases as its manifest describes', async () => {
  const session = await openFile('fixtures/p3/edit-tail.pdf');
  const reply = scan(session);
  assert.deepEqual(reply.items.map(item => `${item.support}:${item.text}`), [
    'candidate:ALPHA', 'candidate: TAIL', 'candidate:BETA', 'candidate:FAR',
    'unsupported:GAMMA', 'unsupported:DELTA',
  ], JSON.stringify(reply.items));
  const byText = (text) => reply.items.find(item => item.text === text);
  assert.equal(byText('ALPHA').tailFollows, true, 'a text-show follows with no repositioning');
  assert.equal(byText(' TAIL').tailFollows, false);
  assert.equal(byText('BETA').tailFollows, false, 'a 120 0 Td repositions before FAR');
  assert.equal(byText('GAMMA').reason, 'ambiguous-text-association', 'ActualText Gamma differs from the payload');
  assert.equal(byText('DELTA').reason, 'unsupported-graphics-state', 'the run sits on an optional-content layer');
  const before = contentOf(session);
  const target = (text) => ({ item: byText(text), generation: reply.generation, revision: reply.revision, page: 0 });

  const moved = edit(session, target('ALPHA'), 'BETA');
  assert.equal(moved.ok, true, `${moved.error} ${moved.detail ?? ''}`);
  assert.ok(Math.abs(moved.shift + 8.01) < 0.01, 'the following run shifts by the measured width delta');
  const freshReply = scan(session);
  const freshTarget = (text) => ({ item: freshReply.items.find(item => item.text === text), generation: freshReply.generation, revision: freshReply.revision, page: 0 });
  const gamma = edit(session, freshTarget('GAMMA'), 'GAMMB');
  assert.equal(gamma.detail, 'not-candidate');
  const delta = edit(session, freshTarget('DELTA'), 'DELTB');
  assert.equal(delta.detail, 'not-candidate');
  assert.equal(contentOf(session).includes('(BETA) Tj'), true);
});

// ------------------------------------------------------- U1b copy-on-write

function pageText(session, page) {
  const target = session.doc.loadPage(page);
  try {
    const structured = target.toStructuredText();
    try { return structured.asText(); } finally { structured.destroy(); }
  } finally { target.destroy(); }
}
function pageDict(session, page, fn) {
  const target = session.doc.loadPage(page);
  try { return fn(target.getObject()); } finally { target.destroy(); }
}
/** `{ name: objectNumber }` of the page's own XObjects, or null without own resources. */
function xobjectsOf(session, page) {
  return pageDict(session, page, object => {
    const resources = object.get('Resources');
    if (!resources || resources.isNull()) return null;
    const out = {};
    const xobjects = resources.get('XObject');
    if (xobjects && !xobjects.isNull()) xobjects.forEach((value, key) => { out[key] = value.asIndirect(); });
    return out;
  });
}
function xobjectBytes(session, page, name) {
  return pageDict(session, page, object => {
    const buffer = object.get('Resources').get('XObject').get(name).readStream();
    try { return Buffer.from(buffer.asUint8Array()).toString('latin1'); } finally { buffer.destroy(); }
  });
}
/** The first /Contents element (scalar or array) as an object number, or its decoded text. */
function contentsNumber(session, page) {
  return pageDict(session, page, object => { const c = object.get('Contents'); return (c.isArray() ? c.get(0) : c).asIndirect(); });
}
function firstContent(session, page) {
  return pageDict(session, page, object => {
    const c = object.get('Contents');
    const buffer = (c.isArray() ? c.get(0) : c).readStream();
    try { return Buffer.from(buffer.asUint8Array()).toString('latin1'); } finally { buffer.destroy(); }
  });
}
function cowTarget(session, text, page = 0, which = 0) {
  const reply = scan(session, page);
  const items = reply.items.filter(entry => entry.kind === 'text' && entry.text === text && entry.support === 'candidate');
  const item = items[which];
  assert.ok(item, `no candidate ${text}#${which}: ${JSON.stringify(reply.items)}`);
  assert.equal(item.cowable, true, `${text} must be cowable`);
  return { item, generation: reply.generation, revision: reply.revision, page };
}
function objectStream(dict, body) {
  const bytes = Buffer.isBuffer(body) ? body : Buffer.from(body, LATIN);
  return Buffer.concat([Buffer.from(`${dict} /Length ${bytes.length} >>\nstream\n`, LATIN), bytes, Buffer.from('endstream', LATIN)]);
}

test('U1b edit-shared.pdf: editing the first /Fm1 placement grafts a private copy and leaves the rest byte-identical', async () => {
  const session = await openFile('fixtures/p0b/edit-shared.pdf');
  const before = { c0: contentOf(session, 0), c1: contentOf(session, 1), xo: xobjectsOf(session, 0), fm1: xobjectBytes(session, 0, 'Fm1') };
  const target = cowTarget(session, 'ALPHA');
  const reply = edit(session, target, 'BRAV');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const text0 = pageText(session, 0);
  assert.match(text0, /BRAV/);
  assert.match(text0, /ALPHA/, 'the second placement still draws the original');
  assert.match(text0, /NEIGHBOR/);
  assert.equal(contentOf(session, 1), before.c1, 'page 2 is untouched');
  assert.match(pageText(session, 1), /PAGE2/);
  const after0 = contentOf(session, 0);
  assert.equal(after0.split('/Fm1c1 Do').length - 1, 1, 'exactly the selected Do is renamed');
  assert.equal(after0.split('/Fm1 Do').length - 1, 1, 'the other placement keeps /Fm1');
  const xo = xobjectsOf(session, 0);
  assert.notEqual(xo.Fm1c1, xo.Fm1, 'two distinct Form objects');
  assert.equal(xo.Fm1, before.xo.Fm1);
  assert.equal(xo.Im1, before.xo.Im1, 'Im1 untouched');
  assert.equal(xobjectBytes(session, 0, 'Fm1'), before.fm1, 'the original Form bytes are untouched');
  assert.match(xobjectBytes(session, 0, 'Fm1c1'), /\(BRAV\) Tj/);
  // One undo step removes the copy, the resource entry and the Do rename.
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentOf(session, 0), before.c0);
  assert.deepEqual(xobjectsOf(session, 0), before.xo, 'no /Fm1c1 after undo');
  assert.doesNotMatch(pageText(session, 0), /BRAV/);
  // Redo, save, reopen: the surviving placement keeps ALPHA, the edited one shows BRAVO.
  assert.equal(op(session, { op: 'redo' }).ok, true);
  const reopened = openBytes(Buffer.from(savedText(session), LATIN));
  assert.match(pageText(reopened, 0), /BRAV/);
  assert.match(pageText(reopened, 0), /ALPHA/);
  assert.match(pageText(reopened, 1), /PAGE2/);
  assert.doesNotMatch(pageText(reopened, 1), /BRAV/);
});

test('U1b targets-streams.pdf: the first /Fm1 Do edits, the Fm1 nested in Fm2 refuses, page 2 keeps its shared Fm1', async () => {
  const session = await openFile('fixtures/p3/targets-streams.pdf');
  const beforeP1 = { c: contentOf(session, 1), fm1: xobjectBytes(session, 1, 'Fm1'), xo: xobjectsOf(session, 1) };
  const scanned = scan(session);
  const nested = scanned.items.find(entry => entry.kind === 'text' && entry.text === 'FORM' && entry.support === 'unsupported');
  assert.equal(nested.reason, 'shared-or-unproven-sharing', 'depth-2 descendant stays refused');
  const refused = edit(session, { item: nested, generation: scanned.generation, revision: scanned.revision, page: 0 }, 'FARM');
  assert.equal(refused.detail, 'not-candidate');
  const reply = edit(session, cowTarget(session, 'FORM'), 'FARM');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const text0 = pageText(session, 0);
  assert.equal((text0.match(/FARM/g) ?? []).length, 1);
  assert.equal((text0.match(/FORM/g) ?? []).length, 2, 'the second direct placement and the nested one keep FORM');
  assert.equal(contentOf(session, 1), beforeP1.c);
  assert.equal(xobjectBytes(session, 1, 'Fm1'), beforeP1.fm1, 'page 2 still draws the shared original');
  assert.deepEqual(xobjectsOf(session, 1), beforeP1.xo);
  assert.doesNotMatch(pageText(session, 1), /FARM/);
});

test('U1b shared /Contents: the edited page gets a private stream, the other page keeps the shared one', async () => {
  const bytes = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents [6 0 R] >>',
    HELVETICA,
    stream('BT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\n'),
  ]);
  for (const page of [0, 1]) {
    const session = openBytes(bytes);
    const other = page === 0 ? 1 : 0;
    const reply = edit(session, cowTarget(session, 'ALPHA', page), 'BRAV');
    assert.equal(reply.ok, true, JSON.stringify(reply));
    assert.match(pageText(session, page), /BRAV/);
    assert.match(pageText(session, other), /ALPHA/);
    assert.doesNotMatch(pageText(session, other), /BRAV/);
    assert.equal(firstContent(session, other).includes('(ALPHA) Tj'), true);
    assert.notEqual(contentsNumber(session, page), 6, 'the edited page owns a new stream');
    assert.equal(op(session, { op: 'undo' }).ok, true);
    assert.equal(contentsNumber(session, page), 6, 'undo restores the shared reference');
    assert.match(pageText(session, page), /ALPHA/);
  }
});

test('U1b inherited /Resources plus a Form invocation: both privatised in one step, both undone in one step', async () => {
  const bytes = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources << /Font << /F1 5 0 R >> /XObject << /Fm1 6 0 R >> >> >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 7 0 R >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 7 0 R >>',
    HELVETICA,
    objectStream('<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>', 'BT /F1 12 Tf 0 0 Td (ALPHA) Tj ET\n'),
    stream('q 1 0 0 1 72 700 cm /Fm1 Do Q\n'),
  ]);
  const session = openBytes(bytes);
  assert.equal(xobjectsOf(session, 0), null, 'no own resources before');
  const reply = edit(session, cowTarget(session, 'ALPHA'), 'BRAV');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const xo = xobjectsOf(session, 0);
  assert.equal(xo.Fm1, 6, 'the inherited entry is carried over');
  assert.notEqual(xo.Fm1c1, undefined);
  assert.equal(contentOf(session, 0).includes('/Fm1c1 Do'), true);
  assert.match(pageText(session, 0), /BRAV/);
  assert.match(pageText(session, 1), /ALPHA/, 'page 2 shares the content stream and the original Form');
  assert.equal(xobjectsOf(session, 1), null, 'page 2 still inherits');
  // Review U4: the copy owns its /XObject dictionary, so the parent (page 2's) never gets Fm1c1.
  const inheritedNames = () => {
    const names = [];
    session.doc.loadPage(1).getObject().getInheritable('Resources').get('XObject').forEach((_, key) => names.push(key));
    return names.join(' ');
  };
  assert.equal(inheritedNames(), 'Fm1', 'the inherited /XObject has no copy key');
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(inheritedNames(), 'Fm1', 'and still none after undo');
  assert.equal(xobjectsOf(session, 0), null, 'undo removes the copied resources');
  assert.equal(contentsNumber(session, 0), 7);
  assert.match(pageText(session, 0), /ALPHA/);
});

test('U1b budget: a copy that would push the scan past the byte ceiling refuses too-large and writes nothing', async () => {
  const filler = `% ${'x'.repeat(1_100_000)}\n`;
  const bytes = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Fm1 5 0 R >> >> /Contents 6 0 R >>',
    HELVETICA,
    objectStream('<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >>', `${filler}BT /F1 12 Tf 0 0 Td (ALPHA) Tj ET\n`),
    stream('q 1 0 0 1 72 700 cm /Fm1 Do Q\n'),
  ]);
  const session = openBytes(bytes);
  const before = contentOf(session, 0);
  const reply = edit(session, cowTarget(session, 'ALPHA'), 'BRAV');
  assert.equal(reply.ok, false);
  assert.equal(reply.detail, 'too-large', JSON.stringify(reply));
  assert.equal(contentOf(session, 0), before);
  assert.deepEqual(Object.keys(xobjectsOf(session, 0)), ['Fm1'], 'the abandoned operation left no copy');
});

test('U1b two Forms sharing one indirect /Resources: the copy gets its own, the originals keep theirs (human accepted 2026-09-17)', async () => {
  const bytes = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Fm1 5 0 R /Fm2 6 0 R >> >> /Contents 4 0 R >>',
    stream('q 1 0 0 1 72 700 cm /Fm1 Do Q q 1 0 0 1 72 600 cm /Fm2 Do Q\n'),
    objectStream('<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources 7 0 R', 'BT /F1 12 Tf 0 0 Td (ALPHA) Tj ET\n'),
    objectStream('<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources 7 0 R', 'BT /F1 12 Tf 0 0 Td (OTHER) Tj ET\n'),
    '<< /Font << /F1 8 0 R >> >>',
    HELVETICA,
  ]);
  const session = openBytes(bytes);
  const reply = edit(session, cowTarget(session, 'ALPHA'), 'BRAV');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const resourcesOf = name => pageDict(session, 0, object => object.get('Resources').get('XObject').get(name).get('Resources').asIndirect());
  assert.equal(resourcesOf('Fm1'), 7);
  assert.equal(resourcesOf('Fm2'), 7, 'the untouched Form still shares the original dictionary');
  assert.notEqual(resourcesOf('Fm1c1'), 7, 'the graft carries its own copy');
  assert.match(xobjectBytes(session, 0, 'Fm1'), /\(ALPHA\)/);
  assert.match(xobjectBytes(session, 0, 'Fm2'), /\(OTHER\)/);
  assert.match(xobjectBytes(session, 0, 'Fm1c1'), /\(BRAV\)/);
});

// ------------------------------------------------------- U3 bounded subset fonts

/**
 * A copy of the committed `edit-subset.pdf` with its page content stream patched
 * by `transform`, so one synthetic page can carry a tail, an inline ActualText or
 * a mismatched tag without a new fixture. The font program is untouched.
 */
async function cidPdf(transform) {
  const source = mupdf.PDFDocument.openDocument(
    new Uint8Array(await readFile('fixtures/p0b/edit-subset.pdf')), 'application/pdf');
  const page = source.loadPage(0);
  try {
    const contents = page.getObject().get('Contents');
    const element = contents.isArray() ? contents.get(0) : contents;
    const buffer = element.readStream();
    let text;
    try {
      text = Buffer.from(buffer.asUint8Array()).toString('latin1');
    } finally {
      buffer.destroy();
    }
    element.writeStream(Buffer.from(transform(text), 'latin1'));
  } finally {
    page.destroy();
  }
  return openBytes(source.saveToBuffer({ compress: true, garbage: true }).asUint8Array(), engine.createSession(), 'cid-scratch.pdf');
}

function savedBytes(session) {
  const reply = op(session, { op: 'save' });
  assert.equal(reply.ok, true, reply.error);
  return new Uint8Array(reply.bytes);
}

/** The structured text of saved bytes, so extraction is read from the written file. */
function extracted(bytes) {
  const document = mupdf.PDFDocument.openDocument(bytes, 'application/pdf');
  const page = document.loadPage(0);
  try {
    const structured = page.toStructuredText();
    try {
      return structured.asText();
    } finally {
      structured.destroy();
    }
  } finally {
    page.destroy();
  }
}

test('U3 replace: a subset Identity-H run is written as a hex permutation of its own codes', async () => {
  const session = await openFile('fixtures/p0b/edit-subset.pdf');
  const target = candidate(session, '日本語テスト');
  assert.equal(target.item.repertoire, 'のステト日本行語隣');
  assert.ok(target.item.bounds.length === 4);
  const reply = edit(session, target, 'テスト日本');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const after = contentOf(session);
  assert.ok(after.includes('<00f400e700f61a021b49> Tj'), after);
  assert.equal(after.includes('<1a021b493ebb00f400e700f6>'), false, 'the old operand is gone');
  const text = extracted(savedBytes(session));
  assert.ok(text.includes('テスト日本'), text);
  assert.equal(text.includes('日本語テスト'), false, text);
  assert.ok(text.includes('隣の行'), 'the neighbour run is untouched');
});

test('U3 refuse: a character outside the page repertoire is refused with the missing list, nothing written', async () => {
  const session = await openFile('fixtures/p0b/edit-subset.pdf');
  const before = contentOf(session);
  const target = candidate(session, '日本語テスト');
  const reply = edit(session, target, '日本語試験');
  assert.equal(reply.ok, false);
  assert.equal(reply.detail, 'unsupported-encoding');
  assert.deepEqual(reply.missing, ['試', '験']);
  assert.equal(contentOf(session), before);
  // A refusal is not a mutation: the same handle is still valid.
  assert.equal(edit(session, target, 'テスト日本').ok, true);
});

test('U3 cjk: a longer run grows within the page free space using the /W numbers', async () => {
  const session = await openFile('fixtures/cjk.pdf');
  const target = candidate(session, '中文测试');
  assert.equal(edit(session, target, '测试').ok, true);
  const shorter = candidate(session, '测试');
  // Every used CJK code is 1000/1000 em at 28 pt (probe §1), so the width is 28 pt per glyph.
  const grown = edit(session, shorter, '测试测试');
  assert.equal(grown.ok, true, `${grown.error} ${grown.detail ?? ''}`);
  assert.equal(grown.shift, 0);
});

test('U3 repertoire is per font object: /F3 draws 試験, /F2 on the same page may not use it', async () => {
  const session = await openFile('fixtures/p3/edit-subset2.pdf');
  const line1 = candidate(session, '日本語テスト');
  assert.equal(line1.item.repertoire.includes('試'), false);
  const refused = edit(session, line1, '試験');
  assert.equal(refused.detail, 'unsupported-encoding');
  assert.deepEqual(refused.missing, ['試', '験']);
  const second = candidate(session, '試験');
  assert.equal(second.item.repertoire, '試験', 'the same font bytes as a second object are a separate repertoire');
  const scanResult = scan(session);
  assert.ok(scanResult.items.some(item => item.kerned === true),
    'the TJ line is a kerned candidate');
});

test('U3 delete: the operand becomes <>, the tag travels, and undo restores every byte', async () => {
  const session = await openFile('fixtures/p0b/edit-subset.pdf');
  const before = contentOf(session);
  const target = candidate(session, '隣の行');
  assert.equal(edit(session, target, '').ok, true);
  const after = contentOf(session);
  assert.ok(after.includes('<> Tj'), after);
  assert.equal(extracted(savedBytes(session)).includes('隣の行'), false);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentOf(session), before);
});

test('U3 tail: a CID run with text following it must keep its exact /W width', async () => {
  // Two shows in one BT block, so the second run's origin depends on the first's advance.
  const session = await cidPdf(text => text.replace(
    'BT /F2 28 Tf 72 600 Td <4ac000a13c69> Tj ET',
    'BT /F2 28 Tf 72 600 Td <4ac000a13c69> Tj <00a1> Tj ET'));
  const target = candidate(session, '隣の行');
  assert.equal(target.item.tailFollows, true);
  assert.equal(edit(session, target, '行隣の').ok, true, 'a permutation keeps the width');
  const shorter = edit(session, candidate(session, '行隣の'), '行隣');
  assert.equal(shorter.ok, true, `${shorter.error} ${shorter.detail ?? ''}`);
  assert.equal(shorter.shift, -28);
});

test('U3 tagged: an inline /ActualText is rewritten as UTF-16BE hex with the operand', async () => {
  const tag = '<FEFF65E5672C8A9E30C630B930C8>';
  // ≥ one space is significant in the content stream: the original run is the same line.
  const session = await cidPdf(text => text
    .replace('BT /F2 28 Tf 72 660 Td <1a021b493ebb00f400e700f6> Tj ET',
      `/Span << /ActualText ${tag} >> BDC\nBT /F2 28 Tf 72 660 Td <1a021b493ebb00f400e700f6> Tj ET\nEMC`));
  const before = contentOf(session);
  assert.ok(before.includes(tag));
  const target = candidate(session, '日本語テスト');
  assert.equal(edit(session, target, 'テスト日本').ok, true);
  const after = contentOf(session);
  assert.ok(after.includes('<feff30c630b930c865e5672c>'), after);
  assert.equal(after.includes('feff65e5672c'), false);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentOf(session), before);
});

test('U3 tagged: an ActualText that does not decode to the run is never a candidate', async () => {
  const session = await cidPdf(text => text.replace(
    'BT /F2 28 Tf 72 660 Td <1a021b493ebb00f400e700f6> Tj ET',
    '/Span << /ActualText <FEFF65E5> >> BDC\nBT /F2 28 Tf 72 660 Td <1a021b493ebb00f400e700f6> Tj ET\nEMC'));
  const before = contentOf(session);
  const reply = scan(session);
  const item = reply.items.find(entry => entry.font?.subtype === 'Type0' && entry.text === '日本語テスト');
  assert.equal(item.support, 'unsupported');
  assert.equal(item.reason, 'ambiguous-text-association');
  const refused = edit(session, { item, generation: reply.generation, revision: reply.revision, page: 0 }, 'テスト日本');
  assert.equal(refused.detail, 'not-candidate');
  assert.equal(refused.reason, 'ambiguous-text-association');
  assert.equal(contentOf(session), before);
});

test('U3 rtl: a decoded Hebrew run is a read-only target, never a write', async () => {
  const session = await openFile('fixtures/rtl.pdf');
  const reply = scan(session);
  const item = reply.items.find(entry => entry.reason === 'rtl-or-shaping');
  assert.ok(item, JSON.stringify(reply.items.map(entry => `${entry.support}:${entry.text}:${entry.reason}`)));
  const refused = op(session, {
    op: 'edit-text', page: 0, targetId: item.targetId,
    generation: reply.generation, revision: reply.revision, text: 'שלום',
  });
  assert.equal(refused.ok, false);
  assert.equal(refused.detail, 'not-candidate');
  assert.equal(refused.reason, 'rtl-or-shaping');
});

/**
 * U3 §2.4: the probe (p3-unit3-probe.md §4) measured `encodeCharacter` returning 0
 * for a WinAnsi character dropped from a subset simple TrueType, so U2's path is
 * kept. `fixtures/p3/simple-subset.pdf` is that case: a subset NotoSansHebrew
 * TrueType, WinAnsi, drawing `ALPHA`, with `Z` outside the subset.
 */
test('U3 simple subset TrueType: the encodeCharacter gate still refuses a dropped character', async () => {
  const session = await openFile('fixtures/p3/simple-subset.pdf');
  const target = candidate(session, 'ALPHA');
  assert.equal(target.item.font.subset, true, 'the fixture font carries a subset prefix');
  assert.equal(target.item.font.embedded, true);
  const reply = edit(session, target, 'ALPHZ');
  assert.equal(reply.detail, 'unsupported-encoding', JSON.stringify(reply));
  assert.deepEqual(reply.missing, ['Z']);
  assert.equal(edit(session, target, 'AHPLA').ok, true, 'the kept glyphs can still be permuted');
});

/** edit-subset.pdf with its /F2 Type0 dictionary changed by `patch(document, font)` (review fixes, opus5). */
async function subsetFontPdf(patch) {
  const source = mupdf.PDFDocument.openDocument(
    new Uint8Array(await readFile('fixtures/p0b/edit-subset.pdf')), 'application/pdf');
  const fonts = source.loadPage(0).getObject().get('Resources').get('Font');
  patch(source, fonts);
  return openBytes(source.saveToBuffer({ compress: true, garbage: true }).asUint8Array(), engine.createSession(), 'font-scratch.pdf');
}

test('U3 review: a Type0 font given as a direct dictionary has no repertoire, so it is never a candidate', async () => {
  // Before the fix the run stayed a candidate and fell through to the U2 literal
  // path, writing `(DP)` into an Identity-H run on a font with Latin glyphs.
  const session = await subsetFontPdf((document, fonts) => {
    const direct = document.newDictionary();
    fonts.get('F2').resolve().forEach((value, key) => direct.put(key, value));
    fonts.put('F2', direct);
  });
  const item = scan(session).items.find(entry => entry.text === '日本語テスト');
  assert.equal(item.support, 'unsupported');
  assert.equal(item.reason, 'unsupported-encoding');
  assert.equal(item.repertoire, undefined);
});

test('U3 review: /W expansion is bounded to 2-byte codes and to the CMap code ceiling', async () => {
  const wide = await subsetFontPdf((document, fonts) => {
    const list = document.newArray();
    // 200 triples of 65 536 codes each, all above 0xFFFF (and below 2^24, where mupdf keeps integers): ignored, the run keeps /DW. Before the fix: ~3 s, ~940 MB heap.
    for (let index = 1; index <= 200; index++) {
      list.push(0x10000 * index);
      list.push(0x10000 * index + 0xffff);
      list.push(500);
    }
    fonts.get('F2').get('DescendantFonts').get(0).put('W', list);
  });
  const started = performance.now();
  assert.equal(candidate(wide, '日本語テスト').item.support, 'candidate');
  assert.ok(performance.now() - started < 2000, 'the scan does not expand codes that cannot exist');
  const over = await subsetFontPdf((document, fonts) => {
    const list = document.newArray();
    // 17 × 65 536 in-range codes > 1 << 20: the font is unresolved, not a candidate.
    for (let index = 0; index < 17; index++) {
      list.push(0);
      list.push(0xffff);
      list.push(1000);
    }
    fonts.get('F2').get('DescendantFonts').get(0).put('W', list);
  });
  const item = scan(over).items.find(entry => entry.text === undefined || entry.text === '日本語テスト');
  assert.notEqual(item.support, 'candidate');
});
