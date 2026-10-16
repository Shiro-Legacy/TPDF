/**
 * page-ids: the page objects' numbers in display order (-1 for a page
 * written directly into /Kids). Read-only, with no failure paths: an empty
 * list without a document, the structure's pages while locked. A page keeps
 * its number through move, rotate, delete, undo and redo. Runs against
 * src/renderer/engine.ts as the worker does; tests/run-rust.mjs runs it
 * against the Rust engine.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';

const engine = createEngine(mupdf);
let nextId = 1;

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

function op(session, request) {
  return engine.dispatch({ id: nextId++, ...request }, session).reply;
}

function ids(session) {
  const reply = op(session, { op: 'page-ids' });
  assert.equal(reply.ok, true, reply.error);
  return reply.ids;
}

async function open(session, name) {
  const pdf = toArrayBuffer(await readFile(`fixtures/${name}`));
  const reply = op(session, { op: 'open', name, pdf });
  assert.equal(reply.ok, true, reply.error);
}

test('page-ids follows each page object through move, rotate, delete, undo and redo', async () => {
  const session = engine.createSession();
  assert.deepEqual(op(session, { op: 'page-ids' }), { id: nextId - 1, ok: true, ids: [] });
  await open(session, 'nav.pdf');
  assert.deepEqual(ids(session), [3, 4, 5]);
  assert.equal(op(session, { op: 'page-move', page: 0, to: 1 }).ok, true);
  assert.deepEqual(ids(session), [4, 3, 5]);
  assert.equal(op(session, { op: 'page-rotate', page: 2, by: 90 }).ok, true);
  assert.deepEqual(ids(session), [4, 3, 5]);
  assert.equal(op(session, { op: 'page-delete', page: 0 }).ok, true);
  assert.deepEqual(ids(session), [3, 5]);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual(ids(session), [4, 3, 5]);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual(ids(session), [3, 4, 5]);
  assert.equal(op(session, { op: 'redo' }).ok, true);
  assert.deepEqual(ids(session), [4, 3, 5]);
  assert.equal(op(session, { op: 'close' }).ok, true);
  assert.deepEqual(ids(session), []);
});

test('page-ids answers while locked and for a page written directly into /Kids', async () => {
  const session = engine.createSession();
  await open(session, 'encrypted.pdf');
  assert.deepEqual(ids(session), [4]);
  assert.equal(op(session, { op: 'authenticate', password: 'pdf4all-user' }).authenticated, true);
  assert.deepEqual(ids(session), [4]);

  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>',
  ];
  const parts = ['%PDF-1.4\n'];
  const offsets = [];
  for (const [index, body] of objects.entries()) {
    offsets.push(parts.join('').length);
    parts.push(`${index + 1} 0 obj\n${body}\nendobj\n`);
  }
  const xref = parts.join('').length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const offset of offsets) parts.push(`${String(offset).padStart(10, '0')} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  const direct = engine.createSession();
  const reply = op(direct, { op: 'open', name: 'direct.pdf', pdf: toArrayBuffer(Buffer.from(parts.join(''), 'latin1')) });
  assert.equal(reply.ok, true, reply.error);
  assert.deepEqual(ids(direct), [3, -1]);
});
