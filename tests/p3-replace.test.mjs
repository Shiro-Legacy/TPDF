/**
 * P3 U5 — scoped find/replace: `edit-replace-preview` / `edit-replace-apply`.
 * Engine direct, no renderer UI.
 *
 * Expected values come from grok's `fixtures/p3/edit-replace.pdf` provenance and
 * the parent contract's rule that one writer (U2/U3) does every rewrite, in one
 * `mutate`, so a set of runs changes or none does.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';

const engine = createEngine(mupdf);

function toArrayBuffer(bytes) {
  if (bytes instanceof ArrayBuffer) return bytes;
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

let nextId = 1200;
async function openFile(path, session = engine.createSession()) {
  const reply = engine.dispatch(
    { id: 1, op: 'open', name: path, pdf: toArrayBuffer(await readFile(path)) },
    session,
  ).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function openBytes(bytes, session = engine.createSession(), name = 'scratch.pdf') {
  const reply = engine.dispatch({ id: nextId++, op: 'open', name, pdf: toArrayBuffer(bytes) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function op(session, request) {
  return engine.dispatch({ id: nextId++, ...request }, session).reply;
}

function scan(session, page = 0) {
  const reply = op(session, { op: 'edit-targets', page });
  assert.equal(reply.ok, true, reply.error);
  return reply;
}

function preview(session, find, replace, extra = {}) {
  const target = scan(session, extra.page ?? 0);
  return op(session, {
    op: 'edit-replace-preview', page: extra.page ?? 0,
    generation: target.generation, revision: target.revision, find, replace,
    ...(extra.targetId ? { targetId: extra.targetId } : {}),
  });
}

function apply(session, find, replace, previewReply, targetIds, extra = {}) {
  return op(session, {
    op: 'edit-replace-apply', page: extra.page ?? 0,
    generation: previewReply.generation, revision: previewReply.revision,
    find, replace, previewToken: previewReply.previewToken,
    targetIds: targetIds ?? previewReply.matches.map(match => match.targetId),
    ...(extra.targetId ? { targetId: extra.targetId } : {}),
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

/** One preview item's shape, so a whole list can be asserted in one line. */
function shape(reply) {
  return {
    matches: reply.matches.map(match => `${match.text} -> ${match.next} (${match.count})`),
    rejected: reply.rejected.map(item => `${item.text}:${item.detail}`),
  };
}

// -------------------------------------------------------------------- preview

test('U5 preview: edit-replace.pdf page 0 matches and rejects exactly as the provenance says', async () => {
  const session = await openFile('fixtures/p3/edit-replace.pdf');
  const reply = preview(session, 'ALPHA', 'BETA');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.deepEqual(shape(reply), {
    matches: ['ALPHA one -> BETA one (1)', 'ALPHA and ALPHA -> BETA and BETA (2)', 'GAMMA ALPHA -> GAMMA BETA (1)', 'ALPHABET -> BETABET (1)'],
    // Growth fits before the right-hand neighbour; the mismatched /ActualText stays refused.
    rejected: ['ALPHA four:ambiguous-text-association'],
  });
  assert.equal(reply.page, 0);
  assert.equal(reply.find, 'ALPHA');
  assert.equal(reply.replace, 'BETA');
  assert.ok(typeof reply.previewToken === 'string' && reply.previewToken.length > 0);
  // Read-only: no operation, no revision move.
  assert.equal(reply.revision, scan(session, 0).revision);
  assert.ok(scan(session, 0).items.some(item => item.text === 'NEIGHBOR'), 'the not-matching run is still listed');

  // p1 has its own ALPHA run; a page-scoped preview never sees it.
  const page1 = preview(session, 'ALPHA', 'BETA', { page: 1 });
  assert.deepEqual(shape(page1), { matches: ['ALPHA page two -> BETA page two (1)'], rejected: [] });
});

test('U5 preview: the selected-run scope looks at one run only', async () => {
  const session = await openFile('fixtures/p3/edit-replace.pdf');
  const reply = scan(session, 0);
  const run = reply.items.find(item => item.text === 'ALPHA and ALPHA');
  const scoped = preview(session, 'ALPHA', 'BETA', { targetId: run.targetId });
  assert.deepEqual(shape(scoped), { matches: ['ALPHA and ALPHA -> BETA and BETA (2)'], rejected: [] });
});

test('U5 preview: a CID run outside the page repertoire is rejected with its missing characters', async () => {
  const session = await openFile('fixtures/p0b/edit-subset.pdf');
  const reply = preview(session, '日本', '試験');
  assert.deepEqual(shape(reply), { matches: [], rejected: ['日本語テスト:unsupported-encoding'] });
  assert.deepEqual(reply.rejected[0].missing, ['試', '験']);
});

test('U5 preview: a shared placement is rejected, a plain one on the same page is a match', async () => {
  const session = await openFile('fixtures/p3/targets-streams.pdf');
  // The two FORM runs live inside Forms (COW placements); the nested one is not a candidate.
  assert.deepEqual(shape(preview(session, 'FORM', 'BLOCK')), {
    matches: [],
    rejected: ['FORM:shared-placement', 'FORM:shared-placement', 'FORM:shared-or-unproven-sharing'],
  });
  assert.deepEqual(shape(preview(session, 'SPLIT', 'JOIN')), {
    matches: ['SPLIT -> JOIN (1)'],
    rejected: [],
  });
});

test('U5 preview: a run with text following it grows and shifts that tail', async () => {
  const session = await openFile('fixtures/p3/edit-tail.pdf');
  const reply = preview(session, 'ALPHA', 'BETA');
  assert.deepEqual(shape(reply), { matches: ['ALPHA -> BETA (1)'], rejected: [] });
  // The same run replaced by a string of exactly its own width is a match: U5 reuses
  // the U2 width rule, and `ALPHA` and `AHPLA` measure the same.
  assert.deepEqual(shape(preview(session, 'ALPHA', 'AHPLA')), {
    matches: ['ALPHA -> AHPLA (1)'],
    rejected: [],
  });
});

// ---------------------------------------------------------------------- apply

test('U5 apply: only the ticked runs change, every other stream is byte-identical, one undo restores all', async () => {
  const session = await openFile('fixtures/p3/edit-replace.pdf');
  const page0 = contentsOf(session, 0);
  const page1 = contentsOf(session, 1);
  const reply = preview(session, 'ALPHA', 'BETA');
  const [first, second] = reply.matches;
  const applied = apply(session, 'ALPHA', 'BETA', reply, [first.targetId, second.targetId]);
  assert.equal(applied.ok, true, JSON.stringify(applied));
  assert.equal(applied.replaced, 2);
  const after = contentsOf(session, 0).join('\n');
  assert.ok(after.includes('(BETA one)'), after);
  assert.ok(after.includes('(BETA and BETA)'), after);
  assert.equal(after.includes('(BETAABET)'), false, 'an unticked match is untouched');
  assert.ok(after.includes('(ALPHABET)'), 'ALPHABET is unticked and unchanged');
  assert.deepEqual(contentsOf(session, 1), page1, 'the other page never changes');

  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.deepEqual(contentsOf(session, 0), page0, 'one undo restores every changed stream');
  assert.equal(op(session, { op: 'redo' }).ok, true);
  assert.equal(contentsOf(session, 0).join('\n'), after, 'redo reapplies the whole set');
});

test('U5 apply: a write that fails on the second plan rolls back the first (nothing written)', async () => {
  const session = await openFile('fixtures/p3/edit-replace.pdf');
  const before = contentsOf(session, 0);
  const reply = preview(session, 'ALPHA', 'BETA');
  const id = reply.matches[0].targetId;
  // The same id twice: the second plan rewrites a span the first already replaced, so
  // the mutate's write fails and the whole operation is abandoned (U1 Gate C).
  const refused = apply(session, 'ALPHA', 'BETA', reply, [id, id]);
  assert.equal(refused.ok, false);
  assert.equal(refused.detail, 'write-failed');
  assert.equal(refused.targetId, id);
  assert.deepEqual(contentsOf(session, 0), before, 'the first write was rolled back');
  assert.equal(op(session, { op: 'undo' }).canUndo, false, 'no operation was journaled');
});

test('U5 apply: a stale preview refuses and writes nothing', async () => {
  const session = await openFile('fixtures/p3/edit-replace.pdf');
  const before = contentsOf(session, 0);
  const reply = preview(session, 'ALPHA', 'BETA');
  // Any edit moves the revision on, so the preview's token no longer matches.
  const target = scan(session, 0).items.find(item => item.text === 'NEIGHBOR');
  assert.equal(op(session, {
    op: 'edit-text', page: 0, targetId: target.targetId,
    generation: scan(session, 0).generation, revision: scan(session, 0).revision, text: 'NEIGHBO',
  }).ok, true);
  const refused = apply(session, 'ALPHA', 'BETA', reply);
  assert.equal(refused.ok, false);
  assert.equal(refused.detail, 'stale-preview');
  assert.equal(contentsOf(session, 0).join('\n').includes('(BETA one)'), false);
  assert.ok(before.length > 0);

  // A token that does not match the recomputed preview is refused the same way.
  const fresh = preview(session, 'ALPHA', 'BETA');
  const wrong = op(session, {
    op: 'edit-replace-apply', page: 0, generation: fresh.generation, revision: fresh.revision,
    find: 'ALPHA', replace: 'BETA', previewToken: 'deadbeef', targetIds: [fresh.matches[0].targetId],
  });
  assert.equal(wrong.detail, 'stale-preview');
});

test('U5 apply: an id that is not in the preview is stale-target, and a bad shape is invalid', async () => {
  const session = await openFile('fixtures/p3/edit-replace.pdf');
  const target = scan(session, 0).items.find(item => item.text === 'NEIGHBOR');
  const reply = preview(session, 'ALPHA', 'BETA');
  const stranger = apply(session, 'ALPHA', 'BETA', reply, [target.targetId]);
  assert.equal(stranger.detail, 'stale-target');
  assert.equal(stranger.targetId, target.targetId);

  const empty = op(session, {
    op: 'edit-replace-apply', page: 0, generation: reply.generation, revision: reply.revision,
    find: 'ALPHA', replace: 'BETA', previewToken: reply.previewToken, targetIds: [],
  });
  assert.equal(empty.ok, false);
  assert.equal(empty.detail, 'invalid');
});

test('U5: preview refuses a bad request shape, and an encrypted document refuses apply but not preview', async () => {
  const session = await openFile('fixtures/p3/edit-replace.pdf');
  const target = scan(session, 0);
  for (const [find, replace, detail] of [['', 'BETA', 'invalid'], ['A'.repeat(257), 'B', 'invalid'], ['ALPHA', 'B'.repeat(4097), 'invalid']]) {
    const refused = op(session, {
      op: 'edit-replace-preview', page: 0, generation: target.generation, revision: target.revision, find, replace,
    });
    assert.equal(refused.ok, false, JSON.stringify([find.length, replace.length]));
    assert.equal(refused.detail, detail);
  }

  const locked = await openFile('fixtures/encrypted-noedit.pdf');
  // Inspection is allowed where copying is; editing is not. Unlock with the fixture password.
  assert.equal(op(locked, { op: 'authenticate', password: 'pdf4all-user' }).ok, true);
  const lockedTarget = scan(locked, 0);
  const lockedPreview = op(locked, {
    op: 'edit-replace-preview', page: 0, generation: lockedTarget.generation, revision: lockedTarget.revision,
    find: 'PDF4All', replace: 'PDF4ALL',
  });
  assert.equal(lockedPreview.ok, true, JSON.stringify(lockedPreview));
  const lockedApply = op(locked, {
    op: 'edit-replace-apply', page: 0, generation: lockedPreview.generation, revision: lockedPreview.revision,
    find: 'PDF4All', replace: 'PDF4ALL', previewToken: lockedPreview.previewToken,
    targetIds: lockedPreview.matches.map(match => match.targetId),
  });
  assert.equal(lockedApply.ok, false);
  assert.equal(lockedApply.reason, 'read-only');
});

// ------------------------------------------------------------ refactor parity

test('U5 refactor parity: edit-text still reports the same numbers and refuses the same characters', async () => {
  // The U2 width numbers the p3-edit suite asserts, re-checked through the shared
  // `editPlanText`/`editWriteText` path that find/replace now reuses.
  const session = await openFile('fixtures/p0b/edit-transformed.pdf');
  const target = scan(session).items.find(item => item.text === 'ALPHA');
  const reply = op(session, {
    op: 'edit-text', page: 0, targetId: target.targetId,
    generation: scan(session).generation, revision: scan(session).revision, text: 'BRAV',
  });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.ok(contentsOf(session, 0).join('\n').includes('(BRAV)'));

  // A CID run keeps the U3 repertoire refusal through the same validator.
  const cid = await openFile('fixtures/p0b/edit-subset.pdf');
  const cidTarget = scan(cid).items.find(item => item.text === '日本語テスト');
  const refused = op(cid, {
    op: 'edit-text', page: 0, targetId: cidTarget.targetId,
    generation: scan(cid).generation, revision: scan(cid).revision, text: '試験',
  });
  assert.equal(refused.detail, 'unsupported-encoding');
  assert.deepEqual(refused.missing, ['試', '験']);
});

// ------------------------------------------------------- amendments §7.1/7.3/7.5/7.6

const LATIN = 'latin1';
function stream(body) {
  const bytes = Buffer.isBuffer(body) ? body : Buffer.from(body, LATIN);
  return Buffer.concat([Buffer.from(`<< /Length ${bytes.length} >>\nstream\n`, LATIN), bytes, Buffer.from('endstream', LATIN)]);
}
function rawPdf(objects) {
  const head = Buffer.from('%PDF-1.4\n%\xe2\xe3\xcf\xd3\n', LATIN);
  const parts = [head];
  const offsets = [];
  let size = head.length;
  for (const [index, body] of objects.entries()) {
    const bytes = Buffer.isBuffer(body) ? body : Buffer.from(body, LATIN);
    const prefix = Buffer.from(`${index + 1} 0 obj\n`, LATIN);
    const suffix = Buffer.from('\nendobj\n', LATIN);
    offsets.push(size);
    parts.push(prefix, bytes, suffix);
    size += prefix.length + bytes.length + suffix.length;
  }
  const xref = size;
  let tail = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) tail += `${String(offset).padStart(10, '0')} 00000 n \n`;
  tail += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  parts.push(Buffer.from(tail, LATIN));
  return Buffer.concat(parts);
}
const HELVETICA = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>';
/** One Helvetica page whose content is exactly `content`. */
function onePage(content) {
  return rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
    HELVETICA,
    stream(content),
  ]);
}
/** One page with `count` unique runs, each `ALPHA NNNNN`, one per line. */
function manyRuns(count) {
  const lines = [];
  for (let index = 0; index < count; index++) {
    const text = `ALPHA ${String(index).padStart(5, '0')}`;
    lines.push(`BT /F1 12 Tf 72 ${720 - index} Td (${text}) Tj ET`);
  }
  return onePage(`${lines.join('\n')}\n`);
}

test('U5 §7.1: the projected replacement is bounded before next is built', async () => {
  const session = await openFile('fixtures/p0b/edit-subset.pdf');
  // 6 chars + 1 × (4096 − 2) = 4100 > 4096: refused as too-large, before the CID
  // repertoire gate (which would say unsupported-encoding) can judge the next text.
  const reply = preview(session, '日本', 'A'.repeat(4096));
  assert.deepEqual(shape(reply), { matches: [], rejected: ['日本語テスト:too-large'] });
});

test('U5 §7.1: the whole reply text is bounded, and the op refuses over the ceiling', async () => {
  // 20 runs × (text 12 + next 4009) is over MAX_EDIT_REPLY_TEXT_CHARS (65536).
  const session = openBytes(manyRuns(20));
  const reply = preview(session, 'ALPHA', 'B'.repeat(4000));
  assert.equal(reply.ok, false);
  assert.equal(reply.detail, 'too-large');
  assert.match(reply.error, /65536/);

  // Under both ceilings the same page previews all 20 runs (a one-character replacement).
  const small = preview(session, 'ALPHA', 'B');
  assert.equal(small.ok, true, JSON.stringify(small));
  assert.equal(small.matches.length, 20);
});

test('U5 §7.3: a 512-run page previews in under two seconds', async () => {
  const session = openBytes(manyRuns(512));
  const started = performance.now();
  const reply = preview(session, 'ALPHA', 'BETA');
  const elapsed = performance.now() - started;
  assert.equal(reply.ok, true, JSON.stringify(reply).slice(0, 200));
  assert.equal(reply.matches.length, 512);
  assert.ok(elapsed < 2000, `preview took ${Math.round(elapsed)} ms`);
});

test('U5 §7.5: count is non-overlapping left to right, like replaceAll', async () => {
  const single = openBytes(onePage('BT /F1 12 Tf 72 700 Td (AAA) Tj ET\n'));
  assert.deepEqual(shape(preview(single, 'AA', 'B')), { matches: ['AAA -> BA (1)'], rejected: [] });

  const double = openBytes(onePage('BT /F1 12 Tf 72 700 Td (AAAA) Tj ET\n'));
  assert.deepEqual(shape(preview(double, 'AA', 'B')), { matches: ['AAAA -> BB (2)'], rejected: [] });
});

test('U5 §7.6: a run scope that names an unknown target is stale-target', async () => {
  const session = await openFile('fixtures/p3/edit-replace.pdf');
  const target = scan(session, 0);
  const reply = op(session, {
    op: 'edit-replace-preview', page: 0, generation: target.generation, revision: target.revision,
    find: 'ALPHA', replace: 'BETA', targetId: 'p0.c9.1.deadbeef',
  });
  assert.equal(reply.ok, false);
  assert.equal(reply.detail, 'stale-target');
});

test('U5 §7.2: inherited /Resources is allowed in a batch and privatised once', async () => {
  // The page inherits /Resources from /Pages, so the scan marks it unproven: the two
  // runs carry a COW record but no Form graft, so a batch may edit them.
  const bytes = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 4 0 R >> >> >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>',
    HELVETICA,
    stream('BT /F1 12 Tf 72 700 Td (ALPHA one) Tj ET\nBT /F1 12 Tf 72 660 Td (ALPHA two) Tj ET\n'),
  ]);
  const session = openBytes(bytes);
  const reply = preview(session, 'ALPHA', 'BETA');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.deepEqual(shape(reply), { matches: ['ALPHA one -> BETA one (1)', 'ALPHA two -> BETA two (1)'], rejected: [] });
  const applied = apply(session, 'ALPHA', 'BETA', reply);
  assert.equal(applied.ok, true, JSON.stringify(applied));
  assert.equal(applied.replaced, 2);
  const before = contentsOf(session, 0)[0];
  assert.ok(contentsOf(session, 0).join('\n').includes('(BETA one)'));
  assert.ok(contentsOf(session, 0).join('\n').includes('(BETA two)'));
  assert.ok(before.includes('(BETA one)'));
  // The privatised copy lives on the page, and the page tree it inherited from is untouched.
  const pageObject = session.doc.loadPage(0).getObject();
  assert.ok(pageObject.get('Resources') !== null, 'the page owns a copied /Resources');
  assert.ok(String(pageObject.get('Resources').get('Font').get('F1').get('BaseFont')).includes('Helvetica'));
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentsOf(session, 0).join('\n').includes('(ALPHA one)'), true, 'undo restores the shared stream');
});

test('U5 §7.10: replace with an empty string empties the match', async () => {
  const session = await openFile('fixtures/p3/edit-replace.pdf');
  const reply = preview(session, 'ALPHA', '');
  assert.deepEqual(shape(reply), {
    matches: ['ALPHA one ->  one (1)', 'ALPHA and ALPHA ->  and  (2)', 'GAMMA ALPHA -> GAMMA  (1)', 'ALPHABET -> BET (1)'],
    rejected: ['ALPHA four:ambiguous-text-association'],
  });
  const applied = apply(session, 'ALPHA', '', reply);
  assert.equal(applied.ok, true, JSON.stringify(applied));
  assert.ok(contentsOf(session, 0).join('\n').includes('( one)'));
});

test('U5 §7.7: the preview token covers the scope as well as the page', async () => {
  const session = await openFile('fixtures/p3/edit-replace.pdf');
  const page = preview(session, 'ALPHA', 'BETA');
  const run = scan(session, 0).items.find(item => item.text === 'ALPHA and ALPHA');
  const scoped = preview(session, 'ALPHA', 'BETA', { targetId: run.targetId });
  assert.notEqual(page.previewToken, scoped.previewToken);
  // A page-scoped apply still refuses the run-scoped token.
  const refused = apply(session, 'ALPHA', 'BETA', scoped, [run.targetId]);
  assert.equal(refused.detail, 'stale-preview');
  // The same scope applies (review U5: the run scope was unappliable).
  const applied = apply(session, 'ALPHA', 'BETA', scoped, [run.targetId], { targetId: run.targetId });
  assert.equal(applied.ok, true, JSON.stringify(applied));
  assert.ok(contentsOf(session, 0).join('\n').includes('(BETA and BETA)'));
  assert.ok(contentsOf(session, 0).join('\n').includes('(ALPHA one)'));
});

test('U5 review: a /Contents stream shared by two pages is privatised once per batch; undo, redo and save hold', async () => {
  const bytes = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
    HELVETICA,
    stream('BT /F1 12 Tf 72 700 Td (ALPHA one) Tj ET\nBT /F1 12 Tf 72 660 Td (ALPHA two) Tj ET\n'),
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
  ]);
  const session = openBytes(bytes);
  const reply = preview(session, 'ALPHA', 'BETA');
  assert.deepEqual(shape(reply), { matches: ['ALPHA one -> BETA one (1)', 'ALPHA two -> BETA two (1)'], rejected: [] });
  const applied = apply(session, 'ALPHA', 'BETA', reply);
  assert.equal(applied.ok, true, JSON.stringify(applied));
  const edited = contentsOf(session, 0)[0];
  assert.ok(edited.includes('(BETA one)') && edited.includes('(BETA two)'), edited);
  assert.ok(contentsOf(session, 1)[0].includes('(ALPHA one)'), 'the other page keeps the original');
  assert.equal(op(session, { op: 'undo' }).ok, true);
  const original = contentsOf(session, 0)[0];
  assert.ok(original.includes('(ALPHA one)') && original.includes('(ALPHA two)'));
  assert.equal(op(session, { op: 'redo' }).ok, true);
  assert.equal(contentsOf(session, 0)[0], edited);
  // Undo, then the same batch again from a fresh preview, then save and reopen.
  assert.equal(op(session, { op: 'undo' }).ok, true);
  const again = apply(session, 'ALPHA', 'BETA', preview(session, 'ALPHA', 'BETA'));
  assert.equal(again.ok, true, JSON.stringify(again));
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const reopened = openBytes(saved.pdf ?? saved.bytes);
  assert.ok(contentsOf(reopened, 0)[0].includes('(BETA two)'));
  assert.ok(contentsOf(reopened, 1)[0].includes('(ALPHA two)'));
});
