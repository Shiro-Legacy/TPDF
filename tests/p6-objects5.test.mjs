import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createEngine, latin1, MAX_EDIT_TARGETS } from './rust-engine.mjs';

// P6 U5 engine (contract-p6-unit5.md §1.1–1.3, §9.1–9.12): `edit-object {action:'duplicate'}`
// and the compound, all-or-nothing `edit-objects`. Harness lines copied from
// tests/p6-direct3.test.mjs:1-70.
const engine = createEngine();
const DIRECT3 = 'fixtures/p6/direct3.pdf';
const DIRECT3_SHA = 'e3ae5a43d9fb17e5b164c00b529d5be50ba4d9178de6c11b66252cc240297466';
const OBJECTS = 'fixtures/p3/edit-objects.pdf';
const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
const OBJECTS_SHA = 'b22a001ff960aea1304bde332870b02a34dbc4dc1a4e79908c09bbbfbd22e1a1';
const sha = async name => createHash('sha256').update(await readFile(name)).digest('hex');

async function open(name = DIRECT3) {
  const s = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: buffer(await readFile(name)) }, s).reply;
  assert.equal(reply.ok, true, reply.error);
  return s;
}
function openBytes(bytes) {
  const s = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'scratch.pdf', pdf: bytes }, s).reply.ok, true);
  return s;
}
const request = (s, req) => engine.dispatch({ id: 2, ...req }, s).reply;
const scan = (s, page) => request(s, { op: 'edit-targets', page });
const object = (sc, name) => sc.items.find(item => item.kind !== 'text' && item.objectName === name);
const objects = sc => sc.items.filter(item => item.kind !== 'text');
const boxes = (sc, name) => objects(sc).filter(item => item.objectName === name).map(item => item.placement.box);
function reopen(s) {
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  return openBytes(saved.bytes);
}
/** Dark pixels inside a page-pt box (y down) of the page rendered at 72 dpi (RGBA; gray < 200). */
function ink(s, page, box) {
  const reply = request(s, { op: 'render', page, scale: 1 });
  assert.equal(reply.ok, true, reply.error);
  const width = reply.pixelWidth, height = reply.pixelHeight, pixels = new Uint8Array(reply.pixels);
  let count = 0;
  for (let y = Math.max(0, Math.floor(box[1])); y < Math.min(height, Math.ceil(box[3])); y++) {
    for (let x = Math.max(0, Math.floor(box[0])); x < Math.min(width, Math.ceil(box[2])); x++) {
      const at = (y * width + x) * 4;
      if (0.3 * pixels[at] + 0.59 * pixels[at + 1] + 0.11 * pixels[at + 2] < 200) count++;
    }
  }
  return count;
}
function contents(s, page = 0) {
  return latin1(s.doc.contents(page));
}
function editObject(s, sc, target, action, box) {
  return request(s, { op: 'edit-object', page: sc.page, targetId: target.targetId, generation: sc.generation, revision: sc.revision, action, box });
}
function editObjects(s, sc, items) {
  return request(s, { op: 'edit-objects', page: sc.page, generation: sc.generation, revision: sc.revision, items });
}
/** The journal slice every mutation reply carries, read from the session (no read op exists). */
const journal = s => ({ ...engine.journalState(s), position: s.journal.position });
function refused(reply, detail, targetId) {
  assert.equal(reply.ok, false, JSON.stringify(reply));
  assert.equal(reply.detail, detail, reply.error);
  if (targetId !== undefined) assert.equal(reply.targetId, targetId);
}
/** Runs a refused request and proves nothing was written: journal and scan deep-equal. */
function nothingWritten(s, page, run) {
  const before = scan(s, page);
  const beforeJournal = journal(s);
  const beforeBytes = contents(s, page);
  const reply = run(before);
  assert.deepEqual(journal(s), beforeJournal, 'journalState deep-equal');
  assert.equal(contents(s, page), beforeBytes, 'contents() bytes identical');
  return reply;
}
/** `bytes` (a classic-xref PDF) with an incremental update appending `objects` ({number: body}). */
function update(bytes, objects) {
  let text = Buffer.from(bytes).toString('latin1');
  const trailer = text.slice(text.lastIndexOf('trailer'));
  const size = Number(/\/Size (\d+)/.exec(trailer)[1]), root = /\/Root (\d+ \d+ R)/.exec(trailer)[1];
  const prev = Number(/startxref\s+(\d+)/.exec(text.slice(text.lastIndexOf('startxref')))[1]);
  if (!text.endsWith('\n')) text += '\n';
  const numbers = Object.keys(objects).map(Number).sort((x, y) => x - y);
  const offsets = numbers.map(n => {
    const at = text.length;
    text += `${n} 0 obj\n${objects[n]}\nendobj\n`;
    return at;
  });
  const xref = text.length;
  text += 'xref\n';
  numbers.forEach((n, k) => { text += `${n} 1\n${String(offsets[k]).padStart(10, '0')} 00000 n \n`; });
  text += `trailer\n<< /Size ${Math.max(size, numbers.at(-1) + 1)} /Root ${root} /Prev ${prev} >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(text, 'latin1');
}
/** The next free object number of a classic-xref PDF. */
const nextObject = bytes => { const text = Buffer.from(bytes).toString('latin1'); return Number(/\/Size (\d+)/.exec(text.slice(text.lastIndexOf('trailer')))[1]); };
/** The body of object `n` as written in an uncompressed PDF. */
const objectText = (bytes, n) => new RegExp(`(?:^|\\n)${n} 0 obj\\n([^]*?)\\nendobj`).exec(Buffer.from(bytes).toString('latin1'))[1];
const streamBody = data => `<< /Length ${data.length} >>\nstream\n${data}\nendstream`;
/** A scratch copy of direct3 with page 0's stream rewritten by `edit` (text → text or text[]), written as an incremental update. */
async function scratch(edit) {
  const bytes = await readFile(DIRECT3);
  const doc = engine.reopen(bytes);
  let text, pageId, contentsId;
  try {
    text = latin1(doc.contents(0));
    pageId = doc.pageIds[0];
    contentsId = doc.page(0).ref('Contents');
  } finally { engine.closeDocument(doc); }
  const parts = [edit(text)].flat();
  if (parts.length === 1) return openBytes(buffer(update(bytes, { [contentsId[0]]: streamBody(parts[0]) })));
  // Probe P3 recipe: /Contents [a b] from two fresh streams.
  const first = nextObject(bytes);
  const objects = { [pageId[0]]: objectText(bytes, pageId[0]).replace(`/Contents ${contentsId[0]} 0 R`, `/Contents [${parts.map((_, k) => `${first + k} 0 R`).join(' ')}]`) };
  parts.forEach((part, k) => { objects[first + k] = streamBody(part); });
  return openBytes(buffer(update(bytes, objects)));
}

test('U5 pins: direct3 and edit-objects.pdf are unchanged before the run', async () => {
  assert.equal(await sha(DIRECT3), DIRECT3_SHA);
  assert.equal(await sha(OBJECTS), OBJECTS_SHA);
});

test('U5 §1.3/§9.6: duplicate is listed exactly where move is', async () => {
  const s = await open();
  const p0 = scan(s, 0);
  assert.deepEqual(object(p0, 'Im1').objectOps, ['move', 'resize', 'delete', 'replace', 'duplicate', 'transform', 'crop']);
  assert.deepEqual(object(p0, 'Im2').objectOps, ['move', 'resize', 'delete', 'replace', 'duplicate', 'transform', 'crop']);
  assert.deepEqual(object(p0, 'Fm1').objectOps, ['move', 'resize', 'delete', 'duplicate', 'transform', 'crop']);
  const p2 = scan(s, 2);
  assert.deepEqual(object(p2, 'Bg').objectOps, ['move', 'resize', 'delete', 'replace', 'duplicate', 'transform', 'crop']);
  // §9.6: Bg is a candidate, so a duplicate on it lands.
  assert.equal(editObject(s, p2, object(p2, 'Bg'), 'duplicate', [10, 10, 622 - 20, 802 - 20]).ok, true);
});

test('U5 §1.1: duplicate Im2 inserts the copy after its Q, survives save, undoes', async () => {
  const s = await open();
  const strip = [462, 43, 469, 81]; // §9.7: copy-only, y down
  assert.equal(ink(s, 0, strip), 0);
  const p0 = scan(s, 0);
  const reply = editObject(s, p0, object(p0, 'Im2'), 'duplicate', [410, 710, 470, 750]);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.revision, p0.revision + 1);
  assert.equal(reply.dirty, true);
  assert.equal(reply.targetId, undefined, 'no targetId: the renderer re-resolves');
  const after = scan(s, 0);
  assert.equal(objects(after).length, 4);
  assert.deepEqual(boxes(after, 'Im2'), [[400, 700, 460, 740], [410, 710, 470, 750]]);
  const ids = objects(after).filter(item => item.objectName === 'Im2').map(item => item.targetId);
  assert.notEqual(ids[0], ids[1]);
  assert.deepEqual(objects(after).filter(item => item.objectName === 'Im2').map(item => item.objectOps.includes('duplicate')), [true, true]);
  const text = contents(s);
  assert.equal(text.split('/Im2 Do').length - 1, 2);
  assert.match(text, /q 60 0 0 40 400 700 cm \/Im2 Do Q\nq 60 0 0 40 410 710 cm \/Im2 Do Q/);
  const fresh = reopen(s);
  assert.deepEqual(boxes(scan(fresh, 0), 'Im2'), [[400, 700, 460, 740], [410, 710, 470, 750]]);
  assert.ok(ink(fresh, 0, strip) > 0, 'the copy paints after reopen');
  const undo = request(s, { op: 'undo' });
  assert.equal(undo.ok, true);
  assert.equal(undo.dirty, false);
  assert.equal(objects(scan(s, 0)).length, 3);
});

test('U5 §1.1/§9.6: single duplicate refusals write nothing', async () => {
  const s = await open();
  for (const box of [[560, 700, 620, 740], [10, 10, 10.5, 20], undefined]) {
    const reply = nothingWritten(s, 0, sc => editObject(s, sc, object(sc, 'Im2'), 'duplicate', box));
    refused(reply, 'out-of-bounds');
    assert.equal(reply.dirty ?? false, false);
  }
  const o = await open(OBJECTS);
  const clipped = nothingWritten(o, 0, sc => editObject(o, sc, objects(sc)[1], 'duplicate', [72, 650, 136, 746]));
  refused(clipped, 'unsupported-transform');
  const rotated = nothingWritten(o, 0, sc => editObject(o, sc, objects(sc)[5], 'replace', [72, 650, 136, 746]));
  refused(rotated, 'not-candidate');
});

test('U5 §1.2: compound move, delete and duplicate each land as one undoable op', async () => {
  const s = await open();
  let p0 = scan(s, 0);
  const fm1 = object(p0, 'Fm1').placement.box;
  let reply = editObjects(s, p0, [
    { targetId: object(p0, 'Im1').targetId, action: 'move', box: [80, 600, 180, 650] },
    { targetId: object(p0, 'Im2').targetId, action: 'move', box: [300, 700, 360, 740] },
  ]);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.count, 2);
  assert.equal(reply.page, 0);
  assert.equal(reply.revision, p0.revision + 1, 'one revision');
  let after = scan(s, 0);
  assert.deepEqual(boxes(after, 'Im1'), [[80, 600, 180, 650]]);
  assert.deepEqual(boxes(after, 'Im2'), [[300, 700, 360, 740]]);
  assert.deepEqual(boxes(after, 'Fm1'), [fm1]);
  let undo = request(s, { op: 'undo' });
  assert.equal(undo.dirty, false);
  after = scan(s, 0);
  assert.deepEqual(boxes(after, 'Im1'), [[72, 600, 172, 650]], 'one undo restores both');
  assert.deepEqual(boxes(after, 'Im2'), [[400, 700, 460, 740]]);

  p0 = scan(s, 0);
  reply = editObjects(s, p0, [
    { targetId: object(p0, 'Im1').targetId, action: 'delete' },
    { targetId: object(p0, 'Fm1').targetId, action: 'delete' },
  ]);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.count, 2);
  assert.deepEqual(objects(scan(s, 0)).map(item => item.objectName), ['Im2']);
  request(s, { op: 'undo' });
  assert.equal(objects(scan(s, 0)).length, 3);

  p0 = scan(s, 0);
  reply = editObjects(s, p0, [
    { targetId: object(p0, 'Im1').targetId, action: 'duplicate', box: [82, 610, 182, 660] },
    { targetId: object(p0, 'Im2').targetId, action: 'duplicate', box: [410, 710, 470, 750] },
  ]);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.count, 2);
  after = scan(s, 0);
  assert.equal(objects(after).length, 5);
  assert.deepEqual(boxes(after, 'Im1'), [[72, 600, 172, 650], [82, 610, 182, 660]]);
  assert.deepEqual(boxes(after, 'Im2'), [[400, 700, 460, 740], [410, 710, 470, 750]]);
  undo = request(s, { op: 'undo' });
  assert.equal(undo.dirty, false);
  assert.equal(objects(scan(s, 0)).length, 3);
});

test('U5 §1.2/§9.2–§9.5: every refusal comes before any write', async () => {
  const s = await open();
  const id = (sc, name) => object(sc, name).targetId;
  let reply = nothingWritten(s, 0, sc => editObjects(s, sc, [
    { targetId: id(sc, 'Im1'), action: 'move', box: [80, 600, 180, 650] },
    { targetId: id(sc, 'Fm1'), action: 'replace' },
  ]));
  refused(reply, 'invalid-selection');
  for (const action of ['crop', 'teleport']) {
    reply = nothingWritten(s, 0, sc => editObjects(s, sc, [
      { targetId: id(sc, 'Im1'), action: 'delete' }, { targetId: id(sc, 'Im2'), action },
    ]));
    refused(reply, 'invalid-selection');
  }
  reply = nothingWritten(s, 0, sc => editObjects(s, sc, [{ targetId: id(sc, 'Im1'), action: 'delete' }, { targetId: '', action: 'delete' }]));
  refused(reply, 'invalid-selection');
  reply = nothingWritten(s, 0, sc => editObjects(s, sc, 'Im1'));
  refused(reply, 'invalid-selection');
  let im2;
  reply = nothingWritten(s, 0, sc => {
    im2 = id(sc, 'Im2');
    return editObjects(s, sc, [
      { targetId: id(sc, 'Im1'), action: 'move', box: [80, 600, 180, 650] },
      { targetId: im2, action: 'move', box: [600, 700, 660, 740] },
    ]);
  });
  refused(reply, 'out-of-bounds', im2);
  let im1;
  reply = nothingWritten(s, 0, sc => {
    im1 = id(sc, 'Im1');
    return editObjects(s, sc, [{ targetId: im1, action: 'delete' }, { targetId: im1, action: 'move', box: [80, 600, 180, 650] }]);
  });
  refused(reply, 'invalid-selection', im1);
  reply = nothingWritten(s, 0, sc => editObjects(s, sc, [{ targetId: id(sc, 'Im1'), action: 'delete' }]));
  refused(reply, 'invalid-selection');
  // §9.3/§9.10: the cap first, before any resolve; distinct fake ids.
  reply = nothingWritten(s, 0, sc => editObjects(s, sc, Array.from({ length: MAX_EDIT_TARGETS + 1 }, (_, n) => n === MAX_EDIT_TARGETS ? {} : ({ targetId: `fake-${n}`, action: 'delete' }))));
  refused(reply, 'too-large');
  reply = nothingWritten(s, 0, sc => request(s, {
    op: 'edit-objects', page: 0, generation: sc.generation, revision: sc.revision + 1,
    items: [{ targetId: id(sc, 'Im1'), action: 'delete' }, { targetId: id(sc, 'Im2'), action: 'delete' }],
  }));
  refused(reply, 'stale-target');
  reply = nothingWritten(s, 0, sc => editObjects(s, sc, [
    { targetId: id(sc, 'Im1'), action: 'move', box: [72, 600, 172, 650] },
    { targetId: id(sc, 'Im2'), action: 'move', box: [400, 700, 460, 740] },
  ]));
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.noop, true);
  assert.equal(reply.dirty, false);
  const p0 = scan(s, 0);
  reply = editObjects(s, p0, [
    { targetId: id(p0, 'Im1'), action: 'move', box: [72, 600, 172, 650] },
    { targetId: id(p0, 'Im2'), action: 'move', box: [300, 700, 360, 740] },
  ]);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.count, 1, 'a same-cm move is dropped from the count');
  assert.equal(reply.noop, undefined);

  // §9.5: the placement's own reason, with the offending item's id.
  const o = await open(OBJECTS);
  let second;
  reply = nothingWritten(o, 0, sc => {
    second = objects(sc)[1].targetId;
    return editObjects(o, sc, [
      { targetId: objects(sc)[0].targetId, action: 'move', box: [80, 650, 144, 746] },
      { targetId: second, action: 'move', box: [80, 650, 144, 746] },
    ]);
  });
  refused(reply, 'unsupported-transform', second);
});

test('U5 §9.8: cow placements on edit-objects.pdf p1 privatise once and undo together', async () => {
  const s = await open(OBJECTS);
  const page0 = scan(s, 0).items;
  const page0Bytes = contents(s, 0);
  const p1 = scan(s, 1);
  const reply = editObjects(s, p1, [
    { targetId: object(p1, 'Im1').targetId, action: 'duplicate', box: [200, 650, 264, 746] },
    { targetId: object(p1, 'Fm1').targetId, action: 'move', box: [72, 400, 192, 420] },
  ]);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.count, 2);
  assert.deepEqual(scan(s, 0).items, page0, 'page 0 keeps its own resources and stream');
  assert.equal(contents(s, 0), page0Bytes, 'page 0 contents() bytes identical');
  const after = scan(s, 1);
  assert.deepEqual(boxes(after, 'Im1'), [[72, 650, 136, 746], [200, 650, 264, 746]]);
  assert.deepEqual(boxes(after, 'Fm1'), [[72, 400, 192, 420]]);
  assert.equal(objects(scan(reopen(s), 1)).length, 3);
  assert.equal(request(s, { op: 'undo' }).dirty, false);
  const undone = scan(s, 1);
  assert.deepEqual(objects(undone).map(item => `${item.objectName}:${item.cowable}:${JSON.stringify(item.placement.box)}`),
    ['Im1:true:[72,650,136,746]', 'Fm1:true:[72,500,192,520]']);
});

test('U5 §1.3: a two-element /Contents groups edits per element in one op', async () => {
  const split = text => { const cut = text.indexOf('q 60 0 0 40 400 700 cm'); return [text.slice(0, cut), text.slice(cut)]; };
  const s = await scratch(split);
  let p0 = scan(s, 0);
  assert.deepEqual(objects(p0).map(item => `${item.objectName}:${item.targetId.split('.')[1]}`), ['Im1:c0', 'Im2:c1', 'Fm1:c1']);
  let reply = editObjects(s, p0, [
    { targetId: object(p0, 'Im1').targetId, action: 'move', box: [80, 600, 180, 650] },
    { targetId: object(p0, 'Im2').targetId, action: 'move', box: [300, 700, 360, 740] },
  ]);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.revision, p0.revision + 1);
  const fresh = scan(reopen(s), 0);
  assert.deepEqual(boxes(fresh, 'Im1'), [[80, 600, 180, 650]]);
  assert.deepEqual(boxes(fresh, 'Im2'), [[300, 700, 360, 740]]);
  request(s, { op: 'undo' });
  p0 = scan(s, 0);
  assert.deepEqual(boxes(p0, 'Im1'), [[72, 600, 172, 650]]);
  assert.deepEqual(boxes(p0, 'Im2'), [[400, 700, 460, 740]]);
  const fm1 = object(p0, 'Fm1').placement.box;
  const beforeBytes = contents(s, 0);
  reply = editObjects(s, p0, [
    { targetId: object(p0, 'Im2').targetId, action: 'delete' },
    { targetId: object(p0, 'Fm1').targetId, action: 'duplicate', box: [fm1[0] + 10, fm1[1] + 10, fm1[2] + 10, fm1[3] + 10] },
  ]);
  assert.equal(reply.ok, true, reply.error);
  const after = scan(s, 0);
  assert.deepEqual(boxes(after, 'Im2'), []);
  assert.deepEqual(boxes(after, 'Fm1'), [fm1, [fm1[0] + 10, fm1[1] + 10, fm1[2] + 10, fm1[3] + 10]]);
  // U6 §1.5: undo is bytes-equal; redo → save → reopen → the scan holds.
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.equal(contents(s, 0), beforeBytes, 'undo restores both elements byte for byte');
  assert.equal(request(s, { op: 'redo' }).ok, true);
  const redone = scan(reopen(s), 0);
  assert.deepEqual(boxes(redone, 'Im2'), []);
  assert.deepEqual(boxes(redone, 'Fm1'), [fm1, [fm1[0] + 10, fm1[1] + 10, fm1[2] + 10, fm1[3] + 10]]);
});

test('U5 §9.4: a duplicate that would push the page over MAX_EDIT_TARGETS refuses first', async () => {
  const base = scan(await open(), 0).items.length;
  const pad = MAX_EDIT_TARGETS - 1 - base;
  const s = await scratch(text => text + 'q 10 0 0 10 20 20 cm /Im1 Do Q\n'.repeat(pad));
  let p0 = scan(s, 0);
  assert.equal(p0.items.length, MAX_EDIT_TARGETS - 1);
  const two = nothingWritten(s, 0, sc => editObjects(s, sc, [
    { targetId: object(sc, 'Im1').targetId, action: 'duplicate', box: [82, 610, 182, 660] },
    { targetId: object(sc, 'Im2').targetId, action: 'duplicate', box: [410, 710, 470, 750] },
  ]));
  refused(two, 'too-large');
  const one = editObject(s, p0, object(p0, 'Im2'), 'duplicate', [410, 710, 470, 750]);
  assert.equal(one.ok, true, one.error);
  p0 = scan(s, 0);
  assert.equal(p0.items.length, MAX_EDIT_TARGETS, 'the next scan still proves the page');
  refused(nothingWritten(s, 0, sc => editObject(s, sc, object(sc, 'Im2'), 'duplicate', [420, 710, 480, 750])), 'too-large');
});

test('U5 §9.10: permission and empty pages', async () => {
  const locked = await open('fixtures/encrypted-noedit.pdf');
  const reply = request(locked, { op: 'edit-objects', page: 0, generation: 0, revision: 0, items: [{ targetId: 'a', action: 'delete' }, { targetId: 'b', action: 'delete' }] });
  assert.equal(reply.ok, false);
  assert.equal(reply.reason, 'read-only');
  assert.equal(reply.detail, undefined);
  const hello = await open('fixtures/hello.pdf');
  const sc = scan(hello, 0);
  refused(editObjects(hello, sc, [{ targetId: 'a', action: 'delete' }, { targetId: 'b', action: 'delete' }]), 'stale-target', 'a');
});

test('U5 review E-1: an indirect /Contents array shared by two pages never carries an edit to the other page', async () => {
  // direct3 with p0's stream split into ONE indirect array [a b], placed on p0 and p1.
  const original = await readFile(DIRECT3);
  const doc = engine.reopen(original);
  let text, ids, contentsId;
  try {
    text = latin1(doc.contents(0));
    ids = [doc.pageIds[0][0], doc.pageIds[1][0]];
    contentsId = [doc.page(0).ref('Contents')[0], doc.page(1).ref('Contents')[0]];
  } finally { engine.closeDocument(doc); }
  const cut = text.indexOf('q 60 0 0 40 400 700 cm');
  assert.ok(cut > 0);
  // One indirect array object of two fresh streams, written as an incremental update.
  const first = nextObject(original);
  const p0Body = objectText(original, ids[0]);
  const resources = /\/Resources (<<[^]*>>) \/Contents/.exec(p0Body)[1];
  const bytes = buffer(update(original, {
    [ids[0]]: p0Body.replace(`/Contents ${contentsId[0]} 0 R`, `/Contents ${first} 0 R`),
    [ids[1]]: objectText(original, ids[1]).replace(`/Contents ${contentsId[1]} 0 R`, `/Contents ${first} 0 R`).replace(/\/Resources <<[^]*>> \/Contents/, `/Resources ${resources} /Contents`),
    [first]: `[${first + 1} 0 R ${first + 2} 0 R]`,
    [first + 1]: streamBody(text.slice(0, cut)),
    [first + 2]: streamBody(text.slice(cut)),
  }));
  const pageBytes = (s, page) => s.doc.page(page).get('Contents').map(part => latin1(part.stream())).join('|');
  const s = openBytes(bytes);
  const p0Before = pageBytes(s, 0), p1Before = pageBytes(s, 1);
  const p1Count = objects(scan(s, 1)).length;
  assert.equal(p1Count, 3);
  for (const run of [
    sc => request(s, { op: 'edit-objects', page: 0, generation: sc.generation, revision: sc.revision, items: [
      { targetId: objects(sc)[0].targetId, action: 'duplicate', box: [82, 610, 182, 660] },
      { targetId: objects(sc)[1].targetId, action: 'move', box: [300, 700, 360, 740] }] }),
    sc => request(s, { op: 'edit-object', page: 0, generation: sc.generation, revision: sc.revision, targetId: objects(sc)[0].targetId, action: 'duplicate', box: [82, 610, 182, 660] }),
    sc => request(s, { op: 'edit-object', page: 0, generation: sc.generation, revision: sc.revision, targetId: objects(sc)[0].targetId, action: 'move', box: [82, 610, 182, 660] }),
    // the edit-text path privatises through the same editPrivatise, so it must not leak either
    sc => request(s, { op: 'edit-text', page: 0, generation: sc.generation, revision: sc.revision, targetId: sc.items.find(item => item.kind === 'text' && item.support === 'candidate').targetId, text: 'E1' }),
  ]) {
    const reply = run(scan(s, 0));
    assert.equal(reply.ok, true, reply.error);
    assert.notEqual(pageBytes(s, 0), p0Before);
    assert.equal(pageBytes(s, 1), p1Before);
    assert.equal(objects(scan(reopen(s), 1)).length, p1Count);
    assert.equal(request(s, { op: 'undo' }).ok, true);
    assert.equal(pageBytes(s, 0), p0Before);
    assert.equal(pageBytes(s, 1), p1Before);
  }
});

test('U5 pins: direct3 and edit-objects.pdf are unchanged after the run', async () => {
  assert.equal(await sha(DIRECT3), DIRECT3_SHA);
  assert.equal(await sha(OBJECTS), OBJECTS_SHA);
});
