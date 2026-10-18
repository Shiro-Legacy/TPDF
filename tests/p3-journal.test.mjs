/**
 * P3 U1 Gate C — the shared mutation helper: abandon-on-failure rollback, the
 * savedPosition branch guard, and cross-save undo/redo. Engine direct.
 *
 * Measured native sequence (grok `p3-unit1-journal-probe.md`, fable51
 * `review-p3-unit1-gateA.md`): `abandonOperation` restores streams and
 * dictionaries and closes the operation; ending a failed operation instead leaves
 * it on the undo stack; and an abandoned operation that wrote anything drops the
 * redo branch natively — exactly as a successful mutation does. That last point is
 * an accepted ceiling, asserted here rather than papered over.
 *
 * Failure injection patches one native method (`PDFPage.prototype.createAnnotation`)
 * for the duration of one dispatch call and restores it in `finally`. No production
 * test hook exists.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';

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

let nextId = 700;
function op(session, request) {
  return engine.dispatch({ id: nextId++, ...request }, session).reply;
}

function state(session) {
  return engine.journalState(session);
}

function pin(session) {
  const current = state(session);
  return { revision: current.revision, dirty: current.dirty, canUndo: current.canUndo, canRedo: current.canRedo };
}

function notes(session, page = 0) {
  const reply = op(session, { op: 'annots', page });
  assert.equal(reply.ok, true, reply.error);
  return reply.items.map(item => item.contents);
}

function addNote(session, contents) {
  return op(session, { op: 'annot-add', page: 0, type: 'Text', point: [100, 100], contents });
}

function save(session) {
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const marked = op(session, { op: 'saved' });
  assert.equal(marked.ok, true, marked.error);
  return saved.bytes;
}

function sameBytes(left, right) {
  const a = new Uint8Array(left);
  const b = new Uint8Array(right);
  if (a.byteLength !== b.byteLength) return false;
  for (let index = 0; index < a.byteLength; index++) if (a[index] !== b[index]) return false;
  return true;
}

/** The decoded /Contents bytes of one page, read straight from the session document. */
function contentBytes(session, page = 0) {
  const target = session.doc.loadPage(page);
  try {
    const buffer = target.getObject().get('Contents').readStream();
    try {
      return Buffer.from(buffer.asUint8Array()).toString('latin1');
    } finally {
      buffer.destroy();
    }
  } finally {
    // Production never holds a page wrapper across a mutation. Holding one here
    // would keep a native page view alive through a rolled-back write, and the
    // page's cached annot list then still shows the abandoned annot (measured;
    // reproduce by dropping this destroy and re-running the first test).
    target.destroy();
  }
}

/** True when the saved file still carries this string anywhere, reachable or not. */
function savedContains(bytes, token) {
  return Buffer.from(new Uint8Array(bytes)).toString('latin1').includes(token);
}

/** Reopen saved bytes in a fresh session, validate it, and report the page-1 note list. */
function reopenNotes(bytes) {
  const session = engine.createSession();
  const opened = engine.dispatch({ id: nextId++, op: 'open', name: 'reopened.pdf', pdf: toBytes(bytes) }, session).reply;
  assert.equal(opened.ok, true, opened.error);
  const valid = engine.dispatch({ id: nextId++, op: 'validate', pdf: toBytes(bytes) }, session).reply;
  assert.equal(valid.ok, true, valid.error);
  const list = op(session, { op: 'annots', page: 0 }).items.map(item => item.contents);
  engine.dispatch({ id: nextId++, op: 'close' }, session);
  return list;
}

function toBytes(bytes) {
  return bytes instanceof ArrayBuffer ? bytes : toArrayBuffer(bytes);
}

/**
 * Run `body` with `PDFPage.prototype.createAnnotation` patched to throw. `after`
 * performs the real native call first, so the failure lands after a write;
 * `before` throws with no write at all.
 */
function withInjectedFailure(mode, body) {
  const proto = mupdf.PDFPage.prototype;
  const original = proto.createAnnotation;
  proto.createAnnotation = function patched(...args) {
    if (mode === 'after') original.apply(this, args);
    throw new Error('p3-journal injected failure');
  };
  try {
    return body();
  } finally {
    proto.createAnnotation = original;
  }
}

// ---------------------------------------------------------------- clean baseline

test('a failure after a native write rolls back and leaves a clean baseline untouched', { skip: 'drives MuPDF internals of the removed TypeScript engine (patches mupdf.PDFPage.prototype.createAnnotation; needs a Rust fault hook)' }, async () => {
  const session = await open('p0b/edit-transformed');
  const before = pin(session);
  const contentBefore = contentBytes(session);

  const reply = withInjectedFailure('after', () => addNote(session, 'ABANDONED-TOKEN-9931'));
  assert.equal(reply.ok, false);
  assert.match(reply.error, /p3-journal injected failure/);
  assert.deepEqual(notes(session), [], 'the annotation is gone from the page');
  assert.deepEqual(pin(session), before, 'revision, dirty, undo and redo are unchanged');
  assert.equal(reply.canRedo, false);
  assert.equal(reply.revision, before.revision);
  assert.equal(contentBytes(session), contentBefore, 'the page content stream is byte-identical');
  // Whole-file identity is deliberately NOT asserted: a rolled-back write leaves
  // allocated-but-empty object slots (measured: xref Size 9 vs 7, no content), which
  // the Gate A contract accepts. What must not survive is the content itself.
  const bytesAfter = save(session);
  assert.equal(savedContains(bytesAfter, 'ABANDONED-TOKEN-9931'), false, 'no trace of the abandoned note in the save');
  assert.equal(savedContains(bytesAfter, '/Text'), false, 'no abandoned annotation dictionary either');
  assert.deepEqual(reopenNotes(bytesAfter), []);
});

test('a failure after a write on a dirty baseline keeps the earlier edits and the journal', { skip: 'drives MuPDF internals of the removed TypeScript engine (patches mupdf.PDFPage.prototype.createAnnotation; needs a Rust fault hook)' }, async () => {
  const session = await open('p0b/edit-transformed');
  assert.equal(addNote(session, 'keeps').ok, true);
  const dirty = pin(session);
  assert.equal(dirty.dirty, true);
  assert.equal(dirty.canUndo, true);

  const reply = withInjectedFailure('after', () => addNote(session, 'must not survive'));
  assert.equal(reply.ok, false);
  assert.deepEqual(notes(session), ['keeps']);
  assert.deepEqual(pin(session), dirty);
});

// ------------------------------------------------------------- redo-available

test('a written abandon after an undo restores content, keeps undo, and drops the redo branch', { skip: 'drives MuPDF internals of the removed TypeScript engine (patches mupdf.PDFPage.prototype.createAnnotation; needs a Rust fault hook)' }, async () => {
  const session = await open('p0b/edit-transformed');
  assert.equal(addNote(session, 'A').ok, true);
  assert.equal(addNote(session, 'B').ok, true);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual(notes(session), ['A']);
  const undone = pin(session);
  assert.equal(undone.canRedo, true, 'B is on the redo branch');

  const reply = withInjectedFailure('after', () => addNote(session, 'C'));
  assert.equal(reply.ok, false);
  assert.equal(reply.canRedo, false, 'the failed reply already reports the cut branch');
  assert.deepEqual(notes(session), ['A'], 'content is restored');
  const after = pin(session);
  assert.equal(after.canRedo, false, 'the accepted ceiling: a written abandon cuts redo');
  assert.equal(after.canUndo, true);
  assert.equal(after.dirty, true);
  assert.equal(after.revision, undone.revision, 'a failed mutation does not bump the revision');

  // Undo of the earlier accepted edit still works.
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual(notes(session), []);
});

test('an abandon with no write keeps the redo branch', { skip: 'drives MuPDF internals of the removed TypeScript engine (patches mupdf.PDFPage.prototype.createAnnotation; needs a Rust fault hook)' }, async () => {
  const session = await open('p0b/edit-transformed');
  assert.equal(addNote(session, 'A').ok, true);
  assert.equal(addNote(session, 'B').ok, true);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  const before = pin(session);

  const reply = withInjectedFailure('before', () => addNote(session, 'never written'));
  assert.equal(reply.ok, false);
  assert.deepEqual(pin(session), before, 'no write means no journal step and no branch loss');
  assert.equal(op(session, { op: 'redo' }).ok, true, 'redo still reaches B');
  assert.deepEqual(notes(session), ['A', 'B']);
});

// ------------------------------------------------------- false-clean regression

test('the saved state cannot be reported clean once its branch is cut (the P2 false-clean defect)', { skip: 'drives MuPDF internals of the removed TypeScript engine (patches mupdf.PDFPage.prototype.createAnnotation; needs a Rust fault hook)' }, async () => {
  // edit A, edit B, save, undo, then a new edit: the step count lands back on the
  // saved position, but the saved bytes are unreachable.
  const session = await open('p0b/edit-transformed');
  assert.equal(addNote(session, 'A').ok, true);
  assert.equal(addNote(session, 'B').ok, true);
  save(session);
  assert.equal(state(session).dirty, false, 'saved at the current position');
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(addNote(session, 'C').ok, true);
  assert.deepEqual(notes(session), ['A', 'C']);
  assert.equal(state(session).dirty, true, 'the document no longer matches the saved file');
  save(session);
  assert.equal(state(session).dirty, false, 'a real save makes it clean again');

  // The same sequence where the new edit fails after writing.
  const failing = await open('p0b/edit-transformed');
  assert.equal(addNote(failing, 'A').ok, true);
  assert.equal(addNote(failing, 'B').ok, true);
  save(failing);
  assert.equal(op(failing, { op: 'undo' }).ok, true);
  const reply = withInjectedFailure('after', () => addNote(failing, 'C'));
  assert.equal(reply.ok, false);
  assert.deepEqual(notes(failing), ['A']);
  assert.equal(state(failing).dirty, true, 'the failed edit still cut the saved branch');
  save(failing);
  assert.equal(state(failing).dirty, false);
});

// ------------------------------------------------------------ cross-save undo

test('edit, save, undo, save, redo, save: every save reopens and undo survives the rewrite', async () => {
  const session = await open('p0b/edit-transformed');
  assert.equal(addNote(session, 'A').ok, true);
  const first = save(session);
  assert.deepEqual(reopenNotes(first), ['A']);

  assert.equal(op(session, { op: 'undo' }).ok, true);
  const second = save(session);
  assert.deepEqual(reopenNotes(second), [], 'the undone edit is not in the second save');

  assert.equal(op(session, { op: 'redo' }).ok, true);
  assert.deepEqual(notes(session), ['A'], 'redo still works on the same document after two saves');
  const third = save(session);
  assert.deepEqual(reopenNotes(third), ['A']);

  // And undo again, on the document that has now been rewritten twice.
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual(notes(session), []);
});

// --------------------------------------------------------------------- no-ops

test('a refused request creates no journal step and cannot dirty the document', async () => {
  const session = await open('p0b/edit-transformed');
  const before = pin(session);
  const reply = op(session, { op: 'page-rotate', page: 9, by: 90 });
  assert.equal(reply.ok, false);
  assert.match(reply.error, /page must be an integer/);
  assert.deepEqual(pin(session), before);
  assert.equal(reply.revision, before.revision);
  assert.equal(reply.dirty, false);

  const badType = op(session, { op: 'annot-add', page: 0, type: 'Nope', contents: 'x' });
  assert.equal(badType.ok, false);
  assert.deepEqual(pin(session), before);
});
