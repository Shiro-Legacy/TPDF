import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine, MAX_EDIT_TARGETS, MAX_REDACT_MARKS } from './rust-engine.mjs';

// P6 U7 engine (contract-p6-unit7.md §1.1–§1.4, §9.1–§9.18, §10.1–§10.28): the worker
// clipboard (`clip-copy`), `clip-paste` as one journal op, and `edit-batch` over mixed kinds.
// Harness copied from tests/p6-objects6.test.mjs:1-112.
const engine = createEngine(mupdf);
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
/** Dark pixels inside a page-pt box (y down) of the page rendered at 72 dpi. */
function ink(s, page, box) {
  const view = s.doc.loadPage(page);
  const pixmap = view.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceGray, false);
  try {
    const width = pixmap.getWidth(), pixels = pixmap.getPixels();
    let count = 0;
    for (let y = Math.max(0, Math.floor(box[1])); y < Math.min(pixmap.getHeight(), Math.ceil(box[3])); y++) {
      for (let x = Math.max(0, Math.floor(box[0])); x < Math.min(width, Math.ceil(box[2])); x++) if (pixels[y * width + x] < 200) count++;
    }
    return count;
  } finally { pixmap.destroy(); view.destroy(); }
}
function contents(s, page = 0) {
  const view = s.doc.loadPage(page);
  try {
    const value = view.getObject().get('Contents');
    const elements = value.isArray() ? [...Array(value.length)].map((_, index) => value.get(index)) : [value];
    return elements.map(element => {
      const data = element.readStream();
      try { return Buffer.from(data.asUint8Array()).toString('latin1'); } finally { data.destroy(); }
    }).join('\n');
  } finally { view.destroy(); }
}
function editObject(s, sc, target, action, box, transform) {
  return request(s, { op: 'edit-object', page: sc.page, targetId: target.targetId, generation: sc.generation, revision: sc.revision, action, box, transform });
}
function editObjects(s, sc, items) {
  return request(s, { op: 'edit-objects', page: sc.page, generation: sc.generation, revision: sc.revision, items });
}
/** The journal slice every mutation reply carries, read from the session (no read op exists). */
const journal = s => {
  const pdf = s.doc.asPDF();
  return { revision: s.revision, savedPosition: s.savedPosition, canUndo: pdf.canUndo(), canRedo: pdf.canRedo() };
};
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
/** A scratch copy of direct3 with page 0's stream rewritten by `edit` (text → text or text[]). */
async function scratch(edit) {
  const pdf = new mupdf.PDFDocument(await readFile(DIRECT3));
  try {
    const view = pdf.loadPage(0);
    try {
      const page = view.getObject();
      const data = page.get('Contents').readStream();
      const text = Buffer.from(data.asUint8Array()).toString('latin1');
      data.destroy();
      const parts = [edit(text)].flat();
      if (parts.length === 1) page.get('Contents').writeStream(Buffer.from(parts[0], 'latin1'));
      else {
        // Probe P3 recipe: /Contents [a b] from two fresh streams.
        const array = pdf.newArray();
        for (const part of parts) array.push(pdf.addStream(Buffer.from(part, 'latin1'), {}));
        page.put('Contents', array);
      }
    } finally { view.destroy(); }
    return openBytes(buffer(pdf.saveToBuffer('').asUint8Array()));
  } finally { pdf.destroy(); }
}

const OBJECTS6 = 'fixtures/p6/objects6.pdf';
const OBJECTS6_SHA = 'ca0a5988eb81cd6b35916918695cd8c9881b8ebba82ae590e38cdf8a97fb2ab8';
const HELLO = 'fixtures/hello.pdf';
const FIELDS = 'fixtures/p3/field-defs.pdf';
const LINKS = 'fixtures/p6/links.pdf';
const SIGFIELD = 'fixtures/p5/empty-sigfield.pdf';
const NOEDIT = 'fixtures/encrypted-noedit.pdf';
const HELLO_SHA = '229ca9488a139d3f32b864ba24055152fe7f0faf670c9db6491144f23d275b94';
const FIELDS_SHA = '43782ca55aa415cc9fc09406d95e9300326ad281fac5a0176b14c88a295f4250';
const LINKS_SHA = '0ac413b0fa9b42062542afbc970f0245bceacab0a789017c90a413733962d0bc';
const USER = 'u7-user';
const OWNER = 'u7-owner';
const id = (sc, name) => object(sc, name).targetId;
const near = (actual, expected, eps = 0.01) =>
  assert.ok(actual.length === expected.length && actual.every((value, k) => Math.abs(value - expected[k]) <= eps), `${JSON.stringify(actual)} ≈ ${JSON.stringify(expected)}`);
/** Runs one successful edit on `page`: one revision, `check(after)`, then undo → bytes-equal. */
function lands(s, run, check, page = 0) {
  const before = scan(s, page);
  const beforeBytes = contents(s, page);
  const reply = run(before);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.revision, before.revision + 1);
  assert.equal(reply.dirty, true);
  check(scan(s, page), reply);
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.equal(contents(s, page), beforeBytes, 'undo restores the stream byte for byte');
  return reply;
}
async function openIn(s, name) {
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: buffer(await readFile(name)) }, s).reply;
  assert.equal(reply.ok, true, reply.error);
}
function copy(s, page, targets) {
  const sc = scan(s, page);
  return request(s, { op: 'clip-copy', page, generation: sc.generation, revision: sc.revision, targets });
}
const paste = (s, page, at) => request(s, { op: 'clip-paste', page, ...(at === undefined ? {} : { at }) });
const batch = (s, page, items) => {
  const sc = scan(s, page);
  return request(s, { op: 'edit-batch', page, generation: sc.generation, revision: sc.revision, items });
};
const fields = (s, page = 0) => request(s, { op: 'fields', page }).items;
const links = (s, page = 0) => request(s, { op: 'links', page }).items;
const marks = (s, page = 0) => request(s, { op: 'annots', page }).items.filter(item => item.type === 'Redact');
/** `nothingWritten` for a page without /Contents: the journal and the page dict are unchanged. */
function still(s, run) {
  const before = journal(s);
  const dict = s.doc.loadPage(0).getObject().toString();
  const reply = run();
  assert.deepEqual(journal(s), before, 'journalState deep-equal');
  assert.equal(s.doc.loadPage(0).getObject().toString(), dict);
  return reply;
}
const text = (sc, value) => sc.items.find(item => item.kind === 'text' && item.text === value);
function rawXObject(doc, page, name) {
  const view = doc.loadPage(page);
  try {
    const data = view.getObject().get('Resources').get('XObject').get(name).readRawStream();
    try { return Buffer.from(data.asUint8Array()); } finally { data.destroy(); }
  } finally { view.destroy(); }
}
/** Re-opens `s`'s document restricted (§0): save under a user password, open, authenticate as user. */
function restrict(s, permissions = { print: true, copy: false, edit: false, annotate: true, form: false }) {
  const set = request(s, { op: 'security-set', mode: 'aes-256', userPassword: USER, ownerPassword: OWNER, permissions });
  assert.equal(set.ok, true, set.error);
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const bytes = saved.bytes;
  const reopenIn = () => {
    assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'restricted.pdf', pdf: bytes.slice(0) }, s).reply.ok, true);
    assert.equal(request(s, { op: 'authenticate', password: USER }).auth, 'user');
  };
  reopenIn();
  return reopenIn;
}
/** A one-page `width`×`height` PDF, or hello.pdf with `marks` raw /Redact annotations. */
async function built({ width = 100, height = 100, redacts = 0 } = {}) {
  const pdf = redacts ? new mupdf.PDFDocument(await readFile(HELLO)) : new mupdf.PDFDocument();
  try {
    if (!redacts) pdf.insertPage(-1, pdf.addPage([0, 0, width, height], 0, pdf.newDictionary(), ''));
    else {
      const page = pdf.findPage(0);
      const annots = pdf.newArray();
      for (let k = 0; k < redacts; k++) {
        const annot = pdf.newDictionary();
        annot.put('Type', pdf.newName('Annot'));
        annot.put('Subtype', pdf.newName('Redact'));
        annot.put('Rect', [10, 10, 20, 20]);
        annot.put('P', page);
        annots.push(pdf.addObject(annot));
      }
      page.put('Annots', annots);
    }
    return buffer(pdf.saveToBuffer('').asUint8Array());
  } finally { pdf.destroy(); }
}

test('U7 §1.1/§1.2: same-page Im1 pastes one step down-right as PA1 in one revision; undo; reopen edits it', async () => {
  const s = await open(OBJECTS6);
  const sc = scan(s, 0);
  const im1 = object(sc, 'Im1');
  const copied = copy(s, 0, [{ kind: 'image', id: im1.targetId }]);
  assert.equal(copied.ok, true, copied.error);
  assert.equal(copied.count, 1);
  assert.deepEqual(copied.kinds, { image: 1, form: 0, text: 0, field: 0, link: 0, mark: 0 });
  assert.equal(copied.revision, 0, 'a copy writes nothing');
  assert.equal(copied.dirty, false);
  const reply = lands(s, () => paste(s, 0), after => {
    const pa = object(after, 'PA1');
    near(pa.placement.box, [82, 590, 182, 640]);
    assert.deepEqual(pa.objectOps, im1.objectOps);
    assert.equal(objects(after).length, objects(sc).length + 1);
  });
  assert.deepEqual(reply.names, ['PA1']);
  assert.equal(reply.count, 1);
  assert.equal(paste(s, 0).ok, true);
  const r = reopen(s);
  near(boxes(scan(r, 0), 'PA1')[0], [82, 590, 182, 640]);
  lands(r, sc2 => editObject(r, sc2, object(sc2, 'PA1'), 'transform', undefined, 'rotate-cw'), after => assert.ok(object(after, 'PA1')));
  lands(r, sc2 => editObject(r, sc2, object(sc2, 'PA1'), 'crop', [92, 600, 172, 630]), after => near(object(after, 'PA1').placement.visible ?? [92, 600, 172, 630], [92, 600, 172, 630], 0.5));
  assert.equal(await sha(OBJECTS6), OBJECTS6_SHA);
});

test('U7 §1.2: objects6 page 0 → page 1 lands PA1 with the same image; page 0 unchanged', async () => {
  const s = await open(OBJECTS6);
  const im1 = object(scan(s, 0), 'Im1');
  assert.equal(copy(s, 0, [{ kind: 'image', id: im1.targetId }]).ok, true);
  const page0 = contents(s, 0);
  lands(s, () => paste(s, 1), after => {
    const pa = object(after, 'PA1');
    assert.equal(pa.image.width, im1.image.width);
    near(pa.placement.box, [82, 590, 182, 640]);
  }, 1);
  assert.equal(contents(s, 0), page0);
});

test('U7 §9.18: cross-document paste keeps the stream bytes; the clipboard survives open and a second paste', async () => {
  const s = await open(OBJECTS);
  const source = object(scan(s, 0), 'Im1');
  assert.equal(copy(s, 0, [{ kind: 'image', id: source.targetId }]).ok, true);
  await openIn(s, OBJECTS6);
  const first = paste(s, 0);
  assert.equal(first.ok, true, first.error);
  near(boxes(scan(s, 0), 'PA1')[0], [72, 650, 136, 746]);
  const original = new mupdf.PDFDocument(await readFile(OBJECTS));
  try {
    assert.ok(rawXObject(s.doc, 0, 'PA1').equals(rawXObject(original, 0, 'Im1')), 'raw stream bytes equal');
  } finally { original.destroy(); }
  const second = paste(s, 0);
  assert.equal(second.ok, true, second.error);
  assert.deepEqual(second.names, ['PA2']);
  near(boxes(scan(s, 0), 'PA2')[0], [82, 640, 146, 736]);
  const r = reopen(s);
  assert.equal(objects(scan(r, 0)).length, 8);
  assert.equal(await sha(OBJECTS), OBJECTS_SHA);
});

test('U7 §1.2: a Form pastes with its fonts and paints; objects6 Fm1 on its own page makes 7 objects', async () => {
  const s = await open(OBJECTS);
  assert.equal(copy(s, 0, [{ kind: 'form', id: id(scan(s, 0), 'Fm2') }]).ok, true);
  await openIn(s, OBJECTS6);
  const reply = paste(s, 0);
  assert.equal(reply.ok, true, reply.error);
  const pa = object(scan(s, 0), reply.names[0]);
  assert.equal(pa.kind, 'form');
  const view = s.doc.loadPage(0);
  try {
    const fonts = view.getObject().get('Resources').get('XObject').get(reply.names[0]).get('Resources').get('Font');
    assert.ok(fonts.isDictionary() && fonts.get('F1').isDictionary(), '/Font/F1 came along');
  } finally { view.destroy(); }
  const [x0, y0, x1, y1] = pa.placement.box;
  assert.ok(ink(s, 0, [x0, 792 - y1, x1, 792 - y0]) > 0, 'the pasted Form paints');
  const o = await open(OBJECTS6);
  assert.equal(copy(o, 0, [{ kind: 'form', id: id(scan(o, 0), 'Fm1') }]).ok, true);
  lands(o, () => paste(o, 0), after => assert.equal(objects(after).length, 7));
});

test('U7 §9.4: orientation survives paste and reopen; a second copy replaces the first', async () => {
  const s = await open(OBJECTS6);
  const sc = scan(s, 0);
  assert.equal(copy(s, 0, [{ kind: 'image', id: id(sc, 'Im2') }, { kind: 'image', id: id(sc, 'Im3') }]).ok, true);
  const reply = paste(s, 0);
  assert.equal(reply.ok, true, reply.error);
  assert.deepEqual(reply.names, ['PA1', 'PA2']);
  const check = after => {
    assert.deepEqual(object(after, 'PA1').placement.orient, object(sc, 'Im2').placement.orient);
    assert.deepEqual(object(after, 'PA2').placement.orient, object(sc, 'Im3').placement.orient);
    assert.deepEqual(object(after, 'PA1').objectOps, object(sc, 'Im2').objectOps);
  };
  check(scan(s, 0));
  check(scan(reopen(s), 0));
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.equal(copy(s, 0, [{ kind: 'image', id: id(sc, 'Im1') }]).ok, true);
  const replaced = copy(s, 0, [{ kind: 'image', id: id(sc, 'Im4') }]);
  assert.equal(replaced.count, 1);
  assert.equal(paste(s, 0).ok, true);
  const box = boxes(scan(s, 0), 'PA1')[0];
  near([box[2] - box[0], box[3] - box[1]], [60, 40]);
});

test('U7 §9.3/§10.13: `at` places the union top-left; the delta clamps inside the page', async () => {
  const s = await open(OBJECTS6);
  assert.equal(copy(s, 0, [{ kind: 'image', id: id(scan(s, 0), 'Im1') }]).ok, true);
  lands(s, () => paste(s, 0, [300, 400]), after => near(boxes(after, 'PA1')[0], [300, 350, 400, 400]));
  lands(s, () => paste(s, 0, [580, 400]), after => near(boxes(after, 'PA1')[0], [512, 350, 612, 400]));
});

test('U7 §1.2 refusals: empty clipboard, malformed at, bigger than the page, read-only, target cap — nothing written', async () => {
  const s = await open(OBJECTS6);
  refused(nothingWritten(s, 0, () => paste(s, 0)), 'empty-clipboard');
  assert.equal(copy(s, 0, [{ kind: 'form', id: id(scan(s, 0), 'Fm1') }]).ok, true);
  refused(nothingWritten(s, 0, () => paste(s, 0, 'x')), 'out-of-bounds');
  refused(nothingWritten(s, 0, () => paste(s, 0, [1, Number.NaN])), 'out-of-bounds');
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'small.pdf', pdf: await built() }, s).reply.ok, true);
  refused(still(s, () => paste(s, 0)), 'out-of-bounds');
  await openIn(s, NOEDIT);
  const locked = paste(s, 0);
  assert.equal(locked.ok, false);
  assert.equal(locked.reason, 'read-only');
  // The target cap: a page already at MAX_EDIT_TARGETS takes no pasted object.
  const d = await open(DIRECT3);
  const count = scan(d, 0).items.length;
  const full = await scratch(body => `${body}\n${'q 100 0 0 50 72 600 cm /Im1 Do Q\n'.repeat(MAX_EDIT_TARGETS - count)}`);
  const fullScan = scan(full, 0);
  assert.equal(fullScan.items.length, MAX_EDIT_TARGETS);
  assert.equal(copy(full, 0, [{ kind: 'image', id: id(fullScan, 'Im1') }]).ok, true);
  refused(nothingWritten(full, 0, () => paste(full, 0)), 'too-large');
  refused(copy(full, 0, [{ kind: 'image', id: 'p0.c0.1.deadbeef' }]), 'stale-target', 'p0.c0.1.deadbeef');
  refused(copy(full, 0, []), 'invalid-selection');
});

test('U7 §1.3: a batch moves a run and an image in one revision, undoes in one step, saves', async () => {
  const s = await open(DIRECT3);
  const sc = scan(s, 0);
  const run = sc.items.find(item => item.kind === 'text' && item.text.startsWith('Alpha'));
  const items = [
    { op: 'edit-text', targetId: run.targetId, move: { dx: 0, dy: 20 } },
    { op: 'edit-object', targetId: id(sc, 'Im1'), action: 'move', box: [82, 590, 182, 640] },
  ];
  lands(s, () => batch(s, 0, items), (after, reply) => {
    assert.equal(reply.count, 2);
    near(boxes(after, 'Im1')[0], [82, 590, 182, 640]);
    const moved = after.items.find(item => item.kind === 'text' && item.text === run.text);
    near(moved.bounds, [run.bounds[0], run.bounds[1] + 20, run.bounds[2], run.bounds[3] + 20], 0.5);
  });
  const stale = [...items, { op: 'edit-object', targetId: 'p0.c0.1.deadbeef', action: 'delete' }];
  const reply = nothingWritten(s, 0, () => batch(s, 0, stale));
  refused(reply, 'stale-target', 'p0.c0.1.deadbeef');
  assert.equal(reply.index, 2);
  assert.equal(batch(s, 0, items).ok, true);
  near(boxes(scan(reopen(s), 0), 'Im1')[0], [82, 590, 182, 640]);
  assert.equal(await sha(DIRECT3), DIRECT3_SHA);
});

test('U7 §1.3: batches over fields, links and marks are one step each', async () => {
  const f = await open(FIELDS);
  const beforeFields = fields(f);
  const moved = batch(f, 0, [{ op: 'field-props', widgetId: 5, rect: [80, 112, 260, 142] }, { op: 'field-delete', widgetId: 10 }]);
  assert.equal(moved.ok, true, moved.error);
  assert.equal(moved.revision, 1);
  near(fields(f).find(item => item.id === 5).rect, [80, 112, 260, 142]);
  assert.equal(fields(f).some(item => item.id === 10), false);
  assert.equal(request(f, { op: 'undo' }).ok, true);
  assert.deepEqual(fields(f), beforeFields);

  const l = await open(LINKS);
  const beforeLinks = links(l);
  const set = batch(l, 0, [{ op: 'link-set', annotId: 10, rect: [80, 72, 208, 92] }, { op: 'link-delete', annotId: 11 }]);
  assert.equal(set.ok, true, set.error);
  near(links(l).find(item => item.annotId === 10).rect, [80, 72, 208, 92]);
  assert.equal(links(l).some(item => item.annotId === 11), false);
  assert.equal(request(l, { op: 'undo' }).ok, true);
  assert.deepEqual(links(l), beforeLinks);
  refused(nothingWritten(l, 0, () => batch(l, 0, [{ op: 'link-delete', annotId: 10 }, { op: 'link-delete', annotId: 99 }])), 'stale-target');

  const h = await open(HELLO);
  assert.equal(request(h, { op: 'redact-add', page: 0, rect: [100, 100, 200, 150] }).ok, true);
  assert.equal(request(h, { op: 'redact-add', page: 0, rect: [100, 200, 200, 250] }).ok, true);
  const [a, b] = marks(h);
  const two = batch(h, 0, [{ op: 'redact-set', annotId: a.id, rect: [110, 100, 210, 150] }, { op: 'annot-delete', annotId: b.id }]);
  assert.equal(two.ok, true, two.error);
  assert.equal(two.revision, 3);
  assert.equal(marks(h).length, 1);
  near(marks(h)[0].rect, [110, 100, 210, 150], 0.5);
  assert.equal(request(h, { op: 'undo' }).ok, true);
  assert.equal(marks(h).length, 2);
});

test('U7 §1.3 refusals: shape, repeats, the cap, and a restricted member — nothing written', async () => {
  const f = await open(FIELDS);
  refused(still(f, () => batch(f, 0, [{ op: 'field-delete', widgetId: 10 }])), 'invalid-selection');
  const repeat = still(f, () => batch(f, 0, [{ op: 'field-delete', widgetId: 10 }, { op: 'field-props', widgetId: 10, rect: [72, 247, 200, 272] }]));
  refused(repeat, 'invalid-selection');
  assert.equal(repeat.index, 1);
  assert.equal(repeat.widgetId, 10);
  const many = Array.from({ length: MAX_EDIT_TARGETS }, (_, k) => ({ op: 'field-delete', widgetId: 1000 + k }));
  refused(still(f, () => batch(f, 0, [...many, { op: 'bogus' }])), 'too-large');
  restrict(f);
  const member = still(f, () => batch(f, 0, [{ op: 'field-props', widgetId: 5, rect: [80, 112, 260, 142] }, { op: 'field-delete', widgetId: 10 }]));
  assert.equal(member.ok, false);
  assert.equal(member.reason, 'read-only');
  assert.equal(member.index, 0);
  assert.equal(member.widgetId, 5);
});

test('D4 text: "Move me" pastes onto rotated page 1 at its view bounds with the source width', async () => {
  const s = await open(DIRECT3);
  const run = text(scan(s, 0), 'Move me');
  assert.equal(copy(s, 0, [{ kind: 'text', id: run.targetId }]).ok, true);
  const reply = lands(s, () => paste(s, 1), after => {
    const pasted = after.items.find(item => item.kind === 'text' && item.text === 'Move me');
    assert.ok(pasted, 'the run scans on page 1');
    near(pasted.bounds, run.bounds, 0.5);
    near([pasted.bounds[2] - pasted.bounds[0]], [run.bounds[2] - run.bounds[0]], 0.5);
  }, 1);
  assert.equal(reply.count, 1);
  assert.equal(reply.targetIds.length, 1);
});

test('D4 text: a run pasted on page 2 writes /PF1, keeps its id hash and reopens after a garbage save', async () => {
  const s = await open(DIRECT3);
  const run = text(scan(s, 0), 'Move me');
  assert.equal(copy(s, 0, [{ kind: 'text', id: run.targetId }]).ok, true);
  const reply = paste(s, 2);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.revision, 1);
  assert.equal(reply.targetIds.length, 1);
  assert.ok(reply.targetIds[0].endsWith('.6c51e7e2'), reply.targetIds[0]);
  assert.match(contents(s, 2), /\/PF1 /);
  const pasted = scan(s, 2).items.find(item => item.targetId === reply.targetIds[0]);
  assert.equal(pasted.text, 'Move me');
  near(pasted.bounds, run.bounds, 0.5);
  const r = reopen(s);
  assert.equal(text(scan(r, 2), 'Move me').text, 'Move me');
  assert.equal(await sha(DIRECT3), DIRECT3_SHA);
});

test('D4 text: a same-page paste lands one step on; both same-text runs then scan as candidates (§10.14, U8 §9.5)', async () => {
  const s = await open(DIRECT3);
  const run = text(scan(s, 0), 'Move me');
  assert.equal(copy(s, 0, [{ kind: 'text', id: run.targetId }]).ok, true);
  let pastedId;
  const reply = lands(s, () => paste(s, 0), after => {
    const both = after.items.filter(item => item.kind === 'text' && item.text === 'Move me');
    assert.equal(both.length, 2);
    // P6 U8 D1: a direct run with a unique match at its own origin is a candidate even when its text repeats.
    assert.ok(both.every(item => item.support === 'candidate'), JSON.stringify(both.map(item => [item.support, item.reason])));
    const pasted = both.find(item => item.targetId !== run.targetId);
    near(pasted.bounds, run.bounds.map(value => value + 10), 0.5);
    pastedId = pasted.targetId;
  });
  assert.equal(reply.targetIds.length, 1);
  assert.equal(reply.targetIds[0], pastedId, 'targetIds names the one pasted run');
  assert.equal(text(scan(s, 0), 'Move me').support, 'candidate', 'undo restores the source run as a candidate');
});

test('D4 text: a run inside a Form refuses not-candidate', async () => {
  const o = await open(OBJECTS);
  const form2 = scan(o, 0).items.find(item => item.kind === 'text' && item.text === 'FORM2');
  refused(copy(o, 0, [{ kind: 'text', id: form2.targetId }]), 'not-candidate', form2.targetId);
});

test('D4 fields: a text field pastes as "nom copy" with its value, default, required and rect + d', async () => {
  const s = await open(FIELDS);
  assert.equal(request(s, { op: 'field-set', page: 0, widgetId: 5, value: 'Zed' }).ok, true);
  assert.equal(copy(s, 0, [{ kind: 'field', id: 5 }]).ok, true);
  const reply = paste(s, 0);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.revision, 2);
  assert.deepEqual(reply.fields, ['nom copy']);
  const nom = fields(s).find(item => item.name === 'nom copy');
  assert.equal(nom.value, 'Zed');
  assert.equal(nom.default, 'défaut');
  assert.equal(nom.required, true);
  near(nom.rect, [82, 122, 262, 152], 0.5);
  assert.equal(fields(reopen(s)).find(item => item.name === 'nom copy').value, 'Zed');
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.equal(fields(s).some(item => item.name === 'nom copy'), false);
});

test('D4 fields: a radio group pastes as one group with all kids and value B', async () => {
  const s = await open(FIELDS);
  const kids = fields(s).filter(item => item.name === 'color');
  assert.equal(copy(s, 0, [{ kind: 'field', id: 7 }]).ok, true);
  const radio = paste(s, 0);
  assert.equal(radio.ok, true, radio.error);
  assert.deepEqual(radio.fields, ['color copy']);
  assert.equal(radio.fieldIds.length, kids.length);
  const copies = fields(s).filter(item => item.name === 'color copy');
  assert.equal(copies.length, kids.length);
  const checked = copies.filter(kid => kid.value === true);
  assert.equal(checked.length, 1);
  near(checked[0].rect, kids.find(kid => kid.value === true).rect.map(v => v + 10), 0.5);
  const dict = s.doc.asPDF().newIndirect(checked[0].id).get('Parent');
  assert.equal(dict.get('V').asName(), 'B');
});

test('D4 fields: a signature field refuses not-candidate', async () => {
  const g = await open(SIGFIELD);
  const reply = copy(g, 0, [{ kind: 'field', id: 7 }]);
  refused(reply, 'not-candidate');
  assert.equal(reply.widgetId, 7);
});

test('D4 links: a uri link pastes as +1 with the same uri at rect + d', async () => {
  const s = await open(LINKS);
  const one = links(s).find(item => item.annotId === 10);
  assert.equal(copy(s, 0, [{ kind: 'link', id: 10 }]).ok, true);
  const reply = paste(s, 0);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.linkIds.length, 1);
  assert.equal(links(s).length, 4);
  const pasted = links(s).find(item => item.annotId === reply.linkIds[0]);
  assert.equal(pasted.uri, one.uri);
  near(pasted.rect, one.rect.map(v => v + 10), 0.5);
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.equal(links(s).length, 3);
});

test('D4 links: a page link past a 1-page target is skipped and the rest pastes', async () => {
  const s = await open(LINKS);
  assert.equal(copy(s, 0, [{ kind: 'link', id: 10 }, { kind: 'link', id: 11 }]).ok, true);
  await openIn(s, HELLO);
  const mixed = paste(s, 0);
  assert.equal(mixed.ok, true, mixed.error);
  assert.equal(mixed.count, 1);
  assert.deepEqual(mixed.skipped, [{ index: 1, kind: 'link', reason: 'no-such-page' }]);
  assert.equal(links(s).length, 1);
});

test('D4 links: a clipboard of only page links past the end refuses no-such-page', async () => {
  const s = await open(LINKS);
  assert.equal(copy(s, 0, [{ kind: 'link', id: 11 }]).ok, true);
  await openIn(s, HELLO);
  refused(nothingWritten(s, 0, () => paste(s, 0)), 'no-such-page');
});

test('D4 links: a blocked link and a missing link refuse', async () => {
  const s = await open(LINKS);
  const blocked = copy(s, 0, [{ kind: 'link', id: 13 }]);
  refused(blocked, 'not-candidate');
  assert.equal(blocked.annotId, 13);
  refused(copy(s, 0, [{ kind: 'link', id: 999 }]), 'stale-target');
  assert.equal(await sha(LINKS), LINKS_SHA);
});

test('D4 marks: a mark pastes as +1 at rect + d', async () => {
  const s = await open(HELLO);
  assert.equal(request(s, { op: 'redact-add', page: 0, rect: [100, 100, 200, 150] }).ok, true);
  const [mark] = marks(s);
  assert.equal(copy(s, 0, [{ kind: 'mark', id: mark.id }]).ok, true);
  const reply = paste(s, 0);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.markIds.length, 1);
  assert.equal(marks(s).length, 2);
  near(marks(s).find(item => item.id === reply.markIds[0]).rect, mark.rect.map(v => v + 10), 0.5);
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.equal(marks(s).length, 1);
  assert.equal(await sha(HELLO), HELLO_SHA);
});

test('D4 marks: a document at MAX_REDACT_MARKS refuses too-many with the member index, bytes unchanged', async () => {
  const s = await open(HELLO);
  assert.equal(request(s, { op: 'redact-add', page: 0, rect: [100, 100, 200, 150] }).ok, true);
  assert.equal(copy(s, 0, [{ kind: 'mark', id: marks(s)[0].id }]).ok, true);
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'marks.pdf', pdf: await built({ redacts: MAX_REDACT_MARKS }) }, s).reply.ok, true);
  const capped = nothingWritten(s, 0, () => paste(s, 0));
  assert.equal(capped.ok, false);
  refused(capped, 'too-many');
  assert.equal(capped.index, 0);
  assert.equal(capped.kind, 'mark');
});

test('U7 §9.6/§10.4: a paste onto unbalanced page content refuses with detail unbalanced-content', async () => {
  const s = await open(DIRECT3);
  assert.equal(copy(s, 0, [{ kind: 'image', id: id(scan(s, 0), 'Im1') }]).ok, true);
  const u = await scratch(body => `${body}\nq BT /F1 12 Tf 1 0 0 1 72 700 Tm (Hello) Tj ET\n`);
  u.clipboard = s.clipboard;
  refused(nothingWritten(u, 0, () => paste(u, 0)), 'unbalanced-content');
});

/** direct3 with a field, a link and a mark added on page 0 (revision 3), and every kind's target. */
async function mixed() {
  const s = await open(DIRECT3);
  assert.equal(request(s, { op: 'field-add', page: 0, kind: 'text', name: 'mx', rect: [300, 300, 400, 320] }).ok, true);
  assert.equal(request(s, { op: 'link-add', page: 0, rect: [300, 330, 400, 350], target: { uri: 'https://example.org/mx' } }).ok, true);
  assert.equal(request(s, { op: 'redact-add', page: 0, rect: [300, 360, 400, 380] }).revision, 3);
  const sc = scan(s, 0);
  const targets = [
    { kind: 'image', id: id(sc, 'Im1') }, { kind: 'form', id: id(sc, 'Fm1') }, { kind: 'text', id: text(sc, 'Move me').targetId },
    { kind: 'field', id: fields(s).find(item => item.name === 'mx').id },
    { kind: 'link', id: links(s).find(item => item.uri === 'https://example.org/mx').annotId },
    { kind: 'mark', id: marks(s)[0].id },
  ];
  return { s, targets };
}

test('D4 mixed: every kind from one page pastes onto page 2 as ONE journal op; undo and redo', async () => {
  const { s, targets } = await mixed();
  const copied = copy(s, 0, targets);
  assert.equal(copied.ok, true, copied.error);
  assert.equal(copied.count, 6);
  assert.deepEqual(copied.kinds, { image: 1, form: 1, text: 1, field: 1, link: 1, mark: 1 });
  const page2 = contents(s, 2);
  const reply = paste(s, 2);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.revision, 4, 'revision +1 only');
  assert.equal(reply.count, 6);
  assert.equal(reply.names.length, 2);
  assert.deepEqual(reply.fields, ['mx copy']);
  assert.equal(reply.linkIds.length, 1);
  assert.equal(reply.markIds.length, 1);
  assert.equal(reply.targetIds.length, 1);
  assert.equal(request(s, { op: 'undo' }).ok, true, 'the whole paste undoes in one step');
  assert.equal(contents(s, 2), page2);
  assert.equal(fields(s, 2).length + links(s, 2).length + marks(s, 2).length, 0);
  assert.equal(request(s, { op: 'redo' }).ok, true);
  assert.equal(fields(s, 2).length, 1);
  assert.equal(links(s, 2).length, 1);
  assert.equal(marks(s, 2).length, 1);
  assert.equal(objects(scan(s, 2)).filter(item => /^PA\d$/.test(item.objectName)).length, 2);
});

test('D4 copy-denied: every kind refuses copy read-only; a paste refuses at its first denied member; marks still paste', async () => {
  const { s, targets } = await mixed();
  assert.equal(copy(s, 0, targets).ok, true);
  const reopenRestricted = restrict(s);
  for (const target of targets) {
    const denied = copy(s, 0, [target]);
    assert.equal(denied.ok, false);
    assert.equal(denied.reason, 'read-only', target.kind);
  }
  const member = nothingWritten(s, 2, () => paste(s, 2));
  assert.equal(member.ok, false);
  assert.equal(member.reason, 'read-only');
  assert.equal(member.index, 0);
  assert.equal(member.kind, 'image');
  await openIn(s, HELLO);
  assert.equal(request(s, { op: 'redact-add', page: 0, rect: [100, 100, 200, 150] }).ok, true);
  assert.equal(copy(s, 0, [{ kind: 'mark', id: marks(s)[0].id }]).ok, true);
  reopenRestricted();
  const annotated = paste(s, 0);
  assert.equal(annotated.ok, true, annotated.error);
  assert.equal(annotated.markIds.length, 1);
});

test('U7 §1.3: a refused annotation item refuses before the op, so the redo branch survives', async () => {
  const f = await open(FIELDS);
  assert.equal(request(f, { op: 'field-props', page: 0, widgetId: 5, rect: [80, 112, 260, 142] }).ok, true);
  assert.equal(request(f, { op: 'undo' }).ok, true);
  const tiny = still(f, () => batch(f, 0, [{ op: 'field-props', widgetId: 5, rect: [80, 112, 260, 142] }, { op: 'field-props', widgetId: 10, rect: [0, 0, 1, 1] }]));
  refused(tiny, 'invalid');
  assert.equal(tiny.index, 1);
  assert.equal(journal(f).canRedo, true);
  const { s } = await mixed();
  assert.equal(request(s, { op: 'undo' }).ok, true);
  const sc = scan(s, 0);
  const content = [
    { op: 'edit-text', targetId: text(sc, 'Move me').targetId, move: { dx: 1, dy: 0 } },
    { op: 'edit-object', targetId: id(sc, 'Im1'), action: 'move', box: object(sc, 'Im1').placement.box.map((v, k) => k % 2 ? v : v + 1) },
  ];
  const stale = nothingWritten(s, 0, () => batch(s, 0, [...content, { op: 'field-delete', widgetId: 9999 }]));
  refused(stale, 'stale-target');
  assert.equal(stale.index, 2);
  assert.equal(journal(s).canRedo, true);
  const annot = nothingWritten(s, 0, () => batch(s, 0, [{ op: 'field-props', widgetId: fields(s).find(item => item.name === 'mx').id, rect: [310, 300, 410, 320] }, { op: 'annot-delete', annotId: 9999 }]));
  assert.equal(annot.ok, false);
  assert.equal(annot.index, 1);
  assert.equal(journal(s).canRedo, true);
  assert.equal(request(s, { op: 'redo' }).ok, true);
});

test('U7 §1.3/§1.4: a run, an image, a field, a link and a mark move in one batch; one undo, redo, reopen', async () => {
  const { s } = await mixed();
  const pageDict = () => { const view = s.doc.loadPage(0); try { return view.getObject().toString(); } finally { view.destroy(); } };
  const before = { c: contents(s, 0), d: pageDict(), f: fields(s), l: links(s), m: marks(s) };
  const sc = scan(s, 0);
  const run = text(sc, 'Move me');
  const box = object(sc, 'Im1').placement.box.map((v, k) => k % 2 ? v : v + 1);
  const f = fields(s).find(item => item.name === 'mx'), l = links(s).find(item => item.uri === 'https://example.org/mx'), m = marks(s)[0];
  const shift = rect => rect.map((v, k) => k % 2 ? v : v + 1);
  const reply = batch(s, 0, [
    { op: 'edit-text', targetId: run.targetId, move: { dx: 1, dy: 0 } },
    { op: 'edit-object', targetId: id(sc, 'Im1'), action: 'move', box },
    { op: 'field-props', widgetId: f.id, rect: shift(f.rect) },
    { op: 'link-set', annotId: l.annotId, rect: shift(l.rect) },
    { op: 'redact-set', annotId: m.id, rect: shift(m.rect) },
  ]);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.revision, 4);
  assert.equal(reply.count, 5);
  const check = t => {
    const after = scan(t, 0);
    near(text(after, 'Move me').bounds, shift(run.bounds), 0.5);
    near(boxes(after, 'Im1')[0], box);
    near(fields(t).find(item => item.name === 'mx').rect, shift(f.rect), 0.5);
    near(links(t).find(item => item.uri === 'https://example.org/mx').rect, shift(l.rect), 0.5);
    near(marks(t)[0].rect, shift(m.rect), 0.5);
  };
  check(s);
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.deepEqual({ c: contents(s, 0), d: pageDict(), f: fields(s), l: links(s), m: marks(s) }, before);
  assert.equal(request(s, { op: 'redo' }).ok, true);
  check(s);
  check(reopen(s));
  assert.equal(await sha(DIRECT3), DIRECT3_SHA);
});

test('U7 §9.9: a copy over the byte budget refuses too-large and the old clipboard survives', async () => {
  const doc = new mupdf.PDFDocument();
  let bytes;
  try {
    const side = 3000, big = new Uint8Array(side * side * 4);
    for (let k = 0; k < big.length; k++) big[k] = (k * 2654435761) >>> 24;
    const image = doc.addRawStream(big, { Type: 'XObject', Subtype: 'Image', Width: side * 2, Height: side * 2, ColorSpace: 'DeviceGray', BitsPerComponent: 8 });
    const resources = doc.newDictionary(), xobjects = doc.newDictionary();
    xobjects.put('B', image);
    resources.put('XObject', xobjects);
    doc.insertPage(-1, doc.addPage([0, 0, 612, 792], 0, resources, 'q 100 0 0 100 100 100 cm /B Do Q\n'));
    bytes = buffer(doc.saveToBuffer('').asUint8Array());
  } finally { doc.destroy(); }
  const s = await open(OBJECTS6);
  assert.equal(copy(s, 0, [{ kind: 'image', id: id(scan(s, 0), 'Im1') }]).ok, true);
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'big.pdf', pdf: bytes }, s).reply.ok, true);
  refused(nothingWritten(s, 0, () => copy(s, 0, [{ kind: 'image', id: id(scan(s, 0), 'B') }])), 'too-large');
  const pasted = paste(s, 0);
  assert.equal(pasted.ok, true, pasted.error);
  assert.deepEqual(pasted.names, ['PA1']);
  assert.equal(await sha(OBJECTS6), OBJECTS6_SHA);
});

test('U7: OP_PERMISSION rows and fixture hashes', async () => {
  const { OP_PERMISSION } = await import('../build-test/engine-core.js');
  assert.equal(OP_PERMISSION['clip-copy'], 'copy');
  assert.equal(OP_PERMISSION['clip-paste'], 'any');
  assert.equal(OP_PERMISSION['edit-batch'], 'any');
  assert.equal(await sha(DIRECT3), DIRECT3_SHA);
  assert.equal(await sha(OBJECTS), OBJECTS_SHA);
  assert.equal(await sha(OBJECTS6), OBJECTS6_SHA);
  assert.equal(await sha(HELLO), HELLO_SHA);
  assert.equal(await sha(FIELDS), FIELDS_SHA);
  assert.equal(await sha(LINKS), LINKS_SHA);
});

// Debt sweep 2 (contract-debt2.md §9.1–§9.3): N1 edit-batch items carry only their op's keys; N2 a
// pasted run keeps its render mode (Tr) with a fixed stroke state.
test('debt2 N1: a field-props item carrying name refuses invalid-selection; the field keeps its name', async () => {
  const f = await open(FIELDS);
  const before = fields(f).find(item => item.id === 5).name;
  const r = still(f, () => batch(f, 0, [{ op: 'field-props', widgetId: 5, rect: [80, 112, 260, 142], name: 'RENAMED' }, { op: 'field-delete', widgetId: 10 }]));
  refused(r, 'invalid-selection');
  assert.match(r.error, /unexpected key name on field-props$/);
  assert.equal(r.index, 0);
  assert.equal(r.widgetId, 5);
  assert.equal(fields(f).find(item => item.id === 5).name, before);
  assert.equal(fields(f).some(item => item.id === 10), true);
});

test('debt2 N1: a page key on a page-0 batch refuses; the page-1 mark stays put (links.pdf)', async () => {
  const s = await open(LINKS);
  assert.equal(request(s, { op: 'redact-add', page: 0, rect: [100, 100, 200, 150] }).ok, true);
  assert.equal(request(s, { op: 'redact-add', page: 0, rect: [100, 200, 200, 250] }).ok, true);
  assert.equal(request(s, { op: 'redact-add', page: 1, rect: [100, 100, 200, 150] }).ok, true);
  const p0 = marks(s, 0)[0], p1 = marks(s, 1)[0];
  const r = nothingWritten(s, 0, () => batch(s, 0, [{ op: 'redact-set', annotId: p1.id, rect: [300, 300, 400, 350], page: 1 }, { op: 'annot-delete', annotId: p0.id }]));
  refused(r, 'invalid-selection');
  assert.match(r.error, /unexpected key page on redact-set$/);
  assert.equal(r.index, 0);
  assert.deepEqual(marks(s, 1)[0].rect, p1.rect);
  assert.equal(marks(s, 0).length, 2);
});

test('debt2 N1: a link-set item carrying target refuses; link 10 keeps its uri', async () => {
  const l = await open(LINKS);
  const before = links(l).find(item => item.annotId === 10);
  assert.equal(before.uri, 'https://example.org/one');
  const r = nothingWritten(l, 0, () => batch(l, 0, [{ op: 'link-set', annotId: 10, rect: [80, 72, 208, 92], target: { uri: 'https://evil.example/' } }, { op: 'link-delete', annotId: 11 }]));
  refused(r, 'invalid-selection');
  assert.match(r.error, /unexpected key target on link-set$/);
  assert.equal(r.index, 0);
  assert.equal(r.annotId, 10);
  assert.deepEqual(links(l).find(item => item.annotId === 10), before);
});

/** The LAST /Contents element of `page` (the one a paste appends). */
function lastContents(s, page = 0) {
  const view = s.doc.loadPage(page);
  try {
    const value = view.getObject().get('Contents');
    const element = value.isArray() ? value.get(value.length - 1) : value;
    const data = element.readStream();
    try { return Buffer.from(data.asUint8Array()).toString('latin1'); } finally { data.destroy(); }
  } finally { view.destroy(); }
}

for (const tr of [1, 2]) {
  test(`debt2 N2: a Tr ${tr} run pastes with ${tr} Tr and a black 1 pt stroke; the rescanned record keeps renderMode ${tr}`, async () => {
    const { scanEditTargets } = await import('../build-oracle/engine/oracle/app-engine/content-edit.js');
    const s = await scratch(body => body.replace('BT /F1 14 Tf 72 400 Td (Move me)', `BT /F1 14 Tf ${tr} Tr 72 400 Td (Move me)`));
    const run = text(scan(s, 0), 'Move me');
    assert.equal(run.support, 'candidate');
    assert.equal(copy(s, 0, [{ kind: 'text', id: run.targetId }]).ok, true);
    const pasted = paste(s, 0);
    assert.equal(pasted.ok, true, pasted.error);
    assert.equal(pasted.targetIds.length, 1);
    const block = lastContents(s, 0);
    assert.match(block, new RegExp(` 0 Ts ${tr} Tr 0 g 0 G 1 w `), block);
    assert.doesNotMatch(block, / 0 Tr /, block);
    assert.equal(scanEditTargets(s.doc, 0).records.get(pasted.targetIds[0]).renderMode, tr);
    // debt2 review E-R3: the render mode survives save and reopen.
    assert.equal(scanEditTargets(reopen(s).doc, 0).records.get(pasted.targetIds[0]).renderMode, tr);
  });
}

// debt2 review E-R2: every op's allow-list is pinned, not only field-props/redact-set/link-set.
test('debt2 N1: a stray page key on each of the 8 batch ops refuses invalid-selection; the journal is unchanged', async () => {
  const s = await open(DIRECT3);
  const ops = {
    'edit-text': { targetId: 'x', move: { dx: 1, dy: 0 } }, 'edit-object': { targetId: 'x', action: 'delete' },
    'field-props': { widgetId: 1, rect: [0, 0, 10, 10] }, 'field-delete': { widgetId: 1 },
    'link-set': { annotId: 1, rect: [0, 0, 10, 10] }, 'link-delete': { annotId: 1 },
    'redact-set': { annotId: 1, rect: [0, 0, 10, 10] }, 'annot-delete': { annotId: 1 },
  };
  for (const [op, keys] of Object.entries(ops)) {
    const r = nothingWritten(s, 0, () => batch(s, 0, [{ op, ...keys, page: 1 }, { op: 'annot-delete', annotId: 999 }]));
    refused(r, 'invalid-selection');
    assert.match(r.error, new RegExp(`unexpected key page on ${op}$`), op);
    assert.equal(r.index, 0, op);
  }
});
