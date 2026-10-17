/**
 * Save tickets (engine/design/app-m9-recovery.md, "Save tickets"): `save` names the state it
 * captured, and `saved` with that ticket marks only that state clean. An edit made while main
 * writes a save in place stays unsaved. Runs on MuPDF here and on Rust under tests/run-rust.mjs.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';
import { fingerprint, RecoveryLog, replay } from '../build/renderer/recovery.js';

const hello = await readFile('fixtures/hello.pdf');
const pdf = () => hello.buffer.slice(hello.byteOffset, hello.byteOffset + hello.byteLength);
const form = await readFile('fixtures/form.pdf');
const formPdf = () => form.buffer.slice(form.byteOffset, form.byteOffset + form.byteLength);

function client(log = null) {
  const engine = createEngine(mupdf);
  const session = engine.createSession();
  let id = 1;
  const send = message => {
    const kept = log?.keep(message) ?? null;
    const reply = engine.dispatch(structuredClone({ ...message, id: id++ }), session).reply;
    // As the renderer: a save's ticket is noted where it captured (onWorkerMessage).
    if (message.op === 'save' && reply.ok !== false) log?.noteSave(reply.ticket);
    if (kept && reply.ok !== false) log.record(kept, reply);
    return reply;
  };
  return send;
}

const ok = (reply, what) => {
  assert.notEqual(reply.ok, false, `${what}: ${reply.error}`);
  return reply;
};
const note = (send, contents) => ok(send({ op: 'annot-add', page: 0, type: 'Text', point: [72, 72], contents, today: '2026-10-17' }), contents);

test('an edit made while the save is written stays unsaved; undoing it is clean', () => {
  const send = client();
  ok(send({ op: 'open', pdf: pdf(), name: 'hello.pdf' }), 'open');
  note(send, 'saved one');
  const save = ok(send({ op: 'save' }), 'save');
  assert.equal(typeof save.ticket, 'string');
  note(send, 'made during the write');
  const marked = ok(send({ op: 'saved', ticket: save.ticket }), 'saved');
  assert.equal(marked.dirty, true, 'the edit after the capture is not on disk');
  assert.equal(ok(send({ op: 'undo' }), 'undo').dirty, false, 'back at the saved state');
  assert.equal(ok(send({ op: 'redo' }), 'redo').dirty, true);
  // Nothing changed between the capture and the acknowledgement: clean.
  const again = ok(send({ op: 'save' }), 'save again');
  assert.equal(ok(send({ op: 'saved', ticket: again.ticket }), 'saved again').dirty, false);
  // Without a ticket, `saved` keeps its old meaning: the current state.
  note(send, 'later');
  assert.equal(ok(send({ op: 'saved' }), 'saved without a ticket').dirty, false);
});

test('a ticket whose state an edit after an undo removed never reads clean', () => {
  const send = client();
  ok(send({ op: 'open', pdf: pdf(), name: 'hello.pdf' }), 'open');
  note(send, 'saved one');
  const save = ok(send({ op: 'save' }), 'save');
  ok(send({ op: 'undo' }), 'undo');
  note(send, 'another branch');
  assert.equal(ok(send({ op: 'saved', ticket: save.ticket }), 'saved').dirty, true);
  assert.equal(ok(send({ op: 'undo' }), 'undo').dirty, true, 'the opened state is not what is on disk');
  assert.equal(ok(send({ op: 'redo' }), 'redo').dirty, true);
});

test('a security change made while the save is written stays pending', () => {
  const send = client();
  ok(send({ op: 'open', pdf: pdf(), name: 'hello.pdf' }), 'open');
  note(send, 'one');
  const save = ok(send({ op: 'save' }), 'save');
  ok(send({ op: 'security-set', mode: 'none' }), 'security');
  assert.equal(ok(send({ op: 'saved', ticket: save.ticket }), 'saved').dirty, true);
  const again = ok(send({ op: 'save' }), 'save again');
  assert.equal(ok(send({ op: 'saved', ticket: again.ticket }), 'saved again').dirty, false);
});

test('a ticket of another document or not issued by the engine is refused', () => {
  const send = client();
  ok(send({ op: 'open', pdf: pdf(), name: 'hello.pdf' }), 'open');
  const save = ok(send({ op: 'save' }), 'save');
  const bogus = send({ op: 'saved', ticket: 'not a ticket' });
  assert.equal(bogus.ok, false);
  assert.equal(bogus.reason, 'invalid');
  ok(send({ op: 'open', pdf: pdf(), name: 'hello.pdf' }), 'reopen');
  const stale = send({ op: 'saved', ticket: save.ticket });
  assert.equal(stale.ok, false);
  assert.equal(stale.reason, 'invalid');
});

test('a logged saved with its ticket replays to the same dirty state', async () => {
  const log = new RecoveryLog();
  const send = client(log);
  ok(send({ op: 'open', pdf: pdf(), name: 'hello.pdf' }), 'open');
  note(send, 'saved one');
  const save = ok(send({ op: 'save' }), 'save');
  note(send, 'made during the write');
  const marked = ok(send({ op: 'saved', ticket: save.ticket }), 'saved');
  assert.deepEqual(log.entries.map(entry => entry.op), ['annot-add', 'annot-add', 'saved']);
  const restarted = client();
  const result = await replay(log, async message => ok(restarted(message), `replay ${message.op}`));
  assert.equal(fingerprint(result.journal), fingerprint(marked));
  assert.equal(result.journal.dirty, true);
  assert.equal(ok(restarted({ op: 'undo' }), 'undo after replay').dirty, false);
});

// M9 re-review (major): the app opens its startup fixture first, so a user's document is
// generation 2 or more, and a log holding only `saved` must still replay at that generation.
test('a ticketed log of a document opened second replays at its generation', async () => {
  const log = new RecoveryLog();
  const send = client(log);
  ok(send({ op: 'open', pdf: formPdf(), name: 'startup.pdf' }), 'the startup document');
  ok(send({ op: 'open', pdf: pdf(), name: 'hello.pdf' }), 'open');
  const save = ok(send({ op: 'save' }), 'save');
  assert.ok(save.ticket.startsWith('2:'), save.ticket);
  ok(send({ op: 'saved', ticket: save.ticket }), 'saved');
  note(send, 'three, unsaved');
  assert.equal(log.generation(), 2, 'the generation comes from the ticket');
  const restarted = client();
  const result = await replay(log, async message => ok(restarted(message), `replay ${message.op}`));
  assert.equal(result.steps, 2);
  assert.equal(result.journal.dirty, true);
  assert.equal(ok(restarted({ op: 'annots', page: 0 }), 'annots').items.length, 1);
  assert.equal(ok(restarted({ op: 'undo' }), 'undo').dirty, false, 'the save is the clean baseline');
});

test('two saves in place, a security change on an earlier document, then a replay', async () => {
  const log = new RecoveryLog();
  const send = client(log);
  ok(send({ op: 'open', pdf: formPdf(), name: 'earlier.pdf' }), 'earlier');
  // Rust counts security changes over the session, not per document.
  ok(send({ op: 'security-set', mode: 'none' }), 'security on the earlier document');
  ok(send({ op: 'open', pdf: pdf(), name: 'hello.pdf' }), 'open');
  note(send, 'one');
  const first = ok(send({ op: 'save' }), 'save one');
  ok(send({ op: 'saved', ticket: first.ticket }), 'saved one');
  note(send, 'two');
  const second = ok(send({ op: 'save' }), 'save two');
  note(send, 'made during the second write');
  const marked = ok(send({ op: 'saved', ticket: second.ticket }), 'saved two');
  assert.equal(marked.dirty, true);
  const restarted = client();
  const result = await replay(log, async message => ok(restarted(message), `replay ${message.op}`));
  assert.equal(fingerprint(result.journal), fingerprint(marked));
  assert.equal(ok(restarted({ op: 'undo' }), 'undo').dirty, false);
});

test('a saved that arrives after a restore is accepted by the restored session', async () => {
  const log = new RecoveryLog();
  const send = client(log);
  ok(send({ op: 'open', pdf: formPdf(), name: 'startup.pdf' }), 'the startup document');
  ok(send({ op: 'security-set', mode: 'none' }), 'security on the startup document');
  ok(send({ op: 'open', pdf: pdf(), name: 'hello.pdf' }), 'open');
  note(send, 'one');
  const save = ok(send({ op: 'save' }), 'save');
  // The worker stops while main writes; the restore replays the note.
  const restarted = client();
  await replay(log, async message => ok(restarted(message), `replay ${message.op}`));
  log.restored();
  const late = restarted({ op: 'saved', ticket: log.ticketNow(save.ticket) });
  assert.notEqual(late.ok, false, late.error);
  assert.equal(late.dirty, false);
});
