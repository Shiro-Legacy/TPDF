import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine, MAX_EDIT_TARGETS } from './rust-engine.mjs';

// P6 U6 engine (contract-p6-unit6.md §1.1–1.4, §9.1–9.12): eight orientations and one
// clip per placement; `edit-object {action:'transform'|'crop'}`, move/resize/duplicate
// through the clip, and `edit-objects` resize/transform items. Harness copied from
// tests/p6-objects5.test.mjs:1-112 (after the U6 §1.5 fix).
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
const IDENTITY = { swap: false, sx: 1, sy: 1 };
const ALL_OPS = ['move', 'resize', 'delete', 'replace', 'duplicate', 'transform', 'crop'];
const id = (sc, name) => object(sc, name).targetId;
/** Runs one successful edit: one revision, then `check(after)`, then undo → bytes-equal. */
function lands(s, run, check) {
  const before = scan(s, 0);
  const beforeBytes = contents(s);
  const reply = run(before);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.revision, before.revision + 1);
  assert.equal(reply.dirty, true);
  check(scan(s, 0));
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.equal(contents(s), beforeBytes, 'undo restores the stream byte for byte');
  return reply;
}
/** A refusal: the token, dirty unchanged (false on a clean document), revision and contents() unchanged. */
function refuses(s, run, detail, targetId, wasDirty = false) {
  const reply = nothingWritten(s, 0, run);
  refused(reply, detail, targetId);
  assert.equal(reply.dirty ?? false, wasDirty);
  return reply;
}
const transform = (s, sc, name, how, box) => editObject(s, sc, object(sc, name), 'transform', box, how);
const reCount = s => (contents(s).match(/ re W n/g) ?? []).length;

test('U6 pin: objects6.pdf is unchanged before the run', async () => {
  assert.equal(await sha(OBJECTS6), OBJECTS6_SHA);
});

test('U6 §1.1: eight orientations and one clip are candidates with transform and crop', async () => {
  const s = await open(OBJECTS6);
  const p0 = scan(s, 0);
  assert.deepEqual(object(p0, 'Im1').placement, { box: [72, 600, 172, 650], orient: IDENTITY });
  assert.deepEqual(object(p0, 'Im2').placement, { box: [400, 650, 450, 750], orient: { swap: true, sx: 1, sy: -1 } });
  assert.deepEqual(object(p0, 'Im3').placement, { box: [200, 600, 300, 650], orient: { swap: false, sx: -1, sy: 1 } });
  assert.deepEqual(object(p0, 'Im4').placement, { box: [72, 400, 132, 440], orient: IDENTITY, full: [72, 400, 172, 450] });
  assert.deepEqual(object(p0, 'Im4').bounds, [72, 400, 132, 440], 'bounds are the visible rect');
  assert.deepEqual(object(p0, 'Fm1').placement, { box: [72, 100, 372, 300], orient: IDENTITY });
  assert.deepEqual(object(p0, 'Im5').placement, { box: [300, 120, 350, 170], orient: IDENTITY });
  for (const name of ['Im1', 'Im2', 'Im3', 'Im4', 'Im5']) {
    assert.equal(object(p0, name).support, 'candidate', name);
    assert.deepEqual(object(p0, name).objectOps, ALL_OPS, name);
  }
  assert.deepEqual(object(p0, 'Fm1').objectOps, ALL_OPS.filter(op => op !== 'replace'));
  const over = p0.items.find(item => item.kind === 'text' && item.text === 'Over');
  assert.equal(over?.support, 'candidate');
});

test('U6 §1.2: rotate Im1 clockwise writes one swapped cm, renders turned, undoes', async () => {
  const s = await open(OBJECTS6);
  const before = contents(s);
  const reply = transform(s, scan(s, 0), 'Im1', 'rotate-cw');
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.revision, 1);
  const im1 = object(scan(s, 0), 'Im1');
  assert.deepEqual(im1.placement, { box: [97, 575, 147, 675], orient: { swap: true, sx: 1, sy: -1 } });
  assert.ok(contents(s).includes('0 -100 50 0 97 675 cm'), contents(s));
  const again = reopen(s);
  assert.deepEqual(object(scan(again, 0), 'Im1').placement.box, [97, 575, 147, 675]);
  assert.ok(ink(again, 0, [100, 120, 144, 130]) > 0, 'the turned picture inks its new strip');
  assert.equal(ink(again, 0, [72, 180, 92, 190]), 0, 'the old bottom-left corner is empty');
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.equal(contents(s), before);
});

test('U6 §1.2: every rotate and flip lands with the pinned orient, box and cm', async () => {
  const s = await open(OBJECTS6);
  const orientBox = (name, orient, box) => after => assert.deepEqual(
    [object(after, name).placement.orient, object(after, name).placement.box], [orient, box], name);
  lands(s, sc => transform(s, sc, 'Im1', 'rotate-ccw'), after => {
    orientBox('Im1', { swap: true, sx: -1, sy: 1 }, [97, 575, 147, 675])(after);
    assert.ok(contents(s).includes('0 100 -50 0 147 575 cm'));
  });
  lands(s, sc => transform(s, sc, 'Im1', 'flip-h'), after => {
    orientBox('Im1', { swap: false, sx: -1, sy: 1 }, [72, 600, 172, 650])(after);
    assert.ok(contents(s).includes('-100 0 0 50 172 600 cm'));
  });
  lands(s, sc => transform(s, sc, 'Im1', 'flip-v'), after => {
    orientBox('Im1', { swap: false, sx: 1, sy: -1 }, [72, 600, 172, 650])(after);
    assert.ok(contents(s).includes('100 0 0 -50 72 650 cm'));
  });
  // Twice clockwise is a half turn.
  const half = scan(s, 0);
  assert.equal(transform(s, half, 'Im1', 'rotate-cw').ok, true);
  assert.equal(transform(s, scan(s, 0), 'Im1', 'rotate-cw').ok, true);
  assert.deepEqual(object(scan(s, 0), 'Im1').placement.orient, { swap: false, sx: -1, sy: -1 });
  request(s, { op: 'undo' });
  request(s, { op: 'undo' });
  lands(s, sc => transform(s, sc, 'Im2', 'rotate-cw'), after => {
    orientBox('Im2', { swap: false, sx: -1, sy: -1 }, [375, 675, 475, 725])(after);
    assert.ok(contents(s).includes('-100 0 0 -50 475 725 cm'));
  });
  lands(s, sc => transform(s, sc, 'Im2', 'flip-h'), orientBox('Im2', { swap: true, sx: -1, sy: -1 }, [400, 650, 450, 750]));
  lands(s, sc => transform(s, sc, 'Im3', 'flip-h'), after => {
    orientBox('Im3', IDENTITY, [200, 600, 300, 650])(after);
    assert.ok(contents(s).includes('100 0 0 50 200 600 cm'));
  });
  lands(s, sc => transform(s, sc, 'Fm1', 'rotate-cw'), after => {
    orientBox('Fm1', { swap: true, sx: 1, sy: -1 }, [122, 50, 322, 350])(after);
    assert.ok(contents(s).includes('0 -1 1 0 122 350 cm'));
    const fm1 = object(scan(reopen(s), 0), 'Fm1');
    assert.equal(fm1.support, 'candidate');
    assert.deepEqual(fm1.placement.box, [122, 50, 322, 350]);
  });
});

test('U6 §1.2/§9.10: a rotation that leaves the page, a bad token and a refit box', async () => {
  const s = await open(OBJECTS6);
  assert.equal(editObject(s, scan(s, 0), object(scan(s, 0), 'Im1'), 'resize', [0, 0, 200, 20]).ok, true);
  const dirty = scan(s, 0);
  refuses(s, sc => transform(s, sc, 'Im1', 'rotate-cw'), 'out-of-bounds', undefined, true);
  assert.equal(request(s, { op: 'edit-targets', page: 0 }).revision, dirty.revision);
  lands(s, sc => transform(s, sc, 'Im1', 'rotate-cw', [90, 0, 110, 200]),
    after => assert.deepEqual(object(after, 'Im1').placement, { box: [90, 0, 110, 200], orient: { swap: true, sx: 1, sy: -1 } }));
  request(s, { op: 'undo' });
  refuses(s, sc => transform(s, sc, 'Im1', 'spin', [90, 0, 110, 200]), 'invalid-transform');
  // U7 §1.5 (N2): a junk box too — invalid-transform is checked before the box.
  refuses(s, sc => transform(s, sc, 'Im1', undefined, 'junk'), 'invalid-transform');
  // The objectOps check comes first: a placement that cannot turn keeps its reason.
  const o = await open(OBJECTS);
  refuses(o, sc => editObject(o, sc, objects(sc)[1], 'transform', undefined, 'spin'), 'unsupported-transform');
});

test('U6 §1.2/§9.3: crop inserts, rewrites and removes one clip; refusals write nothing', async () => {
  const s = await open(OBJECTS6);
  const original = contents(s);
  let reply = editObject(s, scan(s, 0), object(scan(s, 0), 'Im1'), 'crop', [92, 610, 152, 640]);
  assert.equal(reply.ok, true, reply.error);
  assert.ok(contents(s).includes('q\n92 610 60 30 re W n 100 0 0 50 72 600 cm /Im1 Do Q'), contents(s));
  assert.deepEqual(object(scan(s, 0), 'Im1').placement, { box: [92, 610, 152, 640], orient: IDENTITY, full: [72, 600, 172, 650] });
  const again = reopen(s);
  assert.equal(ink(again, 0, [74, 144, 90, 150]), 0, 'the cropped-away strip is empty');
  assert.ok(ink(again, 0, [100, 155, 140, 180]) > 0, 'the kept window inks');
  // The same crop again is a no-op.
  const same = editObject(s, scan(s, 0), object(scan(s, 0), 'Im1'), 'crop', [92, 610, 152, 640]);
  assert.deepEqual([same.ok, same.noop, same.revision], [true, true, 1]);
  refuses(s, sc => editObject(s, sc, object(sc, 'Im1'), 'crop', [60, 600, 172, 650]), 'out-of-bounds', undefined, true);
  refuses(s, sc => editObject(s, sc, object(sc, 'Im1'), 'crop', [92, 610, 92.5, 640]), 'out-of-bounds', undefined, true);
  reply = editObject(s, scan(s, 0), object(scan(s, 0), 'Im1'), 'crop', [72, 600, 172, 650]);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(contents(s), original, 'cropping to the full box removes the clip bytes');
  assert.deepEqual(object(scan(s, 0), 'Im1').placement, { box: [72, 600, 172, 650], orient: IDENTITY });
  request(s, { op: 'undo' });
  request(s, { op: 'undo' });
  assert.equal(contents(s), original);
  // Uncropped and asked for the full box: nothing to do.
  const none = editObject(s, scan(s, 0), object(scan(s, 0), 'Im1'), 'crop', [72, 600, 172, 650]);
  assert.deepEqual([none.ok, none.noop, none.dirty], [true, true, false]);
  lands(s, sc => editObject(s, sc, object(sc, 'Im4'), 'crop', [80, 405, 120, 430]), after => {
    assert.deepEqual(object(after, 'Im4').placement, { box: [80, 405, 120, 430], orient: IDENTITY, full: [72, 400, 172, 450] });
    assert.ok(contents(s).includes('q 80 405 40 25 re W n 100 0 0 50 72 400 cm'), contents(s));
    assert.equal(reCount(s), 1, 'the existing re is rewritten in place');
  });
  lands(s, sc => editObject(s, sc, object(sc, 'Im4'), 'crop', [72, 400, 172, 450]), () => {
    assert.ok(contents(s).includes('q 100 0 0 50 72 400 cm /Im4 Do Q'), contents(s));
    assert.equal(reCount(s), 0);
  });
});

test('U6 §1.2/§9.4/§9.9: move, resize, rotate and duplicate carry the clip', async () => {
  const s = await open(OBJECTS6);
  lands(s, sc => editObject(s, sc, object(sc, 'Im4'), 'move', [172, 400, 232, 440]), after => {
    assert.ok(contents(s).includes('q 172 400 60 40 re W n 100 0 0 50 172 400 cm /Im4 Do Q'), contents(s));
    assert.deepEqual(object(after, 'Im4').placement, { box: [172, 400, 232, 440], orient: IDENTITY, full: [172, 400, 272, 450] });
  });
  lands(s, sc => editObject(s, sc, object(sc, 'Im4'), 'resize', [72, 400, 192, 480]), after =>
    assert.deepEqual(object(after, 'Im4').placement, { box: [72, 400, 192, 480], orient: IDENTITY, full: [72, 400, 272, 500] }));
  lands(s, sc => transform(s, sc, 'Im4', 'rotate-cw'), after =>
    assert.deepEqual(object(after, 'Im4').placement, { box: [82, 390, 122, 450], orient: { swap: true, sx: 1, sy: -1 }, full: [82, 350, 132, 450] }));
  // A same-box move on a clipped placement is a no-op.
  const same = editObject(s, scan(s, 0), object(scan(s, 0), 'Im4'), 'move', [72, 400, 132, 440]);
  assert.deepEqual([same.ok, same.noop, same.dirty], [true, true, false]);
  lands(s, sc => editObject(s, sc, object(sc, 'Im2'), 'duplicate', [410, 660, 460, 760]), after => {
    assert.ok(contents(s).includes('0 -100 50 0 410 760 cm'), contents(s));
    assert.deepEqual(boxes(after, 'Im2'), [[400, 650, 450, 750], [410, 660, 460, 760]]);
    assert.ok(objects(after).filter(item => item.objectName === 'Im2').every(item => item.placement.orient.swap === true));
  });
  lands(s, sc => editObject(s, sc, object(sc, 'Im4'), 'duplicate', [82, 410, 142, 450]), after => {
    assert.equal(reCount(s), 2, 'the copy carries its own clip');
    const copies = objects(after).filter(item => item.objectName === 'Im4');
    assert.deepEqual(copies.map(item => item.placement), [
      { box: [72, 400, 132, 440], orient: IDENTITY, full: [72, 400, 172, 450] },
      { box: [82, 410, 142, 450], orient: IDENTITY, full: [82, 410, 182, 460] },
    ]);
    assert.ok(copies.every(item => item.objectOps.includes('crop')));
    assert.deepEqual(boxes(scan(reopen(s), 0), 'Im4'), [[72, 400, 132, 440], [82, 410, 142, 450]]);
  });
  // §9.9: a 1 pt crop resized to the page width would make the uncropped box too large.
  assert.equal(editObject(s, scan(s, 0), object(scan(s, 0), 'Im4'), 'crop', [72, 400, 73, 440]).ok, true);
  refuses(s, sc => editObject(s, sc, object(sc, 'Im4'), 'resize', [0, 400, 600, 440]), 'too-large', undefined, true);
  request(s, { op: 'undo' });
});

test('U6 §9.7: replace keeps a swapped cm (no aspect refit)', async () => {
  const s = await open(OBJECTS6);
  const image = await readFile('fixtures/img-rgb.png');
  const picture = { name: 'img-rgb.png', bytes: buffer(image) };
  lands(s, sc => request(s, { op: 'edit-object', page: 0, targetId: id(sc, 'Im2'), generation: sc.generation, revision: sc.revision, action: 'replace', image: picture }), after => {
    const im2 = after.items.find(item => item.kind === 'image' && item.placement?.box[0] === 400);
    assert.deepEqual(im2.placement, { box: [400, 650, 450, 750], orient: { swap: true, sx: 1, sy: -1 } });
    assert.ok(contents(s).includes('0 -100 50 0 400 750 cm'), contents(s));
  });
});

test('U6 §1.3: edit-objects resize and transform items, all-or-nothing', async () => {
  const s = await open(OBJECTS6);
  let reply = lands(s, sc => editObjects(s, sc, [
    { targetId: id(sc, 'Im1'), action: 'resize', box: [72, 600, 272, 650] },
    { targetId: id(sc, 'Im3'), action: 'resize', box: [328, 600, 528, 650] },
  ]), after => {
    assert.deepEqual(object(after, 'Im1').placement.box, [72, 600, 272, 650]);
    assert.deepEqual(object(after, 'Im3').placement, { box: [328, 600, 528, 650], orient: { swap: false, sx: -1, sy: 1 } });
  });
  assert.equal(reply.count, 2);
  reply = lands(s, sc => editObjects(s, sc, [
    { targetId: id(sc, 'Im1'), action: 'transform', transform: 'rotate-cw', box: [97, 575, 147, 675] },
    { targetId: id(sc, 'Im3'), action: 'transform', transform: 'rotate-cw', box: [225, 575, 275, 675] },
  ]), after => {
    assert.deepEqual(object(after, 'Im1').placement, { box: [97, 575, 147, 675], orient: { swap: true, sx: 1, sy: -1 } });
    assert.deepEqual(object(after, 'Im3').placement, { box: [225, 575, 275, 675], orient: { swap: true, sx: 1, sy: 1 } });
  });
  assert.equal(reply.count, 2);
  // A same-box resize item is dropped like a same-box move.
  reply = lands(s, sc => editObjects(s, sc, [
    { targetId: id(sc, 'Im1'), action: 'resize', box: [72, 600, 172, 650] },
    { targetId: id(sc, 'Im4'), action: 'resize', box: [72, 400, 192, 480] },
  ]), after => assert.deepEqual(object(after, 'Im4').placement.full, [72, 400, 272, 500]));
  assert.equal(reply.count, 1);
  refuses(s, sc => editObjects(s, sc, [
    { targetId: id(sc, 'Im1'), action: 'crop', box: [92, 610, 152, 640] },
    { targetId: id(sc, 'Im3'), action: 'delete' },
  ]), 'invalid-selection');
  // U7 §1.5 (N1): the id is read before `refuses()` runs, so the all-or-nothing targetId is pinned.
  const im3 = id(scan(s, 0), 'Im3');
  refuses(s, sc => editObjects(s, sc, [
    { targetId: id(sc, 'Im1'), action: 'resize', box: [72, 600, 272, 650] },
    { targetId: im3, action: 'transform', transform: 'rotate-cw', box: [600, 575, 650, 675] },
  ]), 'out-of-bounds', im3);
  refuses(s, sc => editObjects(s, sc, [
    { targetId: id(sc, 'Im1'), action: 'delete' },
    { targetId: im3, action: 'transform', transform: 'spin', box: [225, 575, 275, 675] },
  ]), 'invalid-transform', im3);
});

test('U6 §1.4: page 1 draws a different picture under the same name', async () => {
  const s = await open(OBJECTS6);
  const p0 = object(scan(s, 0), 'Im1');
  const p1 = object(scan(s, 1), 'Im1');
  assert.equal(p1.support, 'candidate');
  assert.deepEqual(p1.placement.box, [72, 600, 172, 650]);
  assert.notEqual(p1.image.width, p0.image.width);
});

test('U6 pin: objects6.pdf is unchanged after the run', async () => {
  assert.equal(await sha(OBJECTS6), OBJECTS6_SHA);
});
