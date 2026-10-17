/**
 * P3 U4 — Image and Form placements: `edit-object` move/resize/delete/replace.
 * Engine direct, no renderer UI.
 *
 * Expected geometry comes from the fixture bytes and the amended contract §7:
 * a simple placement is one `q … cm … Do Q` block in a single /Contents element
 * with an unflipped axis-aligned `cm` and no `cm` in any enclosing state; a Form's
 * box is its `cm` applied to `/Matrix × /BBox`; an Image's is the unit square.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine, editNumber } from './rust-engine.mjs';

const engine = createEngine(mupdf);

function toArrayBuffer(bytes) {
  if (bytes instanceof ArrayBuffer) return bytes;
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

let nextId = 900;
async function openFile(path, session = engine.createSession()) {
  const reply = engine.dispatch(
    { id: 1, op: 'open', name: path, pdf: toArrayBuffer(await readFile(path)) },
    session,
  ).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function op(session, request) {
  return engine.dispatch({ id: nextId++, ...request }, session).reply;
}

function openBytes(bytes, session = engine.createSession(), name = 'scratch.pdf') {
  const reply = engine.dispatch({ id: nextId++, op: 'open', name, pdf: toArrayBuffer(bytes) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function scan(session, page = 0) {
  const reply = op(session, { op: 'edit-targets', page });
  assert.equal(reply.ok, true, reply.error);
  return reply;
}

/** The resource name of the first candidate that allows `action`, on `page`. */
function target(session, name, action, page = 0) {
  const reply = scan(session, page);
  const item = reply.items.find(entry => entry.kind !== 'text' && entry.objectName === name
    && entry.support === 'candidate' && (entry.objectOps ?? []).includes(action));
  assert.ok(item, `no ${action} candidate ${name} in ${JSON.stringify(reply.items.filter(entry => entry.kind !== 'text').map(entry => `${entry.objectName}:${entry.objectOps}:${entry.reason}`))}`);
  return { item, generation: reply.generation, revision: reply.revision, page };
}

function editObject(session, found, action, extra = {}) {
  return op(session, {
    op: 'edit-object', page: found.page, targetId: found.item.targetId,
    generation: found.generation, revision: found.revision, action, ...extra,
  });
}

/** Every decoded /Contents element of one page, in order. */
function contentsOf(session, page = 0) {
  const target = session.doc.loadPage(page);
  try {
    const contents = target.getObject().get('Contents');
    const elements = contents.isArray() ? [...Array(contents.length)].map((_, index) => contents.get(index)) : [contents];
    return elements.map(element => {
      const buffer = element.readStream();
      try {
        return Buffer.from(buffer.asUint8Array()).toString('latin1');
      } finally {
        buffer.destroy();
      }
    });
  } finally {
    target.destroy();
  }
}

function xobjectNames(session, page = 0) {
  const target = session.doc.loadPage(page);
  try {
    const xobjects = target.getObject().get('Resources')?.get('XObject');
    return xobjects ? [...Array(xobjects.length)].map((_, index) => String(xobjects.get(index).get('Subtype'))) : [];
  } finally {
    target.destroy();
  }
}

function savedBytes(session) {
  const reply = op(session, { op: 'save' });
  assert.equal(reply.ok, true, reply.error);
  return new Uint8Array(reply.bytes);
}

function savedDoc(bytes) {
  return mupdf.PDFDocument.openDocument(bytes, 'application/pdf');
}

/** Every image object's pixel size in saved bytes, so a dropped object is visible. */
function imageSizes(bytes) {
  const document = savedDoc(bytes);
  const sizes = [];
  for (let number = 1; number <= document.countObjects(); number++) {
    try {
      const object = document.newIndirect(number);
      const width = object.get('Width');
      const height = object.get('Height');
      if (width && width.isNumber() && height && height.isNumber()) sizes.push(`${width.asNumber()}x${height.asNumber()}`);
    } catch {
      // An object this walk cannot read is not an image fact.
    }
  }
  return sizes;
}

// ------------------------------------------------------------------ discovery

test('U4 discovery: edit-objects.pdf p0 lists the exact objectOps table', async () => {
  const session = await openFile('fixtures/p3/edit-objects.pdf');
  const reply = scan(session, 0);
  const objects = reply.items.filter(item => item.kind !== 'text');
  assert.deepEqual(
    objects.map(item => `${item.kind}:${item.support}:${(item.objectOps ?? []).join('+')}:${item.reason ?? ''}`),
    [
      'image:candidate:move+resize+delete+replace+duplicate+transform+crop:',
      'image:candidate:replace:unsupported-transform', // inside a clip (W n)
      'image:candidate:replace:unsupported-transform', // under an outer scale cm
      'image:candidate:move+resize+delete+duplicate+transform+crop:unsupported-image', // /SMask
      'form:candidate:move+resize+delete+duplicate+transform+crop:',
      'form:candidate:move+resize+delete+duplicate+transform+crop:', // rotated cm
      'form:candidate:move+resize+delete+duplicate+transform+crop:', // /Matrix [1 0 0 1 10 10]
    ],
  );
  const simpleImage = objects[0];
  assert.deepEqual(simpleImage.placement.box, [72, 650, 136, 746]);
  assert.deepEqual(simpleImage.bounds, [72, 650, 136, 746]);
  assert.deepEqual(simpleImage.image, { width: 200, height: 300, smask: false, mask: false, decode: false, interpolate: false, oc: false });
  assert.equal(objects[3].image.smask, true);
  // A Form's box is cm applied to /Matrix × /BBox: [0 0 120 40] + (10,10), at (72,200).
  assert.deepEqual(objects[6].placement.box, [82, 210, 202, 250]);
  // A placement the page cannot edit carries no box and no ops.
  assert.equal(objects[1].placement, undefined);
  assert.equal(objects[1].bounds, null);
  assert.deepEqual(objects[5].bounds, [280, 300, 300, 420]);
});

test('U4 discovery: edit-objects.pdf p1 inherits /Resources, so each placement is cowable', async () => {
  const session = await openFile('fixtures/p3/edit-objects.pdf');
  const reply = scan(session, 1);
  const objects = reply.items.filter(item => item.kind !== 'text');
  assert.deepEqual(objects.map(item => `${item.objectName}:${item.cowable}:${JSON.stringify(item.placement.box)}`),
    ['Im1:true:[72,650,136,746]', 'Fm1:true:[72,500,192,520]']);
});

test('U4 discovery: targets-streams.pdf page-level placements are candidates, the nested Form is not', async () => {
  const session = await openFile('fixtures/p3/targets-streams.pdf');
  const reply = scan(session, 0);
  const objects = reply.items.filter(item => item.kind !== 'text');
  assert.deepEqual(objects.map(item => `${item.kind}:${item.objectName}:${item.support}`), [
    'form:Fm1:candidate', 'form:Fm1:candidate', 'form:Fm2:candidate', 'form:Fm1:unsupported', 'image:Im1:candidate',
  ]);
  assert.deepEqual(objects[0].placement.box, [72, 640, 272, 656]);
  assert.deepEqual(objects[2].placement.box, [72, 560, 272, 600]);
  assert.equal(objects[3].reason, 'shared-or-unproven-sharing', 'a depth-1 Form is the U4 ceiling');
  assert.deepEqual(objects[4].placement.box, [400, 700, 408, 708]);
  assert.equal(objects.slice(0, 3).every(item => item.bounds !== null), true);
});

test('U4 discovery: a stacked cm is replace-only, and a rotated page blocks every placement', async () => {
  const transformed = scan(await openFile('fixtures/p0b/edit-transformed.pdf'));
  const image = transformed.items.find(item => item.kind === 'image');
  assert.deepEqual(image.objectOps, ['replace']);
  assert.equal(image.reason, 'unsupported-transform');
  assert.equal(image.bounds, null);

  // The same canonical blocks on a /Rotate 90 page are uneditable U4 geometry (ceiling §7.6).
  const source = mupdf.PDFDocument.openDocument(
    new Uint8Array(await readFile('fixtures/p3/targets-streams.pdf')), 'application/pdf');
  source.loadPage(0).getObject().put('Rotate', 90);
  const rotated = openBytes(source.saveToBuffer({ compress: true, garbage: true }).asUint8Array(), engine.createSession(), 'rotated.pdf');
  const reply = scan(rotated);
  const objects = reply.items.filter(item => item.kind !== 'text');
  assert.ok(objects.length >= 4, JSON.stringify(reply.items.map(item => item.kind)));
  assert.ok(objects.every(item => item.support === 'unsupported' && item.reason === 'unsupported-transform'),
    JSON.stringify(objects.map(item => `${item.kind}:${item.reason}`)));
});

// --------------------------------------------------------------------- writes

test('U4 move: only the selected placement\'s cm changes, and undo restores the bytes', async () => {
  const session = await openFile('fixtures/p3/targets-streams.pdf');
  const before = contentsOf(session, 0).join('\n');
  const page2 = contentsOf(session, 1).join('\n');
  assert.ok(before.includes('8 0 0 8 400 700 cm'));
  const found = target(session, 'Im1', 'move');
  const reply = editObject(session, found, 'move', { box: [100, 700, 120, 720] });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const after = contentsOf(session, 0).join('\n');
  assert.ok(after.includes('20 0 0 20 100 700 cm'), after);
  assert.equal(after.includes('400 700'), false);
  assert.equal(contentsOf(session, 1).join('\n'), page2, 'the other page is untouched');
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentsOf(session, 0).join('\n'), before);
});

test('U4 resize: a Form with /Matrix [1 0 0 1 10 10] rewrites cm against Matrix × BBox', async () => {
  const session = await openFile('fixtures/p3/edit-objects.pdf');
  const before = contentsOf(session, 0).join('\n');
  assert.ok(before.includes('1 0 0 1 72 200 cm /Fm2 Do Q'), before);
  const reply = editObject(session, target(session, 'Fm2', 'resize'), 'resize', { box: [82, 210, 142, 230] });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  // 60/120 = .5, 20/40 = .5, and e,f put the (10,10)-translated BBox corner at (82,210).
  assert.ok(contentsOf(session, 0).join('\n').includes('0.5 0 0 0.5 77 205 cm /Fm2 Do Q'), contentsOf(session, 0).join('\n'));
});

test('U4 delete: the whole q … Do Q block goes, the neighbour placement stays, undo restores', async () => {
  const session = await openFile('fixtures/p3/targets-streams.pdf');
  const before = contentsOf(session, 0).join('\n');
  const page2 = contentsOf(session, 1).join('\n');
  const reply = editObject(session, target(session, 'Fm1', 'delete'), 'delete');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const after = contentsOf(session, 0).join('\n');
  assert.equal(after.includes('1 0 0 1 72 640 cm /Fm1 Do Q'), false, after);
  assert.equal(after.includes('1 0 0 1 72 600 cm /Fm1 Do Q'), true, 'the second Fm1 placement survives');
  assert.equal(contentsOf(session, 1).join('\n'), page2);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentsOf(session, 0).join('\n'), before);
});

test('U4 replace: the Do is renamed and repointed, the old resource entry kept, undo restores it', async () => {
  const session = await openFile('fixtures/p3/targets-streams.pdf');
  const before = contentsOf(session, 0).join('\n');
  assert.ok(before.includes('8 0 0 8 400 700 cm /Im1 Do Q'));
  const image = await readFile('fixtures/img-rgb.png');
  const found = target(session, 'Im1', 'replace');
  const reply = editObject(session, found, 'replace', {
    image: { name: 'img-rgb.png', bytes: toArrayBuffer(image) },
  });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const after = contentsOf(session, 0).join('\n');
  // 300×200 into an 8×8 box: 8 wide, 8/1.5 high, anchored at the old bottom-left.
  assert.ok(after.includes('8 0 0 5.3333 400 700 cm /Im1r1 Do Q'), after);
  assert.equal(after.includes('/Im1 Do Q'), false);
  const resources = session.doc.loadPage(0).getObject().get('Resources').get('XObject');
  assert.equal(resources.get('Im1').isNull(), false, 'the old entry is never removed (its dictionary may be shared)');
  assert.equal(resources.get('Im1r1').isNull(), false);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentsOf(session, 0).join('\n'), before);
  const restored = session.doc.loadPage(0).getObject().get('Resources').get('XObject');
  assert.equal(restored.get('Im1').isNull(), false, 'the old resource entry is still there after undo');
  assert.equal(restored.get('Im1r1').isNull(), true, 'undo removes the replacement entry');
});

test('U4 replace: a non-simple placement keeps its transform and is stretched', async () => {
  const session = await openFile('fixtures/p3/edit-objects.pdf');
  const before = contentsOf(session, 0).join('\n');
  const image = await readFile('fixtures/img-rgb.png');
  // The clipped Im1 is the replace-only candidate on the page.
  const reply = scan(session, 0);
  const clipped = reply.items.find(item => item.kind === 'image' && item.objectName === 'Im1' && item.reason === 'unsupported-transform');
  const found = { item: clipped, generation: reply.generation, revision: reply.revision, page: 0 };
  assert.equal(found.item.reason, 'unsupported-transform');
  const refused = editObject(session, found, 'move', { box: [80, 640, 144, 736] });
  assert.equal(refused.ok, false);
  assert.equal(refused.detail, 'unsupported-transform');
  const moved = editObject(session, found, 'replace', { image: { name: 'img-rgb.png', bytes: toArrayBuffer(image) } });
  assert.equal(moved.ok, true, JSON.stringify(moved));
  const after = contentsOf(session, 0).join('\n');
  assert.ok(after.includes('64 0 0 64 72 400 cm /Im1r1 Do Q'), after);
  assert.equal(before.includes('64 0 0 64 72 400 cm /Im1 Do Q'), true);
});

test('U4 replace: the old image stays in the file (ceiling, not redaction)', async () => {
  const session = await openFile('fixtures/p0b/edit-transformed.pdf');
  const image = await readFile('fixtures/img-rgb.png');
  const reply = editObject(session, target(session, 'Im1', 'replace'), 'replace', {
    image: { name: 'img-rgb.png', bytes: toArrayBuffer(image) },
  });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const bytes = savedBytes(session);
  // The old /XObject entry is never removed (review U4): its dictionary may be shared
  // with another page, so the old 8x8 object survives next to the 300x200 replacement.
  assert.equal(imageSizes(bytes).some(size => size === '8x8'), true, imageSizes(bytes).join(' '));
  assert.equal(imageSizes(bytes).some(size => size === '300x200'), true, imageSizes(bytes).join(' '));
});

/** Two pages sharing one image resource dictionary (inherited, or one indirect /Resources). */
function sharedResourcesPdf(mode) {
  const content = 'q 64 0 0 64 72 600 cm /Im1 Do Q\n';
  const resources = '<< /XObject << /Im1 6 0 R >> >>';
  const own = mode === 'indirect' ? '/Resources 7 0 R' : '';
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 ${mode === 'inherited' ? `/Resources ${resources}` : ''} >>`,
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] ${own} /Contents 5 0 R >>`,
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] ${own} /Contents 8 0 R >>`,
    `<< /Length ${content.length} >>\nstream\n${content}endstream`,
    '<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\n\x00\nendstream',
    resources,
    `<< /Length ${content.length} >>\nstream\n${content}endstream`,
  ];
  let out = '%PDF-1.4\n';
  const offsets = [];
  objects.forEach((body, index) => { offsets.push(out.length); out += `${index + 1} 0 obj\n${body}\nendobj\n`; });
  const xref = out.length;
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('')}`;
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(out, 'latin1');
}

for (const mode of ['inherited', 'indirect']) {
  test(`U4 review: replace on page 0 keeps page 1's image when their resources are shared (${mode})`, async () => {
    const session = openBytes(sharedResourcesPdf(mode));
    const image = await readFile('fixtures/img-rgb.png');
    const reply = editObject(session, target(session, 'Im1', 'replace'), 'replace', {
      image: { name: 'img-rgb.png', bytes: toArrayBuffer(image) },
    });
    assert.equal(reply.ok, true, JSON.stringify(reply));
    const saved = mupdf.PDFDocument.openDocument(savedBytes(session), 'application/pdf');
    const page1 = saved.loadPage(1).getObject();
    const im1 = page1.getInheritable('Resources').get('XObject').get('Im1');
    assert.equal(im1.isNull(), false, 'page 1 still resolves /Im1');
    assert.equal(im1.get('Subtype').asName(), 'Image');
  });
}

// ------------------------------------------------------------------ refusals

test('U4 refusals: a rotated Form, a masked image, a bad box, a bad image and a stale handle never write', async () => {
  const session = await openFile('fixtures/p3/edit-objects.pdf');
  const before = contentsOf(session, 0).join('\n');

  const rotated = scan(session).items.find(item => item.kind === 'form' && item.placement?.orient?.swap === true);
  const refusedMove = op(session, {
    op: 'edit-object', page: 0, targetId: rotated.targetId, generation: 0, revision: 0, action: 'move',
  });
  assert.equal(refusedMove.detail, 'stale-target', 'a bad handle is caught before the reason');

  const masked = target(session, 'Im2', 'delete');
  const replaceMasked = editObject(session, masked, 'replace', { image: { name: 'x.png', bytes: new ArrayBuffer(8) } });
  assert.equal(replaceMasked.ok, false);
  assert.equal(replaceMasked.detail, 'unsupported-image');

  const image = await readFile('fixtures/img-rgb.png');
  const move = target(session, 'Im1', 'move');
  for (const [box, detail] of [
    [[72, 650, 72, 746], 'out-of-bounds'],
    [[72, 650, 136, 650], 'out-of-bounds'],
    [[-10, 650, 136, 746], 'out-of-bounds'],
    [[700, 650, 780, 746], 'out-of-bounds'],
    [[72, 650, 136, 746 + 14400], 'too-large'],
    [[72, 650, 20000, 746], 'too-large'],
    [[72, 650], 'out-of-bounds'],
  ]) {
    const refused = editObject(session, move, 'move', { box });
    assert.equal(refused.ok, false, `${JSON.stringify(box)} must refuse`);
    assert.equal(refused.detail, detail, JSON.stringify(box));
  }

  const notImage = editObject(session, target(session, 'Im1', 'replace'), 'replace', {
    image: { name: 'x.png', bytes: new ArrayBuffer(8) },
  });
  assert.equal(notImage.detail, 'invalid-image');
  const huge = editObject(session, target(session, 'Im1', 'replace'), 'replace', {
    image: { name: 'huge.png', bytes: new ArrayBuffer(4 * 1024 * 1024 + 1) },
  });
  assert.equal(huge.detail, 'too-large');

  const moved = editObject(session, target(session, 'Im1', 'move'), 'move', { box: [72, 650, 136, 746] });
  assert.deepEqual([moved.ok, moved.noop, moved.revision, moved.dirty], [true, true, move.revision, false]);
  const real = editObject(session, target(session, 'Im1', 'move'), 'move', { box: [80, 650, 144, 746] });
  assert.equal(real.ok, true);
  const stale = op(session, { op: 'edit-object', page: 0, targetId: move.item.targetId, generation: move.generation, revision: move.revision, action: 'move', box: [72, 650, 136, 746] });
  assert.equal(stale.detail, 'stale-target');
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentsOf(session, 0).join('\n'), before);
  assert.ok(before.length > 0);
});

test('U4 refusals: a text target, a missing action and an encrypted document', async () => {
  const session = await openFile('fixtures/p3/edit-objects.pdf');
  const reply = scan(session);
  const text = reply.items.find(item => item.kind === 'text' && item.support === 'candidate');
  const asObject = op(session, {
    op: 'edit-object', page: 0, targetId: text.targetId,
    generation: reply.generation, revision: reply.revision, action: 'move', box: [0, 0, 10, 10],
  });
  assert.equal(asObject.detail, 'not-candidate');

  const object = target(session, 'Im1', 'move');
  const bogus = editObject(session, object, 'teleport');
  assert.equal(bogus.detail, 'not-candidate');

  const locked = await openFile('fixtures/encrypted-noedit.pdf');
  const lockedReply = op(locked, { op: 'edit-object', page: 0, targetId: 'x', generation: 0, revision: 0, action: 'delete' });
  assert.equal(lockedReply.ok, false);
  assert.equal(lockedReply.reason, 'read-only');
  assert.equal(lockedReply.detail, undefined, 'permission refusals keep the worker reason, not a detail');
});

test('U4 replace on p1: inherited resources are privatised, so p0 keeps its own', async () => {
  const session = await openFile('fixtures/p3/edit-objects.pdf');
  const page0 = contentsOf(session, 0).join('\n');
  const reply = editObject(session, target(session, 'Fm1', 'move', 1), 'move', { box: [100, 500, 220, 520] });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.ok(contentsOf(session, 1).join('\n').includes('1 0 0 1 100 500 cm /Fm1 Do Q'));
  assert.equal(contentsOf(session, 0).join('\n'), page0, 'the other page shares only the inherited resources, not the stream');
  assert.ok(session.doc.loadPage(1).getObject().get('Resources') !== null, 'the privatised page owns its resources');
});

// ------------------------------------------------------------------- numbers

test('U4 editNumber: ≤ 4 decimals, no exponent, -0 as 0, trailing zeros stripped', () => {
  assert.equal(editNumber(72), '72');
  assert.equal(editNumber(72.5), '72.5');
  assert.equal(editNumber(1 / 3), '0.3333');
  assert.equal(editNumber(5.333333333), '5.3333');
  assert.equal(editNumber(0.00004), '0');
  assert.equal(editNumber(-0), '0');
  assert.equal(editNumber(-0.0000001), '0');
  assert.equal(editNumber(1e-7 * 0), '0');
  assert.equal(editNumber(14400), '14400');
  assert.equal(/[eE]/.test(editNumber(1 / 3)), false);
});

test('U4 review: replace, undo, replace with the same image saves a resolvable image (mupdf image cache)', async () => {
  const session = await openFile('fixtures/p3/targets-streams.pdf');
  const image = await readFile('fixtures/img-rgb.png');
  const replace = () => editObject(session, target(session, 'Im1', 'replace'), 'replace', {
    image: { name: 'img-rgb.png', bytes: toArrayBuffer(image) },
  });
  assert.equal(replace().ok, true);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(replace().ok, true);
  const saved = mupdf.PDFDocument.openDocument(savedBytes(session), 'application/pdf');
  const entry = saved.loadPage(0).getObject().get('Resources').get('XObject').get('Im1r1');
  assert.equal(entry.isNull(), false, 'Im1r1 resolves after save');
  assert.equal(entry.get('Width').asNumber(), 300);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(op(session, { op: 'redo' }).ok, true);
});

test('U4 review: replace on a page with inherited /Resources, then undo, leaves the parent resources untouched', async () => {
  const session = await openFile('fixtures/p3/edit-objects.pdf');
  const image = await readFile('fixtures/img-rgb.png');
  const namesOf = page => {
    const names = [];
    session.doc.loadPage(page).getObject().getInheritable('Resources').get('XObject').forEach((_, key) => names.push(key));
    return names.sort().join(' ');
  };
  const before = [namesOf(0), namesOf(1)];
  assert.equal(editObject(session, target(session, 'Im1', 'replace', 1), 'replace', {
    image: { name: 'img-rgb.png', bytes: toArrayBuffer(image) },
  }).ok, true);
  // The privatised copy owns its /XObject dictionary: page 0 (which shares the parent's) never sees Im1r1.
  assert.equal(namesOf(0), before[0]);
  assert.ok(namesOf(1).includes('Im1r1'));
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual([namesOf(0), namesOf(1)], before);
  const saved = mupdf.PDFDocument.openDocument(savedBytes(session), 'application/pdf');
  for (const page of [0, 1]) {
    saved.loadPage(page).getObject().getInheritable('Resources').get('XObject')
      .forEach((value, key) => assert.equal(value.isNull(), false, `page ${page} /${key} resolves`));
  }
});
