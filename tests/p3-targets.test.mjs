/**
 * P3 U1 — read-only edit-target discovery (`edit-targets`) and the generation/
 * revision-bound `resolveEditTarget` handle. Engine direct, no renderer UI: the
 * worker forwards this request exactly as the tests dispatch it.
 *
 * Two fixture sources: the accepted P0-b edit fixtures (`fixtures/p0b/edit-*.pdf`,
 * produced for the content-edit probe, so they already carry the hard shapes),
 * and synthetic single-purpose PDFs built in this file (syntax, bounds, inline
 * images, exotic fonts). Nothing here is asserted from the scanner's own output:
 * the expectations come from the fixture bytes (see the `/* stream *\/` comments)
 * or from the P0-b evidence table.
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

async function open(id, session = engine.createSession()) {
  const reply = engine.dispatch(
    { id: 1, op: 'open', name: `${id}.pdf`, pdf: toArrayBuffer(await readFile(`fixtures/${id}.pdf`)) },
    session,
  ).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

async function openFile(path, session = engine.createSession()) {
  const reply = engine.dispatch(
    { id: 1, op: 'open', name: path, pdf: toArrayBuffer(await readFile(path)) },
    session,
  ).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function openBytes(bytes, session = engine.createSession(), name = 'scratch.pdf') {
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: toArrayBuffer(bytes) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

let nextId = 500;
function targets(session, page = 0) {
  const reply = engine.dispatch({ id: nextId++, op: 'edit-targets', page }, session).reply;
  return reply;
}

function itemsOf(session, page = 0) {
  const reply = targets(session, page);
  assert.equal(reply.ok, true, reply.error);
  return reply;
}

function candidates(reply) {
  return reply.items.filter(item => item.support === 'candidate');
}

function texts(reply) {
  return reply.items.filter(item => item.kind === 'text').map(item => `${item.support}:${item.text ?? ''}${item.reason ? ` (${item.reason})` : ''}`);
}

// --------------------------------------------------------------- synthetic PDFs

const LATIN = 'latin1';

function stream(body) {
  const bytes = Buffer.isBuffer(body) ? body : Buffer.from(body, LATIN);
  return Buffer.concat([Buffer.from(`<< /Length ${bytes.length} >>\nstream\n`, LATIN), bytes, Buffer.from('endstream', LATIN)]);
}

/** Classic xref PDF from raw object bodies, exactly the `fixtures/minipdf.py` layout. */
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

/** One page, one /Contents stream, optional extra objects appended after it. */
function onePage(content, { font = HELVETICA, pageResources = null, extraObjects = [] } = {}) {
  const resources = pageResources ?? '<< /Font << /F1 4 0 R >> >>';
  return rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources ${resources} /Contents 5 0 R >>`,
    font,
    stream(content),
    ...extraObjects,
  ]);
}

/** An object whose dictionary is completed here so `/Length` always matches the body. */
function objectStream(dict, body) {
  const bytes = Buffer.isBuffer(body) ? body : Buffer.from(body, LATIN);
  return Buffer.concat([Buffer.from(`${dict} /Length ${bytes.length} >>\nstream\n`, LATIN), bytes, Buffer.from('endstream', LATIN)]);
}

// ------------------------------------------------------------------- fixtures

test('hello.pdf: one candidate text target with real bounds, honest font facts and no state change', async () => {
  const session = await open('hello');
  // The fixture's whole content stream, quoted so the expectation does not come
  // from the scanner: `BT /F1 36 Tf 72 700 Td (PDF4All) Tj ET`.
  const before = contentBytes(session, 0);
  const reply = itemsOf(session);
  assert.equal(reply.generation > 0, true, 'a document generation is allocated on open');
  assert.equal(reply.revision, 0, 'read-only discovery does not bump the revision');

  const text = reply.items.filter(item => item.kind === 'text');
  assert.equal(text.length, 1, JSON.stringify(reply.items));
  assert.equal(text[0].support, 'candidate');
  assert.equal(text[0].text, 'PDF4All');
  assert.equal(text[0].reason, undefined);
  assert.deepEqual(text[0].font, { name: 'Helvetica', subtype: 'Type1', embedded: false, subset: false, writingMode: null });
  assert.equal(text[0].bounds.length, 4);
  assert.ok(text[0].bounds[2] > text[0].bounds[0] && text[0].bounds[3] > text[0].bounds[1], 'bounds have area');
  assert.equal(text[0].targetId.includes('p0.c0'), true, text[0].targetId);

  const state = engine.journalState(session);
  assert.deepEqual({ dirty: state.dirty, canUndo: state.canUndo, canRedo: state.canRedo, revision: state.revision },
    { dirty: false, canUndo: false, canRedo: false, revision: 0 });
  assert.deepEqual(contentBytes(session, 0), before, 'the content stream is byte-identical after a scan');
});

function contentBytes(session, page) {
  const object = session.doc.loadPage(page).getObject().get('Contents');
  const buffer = object.readStream();
  try {
    return [...buffer.asUint8Array()];
  } finally {
    buffer.destroy();
  }
}

test('a transformed page keeps candidates: ALPHA and NEIGHBOR are distinct targets', async () => {
  const session = await openFile('fixtures/p0b/edit-transformed.pdf');
  const reply = itemsOf(session);
  const text = reply.items.filter(item => item.kind === 'text');
  assert.deepEqual(text.map(item => item.text), ['ALPHA', 'NEIGHBOR']);
  assert.deepEqual(text.map(item => item.support), ['candidate', 'candidate']);
  assert.notEqual(text[0].targetId, text[1].targetId);
  const image = reply.items.filter(item => item.kind === 'image');
  assert.equal(image.length, 1, 'the /Im1 Do invocation is one image target');
  assert.equal(image[0].support, 'candidate', 'an unshared image invocation has an exact association');
  assert.equal(image[0].bounds, null, 'an image target never carries a guessed rectangle');
});

test('edit-tagged.pdf: the /BDC property dictionary does not disturb the wrapped text target', async () => {
  const session = await openFile('fixtures/p0b/edit-tagged.pdf');
  const reply = itemsOf(session);
  assert.deepEqual(texts(reply), ['candidate:ALPHA', 'candidate:NEIGHBOR']);
});

test('edit-shared.pdf: Forms and a shared image are U4 object candidates; text inside each placement is cowable (U1b)', async () => {
  const session = await openFile('fixtures/p0b/edit-shared.pdf');
  const reply = itemsOf(session);
  // Page 1: `q ... /Fm1 Do Q` twice, then `/Im1 Do` (Im1 is drawn on page 2 too), then NEIGHBOR.
  const forms = reply.items.filter(item => item.kind === 'form');
  assert.equal(forms.length, 2, 'two Form invocations');
  assert.equal(new Set(forms.map(item => item.targetId)).size, 2, 'same name at a different offset is a different target');
  // U4: sharing an XObject blocks nothing - only the page stream changes.
  assert.deepEqual(forms.map(item => `${item.support}:${item.reason}`), ['candidate:undefined', 'candidate:undefined']);
  assert.deepEqual(forms.map(item => item.objectOps), [['move', 'resize', 'delete', 'duplicate', 'transform', 'crop'], ['move', 'resize', 'delete', 'duplicate', 'transform', 'crop']]);
  const inside = reply.items.filter(item => item.kind === 'text' && item.text === 'ALPHA');
  assert.equal(inside.length, 2, 'text inside each Form invocation is listed separately');
  // U1b: a depth-1 Form invoked from the page is a proven copy-on-write shape.
  assert.deepEqual(inside.map(item => `${item.support}:${item.cowable}:${item.reason}`), ['candidate:true:undefined', 'candidate:true:undefined']);
  assert.deepEqual(inside.map(item => item.bounds), [null, null], 'identical placements: no single one can claim the bounds');
  const image = reply.items.find(item => item.kind === 'image');
  assert.equal(image.support, 'candidate', 'Im1 is drawn on both pages: the page stream is the only write');
  assert.deepEqual(image.objectOps, ['move', 'resize', 'delete', 'replace', 'duplicate', 'transform', 'crop']);
  assert.deepEqual(texts(reply).slice(-1), ['candidate:NEIGHBOR']);
});

test('U3 identity-H Type0 runs decode and carry a page-scoped repertoire; RTL runs stay read-only', async () => {
  // edit-subset.pdf / cjk.pdf / edit-embedded.pdf: every CID run is a candidate with
  // its decoded text and the characters its own font object draws on this page.
  for (const [path, pageText, otherText] of [
    ['fixtures/cjk.pdf', '日本語テスト', '中文测试'],
    ['fixtures/p0b/edit-subset.pdf', '日本語テスト', '隣の行'],
    ['fixtures/p0b/edit-embedded.pdf', '日本語テスト', '隣の行'],
  ]) {
    const session = await openFile(path);
    const reply = itemsOf(session);
    const cid = reply.items.filter(item => item.font && item.font.subtype === 'Type0');
    assert.equal(cid.length, 2, `${path}: two CID runs`);
    for (const item of cid) {
      assert.equal(item.support, 'candidate', `${path}: ${item.text} (${item.reason})`);
      assert.equal(item.reason, undefined, path);
      assert.ok(typeof item.repertoire === 'string' && item.repertoire.length > 0, `${path}: every candidate CID run has a repertoire`);
      for (const char of item.text) assert.ok(item.repertoire.includes(char), `${path}: ${char} drawn with this font is in its repertoire`);
      assert.notEqual(item.bounds, null, path);
    }
    assert.deepEqual(cid.map(item => item.text).sort(), [pageText, otherText].sort(), path);
    // The repertoire is per font object and page-scoped: the other run's characters
    // prove glyphs for the same object, so both runs share one repertoire.
    for (const item of cid) for (const char of otherText) assert.ok(item.repertoire.includes(char), `${path}: ${char} from the other run proves the same font object`);
  }
  // A subset font has no glyph for 試/験, which is exactly what the repertoire must say.
  const subset = itemsOf(await openFile('fixtures/p0b/edit-subset.pdf'));
  const japanese = subset.items.find(item => item.text === '日本語テスト');
  assert.equal(japanese.repertoire.includes('試'), false);
  assert.equal(japanese.repertoire.includes('験'), false);
  assert.equal(japanese.repertoire, 'のステト日本行語隣', 'code order: 00a1 00e7 00f4 00f6 1a02 1b49 3c69 3ebb 4ac0');

  // rtl.pdf: the Hebrew and Arabic runs decode into the read-only ranges. The two
  // Latin markers drawn with the Hebrew font are byte-identical direct runs in one font;
  // P6 U8 D1: each is matched at its own origin, so both are candidates with their own
  // bounds (they were ambiguous before U8). The Helvetica run is untouched.
  const rtl = itemsOf(await openFile('fixtures/rtl.pdf'));
  const cidRtl = rtl.items.filter(item => item.font && item.font.subtype === 'Type0');
  assert.deepEqual(cidRtl.map(item => `${item.support}:${item.reason}:${item.text}`), [
    'unsupported:rtl-or-shaping:םולש',
    'candidate:undefined: PDF4All',
    'unsupported:rtl-or-shaping:ﺎﺒﺣﺮﻣ',
    'candidate:undefined: PDF4All',
  ]);
  assert.deepEqual(cidRtl.map(item => item.repertoire), [undefined, 'םלש וADFP4l', undefined, 'םלש וADFP4l']);
  const markers = cidRtl.filter(item => item.support === 'candidate');
  assert.notEqual(markers[0].targetId, markers[1].targetId);
  [[160, 102, 260, 140], [170, 172, 270, 210]].forEach((box, k) =>
    assert.ok(box.every((value, i) => Math.abs(markers[k].bounds[i] - value) <= 1), JSON.stringify(markers[k].bounds)));
  const marker = rtl.items.find(item => item.support === 'candidate' && item.font.subtype === 'Type1');
  assert.equal(marker.text, 'PDF4All');
  assert.equal(marker.font.subtype, 'Type1');
});

test('large.pdf: the scan is on demand, bounded and small', async () => {
  const session = await open('large');
  const opened = engine.dispatch({ id: 900, op: 'render', page: 0, scale: 0.5 }, session).reply;
  assert.equal(opened.ok, true, opened.error);
  const reply = itemsOf(session);
  assert.equal(reply.items.length, 6, 'the fixture page has six authored lines');
  assert.ok(reply.items.every(item => item.support === 'candidate'), 'all six are plain Helvetica Tj runs');
  assert.equal(reply.revision, 0);
});

test('a locked document refuses without a password, and a copy-denied one refuses with read-only', async () => {
  const locked = await open('encrypted');
  assert.equal(engine.detect(locked.doc).needsPassword, true);
  const refused = targets(locked);
  assert.equal(refused.ok, false);
  assert.match(refused.error, /password is required/);

  // Build a copy-denied file with the engine's own writer, the same route the
  // P2 encryption tests use (no fixture denies `copy`).
  const source = await open('hello');
  const set = engine.dispatch({
    id: 901, op: 'security-set', mode: 'aes-256',
    userPassword: 'edit-targets-user', ownerPassword: 'edit-targets-owner',
    permissions: { print: true, copy: false, edit: false, annotate: true, form: false },
  }, source).reply;
  assert.equal(set.ok, true, set.error);
  const saved = engine.dispatch({ id: 902, op: 'save' }, source).reply;
  assert.equal(saved.ok, true, saved.error);

  const copyDenied = openBytes(saved.bytes, engine.createSession(), 'copy-denied.pdf');
  const auth = engine.dispatch({ id: 903, op: 'authenticate', password: 'edit-targets-user' }, copyDenied).reply;
  assert.equal(auth.ok, true, auth.error);
  assert.equal(engine.detect(copyDenied.doc).permissions.copy, false);
  const reply = targets(copyDenied);
  assert.equal(reply.ok, false);
  assert.equal(reply.reason, 'read-only');
  assert.equal(reply.items, undefined, 'a refused request carries no target data');
});

test('inspection is allowed where copying is, even when editing is not, and on a signed document', async () => {
  // A no-edit but copy-permitted file: inspection yes, and a later writer still
  // has to pass requireMutable, which this unit does not build yet.
  const noEdit = await open('encrypted-noedit');
  const auth = engine.dispatch({ id: 906, op: 'authenticate', password: 'pdf4all-user' }, noEdit).reply;
  assert.equal(auth.ok, true, auth.error);
  // `detect` is handed the session's `locked` flag on purpose: `needsPassword()`
  // re-locks an authenticated document (P2 unit 10 hazard), so asking the
  // document again here would decrypt nothing after this line.
  assert.equal(engine.detect(noEdit.doc, false).permissions.edit, false);
  assert.equal(engine.detect(noEdit.doc, false).permissions.copy, true);
  const inspected = itemsOf(noEdit);
  assert.deepEqual(texts(inspected), ['candidate:PDF4All encrypted no-edit']);

  // A signed document may be inspected where copying is permitted; that grants
  // no mutation right.
  const signed = await openFile('fixtures/p0b/signed-intact.pdf');
  assert.equal(engine.detect(signed.doc).signatures > 0, true, 'the fixture carries a signature');
  const signedReply = itemsOf(signed);
  assert.equal(signedReply.ok, true, signedReply.error);
  assert.ok(signedReply.items.some(item => item.kind === 'text'), JSON.stringify(signedReply.items));
});

// ------------------------------------------------------------------- resolve

test('resolveEditTarget accepts a fresh handle and refuses made-up, cross-page, stale and changed ones', async () => {
  const session = await open('hello');
  const scan = itemsOf(session);
  const fresh = engine.resolveEditTarget(session, {
    generation: scan.generation, revision: scan.revision, page: 0, targetId: scan.items[0].targetId,
  });
  assert.equal(fresh.kind, 'text');
  assert.equal(fresh.text, 'PDF4All');
  assert.equal(fresh.objectNumber > 0, true, 'the record names the stream object');
  assert.ok(fresh.opEnd > fresh.opStart, 'the operator span is recorded');
  assert.ok(fresh.operandEnd > fresh.operandStart, 'the payload span is recorded');

  const refuses = (request) => {
    try {
      engine.resolveEditTarget(session, request);
      return null;
    } catch (err) {
      return err;
    }
  };
  const request = () => ({ generation: scan.generation, revision: scan.revision, page: 0, targetId: scan.items[0].targetId });
  assert.match(refuses({ ...request(), targetId: 'p0.c0.0.deadbeef' }).message, /no target/);
  assert.match(refuses({ ...request(), page: 7 }).message, /page must be an integer/);
  assert.match(refuses({ ...request(), generation: scan.generation + 1 }).message, /stale document generation/);
  assert.match(refuses({ ...request(), revision: scan.revision + 1 }).message, /stale revision/);
  assert.match(refuses({ ...request(), targetId: '' }).message, /non-empty string/);

  // A real mutation changes the revision, so the same handle is refused even
  // though the page bytes may still be identical.
  const mutation = engine.dispatch({ id: 904, op: 'annot-add', page: 0, type: 'Text', point: [100, 100], contents: 'note' }, session).reply;
  assert.equal(mutation.ok, true, mutation.error);
  assert.match(refuses(request()).message, /stale revision/);

  // Re-discovery after the change hands out a handle that resolves again.
  const rescan = itemsOf(session);
  const again = engine.resolveEditTarget(session, {
    generation: rescan.generation, revision: rescan.revision, page: 0, targetId: rescan.items[0].targetId,
  });
  assert.equal(again.text, 'PDF4All');

  // Closing and reopening the same filename is a new generation, never the old one.
  engine.dispatch({ id: 905, op: 'close' }, session);
  assert.match(refuses(request()).message, /no document open/);
  await open('hello', session);
  const reopened = itemsOf(session);
  assert.notEqual(reopened.generation, scan.generation, 'close/open allocates a new generation');
  const staleAfterReopen = refuses({
    generation: scan.generation, revision: reopened.revision, page: 0, targetId: scan.items[0].targetId,
  });
  assert.match(staleAfterReopen.message, /stale document generation/);
});

// ------------------------------------------------------------------- syntax

test('the lexer cannot be fooled by comments, escaped strings, hex strings or numbers', async () => {
  const content = [
    '% (not a target) Tj and /F9 Tf belong to a comment',
    'BT /F1 12 Tf 72 700 Td (PDF4All) Tj ET',
    'BT /F1 12 Tf 72 650 Td (octal: \\101\\102\\103) Tj ET',
    'BT /F1 12 Tf 72 600 Td (nested \\(paren\\) and \\\\ backslash) Tj ET',
    'BT /F1 12 Tf 72 550 Td <50444634416C6C> Tj ET',
    'BT /F1 12 Tf 72 500 Td (a\\)b) Tj ET',
    'BT /F1 12 Tf 72 450 Td (\\) Tj /Fm1 Do) Tj ET',
  ].join('\n');
  const session = openBytes(onePage(content));
  const reply = itemsOf(session);
  const marks = reply.items.map(item => item.text);
  assert.deepEqual(marks, ['PDF4All', 'octal: ABC', 'nested (paren) and \\ backslash', 'PDF4All', 'a)b', ') Tj /Fm1 Do']);
  assert.equal(reply.items.length, 6, 'the comment produced no target and no Do came out of a string');
  assert.ok(reply.items.every(item => item.kind === 'text'), 'an operator-looking literal never becomes a form/image target');
  const unique = reply.items.filter(item => item.text !== 'PDF4All');
  assert.ok(unique.every(item => item.support === 'candidate'), JSON.stringify(unique));
  // The literal and the hex string carry the same text at different origins; P6 U8 D1:
  // each is matched at its own origin, so both are candidates -- with distinct identities
  // and bounds on their own lines (y 700 above y 550).
  const pdf4all = reply.items.filter(item => item.text === 'PDF4All');
  assert.deepEqual(pdf4all.map(item => `${item.support}:${item.reason}`), ['candidate:undefined', 'candidate:undefined']);
  assert.notEqual(pdf4all[0].targetId, pdf4all[1].targetId);
  assert.ok(pdf4all.every(item => Math.abs(item.bounds[0] - 72) <= 1), JSON.stringify(pdf4all.map(item => item.bounds)));
  assert.ok(Math.abs(pdf4all[1].bounds[1] - pdf4all[0].bounds[1] - 150) <= 1, 'bounds at each own origin');
});

test('multi-stream /Contents carry graphics and text state across the array', async () => {
  const first = 'BT /F1 12 Tf 72 700 Td ';
  const second = '(PDF4All) Tj ET\n';
  const bytes = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents [5 0 R 6 0 R] >>',
    HELVETICA,
    stream(first),
    stream(second),
  ]);
  const session = openBytes(bytes);
  const reply = itemsOf(session);
  assert.deepEqual(texts(reply), ['candidate:PDF4All'], JSON.stringify(reply.items));
  assert.equal(reply.items[0].targetId.includes('.c1.'), true, 'the target names the second stream');

  // The same stream twice is a shared /Contents: a proven copy-on-write shape (U1b).
  const shared = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents [6 0 R] >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents [6 0 R] >>',
    HELVETICA,
    stream('BT /F1 12 Tf 72 700 Td (PDF4All) Tj ET\n'),
  ]);
  const sharedSession = openBytes(shared);
  const sharedReply = itemsOf(sharedSession);
  assert.deepEqual(sharedReply.items.map(item => item.reason), [undefined]);
  assert.equal(sharedReply.items[0].support, 'candidate');
  assert.equal(sharedReply.items[0].cowable, true);
});

test('unsupported operators, exotic fonts and bad state are classified, never coerced', async () => {
  const font = (extra) => `<< /Type /Font /Subtype /TrueType /BaseFont /Test ${extra} >>`;
  const type3 = '<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [1 0 0 1 0 0] /CharProcs << >> /Encoding << /Differences [] >> /FirstChar 32 /LastChar 32 /Widths [500] >>';
  const cidFont = '<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Test /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor << /Type /FontDescriptor /FontName /Test /Flags 4 /FontBBox [0 0 1 1] /ItalicAngle 0 /Ascent 1 /Descent 0 /CapHeight 1 /StemV 1 >> /WMode 1 >>';
  const type0Vertical = `<< /Type /Font /Subtype /Type0 /BaseFont /Test /Encoding /Identity-H /DescendantFonts [${cidFont}] >>`;
  const cases = [
    { name: 'TJ array', content: 'BT /F1 12 Tf 72 700 Td [(PD) -10 (F4All)] TJ ET\n', reason: undefined },
    { name: "quote operator", content: "BT /F1 12 Tf 72 700 Td (PDF4All) ' ET\n", reason: 'unsupported-text-operator' },
    { name: 'Tj outside BT/ET', content: '/F1 12 Tf 72 700 Td (PDF4All) Tj\n', reason: 'unsupported-graphics-state' },
    { name: 'unbalanced Q', content: 'Q BT /F1 12 Tf 72 700 Td (PDF4All) Tj ET\n', reason: null, requestReason: 'invalid' },
    {
      name: 'Type3',
      content: 'BT /F1 12 Tf 72 700 Td (PDF4All) Tj ET\n',
      font: type3, reason: 'type3',
    },
    {
      name: 'vertical Type0',
      content: 'BT /F1 12 Tf 72 700 Td (PDF4All) Tj ET\n',
      font: type0Vertical, reason: 'vertical-writing',
    },
    {
      name: 'Differences encoding',
      content: 'BT /F1 12 Tf 72 700 Td (PDF4All) Tj ET\n',
      font: font('/Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [32 /foo] >>'), reason: 'unsupported-encoding',
    },
  ];
  for (const item of cases) {
    const bytes = onePage(item.content, { font: item.font ?? HELVETICA });
    const session = openBytes(bytes);
    const reply = targets(session);
    if (item.requestReason) {
      assert.equal(reply.ok, false, `${item.name}: expected a request refusal`);
      assert.equal(reply.reason, item.requestReason, item.name);
      assert.match(reply.error, /no matching 'q'/, item.name);
      continue;
    }
    assert.equal(reply.ok, true, `${item.name}: ${reply.error}`);
    assert.deepEqual(reply.items.map(entry => entry.reason), [item.reason], `${item.name}: ${JSON.stringify(reply.items)}`);
    assert.equal(reply.items[0].support, item.reason === undefined ? 'candidate' : 'unsupported', item.name);
  }

  // A non-ASCII literal byte and a font name that is not even in /Font.
  const bytes = onePage('BT /F1 12 Tf 72 700 Td (caf\xe9) Tj ET\nBT /F9 12 Tf 72 650 Td (x) Tj ET\n');
  const reply = itemsOf(openBytes(bytes));
  assert.deepEqual(reply.items.map(entry => entry.reason), [undefined, 'unsupported-encoding']);
  assert.deepEqual(reply.items.map(entry => entry.text), ['café', undefined]);
  assert.deepEqual(reply.items[1].font, { name: null, subtype: null, embedded: null, subset: null, writingMode: null },
    'an unresolved font is explicitly unknown');
});

test('Do invocations are resolved: images, forms, missing names and unexpected subtypes', async () => {
  const content = [
    'q /Fm1 Do Q',
    'q /Im1 Do Q',
    'q /Missing Do Q',
    'q /Post Do Q',
  ].join('\n') + '\n';
  const bytes = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> /XObject << /Fm1 4 0 R /Im1 5 0 R /Post 7 0 R >> >> /Contents 8 0 R >>',
    objectStream('<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >>', 'BT /F1 12 Tf 72 700 Td (PDF4All) Tj ET\n'),
    objectStream('<< /Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8', Buffer.from([0x00, 0x11, 0x22, 0x33])),
    HELVETICA,
    objectStream('<< /Type /XObject /Subtype /PS', ''),
    stream(content),
  ]);
  const reply = itemsOf(openBytes(bytes));
  const kinds = reply.items.map(item => `${item.kind}:${item.reason ?? item.support}`);
  assert.deepEqual(kinds, [
    // U4: no `q cm ... Do Q` block, so a Form cannot be moved and an Image is replace-only.
    'form:unsupported-transform',
    'text:candidate',
    'image:unsupported-transform',
    'form:unresolved-resource',
    'form:unsupported-xobject',
  ], JSON.stringify(reply.items));
  // The /PS invocation gets no text of its own, and a form's descendant text is
  // still listed with the exact operator identity rather than collapsed.
  assert.equal(reply.items[1].text, 'PDF4All');
  assert.equal(reply.items[1].cowable, true, 'a depth-1 Form descendant is cowable (U1b)');
  assert.equal(reply.items[2].bounds, null);
});

test('a Form that invokes itself is listed twice and never descends forever', async () => {
  const content = 'q /Fm1 Do Q\nq /Fm1 Do Q\n';
  const cyclic = 'q /Fm1 Do Q\n';
  const bytes = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Fm1 4 0 R >> >> /Contents 5 0 R >>',
    objectStream('<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /XObject << /Fm1 4 0 R >> >>', cyclic),
    stream(content),
  ]);
  const session = openBytes(bytes);
  const reply = itemsOf(session);
  assert.equal(reply.items.length, 4, JSON.stringify(reply.items));
  // U4: the two depth-0 invocations have no `cm`, so they are unsupported-transform;
  // the two cyclic descents are depth 1 and stay shared-or-unproven-sharing.
  assert.deepEqual(reply.items.map(item => item.reason),
    ['unsupported-transform', 'shared-or-unproven-sharing', 'unsupported-transform', 'shared-or-unproven-sharing']);
  // The two outermost invocations are depth 0 and the two cyclic descents depth 1:
  // the record (not the reply) carries depth, and the cycle stopped there.
  const depths = reply.items.map(item => engine.resolveEditTarget(session, {
    generation: reply.generation, revision: reply.revision, page: 0, targetId: item.targetId,
  }).depth);
  assert.deepEqual(depths, [0, 1, 0, 1]);
});

// -------------------------------------------------------------- inline images

test('inline image data is skipped, and an unsafe boundary refuses the whole request', async () => {
  const padding = Buffer.from([0x00, 0x01, 0x80, 0xff, 0x20, 0x45, 0x49, 0x20, 0x00]);
  const body = (prefix, separator) => Buffer.concat([
    Buffer.from(prefix + separator, LATIN),
    padding,
    Buffer.from('\nEI Q\nBT /F1 12 Tf 72 650 Td (PDF4All) Tj ET\n', LATIN),
  ]);

  // Sanity: the payload itself contains the bytes `EI` and the text of a Tj.
  assert.ok(padding.includes(0x45) && padding.includes(0x49));
  const withLength = body(`BT /F1 12 Tf 72 700 Td (PDF4All) Tj ET\nq 1 0 0 1 0 0 cm\nBI /W 3 /H 3 /CS /G /BPC 8 /L ${padding.length}\nID`, ' ');
  const reply = itemsOf(openBytes(onePage(withLength)));
  assert.equal(reply.items.length, 2, JSON.stringify(reply.items));
  assert.deepEqual(reply.items.map(item => item.text), ['PDF4All', 'PDF4All']);
  // P6 U8 D1: two direct runs at their own origins are both candidates (ambiguous before U8).
  assert.deepEqual(reply.items.map(item => `${item.support}:${item.reason}`), ['candidate:undefined', 'candidate:undefined']);
  assert.notEqual(reply.items[0].targetId, reply.items[1].targetId);
  assert.ok(Math.abs(reply.items[1].bounds[1] - reply.items[0].bounds[1] - 50) <= 1, JSON.stringify(reply.items.map(item => item.bounds)));

  // `ID` + a CRLF separator is one EOL, and /L still measures the data only.
  const crlf = body('q BI /W 3 /H 3 /CS /G /BPC 8 /L ' + padding.length + '\nID', '\r\n');
  assert.equal(itemsOf(openBytes(onePage(crlf))).items.length, 1, 'the CRLF-separated image is skipped');

  // A declared length that is off by one must refuse, never skip into the data.
  const wrongLength = body(`q BI /W 3 /H 3 /CS /G /BPC 8 /L ${padding.length + 1}\nID`, ' ');
  const wrongReply = targets(openBytes(onePage(wrongLength)));
  assert.equal(wrongReply.ok, false, 'a wrong /L is refused');
  assert.equal(wrongReply.reason, 'invalid');
  assert.match(wrongReply.error, /does not end at a boundary/);

  // No /L and no bytes that could be an end marker: refuse, do not guess.
  const runaway = onePage(Buffer.from('q BI /W 2 /H 2 /CS /G /BPC 8\nID \x00\x01\x02\x03\x04', LATIN));
  const refused = targets(openBytes(runaway));
  assert.equal(refused.ok, false);
  assert.equal(refused.reason, 'invalid');
  assert.match(refused.error, /inline image/);
});

test('text render modes: only the three visible modes can be candidates', async () => {
  const line = (tr) => `BT /F1 12 Tf ${tr} Tr 72 700 Td (PDF4All) Tj ET\n`;
  for (const [mode, support] of [[0, 'candidate'], [1, 'candidate'], [2, 'candidate'], [3, 'unsupported'], [4, 'unsupported'], [7, 'unsupported']]) {
    // Each file has one visible-or-not run, so the association is never the question here.
    const reply = itemsOf(openBytes(onePage(line(mode))));
    assert.equal(reply.items.length, 1, `Tr ${mode}`);
    assert.equal(reply.items[0].support, support, `Tr ${mode}`);
    if (mode >= 3) assert.equal(reply.items[0].reason, 'unsupported-graphics-state', `Tr ${mode}`);
    else assert.equal(reply.items[0].bounds.length, 4, `Tr ${mode}`);
  }
  // A malformed mode is unknown, which is not a visible mode either.
  const malformed = itemsOf(openBytes(onePage('BT /F1 12 Tf /X Tr 72 700 Td (PDF4All) Tj ET\n')));
  assert.equal(malformed.items[0].reason, 'unsupported-graphics-state');
  // Tr is text state and carries across a q/Q pair like the font does.
  const carried = itemsOf(openBytes(onePage('BT /F1 12 Tf 3 Tr ET q BT 72 700 Td (PDF4All) Tj ET Q\n')));
  assert.equal(carried.items[0].reason, 'unsupported-graphics-state');
});

// -------------------------------------------------------------------- bounds

test('the numeric bounds refuse with too-large and never return a partial list', async () => {
  const many = 'BT /F1 12 Tf ' + Array.from({ length: 600 }, (_, index) => `72 ${700 - index} Td (a) Tj `).join('') + 'ET\n';
  const overflow = targets(openBytes(onePage(many)));
  assert.equal(overflow.ok, false, 'target count bound');
  assert.equal(overflow.reason, 'too-large');
  assert.equal(overflow.items, undefined);

  const operands = 'BT /F1 12 Tf ' + `[${' (a)'.repeat(300)} ] TJ ET\n`;
  const operandReply = targets(openBytes(onePage(operands)));
  assert.equal(operandReply.ok, false, 'operand stack bound');
  assert.equal(operandReply.reason, 'too-large');

  const tokens = 'ex '.repeat(200_001);
  const tokenReply = targets(openBytes(onePage(tokens)));
  assert.equal(tokenReply.ok, false, 'token bound');
  assert.equal(tokenReply.reason, 'too-large');

  // A page with no /Contents at all is not an error: it simply has no targets.
  const empty = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>',
  ]);
  const emptyReply = itemsOf(openBytes(empty));
  assert.deepEqual(emptyReply.items, []);

  // An out-of-range page is refused before any traversal.
  const session = openBytes(onePage('BT /F1 12 Tf 72 700 Td (a) Tj ET\n'));
  const badPage = targets(session, 4);
  assert.equal(badPage.ok, false);
  assert.match(badPage.error, /page must be an integer/);
});

// ------------------------------------------- grok's fixtures/p3 manifest (§6)

/**
 * Expectations in this block come from `fixtures/p3/README.md` and
 * `fixtures/p3/manifest.json` (grok's generator plus pypdf independent counts),
 * not from the scanner. The manifest marks Type3 and vertical writing
 * "untested-pending": they are covered by the synthetic cases above, and the
 * fixtures make no claim about them.
 */

test('targets-syntax.pdf: eight Tj operators, and neither comments nor literals become operators', async () => {
  const session = await openFile('fixtures/p3/targets-syntax.pdf');
  const reply = itemsOf(session);
  assert.equal(reply.items.length, 8, 'manifest: eight Tj operators, zero Do');
  assert.ok(reply.items.every(item => item.kind === 'text'), "the comment's fake /Fm1 Do is not an invocation");
  // P6 U8 D1: ALPHA is shown four times in direct content, each at its own origin, so
  // all four are candidates with their own bounds (ambiguous before U8).
  assert.deepEqual(reply.items.filter(item => item.reason === 'ambiguous-text-association'), []);
  const alpha = reply.items.filter(item => item.text === 'ALPHA');
  assert.deepEqual(alpha.map(item => item.support), Array(4).fill('candidate'), 'ALPHA is shown four times');
  assert.equal(new Set(alpha.map(item => item.targetId)).size, 4, 'each duplicate keeps its own identity');
  assert.deepEqual(alpha.map(item => [Math.round(item.bounds[0]), Math.round(item.bounds[1])]), [[72, 59], [72, 99], [72, 179], [72, 219]]);
  assert.deepEqual(
    reply.items.filter(item => item.support === 'candidate' && item.text !== 'ALPHA').map(item => item.text),
    ['TAIL', 'hello (world)', 'fake Tj and Do tokens', 'écafé'],
    'same-line following text, escaped parens, and WinAnsi literals are decoded',
  );
  const nonAscii = reply.items.at(-1);
  assert.equal(nonAscii.reason, undefined);
  assert.equal(nonAscii.text, 'écafé');
});

test('targets-streams.pdf: text state carries across /Contents, nested Forms keep distinct paths', async () => {
  const session = await openFile('fixtures/p3/targets-streams.pdf');
  const first = itemsOf(session, 0);
  assert.deepEqual(
    first.items.filter(item => item.kind === 'text' && item.support === 'candidate').map(item => `${item.text}${item.cowable ? '*' : ''}`), ['SPLIT', 'FORM*', 'FORM*', 'NEST*'],
    'Tf in stream 1 and Tj in stream 2 belong to one carried text state; depth-1 Form text is cowable (U1b), the Fm1 nested in Fm2 is not',
  );
  const invocations = first.items.filter(item => item.kind !== 'text');
  assert.deepEqual(invocations.map(item => item.kind), ['form', 'form', 'form', 'form', 'image'],
    'manifest page 1: Fm1 x2, Fm2 x1, the nested Fm1 inside Fm2, Im1 x1');
  // U4: the three page-level placements are candidates, the depth-1 nested Form is not.
  assert.deepEqual(invocations.map(item => item.support), ['candidate', 'candidate', 'candidate', 'unsupported', 'candidate']);
  assert.equal(invocations[3].reason, 'shared-or-unproven-sharing');
  const forms = first.items.filter(item => item.text === 'FORM');
  assert.equal(forms.length, 3, 'Fm1 twice plus the Fm1 nested inside Fm2');
  const paths = forms.map(item => engine.resolveEditTarget(session, {
    generation: first.generation, revision: first.revision, page: 0, targetId: item.targetId,
  }).path);
  assert.equal(new Set(paths).size, 3, 'the nested invocation does not collapse into the outer one');
  assert.equal(paths.some(path => path.includes('/Fm2@')), true, JSON.stringify(paths));

  const second = itemsOf(session, 1);
  assert.equal(second.items.length, 4, 'manifest page 2: Fm1, its FORM text, Im1, PAGE2');
  assert.deepEqual(second.items.filter(item => item.kind === 'text' && item.support === 'candidate').map(item => `${item.text}${item.cowable ? '*' : ''}`), ['FORM*', 'PAGE2']);
  assert.deepEqual(second.items.filter(item => item.kind !== 'text').map(item => `${item.kind}:${item.support}`), ['form:candidate', 'image:candidate']);
});

test('targets-unsupported.pdf: a no-length inline image refuses the page before TJ/quote are judged', async () => {
  // manifest: valid TJ, ', ", one Tj, one inline image with no /L. Contract §6
  // allows "safely skipped or conservatively refused"; astra m42 chose refusal,
  // because a no-length EI boundary cannot be told from data.
  const session = await openFile('fixtures/p3/targets-unsupported.pdf');
  const reply = targets(session);
  assert.equal(reply.ok, false);
  assert.equal(reply.reason, 'invalid');
  assert.match(reply.error, /no declared \/L/);
  assert.equal(reply.items, undefined, 'a refused request returns no partial list');

  // The same operators without the inline image are judged individually: the
  // manifest counts one TJ, one ' and one " as unsupported, plus the OK candidate.
  const session2 = openBytes(onePage('BT /F1 12 Tf 72 700 Td [(T) -20 (JARR)] TJ ET\n'
    + "BT /F1 12 Tf 72 660 Td (SQ) ' ET\n"
    + 'BT /F1 12 Tf 72 620 Td 1 0 (DQ) " ET\n'
    + 'BT /F1 12 Tf 72 580 Td (OK) Tj ET\n'));
  const reply2 = itemsOf(session2);
  assert.deepEqual(reply2.items.map(item => item.reason ?? item.support),
    ['candidate', 'unsupported-text-operator', 'unsupported-text-operator', 'candidate']);
  assert.equal(reply2.items[0].kerned, true);
  assert.equal(reply2.items[3].text, 'OK');
  assert.ok(reply2.items.slice(1, 3).every(item => item.text === undefined && item.bounds === null),
    'no text or bounds is fabricated for unsupported quote operators');
});

test('targets-limits.pdf: 1003.9:1 expansion refuses with too-large and no targets', async () => {
  const session = await openFile('fixtures/p3/targets-limits.pdf');
  const reply = targets(session, 0);
  assert.equal(reply.ok, false);
  assert.equal(reply.reason, 'too-large');
  assert.equal(reply.items, undefined, 'a refused request returns no partial list');
  assert.match(reply.error, /2097152 byte scan ceiling/);
  assert.equal(JSON.stringify(reply).includes('HI'), false, 'a refused reply carries no document text');
});

test('resources inherited from the page tree make every target cowable, never a plain candidate', async () => {
  // The /Resources live on the Pages node, so the page's isolation cannot be
  // proved from its own dictionary: the target is listed but never a candidate.
  const bytes = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 4 0 R >> >> >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>',
    HELVETICA,
    stream('BT /F1 12 Tf 72 700 Td (PDF4All) Tj ET\n'),
  ]);
  const session = openBytes(bytes);
  const reply = itemsOf(session);
  assert.equal(reply.items.length, 1);
  // U1b: inherited resources are a proven copy-on-write shape; the run is a single
  // placement, so its association and bounds are real.
  assert.equal(reply.items[0].support, 'candidate');
  assert.equal(reply.items[0].cowable, true);
  assert.equal(reply.items[0].reason, undefined);
  assert.equal(reply.items[0].text, 'PDF4All');
  assert.equal(Array.isArray(reply.items[0].bounds), true);
  assert.deepEqual(reply.items[0].font,
    { name: 'Helvetica', subtype: 'Type1', embedded: false, subset: false, writingMode: null });
});
