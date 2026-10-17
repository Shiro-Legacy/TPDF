import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import {
  createEngine, checkRedactSourceBytes, MAX_REDACT_MARKS, MAX_REDACT_SOURCE_BYTES,
  MAX_REDACT_TEXT, REDACT_SANITIZE_KEYS,
} from './rust-engine.mjs';

let nextId = 1;
const engine = createEngine(mupdf);
const arrayBuffer = buffer => buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
const fixture = async name => arrayBuffer(await readFile(new URL(`../fixtures/${name}`, import.meta.url)));
const op = (session, request) => engine.dispatch({ id: nextId++, ...request }, session).reply;
const all = value => Object.fromEntries(REDACT_SANITIZE_KEYS.map(key => [key, value]));

async function opened(name = 'redact.pdf') {
  const session = engine.createSession();
  const reply = op(session, { op: 'open', pdf: await fixture(name), name });
  assert.equal(reply.ok, true, reply.error);
  return session;
}

test('P4 redaction intermediate rewrite cap accepts 12 MiB exactly and refuses the next byte', () => {
  assert.equal(MAX_REDACT_SOURCE_BYTES, 12 * 1024 * 1024);
  assert.doesNotThrow(() => checkRedactSourceBytes(MAX_REDACT_SOURCE_BYTES));
  assert.throws(
    () => checkRedactSourceBytes(MAX_REDACT_SOURCE_BYTES + 1),
    error => error.reason === 'too-large' && error.message === 'Redaction is limited to documents up to 12 MiB in this version',
  );
});

test('P4 redaction bounds and rect mark are one undoable annotation that an ordinary save keeps', async () => {
  assert.equal(MAX_REDACT_MARKS, 2000);
  assert.equal(MAX_REDACT_TEXT, 256);
  const session = await opened();
  const invalid = op(session, { op: 'redact-add', page: 0, rect: [1, 1, 1.5, 4] });
  assert.equal(invalid.ok, false);
  assert.equal(invalid.reason, 'invalid');
  assert.equal(invalid.dirty, false);

  const added = op(session, { op: 'redact-add', page: 0, rect: [-20, 20, 80, 80] });
  assert.equal(added.ok, true, added.error);
  assert.equal(added.added, 1);
  assert.equal(added.dirty, true);
  assert.deepEqual(op(session, { op: 'annots', page: 0 }).items.filter(item => item.type === 'Redact').map(item => item.rect.length), [4]);
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const copy = mupdf.Document.openDocument(saved.bytes, 'application/pdf');
  try { assert.equal(copy.loadPage(0).getAnnotations().filter(item => item.getType() === 'Redact').length, 1); }
  finally { copy.destroy(); }
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(op(session, { op: 'annots', page: 0 }).items.some(item => item.type === 'Redact'), false);

  const signed = await opened('p0b/signed-intact.pdf');
  const refused = op(signed, { op: 'redact-add', page: 0, rect: [10, 10, 40, 40] });
  assert.equal(refused.reason, 'read-only');
  assert.equal(refused.dirty, false);
});

test('P4 search marks every page, previews inventory, applies and leaves the source session unchanged', async () => {
  const session = await opened();
  const marked = op(session, { op: 'redact-search', text: 'P0B-SECRET-7731' });
  assert.equal(marked.ok, true, marked.error);
  assert.equal(marked.added, 8);
  assert.deepEqual(marked.pages, [0, 1]);
  const noMatch = op(session, { op: 'redact-search', text: 'NO-SUCH-P4-TOKEN' });
  assert.equal(noMatch.reason, 'no-match');
  assert.equal(noMatch.revision, marked.revision, 'a refusal never dirties or journals');

  const preview = op(session, { op: 'redact-preview' });
  assert.equal(preview.ok, true, preview.error);
  assert.deepEqual(preview.pageCounts, [{ page: 0, marks: 7 }, { page: 1, marks: 1 }]);
  assert.equal(preview.shared.length, 0);
  assert.equal(preview.inlineAlt, 1);
  assert.deepEqual(preview.sanitize, {
    info: 1, xmp: true, attachments: 1, outlines: 1, annots: 2,
    links: 1, fields: 1, layers: 1, struct: 2,
  });
  const before = engine.journalState(session);
  const applied = op(session, { op: 'redact-apply', sanitize: all(true) });
  assert.equal(applied.ok, true, applied.error);
  assert.deepEqual(applied.verified, { text: 8, pixels: 8, raw: 1 });
  assert.equal(new TextDecoder('latin1').decode(applied.bytes).includes('P0B-SECRET-7731'), false);
  assert.deepEqual(engine.journalState(session), before, 'apply never mutates the open session');
  assert.equal(op(session, { op: 'redact-preview' }).marks, 8, 'the source keeps its marks');

  const output = mupdf.Document.openDocument(applied.bytes, 'application/pdf');
  try {
    assert.equal(output.asPDF().countVersions(), 1);
    assert.equal(output.asPDF().wasRepaired(), false);
    for (let page = 0; page < output.countPages(); page++) {
      assert.equal(output.loadPage(page).getAnnotations().some(item => item.getType() === 'Redact'), false);
    }
  } finally { output.destroy(); }
});

test('P4 shared Form placement refuses apply until every placement is marked', async () => {
  const session = await opened('redact-shared-only.pdf');
  assert.equal(op(session, { op: 'redact-add', page: 0, rect: [100, 320, 230, 350] }).ok, true);
  const preview = op(session, { op: 'redact-preview' });
  assert.equal(preview.shared.length, 1);
  assert.deepEqual(preview.shared[0], { page: 0, kind: 'form', otherPages: [1] });
  const refused = op(session, { op: 'redact-apply', sanitize: all(true) });
  assert.equal(refused.reason, 'shared-content');
  assert.equal(refused.dirty, true, 'the pre-existing mark remains the only dirty change');
  assert.equal(op(session, { op: 'redact-add', page: 1, rect: [100, 320, 230, 350] }).ok, true);
  assert.equal(op(session, { op: 'redact-preview' }).shared.length, 0);
});

test('P4 sanitize toggles are independent and report surfaces deliberately kept', async () => {
  for (const off of REDACT_SANITIZE_KEYS) {
    const session = await opened();
    assert.equal(op(session, { op: 'redact-search', text: 'P0B-SECRET-7731' }).ok, true);
    const sanitize = all(true);
    sanitize[off] = false;
    const applied = op(session, { op: 'redact-apply', sanitize });
    assert.equal(applied.ok, true, `${off}: ${applied.error}`);
    assert.equal(off === 'xmp' ? applied.sanitized.xmp : applied.sanitized[off], off === 'xmp' ? false : 0);
  }
});

test('P4 redacted output keeps source encryption after user-password unlock', async () => {
  const session = await opened('encrypted-aes256.pdf');
  const auth = op(session, { op: 'authenticate', password: 'pdf4all-user' });
  assert.equal(auth.authenticated, true);
  assert.equal(op(session, { op: 'redact-add', page: 0, rect: [30, 30, 120, 80] }).ok, true);
  const applied = op(session, { op: 'redact-apply', sanitize: all(true) });
  assert.equal(applied.ok, true, applied.error);
  const output = mupdf.Document.openDocument(applied.bytes, 'application/pdf');
  try {
    assert.equal(output.needsPassword(), true);
    assert.equal(output.authenticatePassword('pdf4all-user'), 2);
  } finally { output.destroy(); }
});

test('P4 an output that keeps sibling bookmarks is still one revision (opus5 review fix)', async () => {
  // nav.pdf's outline items carry their own /Prev; a raw byte scan for /Prev
  // refused this correct output before the post-check became structural.
  const session = await opened('nav.pdf');
  assert.equal(op(session, { op: 'redact-add', page: 0, rect: [40, 40, 140, 90] }).ok, true);
  const applied = op(session, { op: 'redact-apply', sanitize: { ...all(true), outlines: false } });
  assert.equal(applied.ok, true, applied.error);
  const output = mupdf.Document.openDocument(applied.bytes, 'application/pdf');
  try {
    assert.equal(output.asPDF().countVersions(), 1);
    assert.equal(output.loadOutline().length, 3, 'the kept outline survives');
  } finally { output.destroy(); }
});

// ---------------------------------------------------------------- P6 U3 redact-set (§3.1, §9.4)
const marks = (session, page = 0) => op(session, { op: 'annots', page }).items.filter(item => item.type === 'Redact');
const near = (actual, expected, tolerance = 0.01) => assert.ok(
  actual.length === expected.length && actual.every((value, index) => Math.abs(value - expected[index]) <= tolerance),
  `${JSON.stringify(actual)} vs ${JSON.stringify(expected)}`,
);
function reopenSaved(session) {
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const fresh = engine.createSession();
  assert.equal(op(fresh, { op: 'open', pdf: saved.bytes, name: 'saved.pdf' }).ok, true);
  return fresh;
}
function rawMark(session, annotId, page = 0) {
  const view = session.doc.loadPage(page);
  try {
    const annot = view.getAnnotations().find(item => item.getObject().asIndirect() === annotId);
    const object = annot.getObject();
    return { rect: [...annot.getRect()], quads: annot.getQuadPoints().length, tag: object.get('PDF4AllSearch').isNull() ? null : object.get('PDF4AllSearch').asString() };
  } finally { view.destroy(); }
}

test('P6 U3 redact-set moves an area mark exactly, is one undoable op, and a same rect is a no-op', async () => {
  const session = await opened('p6/direct3.pdf');
  assert.equal(op(session, { op: 'redact-add', page: 0, rect: [300, 600, 400, 650] }).ok, true);
  const [mark] = marks(session);
  assert.deepEqual([mark.rect, mark.search], [[300, 600, 400, 650], undefined]);
  // Border creep, measured: setRect → clearQuadPoints → update → getRect gives the rect back (g = 0).
  const moved = op(session, { op: 'redact-set', page: 0, annotId: mark.id, rect: [340, 600, 440, 650] });
  assert.deepEqual([moved.ok, moved.noop, moved.revision, moved.dirty], [true, undefined, 2, true], moved.error);
  near(marks(session)[0].rect, [340, 600, 440, 650]);
  for (let nudge = 1; nudge <= 3; nudge++) {
    const rect = marks(session)[0].rect;
    assert.equal(op(session, { op: 'redact-set', page: 0, annotId: mark.id, rect: [rect[0] + 1, rect[1], rect[2] + 1, rect[3]] }).ok, true);
  }
  near(marks(session)[0].rect, [343, 600, 443, 650]);
  const same = op(session, { op: 'redact-set', page: 0, annotId: mark.id, rect: [343.004, 600, 443, 650] });
  assert.deepEqual([same.ok, same.noop, same.revision], [true, true, 5]);
  // Clamped to the page before the no-op test and the write.
  assert.equal(op(session, { op: 'redact-set', page: 0, annotId: mark.id, rect: [580, 600, 680, 650] }).ok, true);
  near(marks(session)[0].rect, [580, 600, 612, 650]);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  near(marks(session)[0].rect, [343, 600, 443, 650]);
  // save → reopen → rescan
  const reopened = reopenSaved(session);
  const [kept] = marks(reopened);
  near(kept.rect, [343, 600, 443, 650]);
  assert.equal(kept.search, undefined);
  assert.equal(op(reopened, { op: 'redact-preview' }).marks, 1);
});

test('P6 U3 redact-set refusals never dirty: bad rect, no such mark, not a Redact mark', async () => {
  const session = reopenSaved(await (async () => {
    const s = await opened();
    assert.equal(op(s, { op: 'redact-add', page: 0, rect: [400, 300, 500, 320] }).ok, true);
    return s;
  })());
  const [mark] = marks(session);
  const note = op(session, { op: 'annots', page: 0 }).items.find(item => item.type !== 'Redact');
  assert.equal(note.type, 'FreeText');
  for (const [request, detail] of [
    [{ annotId: mark.id, rect: [1, 1, 1.5, 4] }, 'rect'],
    [{ annotId: mark.id, rect: [700, 10, 800, 20] }, 'rect'],
    [{ annotId: mark.id, rect: [1, 2, 3] }, 'rect'],
    [{ annotId: 999999, rect: [10, 10, 40, 40] }, 'no-such-annot'],
    [{ annotId: note.id, rect: [10, 10, 40, 40] }, 'not-redact'],
  ]) {
    const refused = op(session, { op: 'redact-set', page: 0, ...request });
    assert.deepEqual([refused.ok, refused.reason, refused.detail, refused.revision, refused.dirty], [false, 'invalid', detail, 0, false], JSON.stringify(request));
  }
  near(marks(session)[0].rect, [400, 300, 500, 320]);
  const signed = await opened('p0b/signed-intact.pdf');
  assert.equal(op(signed, { op: 'redact-set', page: 0, annotId: 1, rect: [10, 10, 40, 40] }).reason, 'read-only');
});

test('P6 U3 a moved search mark becomes an area mark; apply redacts the new rect and keeps the old', async () => {
  const session = await opened('p6/direct3.pdf');
  const searched = op(session, { op: 'redact-search', text: 'Move me' });
  assert.deepEqual([searched.ok, searched.added], [true, 1]);
  const [mark] = marks(session);
  assert.equal(mark.search, true);
  const before = rawMark(session, mark.id);
  assert.deepEqual([before.quads, before.tag], [1, 'Move me']);
  // Onto "Alpha" (paragraph line 1, x 72..100, y-down 80..96), off "Move me".
  const moved = op(session, { op: 'redact-set', page: 0, annotId: mark.id, rect: [70, 80, 100, 96] });
  assert.equal(moved.ok, true, moved.error);
  const after = rawMark(session, mark.id);
  assert.deepEqual([after.quads, after.tag], [0, null], 'no QuadPoints and no /PDF4AllSearch');
  near(after.rect, [70, 80, 100, 96]);
  assert.equal(marks(session)[0].search, undefined);
  assert.equal(op(session, { op: 'redact-preview' }).marks, 1, 'the moved mark is counted once');

  const reopened = reopenSaved(session);
  near(marks(reopened)[0].rect, [70, 80, 100, 96]);
  assert.equal(marks(reopened)[0].search, undefined);
  const journal = engine.journalState(reopened);
  const applied = op(reopened, { op: 'redact-apply', sanitize: all(true) });
  assert.equal(applied.ok, true, applied.error);
  assert.deepEqual(applied.verified, { text: 1, pixels: 1, raw: 0 });
  assert.deepEqual(engine.journalState(reopened), journal);
  const output = mupdf.Document.openDocument(applied.bytes, 'application/pdf');
  try {
    const text = output.loadPage(0).toStructuredText().asText();
    assert.equal(text.includes('Alpha'), false, 'the text under the new rect is gone');
    assert.equal(text.includes('Move me'), true, 'the text under the old rect is kept');
  } finally { output.destroy(); }

  assert.equal(op(session, { op: 'undo' }).ok, true);
  const undone = rawMark(session, mark.id);
  assert.deepEqual([undone.rect, undone.quads, undone.tag], [before.rect, 1, 'Move me'], 'undo restores rect, quads and tag');
  assert.equal(marks(session)[0].search, true);
});

test('P6 U3 moving one of several search marks for the same text fails apply closed', async () => {
  const session = await opened();
  assert.equal(op(session, { op: 'redact-search', text: 'P0B-SECRET-7731' }).added, 8);
  // The "visible … here" hit is plain page content (y-down ≈ 39..56).
  const visible = marks(session).find(item => item.rect[1] < 60);
  assert.equal(op(session, { op: 'redact-set', page: 0, annotId: visible.id, rect: [400, 300, 500, 320] }).ok, true);
  assert.equal(op(session, { op: 'redact-preview' }).marks, 8);
  const journal = engine.journalState(session);
  const refused = op(session, { op: 'redact-apply', sanitize: all(true) });
  assert.deepEqual([refused.ok, refused.reason], [false, 'verify-failed'], refused.error);
  assert.deepEqual(engine.journalState(session), journal, 'a refused apply leaves the journal as it was');
});

test('P6 U3 annot-delete of a redaction mark leaves nothing to preview', async () => {
  const session = await opened('p6/direct3.pdf');
  assert.equal(op(session, { op: 'redact-add', page: 0, rect: [300, 600, 400, 650] }).ok, true);
  const [mark] = marks(session);
  assert.equal(op(session, { op: 'annot-delete', page: 0, annotId: mark.id }).ok, true);
  assert.equal(op(session, { op: 'redact-preview' }).marks, 0);
  assert.equal(marks(reopenSaved(session)).length, 0);
});

test('debt1 row 7: redact-set is pinned to the annotate permission', async () => {
  const { OP_PERMISSION } = await import('./rust-engine.mjs');
  assert.equal(OP_PERMISSION['redact-set'], 'annotate');
});
