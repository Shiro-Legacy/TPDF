import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';

// P6 U8 engine (contract-p6-unit8.md §1, §9.2–§9.6): a direct-content run whose
// structured-text match at its own origin is unique is a candidate even when the same
// text is another whole line on the page; overprints and repeated Forms are unchanged.
// Harness copied from tests/p6-objects6.test.mjs:1-37.
const engine = createEngine(mupdf);
const DIRECT3 = 'fixtures/p6/direct3.pdf';
const DIRECT3_SHA = 'e3ae5a43d9fb17e5b164c00b529d5be50ba4d9178de6c11b66252cc240297466';
const SHARED = 'fixtures/p0b/edit-shared.pdf';
const ORIGINAL = 'p0.c0.314.6c51e7e2';
const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
const sha = async name => createHash('sha256').update(await readFile(name)).digest('hex');

async function open(name = DIRECT3) {
  const s = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: buffer(await readFile(name)) }, s).reply;
  assert.equal(reply.ok, true, reply.error);
  return s;
}
const request = (s, req) => engine.dispatch({ id: 2, ...req }, s).reply;
const scan = (s, page = 0) => request(s, { op: 'edit-targets', page });
const texts = (sc, value) => sc.items.filter(item => item.kind === 'text' && item.text === value);
const add = (s, point, text) => request(s, { op: 'text-add', page: 0, point, text, font: 'Helvetica', size: 12, color: [0, 0, 0] });
const near = (actual, expected, eps = 0.5) =>
  assert.ok(Array.isArray(actual) && actual.length === expected.length && actual.every((value, k) => Math.abs(value - expected[k]) <= eps), `${JSON.stringify(actual)} ≈ ${JSON.stringify(expected)}`);
const TWIN_A = [300, 387.1, 325.33, 403.59];
const TWIN_B = [300, 287.1, 325.33, 303.59];
/** Each /Contents element of page 0, decoded. */
function elements(s) {
  const view = s.doc.loadPage(0);
  try {
    const value = view.getObject().get('Contents');
    const list = value.isArray() ? [...Array(value.length)].map((_, index) => value.get(index)) : [value];
    return list.map(element => {
      const data = element.readStream();
      try { return Buffer.from(data.asUint8Array()).toString('latin1'); } finally { data.destroy(); }
    });
  } finally { view.destroy(); }
}
/** direct3 p0 with "Twin" added at (300,400) and (300,300); returns [A, B] from the scan. */
async function twins() {
  const s = await open();
  assert.equal(add(s, [300, 400], 'Twin').ok, true);
  const second = add(s, [300, 300], 'Twin');
  assert.equal(second.ok, true, second.error);
  const sc = scan(s);
  const [a, b] = texts(sc, 'Twin');
  return { s, sc, a, b, second };
}

test('(a) two direct "Twin" runs are both candidates; editing one leaves the other; undo restores', async () => {
  const { s, sc, a, b, second } = await twins();
  assert.equal(texts(sc, 'Twin').length, 2);
  assert.notEqual(second.bounds, null, 'the second text-add reply carries bounds');
  near(second.bounds, b.bounds);
  assert.deepEqual([a.support, b.support], ['candidate', 'candidate'], JSON.stringify([a.reason, b.reason]));
  assert.notEqual(a.targetId, b.targetId);
  near(a.bounds, TWIN_A);
  near(b.bounds, TWIN_B);
  const beforeA = elements(s)[1];
  const beforeAll = elements(s);
  const edited = request(s, { op: 'edit-text', page: 0, targetId: b.targetId, generation: sc.generation, revision: sc.revision, text: 'Twin 2' });
  assert.equal(edited.ok, true, edited.detail ?? edited.error);
  assert.equal(edited.revision, sc.revision + 1);
  const after = scan(s);
  const [still] = texts(after, 'Twin');
  assert.equal(texts(after, 'Twin').length, 1);
  assert.equal(still.targetId, a.targetId, 'A keeps its identity');
  assert.deepEqual(still.bounds, a.bounds, 'A keeps its bounds');
  assert.equal(elements(s)[1], beforeA, "A's stream bytes are unchanged");
  assert.equal(texts(after, 'Twin 2').length, 1);
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const r = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'twins-saved.pdf', pdf: saved.bytes }, r).reply.ok, true);
  const reopened = scan(r);
  assert.equal(texts(reopened, 'Twin 2').length, 1, 'the edit survives save → reopen');
  assert.deepEqual(texts(reopened, 'Twin').map(item => [item.targetId, item.bounds]), [[a.targetId, a.bounds]], 'A survives save → reopen');
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.deepEqual(elements(s), beforeAll, 'undo restores the /Contents bytes');
  const undone = texts(scan(s), 'Twin');
  assert.deepEqual(undone.map(item => [item.targetId, item.support]), [[a.targetId, 'candidate'], [b.targetId, 'candidate']]);
  assert.equal(await sha(DIRECT3), DIRECT3_SHA);
});

test('(b) an added run equal to an existing whole line: both candidates, lastAdd names the new run', async () => {
  const s = await open();
  assert.equal(scan(s).items.find(item => item.targetId === ORIGINAL)?.text, 'Move me');
  const reply = add(s, [300, 200], 'Move me');
  assert.equal(reply.ok, true, reply.error);
  const both = texts(scan(s), 'Move me');
  assert.equal(both.length, 2);
  assert.deepEqual(both.map(item => item.support), ['candidate', 'candidate'], JSON.stringify(both.map(item => item.reason)));
  const copy = both.find(item => item.targetId !== ORIGINAL);
  assert.ok(both.some(item => item.targetId === ORIGINAL), 'the original keeps its id');
  assert.equal(reply.targetId, copy.targetId, 'text-add reply names the new run');
  near(reply.bounds, copy.bounds);
  near(copy.bounds, [300, 187.1, 349.34, 203.59]);
});

test('(c) two runs at one origin (overprint) stay ambiguous with no bounds', async () => {
  const s = await open();
  assert.equal(add(s, [300, 400], 'Same').ok, true);
  const second = add(s, [300, 400], 'Same');
  assert.equal(second.ok, true);
  assert.equal(second.bounds, null);
  const both = texts(scan(s), 'Same');
  assert.equal(both.length, 2);
  assert.deepEqual(both.map(item => `${item.support}:${item.reason}:${item.bounds ?? null}`), ['unsupported:ambiguous-text-association:null', 'unsupported:ambiguous-text-association:null']);
});

test('(d) guard: a Form drawn twice keeps candidate, cowable, bounds null (depth > 0 keeps the P3 match)', async () => {
  const inside = texts(scan(await open(SHARED)), 'ALPHA');
  assert.equal(inside.length, 2);
  assert.deepEqual(inside.map(item => `${item.support}:${item.cowable}:${item.reason}`), ['candidate:true:undefined', 'candidate:true:undefined']);
  assert.deepEqual(inside.map(item => item.bounds ?? null), [null, null]);
});

test('(e) a scoped find-and-replace of one of two identical direct runs rewrites only that run', async () => {
  const { s, sc, a, b } = await twins();
  const preview = request(s, { op: 'edit-replace-preview', page: 0, generation: sc.generation, revision: sc.revision, find: 'Twin', replace: 'Pair', targetId: b.targetId });
  assert.equal(preview.ok, true, preview.error);
  assert.deepEqual(preview.matches.map(match => match.targetId), [b.targetId], JSON.stringify(preview.rejected));
  const applied = request(s, {
    op: 'edit-replace-apply', page: 0, generation: preview.generation, revision: preview.revision,
    find: 'Twin', replace: 'Pair', previewToken: preview.previewToken, targetIds: [b.targetId], targetId: b.targetId,
  });
  assert.equal(applied.ok, true, applied.detail ?? applied.error);
  const after = scan(s);
  const [still] = texts(after, 'Twin');
  assert.equal(texts(after, 'Twin').length, 1);
  assert.equal(still.targetId, a.targetId);
  assert.deepEqual(still.bounds, a.bounds);
  assert.equal(texts(after, 'Pair').length, 1);
});
