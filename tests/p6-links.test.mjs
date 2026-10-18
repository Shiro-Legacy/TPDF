/**
 * P6 U4 §1.1-§1.3 (+ §9.1-§9.4): link identity (`annotId`) and `link-set`. Every write
 * is proven by save (the engine's own options, garbage=yes) -> reopen through the engine
 * -> `links`, because a page handle held open returns stale link data (probe P4) and a raw
 * `setBounds(pageRect)` reads back right live but reopens mirrored (probe P1).
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine, OP_PERMISSION } from './rust-engine.mjs';

const engine = createEngine(mupdf);
const LINKS_SHA = '0ac413b0fa9b42062542afbc970f0245bceacab0a789017c90a413733962d0bc';
const TABS_SHA = 'b90c5db2ed9d4b1221b453bc9df02682fd8a6c678724263738a68d24bc047326';
// links.provenance.json / p6-unit4-fixture.md: object numbers of L1, L2, L5, L3 (page 0) and L4 (page 1).
const L1 = 10, L2 = 11, L5 = 12, L3 = 13, L4 = 14;

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

function openBytes(bytes, name = 'scratch.pdf') {
  const session = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: bytes }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

async function open(path) {
  return openBytes(toArrayBuffer(await readFile(path)), path);
}

function op(session, request) {
  return engine.dispatch({ id: 900, ...request }, session).reply;
}

function linksOf(session, page) {
  const reply = op(session, { op: 'links', page });
  assert.equal(reply.ok, true, reply.error);
  return reply.items;
}

/** save (engine options) -> reopen through the engine; the old session is closed. */
function roundTrip(session) {
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  engine.closeDocument(session.doc);
  return openBytes(saved.bytes, 'roundtrip.pdf');
}

function near(actual, expected, message) {
  assert.equal(actual.length, expected.length, message);
  actual.forEach((value, index) => assert.ok(Math.abs(value - expected[index]) <= 0.01, `${message}: ${JSON.stringify(actual)} vs ${JSON.stringify(expected)}`));
}

test('P6 U4 fixtures are pinned and link-set needs annotate permission', async () => {
  const sha = async path => createHash('sha256').update(await readFile(path)).digest('hex');
  assert.equal(await sha('fixtures/p6/links.pdf'), LINKS_SHA);
  assert.equal(await sha('fixtures/p6/tabs.pdf'), TABS_SHA);
  assert.equal(OP_PERMISSION['link-set'], 'annotate');
});

test('links carry annotId (the /Annots object number); the bare L5 shifts nothing', async () => {
  const session = await open('fixtures/p6/links.pdf');
  try {
    const first = linksOf(session, 0);
    assert.deepEqual(first.map(item => [item.id, item.annotId, item.kind]), [[0, L1, 'uri'], [1, L2, 'page'], [2, L3, 'blocked']]);
    near(first[0].rect, [72, 72, 200, 92], 'L1 rect');
    near(first[1].rect, [72, 122, 200, 142], 'L2 rect');
    near(first[2].rect, [72, 172, 200, 192], 'L3 rect');
    assert.equal(first[1].page, 2);
    const rotated = linksOf(session, 1);
    assert.deepEqual(rotated.map(item => [item.id, item.annotId, item.uri]), [[0, L4, 'https://example.org/rot']]);
    near(rotated[0].rect, [700, 72, 720, 200], 'L4 page-space rect');
    assert.deepEqual(linksOf(session, 2), []);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('link-set rect on page 0 survives save -> reopen with the same annotId', async () => {
  let session = await open('fixtures/p6/links.pdf');
  try {
    const reply = op(session, { op: 'link-set', page: 0, annotId: L1, rect: [80, 80, 210, 100] });
    assert.equal(reply.ok, true, reply.error);
    assert.equal(reply.revision, 1);
    assert.equal(reply.dirty, true);
    assert.equal(reply.noop, undefined);
    assert.ok(Array.isArray(reply.rotations), 'editState reply, as link-add');
    session = roundTrip(session);
    const items = linksOf(session, 0);
    assert.deepEqual(items.map(item => item.annotId), [L1, L2, L3]);
    near(items[0].rect, [80, 80, 210, 100], 'moved L1 after reopen');
    assert.equal(items[0].uri, 'https://example.org/one', 'the target is untouched');
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('link-set retargets uri -> uri -> page -> uri, and the /Dest link L2 -> uri survives reopen', async () => {
  let session = await open('fixtures/p6/links.pdf');
  try {
    const steps = [
      [L1, { uri: 'https://example.org/two' }, { kind: 'uri', uri: 'https://example.org/two' }],
      [L1, { page: 1 }, { kind: 'page', page: 1 }],
      [L1, { uri: 'mailto:a@example.org' }, { kind: 'uri', uri: 'mailto:a@example.org' }],
      [L2, { uri: 'https://example.org/dest' }, { kind: 'uri', uri: 'https://example.org/dest' }],
    ];
    for (const [annotId, target, expected] of steps) {
      const reply = op(session, { op: 'link-set', page: 0, annotId, target });
      assert.equal(reply.ok, true, reply.error);
      session = roundTrip(session);
      const item = linksOf(session, 0).find(entry => entry.annotId === annotId);
      assert.ok(item, `annotId ${annotId} still listed`);
      assert.equal(item.kind, expected.kind, JSON.stringify(target));
      if (expected.page !== undefined) assert.equal(item.page, expected.page);
      if (expected.uri !== undefined) assert.equal(item.uri, expected.uri);
    }
    // uri -> page on L2 after its /Dest is gone, then back: one op each.
    assert.equal(op(session, { op: 'link-set', page: 0, annotId: L2, target: { page: 0 } }).ok, true);
    session = roundTrip(session);
    const back = linksOf(session, 0).find(entry => entry.annotId === L2);
    assert.deepEqual([back.kind, back.page], ['page', 0]);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('link-set refusals carry one token each and never dirty', async () => {
  const session = await open('fixtures/p6/links.pdf');
  const tabs = await open('fixtures/p6/tabs.pdf');
  try {
    const cases = [
      [session, { page: 0, annotId: L1 }, 'target', 'link-set: rect or target is required'],
      [session, { page: 0, annotId: L1, rect: [72, 900, 200, 930] }, 'rect', /off the page/],
      [session, { page: 0, annotId: L1, rect: [72, 80, 72, 100] }, 'rect', /positive area/],
      [session, { page: 0, annotId: L1, rect: [72, 80, 200] }, 'rect', /4 finite numbers/],
      [session, { page: 0, annotId: 0, rect: [80, 80, 210, 100] }, 'no-such-annot', 'link-set: page 0 has no link 0'],
      [session, { page: 0, annotId: 999, rect: [80, 80, 210, 100] }, 'no-such-annot', 'link-set: page 0 has no link 999'],
      [session, { page: 0, annotId: 'x', rect: [80, 80, 210, 100] }, 'no-such-annot', /has no link x/],
      [session, { page: 0, annotId: L5, rect: [80, 80, 210, 100] }, 'no-such-annot', `link-set: page 0 has no link ${L5}`],
      [session, { page: 0, annotId: L4, rect: [80, 80, 210, 100] }, 'no-such-annot', 'L4 is on page 1'],
      [tabs, { page: 0, annotId: 13, rect: [80, 80, 210, 100] }, 'not-link', 'link-set: annotation 13 is not a link'],
      [session, { page: 0, annotId: L1, target: {} }, 'target', 'link-set: target must be {page} or {uri}'],
      [session, { page: 0, annotId: L1, target: null }, 'target', 'link-set: target must be {page} or {uri}'],
      [session, { page: 0, annotId: L1, target: { page: 9 } }, 'target', 'link-set: page must be an integer in 0..2'],
      [session, { page: 0, annotId: L1, target: { uri: 'javascript:alert(1)' } }, 'scheme', 'link-set: javascript: is not a link this app opens'],
      [session, { page: 0, annotId: L1, target: { uri: 'file:///etc/hosts' } }, 'scheme', 'link-set: file: is not a link this app opens'],
      [session, { page: 0, annotId: L1, target: { uri: 'ftp://nope' } }, 'scheme', 'link-set: ftp: is not a link this app opens'],
      [session, { page: 0, annotId: L1, target: { uri: '' } }, 'scheme', 'link-set: a URI without a scheme is not a link this app opens'],
      [session, { page: 0, annotId: L1, target: { uri: '  https://example.org/pad ' } }, 'scheme', 'link-set: a URI without a scheme is not a link this app opens'],
      [session, { page: 0, annotId: L1, target: { uri: `https://${'x'.repeat(2041)}` } }, 'scheme', 'link-set: a link URI holds at most 2048 characters'],
    ];
    for (const [target, request, token, message] of cases) {
      const reply = op(target, { op: 'link-set', ...request });
      const label = JSON.stringify(request).slice(0, 120);
      assert.equal(reply.ok, false, label);
      assert.equal(reply.reason, 'invalid', label);
      assert.equal(reply.detail, token, label);
      if (typeof message === 'string' && message.startsWith('link-set')) assert.equal(reply.error, message, label);
      else if (message instanceof RegExp) assert.match(reply.error, message, label);
      assert.equal(reply.dirty, false, label);
      assert.equal(reply.revision, 0, label);
    }
    // A page out of range is checkPage's plain Error (no token), as redact-set.
    const badPage = op(session, { op: 'link-set', page: 7, annotId: L1, rect: [80, 80, 210, 100] });
    assert.equal(badPage.ok, false);
    assert.equal(badPage.detail, undefined);
    assert.equal(op(session, { op: 'undo' }).ok, false, 'nothing reached the journal');
    session.doc && linksOf(session, 0).forEach((item, index) => assert.equal(item.annotId, [L1, L2, L3][index]));
  } finally {
    engine.closeDocument(session.doc);
    engine.closeDocument(tabs.doc);
  }
});

test('link-set no-op writes nothing; a rect-only move of the blocked link is allowed', async () => {
  let session = await open('fixtures/p6/links.pdf');
  try {
    const same = op(session, { op: 'link-set', page: 0, annotId: L1, rect: [72.004, 72, 200, 92], target: { uri: 'https://example.org/one' } });
    assert.equal(same.ok, true, same.error);
    assert.equal(same.noop, true);
    assert.equal(same.dirty, false);
    assert.equal(same.revision, 0);
    const samePage = op(session, { op: 'link-set', page: 0, annotId: L2, target: { page: 2 } });
    assert.deepEqual([samePage.noop, samePage.revision, samePage.dirty], [true, 0, false]);
    assert.equal(op(session, { op: 'undo' }).ok, false, 'the no-op journaled nothing');
    const blocked = op(session, { op: 'link-set', page: 0, annotId: L3, rect: [72, 200, 200, 220] });
    assert.equal(blocked.ok, true, blocked.error);
    session = roundTrip(session);
    const item = linksOf(session, 0).find(entry => entry.annotId === L3);
    near(item.rect, [72, 200, 200, 220], 'moved blocked link');
    assert.deepEqual([item.kind, item.uri], ['blocked', 'ftp://example.org/x']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('undo restores rect AND target of one link-set; redo re-applies', async () => {
  let session = await open('fixtures/p6/links.pdf');
  try {
    const reply = op(session, { op: 'link-set', page: 0, annotId: L2, rect: [300, 300, 400, 330], target: { uri: 'https://example.org/both' } });
    assert.equal(reply.ok, true, reply.error);
    assert.equal(reply.revision, 1);
    const undo = op(session, { op: 'undo' });
    assert.equal(undo.step, 'edit link');
    let item = linksOf(session, 0)[1];
    near(item.rect, [72, 122, 200, 142], 'undo restores the rect');
    assert.deepEqual([item.annotId, item.kind, item.page], [L2, 'page', 2]);
    assert.equal(op(session, { op: 'redo' }).step, 'edit link');
    session = roundTrip(session);
    item = linksOf(session, 0).find(entry => entry.annotId === L2);
    near(item.rect, [300, 300, 400, 330], 'redo re-applies the rect');
    assert.deepEqual([item.kind, item.uri], ['uri', 'https://example.org/both']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('rotated page 1: a page-space rect in reads back the same after save -> reopen (P1)', async () => {
  let session = await open('fixtures/p6/links.pdf');
  try {
    const rect = [650, 100, 690, 250];
    const reply = op(session, { op: 'link-set', page: 1, annotId: L4, rect });
    assert.equal(reply.ok, true, reply.error);
    session = roundTrip(session);
    const [item] = linksOf(session, 1);
    assert.equal(item.annotId, L4);
    near(item.rect, rect, 'rotated page rect');
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('link-delete of the middle link keeps the other annotIds; ids renumber', async () => {
  let session = await open('fixtures/p6/links.pdf');
  try {
    assert.equal(op(session, { op: 'link-delete', page: 0, linkId: 1 }).ok, true);
    session = roundTrip(session);
    assert.deepEqual(linksOf(session, 0).map(item => [item.id, item.annotId]), [[0, L1], [1, L3]]);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('linkPairs: inline, reversed-rect, /AA-only and JavaScript links pair by rect (§9.1)', () => {
  const uri = target => `/A << /S /URI /URI (${target}) >>`;
  const session = openBytes(buildPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [<< /Type /Annot /Subtype /Link /Rect [72 740 200 760] ${uri('https://example.org/inline')} >> 4 0 R 5 0 R 6 0 R 7 0 R] >>`,
    `<< /Type /Annot /Subtype /Link /Rect [200 720 72 700] ${uri('https://example.org/reversed')} >>`,
    '<< /Type /Annot /Subtype /Link /Rect [72 650 200 670] /AA << /U << /S /URI /URI (https://example.org/aa) >> >> >>',
    '<< /Type /Annot /Subtype /Link /Rect [72 600 200 620] /A << /S /JavaScript /JS (app.alert\\(1\\)) >> >>',
    `<< /Type /Annot /Subtype /Link /Rect [72 550 200 570] ${uri('https://example.org/last')} >>`,
  ]));
  try {
    const items = linksOf(session, 0);
    assert.deepEqual(items.map(item => item.annotId), [0, 4, 5, 7]);
    const inline = op(session, { op: 'link-set', page: 0, annotId: 0, rect: [80, 30, 200, 50] });
    assert.deepEqual([inline.ok, inline.detail], [false, 'no-such-annot']);
    const js = op(session, { op: 'link-set', page: 0, annotId: 6, rect: [80, 30, 200, 50] });
    assert.deepEqual([js.ok, js.detail], [false, 'no-such-annot'], 'an unlisted JavaScript link is not editable');
    const moved = op(session, { op: 'link-set', page: 0, annotId: 7, rect: [80, 400, 200, 420] });
    assert.equal(moved.ok, true, moved.error);
    const after = roundTrip(session);
    try {
      const reread = linksOf(after, 0);
      assert.deepEqual(reread.map(item => item.annotId), [0, 4, 5, 7]);
      near(reread[3].rect, [80, 400, 200, 420], 'last link moved');
    } finally {
      engine.closeDocument(after.doc);
    }
  } catch (err) {
    engine.closeDocument(session.doc);
    throw err;
  }
});
