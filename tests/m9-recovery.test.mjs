/**
 * M9 recovery for long operations (engine/design/app-m9-recovery.md): the op classification,
 * the recovery log and its budget, and a replay through the real engine (MuPDF here; Rust under
 * `node tests/run-rust.mjs`) that must reproduce ids, revisions, undo history and saved bytes.
 * The renderer's restart path itself is driven in tests/m9-recovery-drive.test.mjs.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';
import {
  fingerprint, MAX_REPLAY_CLOSES, OPS, RECOVERY_BUDGET_BYTES, RecoveryLog, replay, replayed, ReplayMismatch, sizeOf,
} from '../build/renderer/recovery.js';
import { MAX_PDF_BYTES, MAX_SAVE_BYTES } from '../build/renderer/limits.js';

function toArrayBuffer(bytes) {
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

/** Every op of engine.ts's `SessionRequest` union, read from the source. */
async function unionOps() {
  const source = await readFile('src/renderer/engine.ts', 'utf8');
  const union = /export type SessionRequest =([^;]+);/.exec(source)[1];
  const names = union.split('|').map(name => name.trim()).filter(Boolean);
  return names.map(name => {
    const body = new RegExp(`export type ${name} =[\\s\\S]*?op: "([a-z-]+)"`).exec(source);
    assert.ok(body, `${name} names no op`);
    return body[1];
  });
}

test('every SessionRequest op is classified, and only those', async () => {
  const ops = await unionOps();
  assert.equal(new Set(ops).size, ops.length, 'one request type per op');
  assert.deepEqual([...ops].sort(), Object.keys(OPS).sort(), 'recovery.ts OPS must list exactly the SessionRequest ops');
  for (const [op, info] of Object.entries(OPS)) {
    assert.ok(['base', 'mutate', 'unlock', 'clipboard', 'read', 'compare'].includes(info.kind), op);
    assert.match(info.label, /^[A-Z][a-z]/, `${op} has a sentence-case label`);
  }
  // The classification the design note commits to.
  const kinds = kind => Object.keys(OPS).filter(op => OPS[op].kind === kind).sort();
  assert.deepEqual(kinds('base'), ['close', 'open']);
  assert.deepEqual(kinds('unlock'), ['authenticate']);
  assert.deepEqual(kinds('clipboard'), ['clip-copy']);
  for (const op of ['save', 'validate', 'render', 'thumbnail', 'extract-pages', 'pdf-from-images', 'sign-prepare', 'batch-file', 'text-all', 'edit-targets', 'edit-replace-preview', 'redact-preview']) {
    assert.equal(OPS[op].kind, 'read', op);
    assert.equal(replayed(op), false, op);
  }
  for (const op of ['undo', 'redo', 'saved', 'security-set', 'layer-set', 'redact-search', 'redact-apply', 'clip-paste', 'insert-pages', 'stamp']) {
    assert.equal(OPS[op].kind, 'mutate', op);
    assert.equal(replayed(op), true, op);
  }
  assert.equal(replayed('nonsense'), false);
});

test('the budget holds a near-ceiling document, its saved copy and a log', () => {
  assert.equal(RECOVERY_BUDGET_BYTES, 256 * 1024 * 1024);
  assert.ok(MAX_PDF_BYTES + MAX_SAVE_BYTES < RECOVERY_BUDGET_BYTES);
  assert.ok(RECOVERY_BUDGET_BYTES - MAX_PDF_BYTES - MAX_SAVE_BYTES >= 64 * 1024 * 1024);
  assert.equal(sizeOf({ op: 'x', bytes: new ArrayBuffer(1000) }) >= 1000, true);
  assert.ok(sizeOf({ text: 'a'.repeat(500) }) >= 1000, 'strings count two bytes a unit');
  const cyclic = { a: 1 };
  cyclic.self = cyclic;
  assert.ok(sizeOf(cyclic) > 0);
});

test('fingerprints ignore timings and the request id, and see every other field', () => {
  const base = { id: 4, ok: true, revision: 3, dirty: true, annotId: 17, openMs: 12, paintMs: 3, bytes: new ArrayBuffer(8) };
  assert.equal(fingerprint(base), fingerprint({ ...base, id: 99, openMs: 900, paintMs: 1 }));
  assert.notEqual(fingerprint(base), fingerprint({ ...base, annotId: 18 }));
  assert.notEqual(fingerprint(base), fingerprint({ ...base, revision: 4 }));
  assert.notEqual(fingerprint(base), fingerprint({ ...base, bytes: new ArrayBuffer(9) }));
  assert.equal(fingerprint({ ok: true, compareClosed: true }), fingerprint({ ok: true }), 'the compare slot is not part of the document');
});

/** The renderer's request bookkeeping without a worker: keep before posting, record the answer. */
function tracked(log, message, reply) {
  const kept = log.keep(message);
  if (kept && reply.ok !== false) log.record(kept, reply);
  return kept;
}

test('the log keeps changes, skips reads and refusals, and restarts on open and close', () => {
  const log = new RecoveryLog(10_000);
  assert.equal(log.available, false);
  const pdf = new ArrayBuffer(1000);
  tracked(log, { id: 1, op: 'open', pdf, name: 'a.pdf' }, { ok: true, pageCount: 1 });
  assert.equal(log.available, true);
  assert.notEqual(log.base, pdf, 'the base is a copy: the request transfers its own');
  assert.equal(log.base.byteLength, 1000);
  assert.equal(tracked(log, { id: 2, op: 'render', page: 0, scale: 1 }, { ok: true }), null);
  tracked(log, { id: 3, op: 'annot-add', page: 0, type: 'Text', contents: 'x', today: '2026-10-16' }, { ok: true, revision: 1, annotId: 9 });
  tracked(log, { id: 4, op: 'annot-add', page: 0, type: 'Text', contents: 'refused' }, { ok: false, error: 'no' });
  tracked(log, { id: 5, op: 'authenticate', password: 'wrong' }, { ok: true, authenticated: false });
  tracked(log, { id: 6, op: 'undo' }, { ok: true, revision: 2 });
  assert.deepEqual(log.entries.map(entry => entry.op), ['annot-add', 'undo']);
  assert.equal(log.entries[0].message.id, undefined, 'the worker id is not kept');
  assert.equal(log.entries[0].message.today, '2026-10-16', 'injected inputs are kept');
  assert.ok(log.bytes > 1000 && log.bytes < 2000, String(log.bytes));
  tracked(log, { id: 7, op: 'close' }, { ok: true });
  assert.equal(log.available, false);
  assert.equal(log.entries.length, 0);
  assert.equal(log.bytes, 0);
});

test('a log past its budget turns recovery off for the document, until the next open', () => {
  const log = new RecoveryLog(4096);
  tracked(log, { id: 1, op: 'open', pdf: new ArrayBuffer(1000), name: 'a.pdf' }, { ok: true });
  // A refused oversized change keeps recovery on: it changed nothing.
  tracked(log, { id: 2, op: 'insert-pages', pdf: new ArrayBuffer(5000), name: 'b.pdf', at: 0 }, { ok: false });
  assert.equal(log.available, true);
  tracked(log, { id: 3, op: 'annot-add', page: 0, type: 'Text', contents: 'small' }, { ok: true, revision: 1 });
  tracked(log, { id: 4, op: 'insert-pages', pdf: new ArrayBuffer(5000), name: 'b.pdf', at: 0 }, { ok: true, revision: 2 });
  assert.equal(log.available, false);
  assert.equal(log.unavailable, 'budget');
  assert.equal(log.entries.length, 0, 'what it held is freed');
  assert.equal(log.bytes, 0);
  assert.equal(log.fallback(), null);
  tracked(log, { id: 5, op: 'annot-add', page: 0, type: 'Text', contents: 'later' }, { ok: true, revision: 3 });
  assert.equal(log.entries.length, 0);
  tracked(log, { id: 6, op: 'open', pdf: new ArrayBuffer(100), name: 'c.pdf' }, { ok: true });
  assert.equal(log.available, true);
  assert.equal(log.unavailable, null);
  // A document over the budget is never tracked.
  tracked(log, { id: 7, op: 'open', pdf: new ArrayBuffer(5000), name: 'd.pdf' }, { ok: true });
  assert.equal(log.available, false);
  assert.equal(log.unavailable, 'budget');
});

test('a paste is replayable only after a copy the log holds', () => {
  const log = new RecoveryLog();
  tracked(log, { id: 1, op: 'open', pdf: new ArrayBuffer(10), name: 'a.pdf' }, { ok: true });
  tracked(log, { id: 2, op: 'clip-copy', page: 0, targets: [] }, { ok: true });
  tracked(log, { id: 3, op: 'clip-paste', page: 0 }, { ok: true, revision: 1 });
  assert.equal(log.available, true);
  assert.equal(log.clipboardInLog, true);
  // The clipboard outlives an open: a paste into the next document has no copy to replay.
  tracked(log, { id: 4, op: 'open', pdf: new ArrayBuffer(10), name: 'b.pdf' }, { ok: true });
  assert.equal(log.clipboardInLog, false);
  tracked(log, { id: 5, op: 'clip-paste', page: 0 }, { ok: true, revision: 1 });
  assert.equal(log.unavailable, 'clipboard');
});

test('edit handles of one generation only; the replay closes up to that generation', () => {
  const log = new RecoveryLog();
  tracked(log, { id: 1, op: 'open', pdf: new ArrayBuffer(10), name: 'a.pdf' }, { ok: true });
  tracked(log, { id: 2, op: 'edit-text', page: 0, targetId: 't', generation: 5, revision: 0, text: 'A' }, { ok: true, revision: 1 });
  assert.equal(log.generation(), 5);
  tracked(log, { id: 3, op: 'edit-text', page: 0, targetId: 't', generation: 6, revision: 1, text: 'B' }, { ok: true, revision: 2 });
  assert.equal(log.unavailable, 'generation');
  tracked(log, { id: 4, op: 'open', pdf: new ArrayBuffer(10), name: 'a.pdf' }, { ok: true });
  tracked(log, { id: 5, op: 'edit-text', page: 0, targetId: 't', generation: MAX_REPLAY_CLOSES + 2, revision: 0, text: 'A' }, { ok: true, revision: 1 });
  assert.equal(log.unavailable, 'generation');
});

test('an acknowledged save in place keeps its bytes as the fallback; a save a copy changes nothing', () => {
  const log = new RecoveryLog(10_000);
  tracked(log, { id: 1, op: 'open', pdf: new ArrayBuffer(1000), name: 'a.pdf' }, { ok: true });
  assert.equal(log.fallback(), log.base, 'never saved: the opened bytes are the saved state');
  // Save a copy: save and validate are reads, and nothing marks the document saved.
  tracked(log, { id: 2, op: 'save' }, { ok: true, bytes: new ArrayBuffer(1200) });
  tracked(log, { id: 3, op: 'validate', pdf: null }, { ok: true, bytes: new ArrayBuffer(1200) });
  assert.equal(log.entries.length, 0);
  assert.equal(log.fallback(), log.base);
  // Save in place: `saved` is a change (the clean baseline moves) and the written bytes are kept.
  assert.equal(log.fitsSaved(1200), true);
  tracked(log, { id: 4, op: 'saved' }, { ok: true, revision: 0, dirty: false });
  log.savedCopy(new ArrayBuffer(1200));
  assert.deepEqual(log.entries.map(entry => entry.op), ['saved']);
  assert.equal(log.fallback().byteLength, 1200);
  // The next save replaces the copy; one that does not fit leaves no fallback (not the stale one).
  assert.equal(log.fitsSaved(9000), false);
  log.savedCopy(null);
  assert.equal(log.fallback(), null);
  assert.equal(log.available, true, 'the replay itself stays possible');
});

// ------------------------------------------------------------------ replay through the engine

const sessions = [];
function engineSession(engine) {
  const session = engine.createSession();
  sessions.push([engine, session]);
  return session;
}

/** The worker seam: a structured clone of the request (postMessage), the engine's reply. */
function post(engine, session, message, id) {
  return engine.dispatch(structuredClone({ ...message, id }), session).reply;
}

/** A renderer stand-in: posts through the seam and keeps the recovery log as request() does. */
function client(engine, log) {
  const session = engineSession(engine);
  let id = 1;
  const send = message => {
    const kept = log?.keep(message) ?? null;
    const reply = post(engine, session, message, id++);
    if (kept && reply.ok !== false) log.record(kept, reply);
    return reply;
  };
  return { session, send };
}

function ok(reply, what) {
  assert.notEqual(reply.ok, false, `${what}: ${reply.error}`);
  return reply;
}

/** Everything the UI reads back from a document. */
function snapshot(send, pageCount) {
  const pages = [];
  for (let page = 0; page < pageCount; page++) {
    const annots = ok(send({ op: 'annots', page }), 'annots').items;
    const fields = ok(send({ op: 'fields', page }), 'fields').items;
    const targets = ok(send({ op: 'edit-targets', page }), 'edit-targets').items.map(item => [item.targetId, item.text ?? null]);
    pages.push({ annots, fields, targets });
  }
  const outline = ok(send({ op: 'outline' }), 'outline').items;
  const ids = ok(send({ op: 'page-ids' }), 'page-ids');
  return { pages, outline, ids: ids.ids };
}

const journal = reply => ({ revision: reply.revision, dirty: reply.dirty, canUndo: reply.canUndo, canRedo: reply.canRedo });

test('replay in a fresh engine reproduces ids, revisions, undo history and the saved bytes', async () => {
  const hello = toArrayBuffer(await readFile('fixtures/hello.pdf'));
  const replace = toArrayBuffer(await readFile('fixtures/p3/edit-replace.pdf'));
  const image = toArrayBuffer(await readFile('fixtures/img-gray.png'));
  const log = new RecoveryLog();
  const original = client(createEngine(mupdf), log);
  const { send } = original;
  // Earlier documents in the same worker: the edit handles below carry generation 3.
  ok(send({ op: 'open', pdf: hello.slice(0), name: 'hello.pdf' }), 'open hello');
  ok(send({ op: 'close' }), 'close');
  const opened = ok(send({ op: 'open', pdf: replace.slice(0), name: 'edit-replace.pdf' }), 'open');
  const pageCount = opened.pageCount;
  const note = ok(send({ op: 'annot-add', page: 0, type: 'Text', point: [72, 72], contents: 'first note', today: '2026-10-16' }), 'note');
  ok(send({ op: 'annot-add', page: 0, type: 'FreeText', point: [100, 300], contents: 'a text box', fit: true, today: '2026-10-16' }), 'freetext');
  ok(send({
    op: 'annot-add', page: 0, type: 'Stamp', rect: [300, 600, 420, 650], image: { name: 'img-gray.png', bytes: image.slice(0) },
    sign: { name: 'Ada Lovelace' }, contents: '', today: '2026-10-16',
  }), 'signature stamp');
  assert.equal(note.revision, 1);
  const noteId = ok(send({ op: 'annots', page: 0 }), 'annots').items.find(item => item.contents === 'first note').id;
  ok(send({ op: 'annot-set', page: 0, annotId: noteId, contents: 'first note, edited' }), 'annot-set');
  const scan = ok(send({ op: 'edit-targets', page: 0 }), 'scan');
  const preview = ok(send({ op: 'edit-replace-preview', page: 0, generation: scan.generation, revision: scan.revision, find: 'ALPHA', replace: 'BETA' }), 'preview');
  assert.equal(scan.generation, 3);
  ok(send({
    op: 'edit-replace-apply', page: 0, generation: preview.generation, revision: preview.revision, find: 'ALPHA', replace: 'BETA',
    previewToken: preview.previewToken, targetIds: preview.matches.map(match => match.targetId),
  }), 'replace');
  ok(send({ op: 'page-rotate', page: pageCount - 1, by: 90 }), 'rotate');
  ok(send({ op: 'outline-add', title: 'Recovered', page: 0 }), 'outline-add');
  ok(send({ op: 'undo' }), 'undo');
  ok(send({ op: 'redo' }), 'redo');
  ok(send({ op: 'saved' }), 'saved');
  ok(send({ op: 'annot-add', page: 0, type: 'Square', rect: [100, 100, 200, 160], contents: '' }), 'square after the save');
  ok(send({ op: 'undo' }), 'undo the square');
  const before = journal(ok(send({ op: 'annot-add', page: 0, type: 'Circle', rect: [100, 400, 200, 460], contents: '' }), 'circle'));
  assert.equal(log.available, true, `recovery ${log.unavailable}`);
  assert.equal(log.generation(), 3);
  assert.deepEqual(log.entries.map(entry => entry.op), [
    'annot-add', 'annot-add', 'annot-add', 'annot-set', 'edit-replace-apply', 'page-rotate', 'outline-add', 'undo', 'redo', 'saved',
    'annot-add', 'undo', 'annot-add',
  ]);
  // The request that "hung" is never answered, so the log never holds it.
  log.keep({ op: 'annot-add', page: 0, type: 'Text', point: [10, 10], contents: 'hung' });

  // The restarted worker: a fresh engine (its own generation counter), the same log.
  const restarted = client(createEngine(mupdf), null);
  const result = await replay(log, async (message, long) => {
    assert.equal(long, false);
    return ok(restarted.send(message), `replay ${message.op}`);
  });
  assert.equal(result.steps, 13);
  assert.deepEqual(journal(result.journal), before, 'the journal state the UI holds');
  assert.equal(result.pages.pageCount, pageCount);

  const expected = snapshot(original.send, pageCount);
  const actual = snapshot(restarted.send, pageCount);
  assert.deepEqual(actual, expected, 'annotation, field, target and page ids, contents and bookmarks match');
  assert.ok(expected.pages[0].annots.some(item => item.contents === 'first note, edited'));
  assert.ok(!expected.pages[0].annots.some(item => item.contents === 'hung'));

  // Undo history: both undo the circle, then the redo branch is the same.
  assert.equal(fingerprint(original.send({ op: 'undo' })), fingerprint(restarted.send({ op: 'undo' })));
  assert.equal(fingerprint(original.send({ op: 'undo' })), fingerprint(restarted.send({ op: 'undo' })));
  assert.equal(fingerprint(original.send({ op: 'redo' })), fingerprint(restarted.send({ op: 'redo' })));
  // An edit after the restart gets the id the original would have given.
  const nextA = original.send({ op: 'annot-add', page: 0, type: 'Text', point: [50, 50], contents: 'after', today: '2026-10-16' });
  const nextB = restarted.send({ op: 'annot-add', page: 0, type: 'Text', point: [50, 50], contents: 'after', today: '2026-10-16' });
  assert.equal(fingerprint(nextA), fingerprint(nextB));

  const savedA = ok(original.send({ op: 'save' }), 'save original');
  const savedB = ok(restarted.send({ op: 'save' }), 'save replayed');
  // Neither engine writes a clock value here (the signature's date is the recorded `today`).
  assert.deepEqual(new Uint8Array(savedB.bytes), new Uint8Array(savedA.bytes), 'the replayed document saves byte for byte');
});

test('replay stops at the first answer that differs, and at an open that differs', async () => {
  const hello = toArrayBuffer(await readFile('fixtures/hello.pdf'));
  const log = new RecoveryLog();
  const { send } = client(createEngine(mupdf), log);
  ok(send({ op: 'open', pdf: hello, name: 'hello.pdf' }), 'open');
  ok(send({ op: 'annot-add', page: 0, type: 'Text', point: [72, 72], contents: 'one' }), 'one');
  ok(send({ op: 'annot-add', page: 0, type: 'Text', point: [90, 90], contents: 'two' }), 'two');
  log.entries[1].reply = 'tampered';
  const restarted = client(createEngine(mupdf), null);
  const sent = [];
  await assert.rejects(replay(log, async message => { sent.push(message.op); return restarted.send(message); }), error => {
    assert.ok(error instanceof ReplayMismatch);
    assert.equal(error.index, 1);
    assert.equal(error.op, 'annot-add');
    return true;
  });
  assert.deepEqual(sent, ['open', 'annot-add', 'annot-add']);

  log.opened = 'tampered';
  const again = client(createEngine(mupdf), null);
  await assert.rejects(replay(log, async message => again.send(message)), /did not open as it did before/);
  // A refusal or a dead worker rejects the send; the replay passes it on.
  log.opened = fingerprint(ok(client(createEngine(mupdf), null).send({ op: 'open', pdf: hello, name: 'hello.pdf' }), 'reopen'));
  await assert.rejects(replay(log, async message => { if (message.op === 'annot-add') throw new Error('worker stopped responding'); return again.send(message); }), /worker stopped responding/);
});

test('an encrypted document replays its unlock and its pending security change', async () => {
  const bytes = toArrayBuffer(await readFile('fixtures/encrypted.pdf'));
  const log = new RecoveryLog();
  const original = client(createEngine(mupdf), log);
  ok(original.send({ op: 'open', pdf: bytes, name: 'encrypted.pdf' }), 'open');
  ok(original.send({ op: 'authenticate', password: 'wrong' }), 'wrong password');
  const unlocked = ok(original.send({ op: 'authenticate', password: 'pdf4all-owner' }), 'unlock');
  assert.equal(unlocked.authenticated, true, 'the fixture owner password');
  ok(original.send({ op: 'annot-add', page: 0, type: 'Text', point: [72, 72], contents: 'locked note' }), 'note');
  ok(original.send({ op: 'security-set', mode: 'none' }), 'security none');
  assert.deepEqual(log.entries.map(entry => entry.op), ['authenticate', 'annot-add', 'security-set']);
  const restarted = client(createEngine(mupdf), null);
  const result = await replay(log, async message => ok(restarted.send(message), message.op));
  assert.equal(result.journal.dirty, true);
  const plain = ok(restarted.send({ op: 'save' }), 'save');
  const reopened = client(createEngine(mupdf), null);
  const info = ok(reopened.send({ op: 'open', pdf: plain.bytes, name: 'plain.pdf' }), 'reopen').info;
  assert.equal(info.encrypted, false, 'the pending security change was replayed');
  assert.ok(ok(reopened.send({ op: 'annots', page: 0 }), 'annots').items.some(item => item.contents === 'locked note'));
});

test.after(() => {
  for (const [engine, session] of sessions) engine.dispatch({ id: 0, op: 'close' }, session);
});
