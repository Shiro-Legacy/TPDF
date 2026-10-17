/**
 * P2 unit 5 — bookmarks and links: read, navigate, add, rename, delete, and the
 * repair `page-delete` performs. Runs against src/renderer/engine.ts exactly as
 * the worker does.
 *
 * The gate came first (contract-unit5.md, Decisions): every primitive this unit
 * mutates — outline insert/update/delete, createLink/deleteLink and the
 * `/AcroForm /Fields` array edit — had to be covered by the journal, and the two
 * traps that shaped the code are measured here rather than assumed:
 *   - `loadOutline()` is the only accessor that reports children, and
 *   - mupdf repoints a dangling destination instead of leaving it dangling, so
 *     the repair reads every reference *before* `deletePage` runs.
 */
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import test, { after } from 'node:test';
import mupdf from 'mupdf';
import { createEngine, LINK_SCHEMES, MAX_URI_CHARS } from '../build/renderer/engine.js';
import { assertWithin, bestOf } from './timing-budget.mjs';

const engine = createEngine(mupdf);
const run = promisify(execFile);
const repoRoot = fileURLToPath(new URL('..', import.meta.url));

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
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

async function openBytes(bytes, name = 'scratch.pdf', session = engine.createSession()) {
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: bytes }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

async function open(id, session = engine.createSession()) {
  return openBytes(toArrayBuffer(await readFile(`fixtures/${id}.pdf`)), `${id}.pdf`, session);
}

function op(session, request) {
  return engine.dispatch({ id: 900 + (request.id ?? 0), ...request }, session).reply;
}

function outlineOf(session) {
  const reply = op(session, { op: 'outline' });
  assert.equal(reply.ok, true, reply.error);
  return reply.items;
}

function linksOf(session, page) {
  const reply = op(session, { op: 'links', page });
  assert.equal(reply.ok, true, reply.error);
  return reply.items;
}

function reopen(bytes) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  doc.asPDF().disableJS();
  return doc;
}

/** The nav smoke's own sequence, as the engine sees it: bookmark, link, delete page 2. */
function smokeNav(session) {
  assert.equal(op(session, { op: 'outline-add', title: 'PDF4All P2 nav', page: 2 }).ok, true);
  const hit = op(session, { op: 'search', query: 'one' }).hits[0];
  assert.ok(hit, 'nav.pdf says "one" on page 1');
  const quad = hit.quads[0];
  const rect = [
    Math.min(quad[0], quad[2], quad[4], quad[6]),
    Math.min(quad[1], quad[3], quad[5], quad[7]),
    Math.max(quad[0], quad[2], quad[4], quad[6]),
    Math.max(quad[1], quad[3], quad[5], quad[7]),
  ];
  assert.equal(op(session, { op: 'link-add', page: hit.page, rect, target: { page: 2 } }).ok, true);
  const deleted = op(session, { op: 'page-delete', page: 1 });
  assert.equal(deleted.ok, true, deleted.error);
  assert.equal(deleted.pageCount, 2);
}

/**
 * A 3-page scratch whose only link and only outline entry carry a *numeric*
 * `/Dest [2 /Fit]` naming the last page. Deleting that page is the case mupdf
 * complains about: `format error: cannot find page 3 in page tree` on the next
 * page-tree load (measured, p2-pages.md). With the repair the load is silent.
 */
function danglingLastPagePdf() {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /Outlines 7 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R] >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>',
    '<< /Type /Annot /Subtype /Link /Rect [72 680 200 730] /Dest [2 /Fit] >>',
    '<< /Type /Outlines /First 8 0 R /Last 8 0 R /Count 1 >>',
    '<< /Title (Last) /Parent 7 0 R /Dest [2 /Fit] >>',
  ]);
}

/**
 * Two pages and three text fields: `keep` has one widget on page 1, `both` has one
 * on each page, `only` has one only on page 2. Deleting page 2 leaves `only` in
 * `/AcroForm /Fields` with no widget anywhere — the phantom the repair removes.
 */
function phantomFieldPdf() {
  const widget = (parent, rect) => '<< /Type /Annot /Subtype /Widget /FT /Tx '
    + `/V () /Parent ${parent} 0 R /Rect [${rect}] /F 4 >>`;
  const field = (name, kids) => `<< /FT /Tx /T (${name}) /V () /Kids [${kids}] >>`;
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 8 0 R] >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [9 0 R 10 0 R] >>',
    '<< /Fields [7 0 R 11 0 R 12 0 R] /DA (/Helv 12 Tf 0 g) >>',
    widget(7, '72 700 300 720'),
    field('keep', '6 0 R'),
    widget(11, '72 660 300 680'),
    widget(11, '72 660 300 680'),
    widget(12, '72 620 300 640'),
    field('both', '8 0 R 9 0 R'),
    field('only', '10 0 R'),
  ]);
}

/** The names still in `/AcroForm /Fields`, in order. */
function fieldNames(doc) {
  const fields = doc.asPDF().getTrailer().get('Root').get('AcroForm').get('Fields');
  const names = [];
  for (let index = 0; index < fields.length; index++) {
    names.push(fields.get(index).get('T').asString());
  }
  return names;
}

/**
 * Opens the bytes in a *fresh process* and loads the page tree the way a viewer
 * would, so mupdf's own stderr can be asserted: the warning is written to the
 * process, not thrown, and the engine's own process cannot observe it.
 */
async function pageTreeLoad(path) {
  const script = `
    import mupdf from 'mupdf';
    import { readFileSync } from 'node:fs';
    const doc = mupdf.Document.openDocument(readFileSync(${JSON.stringify(path)}), 'application/pdf');
    doc.loadOutline();
    for (let page = 0; page < doc.countPages(); page++) {
      const loaded = doc.loadPage(page);
      loaded.getLinks().forEach(link => doc.resolveLink(link));
      loaded.destroy();
    }
    console.log('PAGE_TREE_OK');
  `;
  try {
    const { stdout, stderr } = await run(process.execPath, ['--input-type=module', '-e', script], { cwd: repoRoot });
    return { ok: true, stdout, stderr };
  } catch (err) {
    return { ok: false, stdout: err.stdout ?? '', stderr: err.stderr ?? String(err) };
  }
}

async function writeSaved(session, path) {
  const save = engine.dispatch({ id: 20, op: 'save' }, session).reply;
  assert.equal(save.ok, true, save.error);
  await writeFile(path, Buffer.from(save.bytes));
  return path;
}

/** A per-run file in the OS temp directory; the scratch never lands in the repository and is removed after the file's tests. */
const scratchFiles = [];
function scratchPath(name) {
  const path = join(tmpdir(), `pdf4all-p2u5-${process.pid}-${name}.pdf`);
  scratchFiles.push(path);
  return path;
}
after(() => Promise.all(scratchFiles.map(path => rm(path, { force: true }))));

/**
 * One page with two bookmarks that name no page: a web URI and a JavaScript action.
 * Neither is ever followed; the listing still reports the external destination.
 */
function externalBookmarkPdf() {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>',
    '<< /Type /Outlines /First 5 0 R /Last 6 0 R /Count 2 >>',
    '<< /Title (Web) /Parent 4 0 R /Next 6 0 R /A << /S /URI /URI (https://example.com/) >> >>',
    '<< /Title (Script) /Parent 4 0 R /Prev 5 0 R /A << /S /JavaScript /JS (app.alert(1)) >> >>',
  ]);
}

// ------------------------------------------------------------------ gate

test('gate: the journal covers the outline iterator, createLink/deleteLink and the /Fields edit', async () => {
  const nav = toArrayBuffer(await readFile('fixtures/nav.pdf'));

  // 1. OutlineIterator insert / update / delete, each its own operation.
  {
    const doc = mupdf.Document.openDocument(nav, 'application/pdf');
    const pdf = doc.asPDF();
    try {
      pdf.enableJournal();
      assert.equal(pdf.getJournal().position, 0);
      const it = doc.outlineIterator();
      it.insert({ title: 'gate', uri: doc.formatLinkURI({ type: 'Fit', page: 0 }), open: true });
      assert.equal(pdf.getJournal().position, 1, 'insert is journaled');
      it.destroy();
      // insert() places before the cursor, and a fresh cursor is on the first entry.
      const inserted = doc.outlineIterator();
      assert.equal(inserted.item()?.title, 'gate');
      inserted.update({ title: 'gate2', uri: inserted.item().uri, open: true });
      assert.equal(pdf.getJournal().position, 2, 'update is journaled');
      assert.equal(inserted.delete(), 0);
      assert.equal(pdf.getJournal().position, 3, 'delete is journaled');
      inserted.destroy();
      pdf.undo();
      pdf.undo();
      pdf.undo();
      assert.equal(pdf.getJournal().position, 0, 'three undos return to the opening position');
      const flat = [];
      const walk = (nodes) => { for (const node of nodes) { flat.push(node.title); if (node.down) walk(node.down); } };
      walk(doc.loadOutline() ?? []);
      assert.deepEqual(flat, ['One', 'Two', 'Two-A', 'Three'], 'and the original outline is back');
    } finally {
      doc.destroy();
    }
  }

  // 2. createLink / deleteLink, each its own operation, round-tripped by undo/redo.
  //    The delete runs on a *reloaded* page: deleting a link from the same live page
  //    object that created it records no new step (measured; the engine always reloads).
  {
    const doc = mupdf.Document.openDocument(nav, 'application/pdf');
    const pdf = doc.asPDF();
    try {
      pdf.enableJournal();
      let page = doc.loadPage(0);
      page.createLink([72, 400, 200, 430], 'https://example.org/gate');
      page.destroy();
      assert.equal(pdf.getJournal().position, 1, 'createLink is journaled');
      page = doc.loadPage(0);
      page.deleteLink(page.getLinks()[2]);
      page.destroy();
      assert.equal(pdf.getJournal().position, 2, 'deleteLink is journaled');
      pdf.undo();
      assert.equal(doc.loadPage(0).getLinks().length, 3, 'undo restores the link');
      pdf.undo();
      assert.equal(doc.loadPage(0).getLinks().length, 2, 'and the second undo removes it');
      pdf.redo();
      assert.equal(doc.loadPage(0).getLinks().length, 3, 'redo creates it again');
    } finally {
      doc.destroy();
    }
  }

  // 3. The /AcroForm /Fields array edit inside one operation.
  {
    const doc = mupdf.Document.openDocument(toArrayBuffer(await readFile('fixtures/form2.pdf')), 'application/pdf');
    const pdf = doc.asPDF();
    try {
      pdf.enableJournal();
      const fields = pdf.getTrailer().get('Root').get('AcroForm').get('Fields');
      const before = fields.length;
      pdf.beginOperation('remove field');
      fields.delete(0);
      pdf.endOperation();
      assert.equal(pdf.getJournal().position, 1, 'the /Fields delete is journaled');
      assert.equal(fields.length, before - 1);
      pdf.undo();
      assert.equal(pdf.getTrailer().get('Root').get('AcroForm').get('Fields').length, before,
        'undo restores the removed field');
    } finally {
      doc.destroy();
    }
  }
});

// --------------------------------------------------------------- outline

test('nav.pdf: the outline lists in preorder with depths, and adding a bookmark appends', async () => {
  const session = await open('nav');
  try {
    const items = outlineOf(session);
    assert.deepEqual(items.map(item => [item.title, item.depth, item.page]),
      [['One', 0, 0], ['Two', 0, 1], ['Two-A', 1, 1], ['Three', 0, 2]]);
    assert.deepEqual(items.map(item => item.id), [0, 1, 2, 3], 'id is the preorder index');
    assert.ok(items.every(item => item.uri === undefined), 'page destinations carry no uri');

    const added = op(session, { op: 'outline-add', title: 'PDF4All P2 nav', page: 2 });
    assert.equal(added.ok, true, added.error);
    assert.equal(added.revision, 1);
    assert.equal(added.dirty, true);
    const after = outlineOf(session);
    assert.equal(after.length, 5);
    assert.deepEqual(after.at(-1), { id: 4, title: 'PDF4All P2 nav', depth: 0, page: 2 },
      'the new entry is last, top level, on the page it was added for');

    const undone = op(session, { op: 'undo' });
    assert.equal(undone.step, 'add bookmark');
    assert.equal(outlineOf(session).length, 4, 'undo removes it');
    const redone = op(session, { op: 'redo' });
    assert.equal(redone.step, 'add bookmark');
    assert.equal(outlineOf(session).length, 5, 'redo adds it back');
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('outline-set renames in place and outline-delete takes the subtree', async () => {
  const session = await open('nav');
  try {
    assert.equal(op(session, { op: 'outline-set', outlineId: 1, title: 'Two renamed' }).ok, true);
    const renamed = outlineOf(session);
    assert.deepEqual(renamed.map(item => [item.title, item.depth, item.page]),
      [['One', 0, 0], ['Two renamed', 0, 1], ['Two-A', 1, 1], ['Three', 0, 2]],
      'renaming a parent keeps its child and its destination');

    const deleted = op(session, { op: 'outline-delete', outlineId: 1 });
    assert.equal(deleted.ok, true, deleted.error);
    assert.deepEqual(outlineOf(session).map(item => item.title), ['One', 'Three'],
      'the entry goes with its whole subtree');

    const undone = op(session, { op: 'undo' });
    assert.equal(undone.step, 'delete bookmark');
    assert.deepEqual(outlineOf(session).map(item => item.title), ['One', 'Two renamed', 'Two-A', 'Three']);

    const redone = op(session, { op: 'redo' });
    assert.equal(redone.step, 'delete bookmark');
    assert.deepEqual(outlineOf(session).map(item => item.title), ['One', 'Three']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('an external bookmark lists its uri and no page; a script action lists neither', async () => {
  const session = await openBytes(externalBookmarkPdf(), 'external.pdf');
  try {
    assert.deepEqual(outlineOf(session), [
      { id: 0, title: 'Web', depth: 0, page: null, uri: 'https://example.com/' },
      { id: 1, title: 'Script', depth: 0, page: null },
    ], 'a destination that is not a page reports no page; the URI travels for the panel to check');

    // The list still round-trips a mutation on such a document (append after both).
    assert.equal(op(session, { op: 'outline-add', title: 'Local', page: 0 }).ok, true);
    assert.deepEqual(outlineOf(session).map(item => [item.title, item.page]),
      [['Web', null], ['Script', null], ['Local', 0]]);
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ----------------------------------------------------------------- links

test('nav.pdf: links list as page, uri and blocked, in /Annots order', async () => {
  const session = await open('nav');
  try {
    const first = linksOf(session, 0);
    assert.deepEqual(first.map(item => [item.id, item.kind]), [[0, 'page'], [1, 'uri']]);
    assert.deepEqual(first[0].rect, [72, 62, 200, 112], 'the rect is y-down page points');
    assert.equal(first[0].page, 1);
    assert.equal(first[1].uri, 'https://example.com/');

    assert.deepEqual(linksOf(session, 1), [], 'the middle page has no links');

    const third = linksOf(session, 2);
    assert.deepEqual(third.map(item => [item.id, item.kind]), [[0, 'page'], [1, 'blocked']]);
    assert.equal(third[1].uri, 'file:///etc/hosts', 'the raw URI travels so the panel can name the scheme');
    assert.equal(third[1].page, undefined);
    assert.equal(third[0].page, 1);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('link-add writes a GoTo and a URI link through one journal operation each', async () => {
  const session = await open('nav');
  try {
    const page = op(session, { op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: { page: 2 } });
    assert.equal(page.ok, true, page.error);
    assert.equal(page.revision, 1);
    const uri = op(session, { op: 'link-add', page: 0, rect: [72, 350, 200, 380], target: { uri: 'https://example.org/x' } });
    assert.equal(uri.ok, true, uri.error);
    assert.equal(uri.revision, 2);

    const items = linksOf(session, 0);
    assert.deepEqual(items.map(item => item.kind), ['page', 'uri', 'page', 'uri'], 'new links append in order');
    assert.deepEqual(items[2].rect, [72, 400, 200, 430], 'the rect round-trips through mupdf in page points');
    assert.equal(items[2].page, 2);
    assert.equal(items[3].uri, 'https://example.org/x');

    assert.equal(op(session, { op: 'undo' }).step, 'add link');
    assert.deepEqual(linksOf(session, 0).map(item => item.kind), ['page', 'uri', 'page']);
    assert.equal(op(session, { op: 'redo' }).step, 'add link');
    assert.deepEqual(linksOf(session, 0).map(item => item.kind), ['page', 'uri', 'page', 'uri']);

    const deleted = op(session, { op: 'link-delete', page: 0, linkId: 2 });
    assert.equal(deleted.ok, true, deleted.error);
    assert.equal(op(session, { op: 'undo' }).step, 'delete link');
    assert.deepEqual(linksOf(session, 0).map(item => item.kind), ['page', 'uri', 'page', 'uri']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('the link and bookmark ops refuse what they cannot do, without dirtying', async () => {
  const session = await open('nav');
  try {
    const boxRefusals = [
      [{ op: 'link-add', page: 0, rect: [72, 900, 200, 930], target: { page: 1 } }, /off the page/],
      [{ op: 'link-add', page: 0, rect: [-20, 400, 200, 430], target: { page: 1 } }, /off the page/],
      [{ op: 'link-add', page: 0, rect: [72, 400, 72, 430], target: { page: 1 } }, /positive area/],
      [{ op: 'link-add', page: 0, rect: [72, 400, 200], target: { page: 1 } }, /4 finite numbers/],
      [{ op: 'link-add', page: 9, rect: [72, 400, 200, 430], target: { page: 1 } }, /0\.\.2/],
      [{ op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: { page: 7 } }, /0\.\.2/],
      [{ op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: { uri: 'file:///etc/hosts' } }, /file:/],
      [{ op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: { uri: 'javascript:alert(1)' } }, /javascript:/],
      [{ op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: { uri: '#page=2&view=Fit' } }, /scheme/],
      [{ op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: { uri: '  https://example.org/pad ' } }, /without a scheme/],
      [{ op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: { uri: `mailto:${'x'.repeat(MAX_URI_CHARS)}@example.com` } }, /2048/],
      [{ op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: {} }, /target must be/],
      [{ op: 'link-add', page: 0, rect: [72, 400, 200, 430] }, /target must be/],
      [{ op: 'link-delete', page: 0, linkId: 9 }, /no link 9/],
      [{ op: 'link-delete', page: 0, linkId: 'x' }, /no link x/],
      [{ op: 'link-delete', page: 0, linkId: -1 }, /no link -1/],
      [{ op: 'outline-add', title: '   ', page: 0 }, /needs a title/],
      [{ op: 'outline-add', title: 'x'.repeat(4097), page: 0 }, /at most 4096/],
      [{ op: 'outline-add', title: 'fine', page: 3 }, /0\.\.2/],
      [{ op: 'outline-set', outlineId: 9, title: 'x' }, /no bookmark 9/],
      [{ op: 'outline-set', outlineId: -1, title: 'x' }, /non-negative/],
      [{ op: 'outline-set', outlineId: 0, title: '' }, /needs a title/],
      [{ op: 'outline-delete', outlineId: 9 }, /no bookmark 9/],
    ];
    for (const [request, pattern] of boxRefusals) {
      const refused = op(session, request);
      assert.equal(refused.ok, false, JSON.stringify(request));
      assert.match(refused.error, pattern, JSON.stringify(request));
    }
    assert.equal(op(session, { op: 'undo' }).ok, false, 'a refused op never dirties the document');
    assert.equal(op(session, { op: 'outline' }).items.length, 4, 'and nothing moved');
    assert.equal(linksOf(session, 0).length, 2);

    // The allowlist itself is the exported constant, so the UI and the engine cannot drift.
    assert.deepEqual([...LINK_SCHEMES], ['http:', 'https:', 'mailto:']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ------------------------------------------------------- page-delete repair

test('page-delete drops dangling outline destinations and links to the removed page', async () => {
  const session = await open('nav');
  try {
    const deleted = op(session, { op: 'page-delete', page: 1 });
    assert.equal(deleted.ok, true, deleted.error);
    assert.equal(deleted.pageCount, 2);

    const items = outlineOf(session);
    assert.deepEqual(items.map(item => [item.title, item.page]),
      [['One', 0], ['Two', null], ['Two-A', null], ['Three', 1]],
      'titles and subtrees stay; only the destinations that named page 2 are dropped');

    assert.deepEqual(linksOf(session, 0).map(item => [item.kind, item.uri ?? item.page]),
      [['uri', 'https://example.com/']], 'the GoTo to the removed page is gone, the URI link stays');
    assert.deepEqual(linksOf(session, 1).map(item => [item.kind, item.uri ?? item.page]),
      [['blocked', 'file:///etc/hosts']], 'and the same on the page that took its index');

    const undone = op(session, { op: 'undo' });
    assert.equal(undone.step, 'delete page');
    assert.equal(undone.pageCount, 3);
    assert.deepEqual(outlineOf(session).map(item => [item.title, item.page]),
      [['One', 0], ['Two', 1], ['Two-A', 1], ['Three', 2]], 'one undo restores every destination');
    assert.equal(linksOf(session, 0).length, 2, 'and both links');
    assert.equal(linksOf(session, 2).length, 2);

    assert.equal(op(session, { op: 'redo' }).step, 'delete page');
    assert.deepEqual(outlineOf(session).map(item => item.page), [0, null, null, 1]);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('page-delete removes a form field left without a widget, and undo restores it', async () => {
  const session = await openBytes(phantomFieldPdf(), 'phantom.pdf');
  try {
    assert.deepEqual(fieldNames(session.doc), ['keep', 'both', 'only']);
    assert.deepEqual(op(session, { op: 'fields', page: 0 }).items.map(item => item.name), ['keep', 'both']);
    assert.deepEqual(op(session, { op: 'fields', page: 1 }).items.map(item => item.name), ['both', 'only']);
    const deleted = op(session, { op: 'page-delete', page: 1 });
    assert.equal(deleted.ok, true, deleted.error);
    assert.deepEqual(fieldNames(session.doc), ['keep', 'both'],
      'the field whose only widget was on the removed page is gone; the shared one survives');

    const undone = op(session, { op: 'undo' });
    assert.equal(undone.ok, true, undone.error);
    assert.deepEqual(fieldNames(session.doc), ['keep', 'both', 'only'], 'one undo restores the field');

    const redone = op(session, { op: 'redo' });
    assert.equal(redone.ok, true, redone.error);
    assert.deepEqual(fieldNames(session.doc), ['keep', 'both']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('after the repaired delete the next page-tree load prints nothing to stderr', async () => {
  // The baseline: the same scratch deleted without the repair warns, so the
  // assertion below is not vacuous (measured, `.quad/shared/p2/p2-nav.md` §Gate).
  const raw = reopen(danglingLastPagePdf());
  raw.asPDF().deletePage(2);
  const rawBuffer = raw.asPDF().saveToBuffer('incremental=no');
  const rawPath = scratchPath('dangling-raw');
  await writeFile(rawPath, Buffer.from(rawBuffer.asUint8Array()));
  rawBuffer.destroy();
  raw.destroy();
  const warned = await pageTreeLoad(rawPath);
  assert.equal(warned.ok, false, 'the unrepaired scratch is the case mupdf complains about');
  assert.match(warned.stderr, /cannot find page 3 in page tree/);

  const session = await openBytes(danglingLastPagePdf(), 'dangling.pdf');
  try {
    assert.deepEqual(outlineOf(session).map(item => [item.title, item.page]), [['Last', 2]]);
    assert.deepEqual(linksOf(session, 0).map(item => [item.kind, item.page]), [['page', 2]]);

    const deleted = op(session, { op: 'page-delete', page: 2 });
    assert.equal(deleted.ok, true, deleted.error);
    assert.deepEqual(outlineOf(session).map(item => [item.title, item.page]), [['Last', null]]);
    assert.deepEqual(linksOf(session, 0), [], 'the link that named the removed page is gone');

    const save = engine.dispatch({ id: 20, op: 'save' }, session).reply;
    assert.equal(save.ok, true, save.error);
    const path = scratchPath('dangling-repaired');
    await writeFile(path, Buffer.from(save.bytes));

    const loaded = await pageTreeLoad(path);
    assert.equal(loaded.ok, true, loaded.stderr);
    assert.equal(loaded.stdout.trim(), 'PAGE_TREE_OK');
    assert.equal(loaded.stderr, '', 'a repaired delete leaves no page reference for mupdf to miss');
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ------------------------------------------------------ round-trip and cost

test('save, validate and reopen keep the outline and the links, after the smoke script', async () => {
  const session = await open('nav');
  try {
    smokeNav(session);
    assert.deepEqual(outlineOf(session).map(item => [item.title, item.page]),
      [['One', 0], ['Two', null], ['Two-A', null], ['Three', 1], ['PDF4All P2 nav', 1]]);
    assert.deepEqual(linksOf(session, 0).map(item => item.kind), ['uri', 'page'],
      'the link to the removed page went; the smoke link now points at page 2');
    assert.deepEqual(linksOf(session, 1).map(item => item.kind), ['blocked']);

    const save = engine.dispatch({ id: 20, op: 'save' }, session).reply;
    assert.equal(save.ok, true, save.error);
    const check = engine.dispatch({ id: 21, op: 'validate', pdf: save.bytes }, session).reply;
    assert.equal(check.ok, true, check.error);
    assert.equal(check.pageCount, 2);

    const re = reopen(check.bytes);
    try {
      assert.deepEqual((re.loadOutline() ?? []).map(node => [node.title, node.page ?? null, (node.down ?? []).map(kid => [kid.title, kid.page ?? null])]),
        [['One', 0, []], ['Two', null, [['Two-A', null]]], ['Three', 1, []], ['PDF4All P2 nav', 1, []]],
        'the reopened outline keeps titles, nesting and the surviving destinations');
      assert.deepEqual(re.loadPage(0).getLinks().map(link => [link.getURI(), re.resolveLink(link)]),
        [['https://example.com/', -1], ['#page=2&view=Fit', 1]]);
      assert.deepEqual(re.loadPage(1).getLinks().map(link => [link.getURI(), re.resolveLink(link)]),
        [['file:///etc/hosts', -1]]);
    } finally {
      re.destroy();
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('hello.pdf lists nothing; the new ops refuse a read-only document', async () => {
  const session = await open('hello');
  try {
    assert.deepEqual(outlineOf(session), []);
    assert.deepEqual(linksOf(session, 0), []);
  } finally {
    engine.closeDocument(session.doc);
  }

  const refusals = [
    { op: 'outline-add', title: 'x', page: 0 },
    { op: 'outline-set', outlineId: 0, title: 'x' },
    { op: 'outline-delete', outlineId: 0 },
    { op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: { page: 0 } },
    { op: 'link-delete', page: 0, linkId: 0 },
  ];
  for (const [label, build] of [['signed', signedPdf], ['certified', certifiedPdf]]) {
    const locked = await openBytes(build(), `${label}.pdf`);
    for (const request of refusals) {
      const refused = op(locked, request);
      assert.equal(refused.ok, false, `${label} ${request.op}`);
      assert.equal(refused.reason, 'read-only', `${label} ${request.op}`);
    }
    engine.closeDocument(locked.doc);
  }

  // AES-256 with an empty user password is unlocked and grants every permission, so the
  // two adds work now (P2 unit 10); a rename or delete still needs a real item to name.
  const plain = mupdf.Document.openDocument(toArrayBuffer(await readFile('fixtures/hello.pdf')), 'application/pdf').asPDF();
  const buffer = plain.saveToBuffer('incremental=no,encrypt=aes-256,user-password=,owner-password=owner');
  const encrypted = toArrayBuffer(buffer.asUint8Array());
  buffer.destroy();
  plain.destroy();
  const locked = await openBytes(encrypted, 'empty-password.pdf');
  try {
    const outlineAdd = op(locked, { op: 'outline-add', title: 'x', page: 0 });
    assert.equal(outlineAdd.ok, true, outlineAdd.error);
    const linkAdd = op(locked, { op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: { page: 0 } });
    assert.equal(linkAdd.ok, true, linkAdd.error);
  } finally {
    engine.closeDocument(locked.doc);
  }
});

test('large.pdf: the outline and link panels are cheap, and the repair does not walk the world', async () => {
  const session = await open('large');
  try {
    assert.equal(session.doc.countPages(), 500);
    const { ms: outlineMs, result: outline } = bestOf(3, () => op(session, { op: 'outline' }));
    assert.equal(outline.ok, true, outline.error);
    assert.deepEqual(outline.items, [], 'large.pdf has no bookmarks');

    const { ms: linksMs, result: links } = bestOf(3, () => op(session, { op: 'links', page: 0 }));
    assert.equal(links.ok, true, links.error);
    assert.deepEqual(links.items, []);

    const deleteStarted = performance.now();
    const deleted = op(session, { op: 'page-delete', page: 499 });
    const deleteMs = performance.now() - deleteStarted;
    assert.equal(deleted.ok, true, deleted.error);
    assert.equal(deleted.pageCount, 499);

    console.log(`U5_LARGE ${JSON.stringify({
      outlineMs: Math.round(outlineMs * 10) / 10,
      linksMs: Math.round(linksMs * 10) / 10,
      deleteMs: Math.round(deleteMs * 10) / 10,
    })}`);
    assertWithin(outlineMs, 50, 'the outline');
    assertWithin(linksMs, 50, 'the links of a page');
    assertWithin(deleteMs, 500, 'a repaired 500-page delete');
  } finally {
    engine.closeDocument(session.doc);
  }
});

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

// P6 U4 §6.1 (additive): every link carries its /Annots object number, and deleting one
// link leaves the others' annotIds alone while their positional ids renumber.
test('nav.pdf links carry annotId; link-delete keeps the others', async () => {
  const session = await open('nav');
  let after = null;
  try {
    assert.equal(op(session, { op: 'link-add', page: 0, rect: [72, 400, 200, 430], target: { uri: 'https://example.org/x' } }).ok, true);
    const before = linksOf(session, 0);
    assert.equal(before.length, 3);
    for (const item of before) assert.ok(Number.isInteger(item.annotId) && item.annotId > 0, JSON.stringify(item));
    assert.equal(new Set(before.map(item => item.annotId)).size, 3);
    assert.equal(op(session, { op: 'link-delete', page: 0, linkId: 1 }).ok, true);
    after = await openBytes(op(session, { op: 'save' }).bytes, 'nav-roundtrip.pdf');
    assert.deepEqual(linksOf(after, 0).map(item => [item.id, item.annotId]), [[0, before[0].annotId], [1, before[2].annotId]]);
  } finally {
    engine.closeDocument(session.doc);
    if (after) engine.closeDocument(after.doc);
  }
});
