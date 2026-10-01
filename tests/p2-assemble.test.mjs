/**
 * P2 unit 6 — assemble: insert pages from another PDF, extract pages to a new one.
 * Runs against src/renderer/engine.ts exactly as the worker does.
 *
 * The gate came first (contract-unit6.md, Decisions): `graftPage` must be journaled
 * and one undo must remove every inserted page. It is — and the measurement also
 * showed that `graftPage` copies the page box, rotation, resources and contents but
 * **drops `/Annots`** (with `/AA`, `/Dur`, `/StructParents`), so the engine grafts
 * each source page's annotations separately through the same graft map. Without that,
 * an inserted page arrives with no notes, no links and no form widgets.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine, MAX_GRAFT_PAGES } from '../build/renderer/engine.js';

const engine = createEngine(mupdf);

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

/** A Buffer, a Uint8Array or an ArrayBuffer as the worker seam's ArrayBuffer. */
function asBytes(value) {
  return value instanceof ArrayBuffer ? value : toArrayBuffer(value);
}

async function openBytes(bytes, name = 'scratch.pdf', session = engine.createSession()) {
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: bytes }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

async function open(id, session = engine.createSession()) {
  return openBytes(toArrayBuffer(await readFile(`fixtures/${id}.pdf`)), `${id}.pdf`, session);
}

function source(id) {
  return readFile(`fixtures/${id}.pdf`);
}

function op(session, request) {
  return engine.dispatch({ id: 900 + (request.id ?? 0), ...request }, session).reply;
}

function insert(session, bytes, at, extra = {}) {
  return op(session, { op: 'insert-pages', pdf: asBytes(bytes), name: 'source.pdf', at, ...extra });
}

function texts(doc) {
  return Array.from({ length: doc.countPages() }, (_, page) => {
    const loaded = doc.loadPage(page);
    try {
      return loaded.toStructuredText().asText().trim();
    } finally {
      loaded.destroy();
    }
  });
}

function reopen(bytes) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  doc.asPDF().disableJS();
  return doc;
}

/** The top-level `/AcroForm /Fields` names, in order. */
function fieldNames(doc) {
  const fields = doc.asPDF().getTrailer().get('Root').get('AcroForm').get('Fields');
  const names = [];
  for (let index = 0; index < fields.length; index++) {
    names.push(fields.get(index).get('T').asString());
  }
  return names;
}

/** Every widget in the file, with whether it carries an appearance. */
function widgets(doc) {
  const found = [];
  for (let page = 0; page < doc.countPages(); page++) {
    const loaded = doc.loadPage(page);
    try {
      for (const widget of loaded.getWidgets()) {
        const dict = widget.getObject();
        let ap = null;
        try { ap = dict.get('AP'); } catch { /* none */ }
        found.push({ name: widget.getName(), ap: !!ap && !ap.isNull() });
      }
    } finally {
      loaded.destroy();
    }
  }
  return found;
}

/** AES-256 with an empty user password: it opens with no prompt but is encrypted. */
function encryptedBytes(bytes) {
  const plain = mupdf.Document.openDocument(toArrayBuffer(bytes), 'application/pdf').asPDF();
  const buffer = plain.saveToBuffer('incremental=no,encrypt=aes-256,user-password=,owner-password=owner');
  const encrypted = toArrayBuffer(buffer.asUint8Array());
  buffer.destroy();
  plain.destroy();
  return encrypted;
}

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

// ------------------------------------------------------------------- gate

test('gate: graftPage is journaled, one undo removes every inserted page, and it drops /Annots', async () => {
  const target = mupdf.Document.openDocument(toArrayBuffer(await source('hello')), 'application/pdf');
  const pdf = target.asPDF();
  try {
    pdf.enableJournal();
    assert.equal(pdf.getJournal().position, 0);
    const sourceDoc = new mupdf.PDFDocument(toArrayBuffer(await source('nav')));
    const sourcePage = sourceDoc.asPDF().findPage(0);
    const sourceAnnots = sourcePage.get('Annots');
    assert.ok(sourceAnnots.isArray() && sourceAnnots.length > 0, 'nav.pdf page 1 has links to bring across');

    pdf.beginOperation('insert pages');
    pdf.graftPage(1, sourceDoc, 0);
    pdf.graftPage(2, sourceDoc, 1);
    pdf.endOperation();
    assert.equal(target.countPages(), 3);
    assert.equal(pdf.getJournal().position, 1, 'both grafts are one journal step');
    assert.equal(pdf.getJournal().steps[0], 'insert pages');

    const grafted = pdf.findPage(1);
    const copied = grafted.get('Annots');
    assert.ok(!copied || copied.isNull(), 'graftPage drops /Annots: the raw copy has none');
    assert.deepEqual(target.loadPage(1).getLinks(), [], 'so the links are gone without the repair');

    pdf.undo();
    assert.equal(target.countPages(), 1, 'one undo removes every inserted page');
    pdf.redo();
    assert.equal(target.countPages(), 3);
    sourceDoc.destroy();
  } finally {
    target.destroy();
  }
});

// ----------------------------------------------------------------- insert

test('insert hello.pdf into nav.pdf at 0, after page 1 and at the end, with undo/redo', async () => {
  const hello = await source('hello');

  for (const [label, at, expected] of [
    ['before the first page', 0, ['PDF4All', 'one', 'two', 'three']],
    ['after page 1', 1, ['one', 'PDF4All', 'two', 'three']],
    ['at the end', 3, ['one', 'two', 'three', 'PDF4All']],
  ]) {
    const session = await open('nav');
    try {
      assert.equal(at <= session.doc.countPages(), true, `${label}: the target has room`);
      const inserted = insert(session, hello, at);
      assert.equal(inserted.ok, true, inserted.error);
      assert.equal(inserted.pageCount, 4, label);
      assert.equal(inserted.inserted, 1, label);
      assert.deepEqual(inserted.pages.length, 4);
      assert.deepEqual(texts(session.doc), expected, label);

      const undone = op(session, { op: 'undo' });
      assert.equal(undone.pageCount, 3, `${label}: undo`);
      assert.deepEqual(texts(session.doc), ['one', 'two', 'three']);
      const redone = op(session, { op: 'redo' });
      assert.equal(redone.pageCount, 4, `${label}: redo`);
      assert.deepEqual(texts(session.doc), expected);
    } finally {
      engine.closeDocument(session.doc);
    }
  }
});

/** A 300x600 page with `/Rotate 90` and a Text note: the page-level data graftPage keeps. */
function rotatedNotePdf() {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 600] /Rotate 90 /Annots [4 0 R] >>',
    '<< /Type /Annot /Subtype /Text /Rect [12 12 32 32] /Contents (rotated note) >>',
  ]);
}

test('a grafted page keeps its rotation and box, and the annotation repair brings notes and links back', async () => {
  // Build a source page that has a Text note, through the engine itself.
  const noteSession = await open('hello');
  let annotated;
  try {
    assert.equal(op(noteSession, { op: 'annot-add', page: 0, type: 'Text', point: [30, 30], contents: 'grafted note' }).ok, true);
    const save = engine.dispatch({ id: 20, op: 'save' }, noteSession).reply;
    assert.equal(save.ok, true, save.error);
    annotated = save.bytes;
  } finally {
    engine.closeDocument(noteSession.doc);
  }

  const session = await open('nav');
  try {
    // A rotated page: the reply (and every later render) reports the rotated box.
    const rotated = insert(session, rotatedNotePdf(), 0);
    assert.equal(rotated.ok, true, rotated.error);
    assert.equal(rotated.rotations[0], 90, 'the source page\'s /Rotate comes across');
    assert.deepEqual(rotated.pages[0], { width: 600, height: 300 }, 'and its bounds are the rotated ones');
    assert.deepEqual(op(session, { op: 'annots', page: 0 }).items.map(item => item.contents), ['rotated note']);
    op(session, { op: 'undo' });

    const inserted = insert(session, annotated, 1);
    assert.equal(inserted.ok, true, inserted.error);
    assert.equal(inserted.pageCount, 4);
    // The order is nav's page 1, the inserted hello page, then nav's other two pages.
    assert.deepEqual(op(session, { op: 'links', page: 0 }).items.map(item => item.kind), ['page', 'uri']);
    assert.deepEqual(op(session, { op: 'links', page: 3 }).items.map(item => item.kind), ['page', 'blocked']);
    // The inserted hello page carries its note.
    assert.deepEqual(op(session, { op: 'annots', page: 1 }).items.map(item => item.contents), ['grafted note']);

    const save = engine.dispatch({ id: 21, op: 'save' }, session).reply;
    assert.equal(save.ok, true, save.error);
    const check = engine.dispatch({ id: 22, op: 'validate', pdf: save.bytes }, session).reply;
    assert.equal(check.ok, true, check.error);
    const re = reopen(check.bytes);
    try {
      assert.equal(re.countPages(), 4);
      assert.deepEqual(texts(re).map(text => text.split('\n')[0]), ['one', 'PDF4All', 'two', 'three']);
      // nav page 1's GoTo named nav page 2, which was not part of this insert: mupdf
      // leaves that destination to repoint at whatever page it can still find, so only
      // the link's presence is asserted here (the stated limit, measured in p2-assemble.md).
      const graftedLinks = re.loadPage(0).getLinks();
      assert.equal(graftedLinks.length, 2, 'both links come across');
      assert.equal(graftedLinks[1].getURI(), 'https://example.com/');
      assert.ok(re.resolveLink(graftedLinks[0]) >= 0, 'the GoTo still resolves to a page');
      const note = re.loadPage(1).getAnnotations().filter(item => item.getType() === 'Text').map(item => item.getContents());
      assert.deepEqual(note, ['grafted note'], 'the note survives save, validate and reopen');
    } finally {
      re.destroy();
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('insert form2 into form2: 18 widgets, 18 unique top-level names, appearances kept', async () => {
  const form2 = await source('form2');
  const session = await open('form2');
  try {
    const before = widgets(session.doc);
    assert.equal(before.length, 11, 'the source page has 11 widgets');
    const withAppearance = before.filter(widget => widget.ap).length;
    assert.ok(withAppearance > 0, 'some widgets carry an appearance');

    const inserted = insert(session, form2, 1);
    assert.equal(inserted.ok, true, inserted.error);
    assert.equal(inserted.inserted, 1);
    assert.equal(inserted.pageCount, 2);
    assert.equal(inserted.fieldNames.length, 18);
    assert.equal(new Set(inserted.fieldNames).size, 18, 'no two top-level fields share a /T');
    assert.deepEqual(inserted.fieldNames.slice(9), [
      'name (2)', 'notes (2)', 'agree (2)', 'size (2)', 'choice (2)', 'pick (2)', 'fixed (2)', 'amount (2)', 'sig (2)',
    ], 'only the grafted copies are renamed');
    assert.deepEqual(fieldNames(session.doc), inserted.fieldNames, 'the reply is what the document now holds');

    const all = widgets(session.doc);
    assert.equal(all.length, 22, '11 original + 11 grafted');
    assert.equal(all.filter(widget => widget.ap).length, withAppearance * 2,
      '/AP is copied verbatim: mupdf does not synthesize an appearance for a widget that has none');
    assert.deepEqual(op(session, { op: 'fields', page: 1 }).items.map(item => item.name),
      ['name (2)', 'notes (2)', 'agree (2)', 'size (2)', 'size (2)', 'size (2)', 'choice (2)', 'pick (2)', 'fixed (2)', 'amount (2)', 'sig (2)']);

    const save = engine.dispatch({ id: 20, op: 'save' }, session).reply;
    assert.equal(save.ok, true, save.error);
    const check = engine.dispatch({ id: 21, op: 'validate', pdf: save.bytes }, session).reply;
    assert.equal(check.ok, true, check.error);
    assert.equal(check.pageCount, 2);
    const re = reopen(check.bytes);
    try {
      assert.deepEqual(fieldNames(re), inserted.fieldNames);
    } finally {
      re.destroy();
    }

    const undone = op(session, { op: 'undo' });
    assert.equal(undone.pageCount, 1);
    assert.deepEqual(fieldNames(session.doc), inserted.fieldNames.slice(0, 9), 'undo takes the grafted fields with the page');
    assert.equal(widgets(session.doc).length, 11);
    assert.equal(op(session, { op: 'redo' }).pageCount, 2);
    assert.equal(fieldNames(session.doc).length, 18);
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ---------------------------------------------------------------- extract

test('extract [1] and [2, 0] from nav.pdf: order, text, links, and the open document untouched', async () => {
  const session = await open('nav');
  try {
    const before = engine.dispatch({ id: 5, op: 'save' }, session).reply;
    const revision = session.revision;

    const one = op(session, { op: 'extract-pages', pages: [1] });
    assert.equal(one.ok, true, one.error);
    assert.equal(one.pageCount, 1);
    assert.equal(one.dirty, undefined, 'an extract is not a mutation, so it carries no journal state');
    const first = reopen(one.bytes);
    try {
      assert.deepEqual(texts(first), ['two']);
      assert.deepEqual(first.loadPage(0).getLinks(), [], 'the middle page has no links');
    } finally {
      first.destroy();
    }

    const two = op(session, { op: 'extract-pages', pages: [2, 0] });
    assert.equal(two.ok, true, two.error);
    assert.equal(two.pageCount, 2);
    const second = reopen(two.bytes);
    try {
      assert.deepEqual(texts(second), ['three', 'one'], 'the requested order is kept');
      assert.deepEqual(second.loadPage(0).getLinks().map(link => link.getURI()),
        ['#page=1&view=Fit', 'file:///etc/hosts'], 'the blocked link is copied as it is, never followed');
      assert.deepEqual(second.loadPage(1).getLinks().map(link => link.getURI()), ['#page=1&view=Fit', 'https://example.com/']);
    } finally {
      second.destroy();
    }

    assert.equal(session.doc.countPages(), 3, 'the open document is untouched');
    assert.equal(session.revision, revision, 'and its revision never moved');
    const after = engine.dispatch({ id: 6, op: 'save' }, session).reply;
    assert.equal(after.dirty, false, 'still not dirty');
    assert.equal(after.bytes.byteLength, before.bytes.byteLength, 'and its bytes are unchanged');
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('extract keeps a form: the widgets and their fields come across', async () => {
  const session = await open('form2');
  try {
    const one = op(session, { op: 'extract-pages', pages: [0] });
    assert.equal(one.ok, true, one.error);
    const out = reopen(one.bytes);
    try {
      assert.equal(out.countPages(), 1);
      assert.deepEqual(fieldNames(out), ['name', 'notes', 'agree', 'size', 'choice', 'pick', 'fixed', 'amount', 'sig']);
      assert.equal(widgets(out).length, 11, 'every widget is on the extracted page and registered');
      assert.deepEqual(op(session, { op: 'fields', page: 0 }).items.length, 11, 'the open document is unchanged');
    } finally {
      out.destroy();
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

// --------------------------------------------------------------- refusals

test('insert and extract refuse what they cannot do, without dirtying anything', async () => {
  const nav = await source('nav');
  const session = await open('nav');
  try {
    const refusals = [
      [insert(session, nav, 4), /at must be an integer in 0\.\.3/],
      [insert(session, nav, -1), /at must be an integer in 0\.\.3/],
      [insert(session, nav, 1.5), /at must be an integer in 0\.\.3/],
      [insert(session, nav, 0, { pages: [] }), /1\.\.2000/],
      [insert(session, nav, 0, { pages: [3] }), /page 0 must be an integer in 0\.\.2/],
      [insert(session, nav, 0, { pages: Array.from({ length: MAX_GRAFT_PAGES + 1 }, () => 0) }), /1\.\.2000/],
      [insert(session, nav, 0, { range: 'nonsense' }), /must look like/],
      [insert(session, new Uint8Array(10), 0), /could not be read/],
      [insert(session, nav, 0, { name: '' }), /missing source name/],
      [op(session, { op: 'extract-pages', pages: [] }), /1\.\.2000/],
      [op(session, { op: 'extract-pages', pages: [3] }), /page 0 must be an integer in 0\.\.2/],
      [op(session, { op: 'extract-pages', pages: ['x'] }), /page 0 must be an integer in 0\.\.2/],
    ];
    for (const [reply, pattern] of refusals) {
      assert.equal(reply.ok, false, JSON.stringify(reply));
      assert.match(reply.error, pattern);
    }
    assert.equal(op(session, { op: 'undo' }).ok, false, 'nothing dirtied the document');
    assert.equal(session.doc.countPages(), 3);

    // A range inserts only the pages it names, and `2-3` is the open-ended form.
    const ranged = insert(session, nav, 0, { range: '2-3' });
    assert.equal(ranged.ok, true, ranged.error);
    assert.equal(ranged.inserted, 2);
    assert.deepEqual(texts(session.doc), ['two', 'three', 'one', 'two', 'three']);
    op(session, { op: 'undo' });

    // An encrypted source (empty user password opens it, permissions still apply)
    assert.equal(insert(session, encryptedBytes(await source('nav')), 0).ok, false);
    assert.match(insert(session, encryptedBytes(await source('nav')), 0).error, /encrypted source/);
    // and one that needs a password.
    const secret = mupdf.Document.openDocument(toArrayBuffer(await source('nav')), 'application/pdf').asPDF()
      .saveToBuffer('incremental=no,encrypt=aes-256,user-password=secret,owner-password=owner');
    const locked = toArrayBuffer(secret.asUint8Array());
    secret.destroy();
    const lockedReply = insert(session, locked, 0);
    assert.equal(lockedReply.ok, false);
    assert.match(lockedReply.error, /needs a password/);
    assert.equal(op(session, { op: 'undo' }).ok, false);
  } finally {
    engine.closeDocument(session.doc);
  }

  // A signed target refuses an insert, and allows an extract.
  const signed = await openBytes(signedPdf(), 'signed.pdf');
  try {
    const refused = insert(signed, nav, 0);
    assert.equal(refused.ok, false);
    assert.equal(refused.reason, 'read-only');
    const extracted = op(signed, { op: 'extract-pages', pages: [0] });
    assert.equal(extracted.ok, true, extracted.error);
    assert.equal(extracted.pageCount, 1);
  } finally {
    engine.closeDocument(signed.doc);
  }

  // An encrypted (empty-password) target is unlocked and grants every permission,
  // so the insert is allowed now (P2 unit 10); extraction still works too.
  const encryptedTarget = await openBytes(encryptedBytes(await source('nav')), 'encrypted.pdf');
  try {
    const inserted = insert(encryptedTarget, nav, 0);
    assert.equal(inserted.ok, true, inserted.error);
    const extracted = op(encryptedTarget, { op: 'extract-pages', pages: [0, 2] });
    assert.equal(extracted.ok, true, extracted.error);
    assert.equal(extracted.pageCount, 2);
  } finally {
    engine.closeDocument(encryptedTarget.doc);
  }
});

// -------------------------------------------------------------------- cost

test('large.pdf: insert one page and extract twenty stay inside the budget', async () => {
  const session = await open('large');
  try {
    assert.equal(session.doc.countPages(), 500);
    const nav = await source('nav');

    const insertStarted = performance.now();
    const inserted = insert(session, nav, 250, { pages: [0] });
    const insertMs = performance.now() - insertStarted;
    assert.equal(inserted.ok, true, inserted.error);
    assert.equal(inserted.inserted, 1);
    assert.equal(inserted.pageCount, 501);

    const extractStarted = performance.now();
    const extracted = op(session, { op: 'extract-pages', pages: Array.from({ length: 20 }, (_, index) => index * 25) });
    const extractMs = performance.now() - extractStarted;
    assert.equal(extracted.ok, true, extracted.error);
    assert.equal(extracted.pageCount, 20);

    console.log(`U6_LARGE ${JSON.stringify({
      insertMs: Math.round(insertMs * 10) / 10,
      extractMs: Math.round(extractMs * 10) / 10,
      extractBytes: extracted.bytes.byteLength,
    })}`);
    assert.ok(insertMs < 200, `inserting one page into 500 must not take seconds, not ${insertMs.toFixed(1)} ms`);
    assert.ok(extractMs < 500, `extracting 20 pages must not take seconds, not ${extractMs.toFixed(1)} ms`);
  } finally {
    engine.closeDocument(session.doc);
  }
});
