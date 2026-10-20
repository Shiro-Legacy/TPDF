/**
 * P3 U1 Gate C — the shared mutation helper: cross-save undo/redo and refused
 * requests that leave the journal alone. Engine direct. The abandon-on-failure
 * and savedPosition branch-guard cases patched MuPDF's `createAnnotation` inside
 * the removed TypeScript engine and went with it (a Rust counterpart needs a
 * fault hook in the engine).
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
