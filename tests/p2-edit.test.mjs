/**
 * P2 unit 1 — mutation spine: MuPDF journal undo/redo, Text sticky notes,
 * revision/dirty state. Runs against src/renderer/engine.ts exactly as the
 * worker does, so `dispatch` staying throw-free is what the worker relies on.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine, MAX_NOTE_CHARS, NOTE_SIZE } from '../build/renderer/engine.js';

const engine = createEngine(mupdf);
const HELLO = 'fixtures/hello.pdf';

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

async function open(id, session = engine.createSession()) {
  const bytes = await readFile(`fixtures/${id}.pdf`);
  const reply = engine.dispatch({ id: 1, op: 'open', name: `${id}.pdf`, pdf: toArrayBuffer(bytes) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function noteTexts(session, page = 0) {
  const items = engine.dispatch({ id: 90, op: 'annots', page }, session).reply;
  assert.equal(items.ok, true, items.error);
  return items.items.filter(item => item.type === 'Text');
}

/** The dirty/undo slice every mutation reply carries. */
function pick(reply) {
  return { revision: reply.revision, dirty: reply.dirty, canUndo: reply.canUndo, canRedo: reply.canRedo };
}

/** The union of a quad list: the same box the `annots` reply reports for markup. */
function quadBox(quads) {
  const xs = quads.flatMap(quad => [quad[0], quad[2], quad[4], quad[6]]);
  const ys = quads.flatMap(quad => [quad[1], quad[3], quad[5], quad[7]]);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}

function assertClose(actual, expected, label, epsilon = 1e-3) {
  assert.equal(actual.length, expected.length, label);
  actual.forEach((value, index) => {
    assert.ok(Math.abs(value - expected[index]) < epsilon, `${label}[${index}]: ${value} vs ${expected[index]}`);
  });
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

function pdfWithSignatureField() {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>',
    '<< /Length 2 >>\nstream\nq \nendstream',
    '<< /Fields [6 0 R] /SigFlags 3 >>',
    '<< /Type /Annot /Subtype /Widget /FT /Sig /T (sig1) /Rect [72 72 300 120] /P 3 0 R /V 7 0 R >>',
    '<< /Type /Sig /Filter /Adobe.PPKLite >>',
  ]);
}

function pdfCertified() {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /Perms << /DocMDP 3 0 R >> >>',
    '<< /Type /Pages /Kids [4 0 R] /Count 1 >>',
    '<< /Type /Sig /Filter /Adobe.PPKLite >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>',
    '<< /Length 2 >>\nstream\nq \nendstream',
  ]);
}

async function encryptedFixture(session) {
  const bytes = await readFile('fixtures/encrypted.pdf');
  const reply = engine.dispatch({ id: 1, op: 'open', name: 'encrypted.pdf', pdf: toArrayBuffer(bytes) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

/** AES-256 with an empty user password: it opens without a prompt but is still encrypted. */
async function emptyPasswordFixture(session) {
  const plain = mupdf.Document.openDocument(toArrayBuffer(await readFile(HELLO)), 'application/pdf').asPDF();
  const buffer = plain.saveToBuffer('incremental=no,encrypt=aes-256,user-password=,owner-password=owner');
  const bytes = toArrayBuffer(buffer.asUint8Array());
  buffer.destroy();
  plain.destroy();
  const reply = engine.dispatch({ id: 1, op: 'open', name: 'empty-password.pdf', pdf: bytes }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

test('the journal is enabled at open: begin/endOperation is undoable, disableJS first', async () => {
  const bytes = await readFile(HELLO);
  const { doc } = engine.openDocument(toArrayBuffer(bytes));
  try {
    const pdf = doc.asPDF();
    assert.equal(pdf.getJournal().position, 0, 'a fresh open starts at journal position 0');
    pdf.beginOperation('add note');
    const page = doc.loadPage(0);
    const annot = page.createAnnotation('Text');
    annot.setRect([100, 100, 100 + NOTE_SIZE, 100 + NOTE_SIZE]);
    annot.setContents('PDF4All journal probe');
    annot.update();
    page.destroy();
    pdf.endOperation();
    assert.equal(pdf.canUndo(), true, 'without enableJournal() this stays false and nothing is journaled');
    assert.equal(pdf.canRedo(), false);
    assert.equal(pdf.getJournal().position, 1);
    pdf.undo();
    assert.equal(pdf.canUndo(), false);
    assert.equal(pdf.canRedo(), true);
    pdf.redo();
    assert.equal(pdf.canUndo(), true);
  } finally {
    engine.closeDocument(doc);
  }
});

test('add, list, undo and redo a sticky note with revision, dirty and canUndo in every reply', async () => {
  const session = await open('hello');
  const before = engine.dispatch({ id: 2, op: 'annots', page: 0 }, session).reply;
  assert.equal(before.ok, true);
  assert.deepEqual(before.items, []);

  const added = engine.dispatch({
    id: 3, op: 'annot-add', page: 0, type: 'Text', point: [100, 100], contents: 'PDF4All note',
  }, session).reply;
  assert.equal(added.ok, true, added.error);
  assert.equal(added.revision, 1);
  assert.equal(added.dirty, true);
  assert.equal(added.canUndo, true);
  assert.equal(added.canRedo, false);

  const notes = noteTexts(session);
  assert.equal(notes.length, 1);
  assert.equal(notes[0].type, 'Text');
  assert.equal(notes[0].contents, 'PDF4All note');
  assert.equal(notes[0].editable, true);
  assert.deepEqual(notes[0].rect, [100, 100, 100 + NOTE_SIZE, 100 + NOTE_SIZE], 'the point is the top-left of a 20x20 note');
  assert.ok(notes[0].id > 0, 'the id is the annotation object number');

  const rendered = engine.dispatch({ id: 4, op: 'render', page: 0, scale: 1 }, session).reply;
  assert.equal(rendered.ok, true, rendered.error);
  assert.equal(rendered.revision, 1, 'every render reply carries the revision caches key on');

  const undone = engine.dispatch({ id: 5, op: 'undo' }, session).reply;
  assert.equal(undone.ok, true, undone.error);
  assert.deepEqual({ revision: undone.revision, dirty: undone.dirty, canUndo: undone.canUndo, canRedo: undone.canRedo },
    { revision: 2, dirty: false, canUndo: false, canRedo: true });
  assert.deepEqual(noteTexts(session), [], 'undo removes the note from the page listing');

  const redone = engine.dispatch({ id: 6, op: 'redo' }, session).reply;
  assert.equal(redone.ok, true, redone.error);
  assert.deepEqual({ revision: redone.revision, dirty: redone.dirty, canUndo: redone.canUndo, canRedo: redone.canRedo },
    { revision: 3, dirty: true, canUndo: true, canRedo: false });
  assert.equal(noteTexts(session)[0].id, notes[0].id, 'redo restores the same object number');
  assert.equal(noteTexts(session)[0].contents, 'PDF4All note');

  assert.equal(engine.dispatch({ id: 7, op: 'redo' }, session).reply.ok, false, 'redo with nothing to redo is an error');
  assert.match(engine.dispatch({ id: 8, op: 'redo' }, session).reply.error, /nothing to redo/);
  engine.closeDocument(session.doc);
});

test('edit and delete a note, and ids stay put across an unrelated add', async () => {
  const session = await open('hello');
  const first = engine.dispatch({
    id: 2, op: 'annot-add', page: 0, type: 'Text', point: [72, 72], contents: 'first',
  }, session).reply;
  assert.equal(first.ok, true, first.error);
  const firstId = noteTexts(session)[0].id;

  const second = engine.dispatch({
    id: 3, op: 'annot-add', page: 0, type: 'Text', point: [200, 200], contents: 'second',
  }, session).reply;
  assert.equal(second.ok, true, second.error);
  assert.equal(second.revision, 2);
  const ids = noteTexts(session).map(note => note.id);
  assert.equal(ids.length, 2);
  assert.equal(ids[0], firstId, 'an unrelated add does not renumber the earlier note');

  const edited = engine.dispatch({ id: 4, op: 'annot-set', page: 0, annotId: firstId, contents: 'first edited' }, session).reply;
  assert.equal(edited.ok, true, edited.error);
  assert.equal(edited.revision, 3);
  assert.equal(edited.dirty, true);
  assert.equal(noteTexts(session).find(note => note.id === firstId).contents, 'first edited');

  const deleted = engine.dispatch({ id: 5, op: 'annot-delete', page: 0, annotId: firstId }, session).reply;
  assert.equal(deleted.ok, true, deleted.error);
  assert.equal(deleted.revision, 4);
  assert.deepEqual(noteTexts(session).map(note => note.id), [ids[1]]);

  // A deleted note takes its auto-created Popup with it: /Annots holds only the survivor and its Popup.
  const page = session.doc.loadPage(0);
  const referenced = page.getObject().get('Annots');
  const kinds = [];
  for (let index = 0; index < referenced.length; index++) {
    kinds.push(referenced.get(index).get('Subtype').asName());
  }
  page.destroy();
  assert.deepEqual(kinds.sort(), ['Popup', 'Text'], 'no orphan Popup from the deleted note stays in /Annots');
  engine.closeDocument(session.doc);
});

test('a mutation refuses locked, signed and certified documents, and an empty-password one is editable', async () => {
  const cases = [];
  const locked = await encryptedFixture(engine.createSession());
  assert.equal(engine.dispatch({ id: 2, op: 'annots', page: 0 }, locked).reply.ok, false, 'listing a locked document is refused');
  const lockedAdd = engine.dispatch({ id: 3, op: 'annot-add', page: 0, type: 'Text', point: [10, 10], contents: 'x' }, locked).reply;
  assert.equal(lockedAdd.ok, false);
  assert.equal(lockedAdd.reason, 'read-only');
  cases.push(['locked', lockedAdd.reason]);
  engine.closeDocument(locked.doc);

  // P2 unit 10: an AES-256 document whose empty user password opened it is unlocked
  // and grants every permission, so it is editable - but it is still encrypted, and
  // the copy must keep that encryption.
  const unlocked = await emptyPasswordFixture(engine.createSession());
  const unlockedAdd = engine.dispatch({ id: 2, op: 'annot-add', page: 0, type: 'Text', point: [10, 10], contents: 'x' }, unlocked).reply;
  assert.equal(unlockedAdd.ok, true, unlockedAdd.error);
  const kept = engine.dispatch({ id: 3, op: 'save' }, unlocked).reply;
  assert.equal(kept.ok, true, kept.error);
  cases.push(['empty-password', 'editable']);
  engine.closeDocument(unlocked.doc);

  for (const [label, pdf] of [['signed', pdfWithSignatureField()], ['certified', pdfCertified()]]) {
    const session = engine.createSession();
    assert.equal(engine.dispatch({ id: 1, op: 'open', name: `${label}.pdf`, pdf }, session).reply.ok, true);
    const refused = engine.dispatch({ id: 2, op: 'annot-add', page: 0, type: 'Text', point: [10, 10], contents: 'x' }, session).reply;
    assert.equal(refused.ok, false, label);
    assert.equal(refused.reason, 'read-only', label);
    cases.push([label, refused.reason]);
    // Reading the list is still allowed; nothing is editable on a read-only document.
    const listed = engine.dispatch({ id: 3, op: 'annots', page: 0 }, session).reply;
    assert.equal(listed.ok, true, label);
    assert.equal(listed.items.every(item => item.editable === false), true, label);
    engine.closeDocument(session.doc);
  }
  assert.deepEqual(cases.map(([, reason]) => reason), ['read-only', 'editable', 'read-only', 'read-only']);
});

test('bad page, id, type and contents are refused without mutating or dirtying the document', async () => {
  const session = await open('hello');
  const rejected = [
    ['page out of range', { op: 'annot-add', page: 5, type: 'Text', point: [10, 10], contents: 'x' }],
    ['fractional page', { op: 'annot-add', page: 0.5, type: 'Text', point: [10, 10], contents: 'x' }],
    ['unsupported type', { op: 'annot-add', page: 0, type: 'Square', point: [10, 10], contents: 'x' }],
    ['missing point', { op: 'annot-add', page: 0, type: 'Text', contents: 'x' }],
    ['short point', { op: 'annot-add', page: 0, type: 'Text', point: [10], contents: 'x' }],
    ['NaN point', { op: 'annot-add', page: 0, type: 'Text', point: [Number.NaN, 10], contents: 'x' }],
    ['non-string contents', { op: 'annot-add', page: 0, type: 'Text', point: [10, 10], contents: 42 }],
    ['oversized contents', { op: 'annot-add', page: 0, type: 'Text', point: [10, 10], contents: 'a'.repeat(MAX_NOTE_CHARS + 1) }],
    ['unknown annotation id', { op: 'annot-set', page: 0, annotId: 999, contents: 'x' }],
    ['non-integer id', { op: 'annot-set', page: 0, annotId: 1.5, contents: 'x' }],
    ['delete unknown id', { op: 'annot-delete', page: 0, annotId: 999 }],
  ];
  for (const [index, [label, request]] of rejected.entries()) {
    const reply = engine.dispatch({ id: 100 + index, ...request }, session).reply;
    assert.equal(reply.ok, false, label);
    assert.equal(typeof reply.error, 'string', label);
  }
  assert.deepEqual(noteTexts(session), [], 'nothing was created');
  assert.equal(session.revision, 0, 'no revision bump for a refused mutation');
  assert.equal(engine.journalState(session).dirty, false, 'a refused mutation leaves the document clean');

  assert.equal(engine.dispatch({ id: 200, op: 'save' }, engine.createSession()).reply.reason, 'no-document');
  assert.equal(engine.dispatch({ id: 201, op: 'undo' }, engine.createSession()).reply.reason, 'no-document');
  assert.equal(engine.dispatch({ id: 202, op: 'saved' }, engine.createSession()).reply.reason, 'no-document');
  engine.closeDocument(session.doc);
});

test('saved resets the dirty baseline, and undoing back to it is clean again', async () => {
  const session = await open('hello');
  const added = engine.dispatch({ id: 2, op: 'annot-add', page: 0, type: 'Text', point: [50, 50], contents: 'keep' }, session).reply;
  assert.equal(added.dirty, true);

  const saved = engine.dispatch({ id: 3, op: 'save' }, session).reply;
  assert.equal(saved.ok, true, saved.error);
  assert.equal(saved.dirty, true, 'serializing does not clear dirty; only the write does');
  // MuPDF has no mark-clean: hasUnsavedChanges() stays true after a saveToBuffer
  // (garbage=yes included), which is why dirty is the journal position compared
  // with a saved baseline.
  assert.equal(session.doc.asPDF().hasUnsavedChanges(), true, 'hasUnsavedChanges cannot be the dirty flag');

  const validated = engine.dispatch({ id: 4, op: 'validate', pdf: saved.bytes }, session).reply;
  assert.equal(validated.ok, true, validated.error);

  const marked = engine.dispatch({ id: 5, op: 'saved' }, session).reply;
  assert.equal(marked.ok, true, marked.error);
  assert.equal(marked.dirty, false, 'the saved op is the only thing that clears dirty');
  assert.equal(marked.revision, 1, 'saved is not a mutation');

  const second = engine.dispatch({ id: 6, op: 'annot-add', page: 0, type: 'Text', point: [80, 80], contents: 'new' }, session).reply;
  assert.equal(second.dirty, true);
  assert.equal(second.revision, 2);
  const undo = engine.dispatch({ id: 7, op: 'undo' }, session).reply;
  assert.equal(undo.dirty, false, 'undoing back to the saved state is clean again');
  assert.equal(undo.revision, 3);
  engine.closeDocument(session.doc);
});

test('a saved note reopens with the contents, and every page op still refuses a locked document', async () => {
  const session = await open('hello');
  assert.equal(engine.dispatch({ id: 2, op: 'annot-add', page: 0, type: 'Text', point: [120, 120], contents: 'PDF4All P2 note' }, session).reply.ok, true);
  const saved = engine.dispatch({ id: 3, op: 'save' }, session).reply;
  const validated = engine.dispatch({ id: 4, op: 'validate', pdf: saved.bytes }, session).reply;
  assert.equal(validated.ok, true, validated.error);

  const reopened = engine.openDocument(validated.bytes);
  try {
    const page = reopened.doc.loadPage(0);
    const notes = page.getAnnotations().filter(annot => annot.getType() === 'Text');
    assert.equal(notes.length, 1);
    assert.equal(notes[0].getContents(), 'PDF4All P2 note');
    assert.equal(notes[0].getAuthor(), '', 'nothing personal is written into the annotation');
    page.destroy();
  } finally {
    engine.closeDocument(reopened.doc);
  }

  const locked = await encryptedFixture(engine.createSession());
  for (const [label, request] of [
    ['thumbnail', { id: 5, op: 'thumbnail', page: 0, maxEdge: 128 }],
    ['annots', { id: 6, op: 'annots', page: 0 }],
  ]) {
    const reply = engine.dispatch(request, locked).reply;
    assert.equal(reply.ok, false, label);
    assert.match(reply.error, /encrypted/, label);
  }
  engine.closeDocument(locked.doc);
  engine.closeDocument(session.doc);
});

// ------------------------------------------------------------------ unit 2

test('quad space proof: search hit quads round-trip through setQuadPoints and the saved rect encloses the hit', async () => {
  const session = await open('hello');
  const search = engine.dispatch({ id: 2, op: 'search', query: 'PDF4All' }, session).reply;
  assert.equal(search.ok, true, search.error);
  assert.equal(search.hits.length, 1, 'hello.pdf has exactly one match');
  const hit = search.hits[0];
  assert.equal(hit.page, 0);

  const added = engine.dispatch({ id: 3, op: 'annot-add', page: 0, type: 'Highlight', quads: hit.quads, contents: '' }, session).reply;
  assert.equal(added.ok, true, added.error);
  assert.deepEqual(pick(added), { revision: 1, dirty: true, canUndo: true, canRedo: false });

  // In memory: getQuadPoints returns the hit's own y-down page points (float32 rounding only),
  // and the listed rect is the quad union because markup has no getRect() accessor in this build.
  const listed = engine.dispatch({ id: 4, op: 'annots', page: 0 }, session).reply;
  assert.equal(listed.ok, true, listed.error);
  assert.deepEqual(listed.items.map(item => item.type), ['Highlight']);
  assert.equal(listed.items[0].editable, true, 'markup is editable');
  assertClose(listed.items[0].rect, quadBox(hit.quads), 'listed rect');
  const page = session.doc.loadPage(0);
  const annot = page.getAnnotations().find(candidate => candidate.getType() === 'Highlight');
  assert.ok(annot, 'the highlight is on the page');
  assert.equal(annot.getObject().get('AP').isNull(), false, 'update() wrote an appearance stream');
  const returned = annot.getQuadPoints();
  assert.equal(returned.length, hit.quads.length);
  hit.quads.forEach((quad, index) => assertClose(returned[index], quad, `quad ${index}`));
  page.destroy();

  // The saved PDF: the writer flips the quads to PDF y-up and writes the /Rect itself.
  const saved = engine.dispatch({ id: 5, op: 'save' }, session).reply;
  assert.equal(saved.ok, true, saved.error);
  const reopened = engine.openDocument(saved.bytes);
  try {
    const rpage = reopened.doc.loadPage(0);
    const rannot = rpage.getAnnotations().find(candidate => candidate.getType() === 'Highlight');
    const bounds = rannot.getBounds(); // the y-down accessor markup does have
    const [x0, y0, x1, y1] = quadBox(hit.quads);
    assert.ok(bounds[0] <= x0 + 1e-3 && bounds[1] <= y0 + 1e-3 && bounds[2] >= x1 - 1e-3 && bounds[3] >= y1 - 1e-3,
      `getBounds() ${bounds} encloses the hit`);
    assert.equal(rannot.getObject().get('Rect').isNull(), false, 'the saved annotation has a /Rect');
    rpage.destroy();
  } finally {
    engine.closeDocument(reopened.doc);
  }
  engine.closeDocument(session.doc);
});

const NEW_TYPES = [
  ['Highlight', quads => ({ op: 'annot-add', page: 0, type: 'Highlight', quads, contents: 'PDF4All comment' })],
  ['Underline', quads => ({ op: 'annot-add', page: 0, type: 'Underline', quads, contents: 'PDF4All comment' })],
  ['StrikeOut', quads => ({ op: 'annot-add', page: 0, type: 'StrikeOut', quads, contents: 'PDF4All comment' })],
  ['FreeText', () => ({ op: 'annot-add', page: 0, type: 'FreeText', point: [200, 300], contents: 'PDF4All text box' })],
];

for (const [type, shape] of NEW_TYPES) {
  test(`add, undo, redo and delete a ${type} keeps revision and dirty coherent`, async () => {
    const session = await open('hello');
    const quads = engine.dispatch({ id: 2, op: 'search', query: 'PDF4All' }, session).reply.hits[0].quads;

    const added = engine.dispatch({ id: 3, ...shape(quads) }, session).reply;
    assert.equal(added.ok, true, added.error);
    assert.deepEqual(pick(added), { revision: 1, dirty: true, canUndo: true, canRedo: false }, type);
    const listed = engine.dispatch({ id: 4, op: 'annots', page: 0 }, session).reply;
    assert.equal(listed.ok, true, listed.error);
    assert.deepEqual(listed.items.map(item => item.type), [type]);
    assert.equal(listed.items[0].editable, true, type);
    const id = listed.items[0].id;

    const undone = engine.dispatch({ id: 5, op: 'undo' }, session).reply;
    assert.deepEqual(pick(undone), { revision: 2, dirty: false, canUndo: false, canRedo: true }, type);
    assert.deepEqual(engine.dispatch({ id: 6, op: 'annots', page: 0 }, session).reply.items, []);

    const redone = engine.dispatch({ id: 7, op: 'redo' }, session).reply;
    assert.deepEqual(pick(redone), { revision: 3, dirty: true, canUndo: true, canRedo: false }, type);
    assert.deepEqual(engine.dispatch({ id: 8, op: 'annots', page: 0 }, session).reply.items.map(item => item.id), [id]);

    const deleted = engine.dispatch({ id: 9, op: 'annot-delete', page: 0, annotId: id }, session).reply;
    assert.deepEqual(pick(deleted), { revision: 4, dirty: true, canUndo: true, canRedo: false }, type);
    assert.deepEqual(engine.dispatch({ id: 10, op: 'annots', page: 0 }, session).reply.items, []);
    // No orphan Popup or leftover object survives a markup/FreeText delete.
    const page = session.doc.loadPage(0);
    const annots = page.getObject().get('Annots');
    assert.equal(annots.isNull() ? 0 : annots.length, 0, `${type} leaves nothing in /Annots`);
    page.destroy();
    engine.closeDocument(session.doc);
  });
}

test('an edit reaches every new type, and the five types save, validate and reopen with no /T', async () => {
  const session = await open('hello');
  const quads = engine.dispatch({ id: 2, op: 'search', query: 'PDF4All' }, session).reply.hits[0].quads;
  const adds = [
    { op: 'annot-add', page: 0, type: 'Text', point: [100, 100], contents: 'PDF4All note' },
    { op: 'annot-add', page: 0, type: 'Highlight', quads, contents: 'PDF4All comment' },
    { op: 'annot-add', page: 0, type: 'Underline', quads, contents: 'PDF4All comment' },
    { op: 'annot-add', page: 0, type: 'StrikeOut', quads, contents: 'PDF4All comment' },
    { op: 'annot-add', page: 0, type: 'FreeText', point: [200, 300], contents: 'PDF4All text box' },
  ];
  let revision = 0;
  for (const add of adds) {
    const reply = engine.dispatch({ id: 100 + revision, ...add }, session).reply;
    assert.equal(reply.ok, true, `${add.type}: ${reply.error}`);
    revision += 1;
    assert.equal(reply.revision, revision, add.type);
  }

  const listed = engine.dispatch({ id: 200, op: 'annots', page: 0 }, session).reply;
  assert.equal(listed.ok, true, listed.error);
  assert.deepEqual(listed.items.map(item => item.type), ['Text', 'Highlight', 'Underline', 'StrikeOut', 'FreeText']);
  assert.equal(listed.items.every(item => item.editable), true, 'all five types are editable');

  for (const item of listed.items) {
    const edited = engine.dispatch({ id: 300, op: 'annot-set', page: 0, annotId: item.id, contents: `edited ${item.type}` }, session).reply;
    assert.equal(edited.ok, true, `${item.type}: ${edited.error}`);
    revision += 1;
    assert.equal(edited.revision, revision, item.type);
  }

  const saved = engine.dispatch({ id: 400, op: 'save' }, session).reply;
  const validated = engine.dispatch({ id: 401, op: 'validate', pdf: saved.bytes }, session).reply;
  assert.equal(validated.ok, true, validated.error);
  const reopened = engine.openDocument(validated.bytes);
  try {
    const page = reopened.doc.loadPage(0);
    const annots = page.getAnnotations().filter(annot => annot.getType() !== 'Popup');
    const byType = Object.fromEntries(annots.map(annot => [annot.getType(), annot]));
    for (const type of ['Text', 'Highlight', 'Underline', 'StrikeOut', 'FreeText']) {
      assert.ok(byType[type], `${type} survived the round trip`);
      assert.equal(byType[type].getContents(), `edited ${type}`);
      assert.equal(byType[type].getAuthor(), '', `${type} writes no /T`);
      assert.equal(byType[type].getObject().get('AP').isNull(), false, `${type} has an appearance stream`);
    }
    assert.equal(byType.FreeText.getDefaultAppearance().font, 'Helv');
    assertClose(byType.FreeText.getRect(), [200, 300, 360, 340], 'the text box is 160x40 from the point');
    page.destroy();
  } finally {
    engine.closeDocument(reopened.doc);
  }
  engine.closeDocument(session.doc);
});

test('off-page and malformed markup geometry is refused without dirtying the document', async () => {
  const session = await open('hello');
  const quads = engine.dispatch({ id: 2, op: 'search', query: 'PDF4All' }, session).reply.hits[0].quads;
  const rejected = [
    ['off-page point', { op: 'annot-add', page: 0, type: 'FreeText', point: [700, 300], contents: 'x' }],
    ['off-page quad', { op: 'annot-add', page: 0, type: 'Highlight', quads: [[700, 10, 800, 10, 700, 30, 800, 30]], contents: 'x' }],
    ['65 quads', { op: 'annot-add', page: 0, type: 'Highlight', quads: Array.from({ length: 65 }, () => quads[0]), contents: 'x' }],
    ['no quads', { op: 'annot-add', page: 0, type: 'Highlight', contents: 'x' }],
    ['7-number quad', { op: 'annot-add', page: 0, type: 'Highlight', quads: [[1, 2, 3, 4, 5, 6, 7]], contents: 'x' }],
    ['NaN quad', { op: 'annot-add', page: 0, type: 'Underline', quads: [[0, 0, 1, 0, 1, 1, Number.NaN, 1]], contents: 'x' }],
    ['non-string contents', { op: 'annot-add', page: 0, type: 'StrikeOut', quads, contents: 42 }],
    ['unsupported type', { op: 'annot-add', page: 0, type: 'Ink', point: [10, 10], contents: 'x' }],
    ['off-page text box', { op: 'annot-add', page: 0, type: 'FreeText', point: [-1, 300], contents: 'x' }],
  ];
  for (const [index, [label, request]] of rejected.entries()) {
    const reply = engine.dispatch({ id: 50 + index, ...request }, session).reply;
    assert.equal(reply.ok, false, label);
    assert.equal(typeof reply.error, 'string', label);
  }
  assert.deepEqual(engine.dispatch({ id: 99, op: 'annots', page: 0 }, session).reply.items, [], 'nothing was created');
  assert.equal(session.revision, 0, 'no revision bump for a refused add');
  assert.equal(engine.journalState(session).dirty, false, 'a refused add leaves the document clean');
  engine.closeDocument(session.doc);
});

test('a text box whose point is near the page edge is clamped onto the page, not refused', async () => {
  const session = await open('hello');
  const added = engine.dispatch({ id: 2, op: 'annot-add', page: 0, type: 'FreeText', point: [600, 780], contents: 'edge' }, session).reply;
  assert.equal(added.ok, true, added.error);
  const item = engine.dispatch({ id: 3, op: 'annots', page: 0 }, session).reply.items[0];
  assert.deepEqual(item.rect, [452, 752, 612, 792], 'the 160x40 box is moved inside the 612x792 page');
  engine.closeDocument(session.doc);
});
