/**
 * P2 unit 3 — page organisation: rotate, delete, move, all through MuPDF's own
 * journal. Runs against the app engine (tests/rust-engine.mjs) exactly as the worker does.
 *
 * The gate came first: the journal had to cover `/Rotate` put, `deletePage` and
 * `rearrangePages`, or the unit stopped (contract-unit3.md, Decisions). That gate
 * drove MuPDF's journal inside the removed TypeScript engine and went with it.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';
import { makeThreePagePdf } from '../fixtures/textpages.mjs';
import { makeTextPages } from '../fixtures/textpdf.mjs';
import { assertWithin } from './timing-budget.mjs';

const engine = createEngine(mupdf);
const HELLO = 'fixtures/hello.pdf';

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

/** The scratch: three pages whose text is one/two/three, built here, never committed. */
function threePage() {
  return toArrayBuffer(makeThreePagePdf());
}

async function openBytes(bytes, name = 'three.pdf', session = engine.createSession()) {
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: bytes }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

async function open(id, session = engine.createSession()) {
  return openBytes(toArrayBuffer(await readFile(`fixtures/${id}.pdf`)), `${id}.pdf`, session);
}

function op(session, request) {
  const reply = engine.dispatch({ id: 900 + (request.id ?? 0), ...request }, session).reply;
  return reply;
}

function rotationsOf(doc) {
  return Array.from({ length: doc.countPages() }, (_, index) => {
    const value = doc.asPDF().findPage(index).get('Rotate');
    return value.isNull() ? 0 : value.asNumber();
  });
}

/** The text of every page, in order: the order/identity check independent of page indices. */
function pageTexts(doc) {
  return Array.from({ length: doc.countPages() }, (_, index) => {
    const page = doc.loadPage(index);
    try {
      return page.toStructuredText().asText().trim();
    } finally {
      page.destroy();
    }
  });
}

function annotations(session, page) {
  const reply = engine.dispatch({ id: 990, op: 'annots', page }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return reply.items;
}

/** Reopens saved bytes in a throwaway document, the way an independent reader would. */
function reopen(bytes) {
  return mupdf.Document.openDocument(bytes, 'application/pdf');
}

// ------------------------------------------------------------------ ops

test('rotate: /Rotate is (current + by) mod 360, four rotations return to 0', async () => {
  const session = await openBytes(threePage());
  try {
    const first = op(session, { op: 'page-rotate', page: 0, by: 90 });
    assert.equal(first.ok, true, first.error);
    assert.deepEqual(first.rotations, [90, 0, 0]);
    assert.equal(first.pageCount, 3);
    assert.equal(first.revision, 1);
    assert.equal(first.dirty, true);
    assert.equal(first.canUndo, true);
    assert.equal(first.pages.length, 3, 'every page op reply carries the page list');

    let last = first;
    for (const by of [90, 90, 90]) last = op(session, { op: 'page-rotate', page: 0, by });
    assert.deepEqual(last.rotations, [0, 0, 0], 'four right rotations are back to 0');

    // A page that arrives with its own /Rotate starts from that value, not from 0.
    const declared = engine.createSession();
    const bytes = toArrayBuffer(makeThreePagePdf());
    const raw = mupdf.Document.openDocument(bytes, 'application/pdf');
    raw.asPDF().findPage(0).put('Rotate', raw.asPDF().newInteger(270));
    raw.asPDF().findPage(1).put('Rotate', raw.asPDF().newInteger(180));
    const declaredBytes = toArrayBuffer(raw.saveToBuffer({ compress: true, garbage: true }).asUint8Array());
    raw.destroy();
    await openBytes(declaredBytes, 'declared.pdf', declared);
    assert.deepEqual(op(declared, { op: 'page-rotate', page: 0, by: 90 }).rotations, [0, 180, 0]);
    assert.deepEqual(op(declared, { op: 'page-rotate', page: 1, by: -90 }).rotations, [0, 90, 0]);
    engine.closeDocument(declared.doc);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('rotated.pdf keeps its own /Rotate as the starting point', async () => {
  const session = await open('rotated');
  try {
    // rotated.pdf draws rotated glyphs but declares no /Rotate, so the first rotate is the first /Rotate.
    assert.deepEqual(rotationsOf(session.doc), [0]);
    assert.deepEqual(op(session, { op: 'page-rotate', page: 0, by: 90 }).rotations, [90]);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('a rotated page swaps its bounds and renders rotated, and a reply says so', async () => {
  const bytes = toArrayBuffer(makeTextPages([
    { width: 300, height: 150, runs: [{ font: { name: 'Helvetica', resource: 'F1' }, text: 'wide page', size: 18, x: 20, y: 80 }] },
  ]));
  const session = await openBytes(bytes, 'wide.pdf');
  try {
    const open = engine.dispatch({ id: 1, op: 'open', name: 'wide.pdf', pdf: bytes }, engine.createSession()).reply;
    assert.deepEqual([open.pages[0].width, open.pages[0].height], [300, 150]);

    const rotated = op(session, { op: 'page-rotate', page: 0, by: 90 });
    assert.deepEqual([rotated.pages[0].width, rotated.pages[0].height], [150, 300],
      'the reply reports the swapped size, not the original box');

    const render = engine.dispatch({ id: 5, op: 'render', page: 0, scale: 2 }, session).reply;
    assert.equal(render.ok, true, render.error);
    assert.equal(render.width, 150, 'the render itself uses the rotated page box');
    assert.equal(render.height, 300);
    assert.ok(render.nonWhite > 100, `rotated render is not blank (${render.nonWhite} non-white)`);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('a note added on a rotated page stays at the visible point after save and reopen', async () => {
  // A blank page: the only ink is the note, so the rendered ink box is the note.
  const bytes = toArrayBuffer(makeTextPages([{ width: 300, height: 600, runs: [] }]));
  const session = await openBytes(bytes, 'blank.pdf');
  try {
    assert.equal(op(session, { op: 'page-rotate', page: 0, by: 90 }).ok, true);
    const added = op(session, { op: 'annot-add', page: 0, type: 'Text', point: [12, 12], contents: 'top left' });
    assert.equal(added.ok, true, added.error);
    assert.deepEqual(added.pages, [{ width: 600, height: 300 }], 'the note add reply carries the page geometry too');

    const render = engine.dispatch({ id: 6, op: 'render', page: 0, scale: 1 }, session).reply;
    assert.equal(render.ok, true, render.error);
    const box = inkBox(render);
    assert.ok(box, 'the note is visible');
    assert.ok(box.x1 < render.pixelWidth / 2 && box.y1 < render.pixelHeight / 2,
      `the note stays at the visible top-left (ink box ${JSON.stringify(box)} in ${render.pixelWidth}x${render.pixelHeight})`);

    const save = engine.dispatch({ id: 7, op: 'save' }, session).reply;
    const check = engine.dispatch({ id: 8, op: 'validate', pdf: save.bytes }, session).reply;
    assert.equal(check.ok, true, check.error);
    const reopened = await openBytes(check.bytes, 'blank.pdf', engine.createSession());
    try {
      assert.deepEqual(rotationsOf(reopened.doc), [90], 'the saved copy keeps /Rotate');
      const items = annotations(reopened, 0);
      assert.equal(items.length, 1);
      assert.deepEqual(items[0].rect, [12, 12, 32, 32], 'the note sits at the visible top-left, y down');
    } finally {
      engine.closeDocument(reopened.doc);
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('delete removes the page and its annotations; undo brings both back', async () => {
  const session = await openBytes(threePage());
  try {
    assert.equal(op(session, { op: 'annot-add', page: 1, type: 'Text', point: [40, 40], contents: 'on two' }).ok, true);
    assert.deepEqual(annotations(session, 1).map(item => item.contents), ['on two']);

    const deleted = op(session, { op: 'page-delete', page: 1 });
    assert.equal(deleted.ok, true, deleted.error);
    assert.equal(deleted.pageCount, 2);
    assert.deepEqual(deleted.pages.length, 2);
    assert.equal(deleted.dirty, true);
    assert.deepEqual(annotations(session, 1).map(item => item.contents), [],
      'the annotations of the deleted page are gone from the index that replaced it');

    const undone = op(session, { op: 'undo' });
    assert.equal(undone.ok, true, undone.error);
    assert.equal(undone.step, 'delete page', 'the reply names the step it reverted');
    assert.equal(undone.pageCount, 3, 'undo restores the count');
    assert.deepEqual(annotations(session, 1).map(item => item.contents), ['on two'], 'and the annotations');

    const redone = op(session, { op: 'redo' });
    assert.equal(redone.step, 'delete page');
    assert.equal(redone.pageCount, 2);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('a moved page carries its annotations, and undo puts them back at the old index', async () => {
  const session = await openBytes(threePage());
  try {
    assert.equal(op(session, { op: 'annot-add', page: 0, type: 'Text', point: [30, 30], contents: 'on one' }).ok, true);

    const moved = op(session, { op: 'page-move', page: 0, to: 2 });
    assert.equal(moved.ok, true, moved.error);
    assert.equal(moved.step, undefined, 'a mutation itself has no step to report');
    assert.equal(moved.pageCount, 3);
    assert.deepEqual(annotations(session, 0), [], 'the note is not at the old index');
    assert.deepEqual(annotations(session, 2).map(item => item.contents), ['on one'], 'it follows the page to index 2');

    const undone = op(session, { op: 'undo' });
    assert.equal(undone.step, 'move page');
    assert.deepEqual(annotations(session, 0).map(item => item.contents), ['on one'], 'undo returns it to index 0');
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('rotate, move and delete refuse what they cannot do, without mutating', async () => {
  const session = await openBytes(threePage());
  try {
    assert.deepEqual(op(session, { op: 'page-rotate', page: 0, by: 45 }).error, 'page-rotate: by must be 90 or -90');
    assert.match(op(session, { op: 'page-move', page: 1, to: 1 }).error, /already there/);
    assert.match(op(session, { op: 'page-move', page: 1, to: 3 }).error, /0\.\.2/);
    assert.match(op(session, { op: 'page-move', page: 1, to: 'x' }).error, /0\.\.2/);
    assert.match(op(session, { op: 'page-rotate', page: 9, by: 90 }).error, /0\.\.2/);
    assert.match(op(session, { op: 'page-delete', page: -1 }).error, /0\.\.2/);
    const state = engine.dispatch({ id: 9, op: 'undo' }, session).reply;
    assert.equal(state.ok, false, 'a refused op never dirties the document');
    assert.match(state.error, /nothing to undo/);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('delete refuses the last page and always keeps one', async () => {
  const single = await open('hello');
  try {
    assert.equal(single.doc.countPages(), 1);
    const refused = op(single, { op: 'page-delete', page: 0 });
    assert.equal(refused.ok, false);
    assert.match(refused.error, /one-page document/);
    assert.equal(single.doc.countPages(), 1);
    assert.equal(engine.dispatch({ id: 9, op: 'undo' }, single).reply.ok, false, 'and it did not dirty the document');
  } finally {
    engine.closeDocument(single.doc);
  }

  const session = await openBytes(threePage());
  try {
    assert.equal(op(session, { op: 'page-delete', page: 0 }).ok, true);
    assert.equal(op(session, { op: 'page-delete', page: 0 }).ok, true);
    assert.equal(session.doc.countPages(), 1, 'two deletes leave exactly one page');
    assert.equal(op(session, { op: 'page-delete', page: 0 }).ok, false, 'and the last page cannot go');
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('page ops refuse a signed or certified document, and work on an unlocked encrypted one', async () => {
  const pageOps = [
    { op: 'page-rotate', page: 0, by: 90 },
    { op: 'page-delete', page: 0 },
    { op: 'page-move', page: 0, to: 1 },
  ];

  // AES-256 with an empty user password is unlocked and grants every permission
  // (P2 unit 10), so each op works and the copy keeps its encryption.
  const plain = mupdf.Document.openDocument(threePage(), 'application/pdf').asPDF();
  const buffer = plain.saveToBuffer('incremental=no,encrypt=aes-256,user-password=,owner-password=owner');
  const encryptedBytes = toArrayBuffer(buffer.asUint8Array());
  buffer.destroy();
  plain.destroy();
  for (const request of pageOps) {
    const session = engine.createSession();
    assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'empty-password.pdf', pdf: encryptedBytes }, session).reply.ok, true);
    const reply = op(session, request);
    assert.equal(reply.ok, true, `${request.op}: ${reply.error}`);
    engine.closeDocument(session.doc);
  }
  const encrypted = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'empty-password.pdf', pdf: encryptedBytes }, encrypted).reply.ok, true);
  assert.equal(op(encrypted, { op: 'page-rotate', page: 0, by: 90 }).ok, true);
  const kept = engine.dispatch({ id: 2, op: 'save' }, encrypted).reply;
  assert.equal(kept.ok, true, kept.error);
  assert.equal(kept.security.algorithm, 'aes-256', 'the saved copy keeps its encryption');
  engine.closeDocument(encrypted.doc);

  // A signed and a certified document: the same refusal, before any page walk.
  for (const [label, build] of [['signed', signedPdf], ['certified', certifiedPdf]]) {
    const session = engine.createSession();
    assert.equal(engine.dispatch({ id: 1, op: 'open', name: `${label}.pdf`, pdf: build() }, session).reply.ok, true);
    for (const request of pageOps) {
      const refused = op(session, request);
      assert.equal(refused.ok, false, `${label} ${request.op}`);
      assert.equal(refused.reason, 'read-only', `${label} ${request.op}`);
    }
    engine.closeDocument(session.doc);
  }
});

test('a rotated and rearranged document round-trips through save, validate and reopen', async () => {
  const session = await openBytes(threePage());
  try {
    // rotate page 1, move page 3 up twice, delete the last page: exactly the smoke script.
    assert.equal(op(session, { op: 'page-rotate', page: 0, by: 90 }).ok, true);
    assert.equal(op(session, { op: 'page-move', page: 2, to: 1 }).ok, true);
    assert.equal(op(session, { op: 'page-move', page: 1, to: 0 }).ok, true);
    assert.equal(op(session, { op: 'annot-add', page: 1, type: 'Text', point: [50, 50], contents: 'moved with page one' }).ok, true);
    const deleted = op(session, { op: 'page-delete', page: 2 });
    assert.equal(deleted.ok, true, deleted.error);
    assert.equal(deleted.pageCount, 2);
    assert.deepEqual(deleted.rotations, [0, 90]);

    const save = engine.dispatch({ id: 20, op: 'save' }, session).reply;
    assert.equal(save.ok, true, save.error);
    const check = engine.dispatch({ id: 21, op: 'validate', pdf: save.bytes }, session).reply;
    assert.equal(check.ok, true, check.error);
    assert.equal(check.pageCount, 2, 'validate agrees on the count');

    const re = reopen(check.bytes);
    try {
      assert.equal(re.countPages(), 2);
      assert.deepEqual(pageTexts(re), ['three', 'one'], 'the saved order, as text per page');
      assert.deepEqual(rotationsOf(re), [0, 90], 'and the /Rotate values');
      const items = re.loadPage(1).getAnnotations().map(item => item.getContents());
      assert.deepEqual(items, ['moved with page one'], 'the note moved with its page');
    } finally {
      re.destroy();
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('large.pdf: 500-page page ops stay inside the reply budget', async () => {
  const session = await open('large');
  try {
    assert.equal(session.doc.countPages(), 500);

    // The budget in the contract is for the `pages` slice itself (500 loadPage+getBounds
    // plus 500 /Rotate reads). An annotation mutation replies with the same slice.
    const walkStarted = performance.now();
    const walk = op(session, { op: 'annot-add', page: 0, type: 'Text', point: [10, 10], contents: 'timing' });
    const pageListMs = performance.now() - walkStarted;
    assert.equal(walk.ok, true, walk.error);
    assert.equal(walk.pages.length, 500);
    assert.equal(walk.rotations.length, 500);
    assertWithin(pageListMs, 50, 'a 500-page page list');

    const timings = [];
    for (const request of [
      { op: 'page-rotate', page: 0, by: 90 },
      { op: 'page-delete', page: 499 },
      { op: 'page-move', page: 2, to: 0 },
    ]) {
      const started = performance.now();
      const reply = op(session, request);
      const ms = performance.now() - started;
      assert.equal(reply.ok, true, reply.error);
      assert.ok(reply.pages.length >= 498, `${request.op}: every page has a size`);
      assert.equal(reply.pages.length, reply.pageCount);
      assert.equal(reply.rotations.length, reply.pageCount);
      timings.push({ op: request.op, ms: Math.round(ms * 10) / 10, count: reply.pageCount, replyBytes: JSON.stringify(reply).length });
    }
    console.log(`U3_LARGE ${JSON.stringify({ pageListMs: Math.round(pageListMs * 10) / 10, ops: timings })}`);
    // The page list is bounded; the mutation itself (rearrangePages over 499 pages) is
    // measured and reported, and is 30-100 ms warm - not a reply-cost regression.
    for (const item of timings) assertWithin(item.ms, 500, `a 500-page ${item.op}`);
    assert.ok(timings.at(-1).replyBytes < 20_000, 'the page list stays small enough for one IPC message');
  } finally {
    engine.closeDocument(session.doc);
  }
});

/** The rendered ink bounding box in pixels, or null for a blank render. */
function inkBox(render) {
  const pixels = new Uint8ClampedArray(render.pixels);
  let x0 = render.pixelWidth, y0 = render.pixelHeight, x1 = -1, y1 = -1;
  for (let y = 0; y < render.pixelHeight; y++) {
    for (let x = 0; x < render.pixelWidth; x++) {
      const index = (y * render.pixelWidth + x) * 4;
      if (pixels[index] < 250 || pixels[index + 1] < 250 || pixels[index + 2] < 250) {
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
    }
  }
  return x1 < 0 ? null : { x0, y0, x1, y1 };
}

/** Object-numbering-safe PDF from a 1-based list of indirect object bodies. */
function buildPdf(objects) {
  const parts = ['%PDF-1.4\n'];
  const offsets = [];
  objects.forEach((body, index) => {
    offsets.push(parts.join('').length);
    parts.push(`${index + 1} 0 obj\n${body}\nendobj\n`);
  });
  const xref = parts.join('').length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const offset of offsets) parts.push(`${String(offset).padStart(10, '0')} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  return toArrayBuffer(Buffer.from(parts.join(''), 'latin1'));
}

function signedPdf() {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>',
    '<< /Length 2 >>\nstream\nq \nendstream',
    '<< /Fields [6 0 R] /SigFlags 3 >>',
    '<< /Type /Annot /Subtype /Widget /FT /Sig /T (sig1) /Rect [72 72 300 120] /P 3 0 R /V 7 0 R >>',
    '<< /Type /Sig /Filter /Adobe.PPKLite >>',
  ]);
}

function certifiedPdf() {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /Perms << /DocMDP 3 0 R >> >>',
    '<< /Type /Pages /Kids [4 0 R] /Count 1 >>',
    '<< /Type /Sig /Filter /Adobe.PPKLite >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>',
    '<< /Length 2 >>\nstream\nq \nendstream',
  ]);
}

test('every page-op reply keeps the journal fields the renderer mirrors', async () => {
  const session = await openBytes(threePage());
  try {
    const replies = [
      op(session, { op: 'page-rotate', page: 0, by: 90 }),
      op(session, { op: 'page-move', page: 0, to: 1 }),
      op(session, { op: 'page-delete', page: 0 }),
      op(session, { op: 'undo' }),
      op(session, { op: 'redo' }),
      op(session, { op: 'annot-add', page: 0, type: 'Text', point: [10, 10], contents: 'x' }),
      op(session, { op: 'undo' }),
    ];
    for (const reply of replies) {
      assert.equal(reply.ok, true, reply.error);
      for (const key of ['revision', 'dirty', 'canUndo', 'canRedo', 'pageCount', 'pages', 'rotations']) {
        assert.ok(reply[key] !== undefined, `${key} missing from ${JSON.stringify(reply).slice(0, 80)}`);
      }
      assert.equal(reply.pages.length, reply.pageCount);
      assert.equal(reply.rotations.length, reply.pageCount);
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});
