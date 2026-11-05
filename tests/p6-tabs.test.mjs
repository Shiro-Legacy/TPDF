/**
 * P6 U4 §2.1 (+ §9.11): the `fields` reply carries the page's `/Tabs` and, on an `S`
 * page, each widget's position in a /K-only depth-first walk of the struct tree.
 * Read-only: nothing here writes.
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createEngine } from './rust-engine.mjs';

const engine = createEngine();
const TABS_SHA = 'b90c5db2ed9d4b1221b453bc9df02682fd8a6c678724263738a68d24bc047326';

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

async function open(path) {
  const session = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: 'open', name: path, pdf: toArrayBuffer(await readFile(path)) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function fields(session, page) {
  const reply = engine.dispatch({ id: 2, op: 'fields', page }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return reply;
}

test('tabs.pdf is pinned', async () => {
  assert.equal(createHash('sha256').update(await readFile('fixtures/p6/tabs.pdf')).digest('hex'), TABS_SHA);
});

test('fields reports /Tabs per page and structIndex only on a reachable S page', async () => {
  const session = await open('fixtures/p6/tabs.pdf');
  try {
    const p0 = fields(session, 0);
    assert.equal(p0.tabs, 'C');
    assert.deepEqual(p0.items.map(item => item.name), ['b3', 'a1', 'b1', 'a2', 'b2', 'a3'], '/Annots order');
    assert.ok(p0.items.every(item => !('structIndex' in item)));

    const p1 = fields(session, 1);
    assert.equal(p1.tabs, 'A');
    assert.deepEqual(p1.items.map(item => item.name), ['t3', 't1', 't2']);

    const p2 = fields(session, 2);
    assert.equal(p2.tabs, 'S');
    const index = Object.fromEntries(p2.items.map(item => [item.name, item.structIndex]));
    assert.deepEqual(index, { s1: 2, s2: 0, s3: 1 }, 'struct order s2 < s3 < s1');

    const p3 = fields(session, 3);
    assert.equal(p3.tabs, 'S');
    assert.deepEqual(p3.items.map(item => item.name), ['u2', 'u1']);
    assert.ok(p3.items.every(item => !('structIndex' in item)), 'u1/u2 are not in the struct tree');

    // Every existing key keeps its shape (the new keys aside).
    const a1 = p0.items.find(item => item.name === 'a1');
    assert.deepEqual(a1, {
      id: 13, name: 'a1', label: 'a1', kind: 'text', value: '', readOnly: false, required: false,
      scripted: false, rect: [72, 72, 192, 92], default: '',
    });
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('a page without /Tabs reports null (form.pdf)', async () => {
  const session = await open('fixtures/form.pdf');
  try {
    const reply = fields(session, 0);
    assert.equal(reply.tabs, null);
    assert.ok(reply.items.length > 0);
    assert.ok(reply.items.every(item => !('structIndex' in item)));
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('debt1 row 4: /K arrays are not depth levels (40 dicts, 80 levels with arrays, still ordered)', async () => {
  const original = await readFile('fixtures/p6/tabs.pdf');
  const doc = engine.reopen(original);
  let s2, treeRoot;
  try {
    s2 = doc.widget(2, 's2').objectNumber;
    treeRoot = doc.root.ref('StructTreeRoot')[0];
  } finally {
    engine.closeDocument(doc);
  }
  let node = `<< /Type /OBJR /Obj ${s2} 0 R >>`;
  for (let level = 0; level < 40; level++) node = `<< /K [${node}] >>`;
  // The struct tree root rewritten with the nested /K, as an incremental update of the fixture.
  const text = original.toString('latin1');
  const body = new RegExp(`\\n${treeRoot} 0 obj\\n([^]*?)\\nendobj`).exec(text)[1];
  const prev = Number(/startxref\s+(\d+)/.exec(text.slice(text.lastIndexOf('startxref')))[1]);
  const trailer = text.slice(text.lastIndexOf('trailer'));
  let out = text.endsWith('\n') ? text : `${text}\n`;
  const at = out.length;
  out += `${treeRoot} 0 obj\n${body.replace(/\/K \[[^\]]*\]/, `/K ${node}`)}\nendobj\n`;
  const xref = out.length;
  out += `xref\n${treeRoot} 1\n${String(at).padStart(10, '0')} 00000 n \n`;
  out += `trailer\n<< /Size ${/\/Size (\d+)/.exec(trailer)[1]} /Root ${/\/Root (\d+ \d+ R)/.exec(trailer)[1]} /Prev ${prev} >>\nstartxref\n${xref}\n%%EOF\n`;
  const bytes = toArrayBuffer(Buffer.from(out, 'latin1'));
  const session = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: 'open', name: 'deep.pdf', pdf: bytes }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  try {
    const p2 = fields(session, 2);
    assert.equal(p2.tabs, 'S');
    assert.deepEqual(Object.fromEntries(p2.items.filter(item => 'structIndex' in item).map(item => [item.name, item.structIndex])), { s2: 0 });
  } finally {
    engine.closeDocument(session.doc);
  }
});
