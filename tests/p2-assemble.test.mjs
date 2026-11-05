/**
 * P2 unit 6 — assemble: insert pages from another PDF, extract pages to a new one.
 * Runs against the app engine (tests/rust-engine.mjs) exactly as the worker does.
 *
 * The gate came first (contract-unit6.md, Decisions): an insert must be journaled
 * and one undo must remove every inserted page. The MuPDF `graftPage` an earlier engine
 * used was measured then to copy the page box, rotation, resources and contents but
 * **drop `/Annots`** (with `/AA`, `/Dur`, `/StructParents`), so the engine grafts
 * each source page's annotations separately through the same graft map. Without that,
 * an inserted page arrives with no notes, no links and no form widgets.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createEngine, fieldName, List, MAX_GRAFT_PAGES } from './rust-engine.mjs';
import { createHash } from 'node:crypto';
import { assertWithin } from './timing-budget.mjs';

const engine = createEngine();

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
  return Array.from({ length: doc.pageCount }, (_, page) => doc.text(page).trim());
}

function reopen(bytes) {
  return engine.reopen(bytes);
}

/** The top-level `/AcroForm /Fields` names, in order. */
function fieldNames(doc) {
  const fields = doc.root.get('AcroForm').get('Fields');
  const names = [];
  for (let index = 0; index < fields.length; index++) {
    names.push(fields.get(index).string('T'));
  }
  return names;
}

/** Every widget in the file, with whether it carries an appearance. */
function widgets(doc) {
  const found = [];
  for (let page = 0; page < doc.pageCount; page++) {
    for (const widget of doc.widgets(page)) {
      found.push({ name: fieldName(widget), ap: widget.get('AP') !== null });
    }
  }
  return found;
}

/**
 * A page's links as the file holds them: `uri` from `/A /URI`, `page` the index of the
 * page a GoTo's (or `/Dest`'s) explicit destination names, else null.
 */
function linksOf(doc, page) {
  return doc.annotsOf(page, 'Link').map(link => {
    const action = link.dict('A');
    const dest = action?.get('D') ?? link.get('Dest');
    const target = dest instanceof List ? dest.ref(0) : null;
    const index = target ? doc.pageIds.findIndex(id => id && id[0] === target[0] && id[1] === target[1]) : -1;
    return { uri: action?.string('URI') ?? null, page: index >= 0 ? index : null };
  });
}

/** RC4, for the one stream the empty-password fixture encrypts. */
function rc4(key, data) {
  const box = Array.from({ length: 256 }, (_, index) => index);
  for (let i = 0, j = 0; i < 256; i++) {
    j = (j + box[i] + key[i % key.length]) & 255;
    [box[i], box[j]] = [box[j], box[i]];
  }
  let i = 0;
  let j = 0;
  return Buffer.from(data.map(byte => {
    i = (i + 1) & 255;
    j = (j + box[i]) & 255;
    [box[i], box[j]] = [box[j], box[i]];
    return byte ^ box[(box[i] + box[j]) & 255];
  }));
}

/**
 * An encrypted document with an empty user password (RC4-40, V1 R2, every permission):
 * it opens with no prompt but is encrypted. `pages` pages, each with one line of text.
 * The engine writes encryption only with a non-empty user password, so this is written by hand.
 */
function encryptedBytes(pages = 3) {
  const pad = Buffer.from('28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A', 'hex');
  const id = Buffer.alloc(16, 7);
  const owner = Buffer.alloc(32, 1); // only an owner password check reads /O
  const p = -4;
  const permissions = Buffer.alloc(4);
  permissions.writeInt32LE(p);
  const key = createHash('md5').update(Buffer.concat([pad, owner, permissions, id])).digest().subarray(0, 5);
  const user = rc4([...key], [...pad]);
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [${Array.from({ length: pages }, (_, index) => `${4 + index * 2} 0 R`).join(' ')}] /Count ${pages} >>`,
    `<< /Filter /Standard /V 1 /R 2 /O <${owner.toString('hex')}> /U <${user.toString('hex')}> /P ${p} >>`,
  ];
  for (let index = 0; index < pages; index++) {
    const number = 5 + index * 2;
    const content = `BT /F1 12 Tf 72 720 Td (page ${index + 1}) Tj ET\n`;
    const objectKey = createHash('md5')
      .update(Buffer.concat([key, Buffer.from([number & 255, (number >> 8) & 255, number >> 16, 0, 0])]))
      .digest().subarray(0, 10);
    const sealed = rc4([...objectKey], [...Buffer.from(content, 'latin1')]).toString('latin1');
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents ${number} 0 R `
      + '/Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> >>');
    objects.push(`<< /Length ${sealed.length} >>\nstream\n${sealed}\nendstream`);
  }
  return buildPdf(objects, `/Encrypt 3 0 R /ID [<${id.toString('hex')}> <${id.toString('hex')}>]`);
}

function buildPdf(objects, trailer = '') {
  const parts = ['%PDF-1.4\n'];
  const offsets = [];
  objects.forEach((body, index) => {
    offsets.push(parts.join('').length);
    parts.push(`${index + 1} 0 obj\n${body}\nendobj\n`);
  });
  const xref = parts.join('').length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const offset of offsets) parts.push(`${String(offset).padStart(10, '0')} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R ${trailer}>>\nstartxref\n${xref}\n%%EOF\n`);
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
  // The engine's insert, on its journal. The /Annots drop was MuPDF's graftPage; the
  // engine's repair grafts the annotations, so the copy keeps nav page 1's two links.
  const nav = await source('nav');
  const sourceDoc = reopen(nav);
  try {
    const sourceAnnots = sourceDoc.page(0).get('Annots');
    assert.ok(sourceAnnots instanceof List && sourceAnnots.length > 0, 'nav.pdf page 1 has links to bring across');
  } finally {
    engine.closeDocument(sourceDoc);
  }
  const target = await open('hello');
  try {
    assert.equal(target.journal.position, 0);
    const inserted = insert(target, nav, 1, { pages: [0, 1] });
    assert.equal(inserted.ok, true, inserted.error);
    assert.equal(target.doc.pageCount, 3);
    assert.equal(target.journal.position, 1, 'both grafts are one journal step');
    assert.equal(target.journal.steps[0], 'insert pages');

    const copied = target.doc.page(1).get('Annots');
    assert.equal(copied instanceof List ? copied.length : 0, 2, 'the repair brings the /Annots across');
    assert.deepEqual(op(target, { op: 'links', page: 1 }).items.map(item => item.kind), ['page', 'uri']);

    op(target, { op: 'undo' });
    assert.equal(target.doc.pageCount, 1, 'one undo removes every inserted page');
    op(target, { op: 'redo' });
    assert.equal(target.doc.pageCount, 3);
  } finally {
    engine.closeDocument(target.doc);
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
      assert.equal(at <= session.doc.pageCount, true, `${label}: the target has room`);
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
      assert.equal(re.pageCount, 4);
      assert.deepEqual(texts(re).map(text => text.split('\n')[0]), ['one', 'PDF4All', 'two', 'three']);
      // nav page 1's GoTo named nav page 2, which was not part of this insert: only the
      // link's presence and that it names a page are asserted here (the stated limit,
      // measured in p2-assemble.md).
      const graftedLinks = linksOf(re, 0);
      assert.equal(graftedLinks.length, 2, 'both links come across');
      assert.equal(graftedLinks[1].uri, 'https://example.com/');
      assert.ok(graftedLinks[0].page >= 0, 'the GoTo still resolves to a page');
      const note = re.annotsOf(1, 'Text').map(item => item.string('Contents'));
      assert.deepEqual(note, ['grafted note'], 'the note survives save, validate and reopen');
    } finally {
      engine.closeDocument(re);
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
      '/AP is copied verbatim: no appearance is synthesized for a widget that has none');
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
      engine.closeDocument(re);
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
      assert.deepEqual(linksOf(first, 0), [], 'the middle page has no links');
    } finally {
      engine.closeDocument(first);
    }

    const two = op(session, { op: 'extract-pages', pages: [2, 0] });
    assert.equal(two.ok, true, two.error);
    assert.equal(two.pageCount, 2);
    const second = reopen(two.bytes);
    try {
      assert.deepEqual(texts(second), ['three', 'one'], 'the requested order is kept');
      // A GoTo to page "two", which is not extracted: the engine never copies an
      // unselected page and drops the link (MuPDF's graft copied that page).
      assert.deepEqual(linksOf(second, 0).map(link => link.uri),
        ['file:///etc/hosts'], 'the blocked link is copied as it is, never followed');
      assert.deepEqual(linksOf(second, 1).map(link => link.uri), ['https://example.com/']);
    } finally {
      engine.closeDocument(second);
    }

    assert.equal(session.doc.pageCount, 3, 'the open document is untouched');
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
      assert.equal(out.pageCount, 1);
      assert.deepEqual(fieldNames(out), ['name', 'notes', 'agree', 'size', 'choice', 'pick', 'fixed', 'amount', 'sig']);
      assert.equal(widgets(out).length, 11, 'every widget is on the extracted page and registered');
      assert.deepEqual(op(session, { op: 'fields', page: 0 }).items.length, 11, 'the open document is unchanged');
    } finally {
      engine.closeDocument(out);
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
    assert.equal(session.doc.pageCount, 3);

    // A range inserts only the pages it names, and `2-3` is the open-ended form.
    const ranged = insert(session, nav, 0, { range: '2-3' });
    assert.equal(ranged.ok, true, ranged.error);
    assert.equal(ranged.inserted, 2);
    assert.deepEqual(texts(session.doc), ['two', 'three', 'one', 'two', 'three']);
    op(session, { op: 'undo' });

    // An encrypted source (empty user password opens it, permissions still apply)
    assert.equal(insert(session, encryptedBytes(), 0).ok, false);
    assert.match(insert(session, encryptedBytes(), 0).error, /encrypted source/);
    // and one that needs a password.
    const sealer = await open('nav');
    assert.equal(op(sealer, { op: 'security-set', mode: 'aes-256', userPassword: 'secret', ownerPassword: 'owner' }).ok, true);
    const secret = op(sealer, { op: 'save' });
    assert.equal(secret.ok, true, secret.error);
    engine.closeDocument(sealer.doc);
    const locked = secret.bytes;
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
  const encryptedTarget = await openBytes(encryptedBytes(), 'encrypted.pdf');
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
    assert.equal(session.doc.pageCount, 500);
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
    assertWithin(insertMs, 200, 'inserting one page into 500');
    assertWithin(extractMs, 500, 'extracting 20 pages');
  } finally {
    engine.closeDocument(session.doc);
  }
});
