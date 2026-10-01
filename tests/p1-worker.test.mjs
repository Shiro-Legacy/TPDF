/**
 * P1 worker/engine tests. The worker itself is a browser global (`self`), so
 * the runnable logic it uses — src/renderer/engine.ts — is imported here
 * against Node's `mupdf` exactly as the worker imports ./vendor/mupdf.js.
 * The never-throws contract of `dispatch` is what worker.onmessage posts.
 */
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import {
  createEngine,
  createGate,
  createLatestGuard,
  createRenderCache,
  MAX_PASSWORD_CHARS,
  parsePageRange,
  THUMB_CACHE_BYTES,
  THUMB_CACHE_ENTRIES,
  THUMB_MAX_EDGE,
  THUMB_MIN_EDGE,
  visibleRange,
} from '../build/renderer/engine.js';
import * as policy from '../build/policy.js';

const engine = createEngine(mupdf);
const LARGE = 'large';

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

async function openFixture(id) {
  const bytes = await readFile(`fixtures/${id}.pdf`);
  return { ...engine.openDocument(toArrayBuffer(bytes)), bytes };
}

test('every fixture opens with the page count and digest recorded in provenance', async () => {
  const names = (await readdir('fixtures')).filter((name) => name.endsWith('.provenance.json'));
  assert.ok(names.length >= 6, `expected the fixture set, found ${names.join(', ')}`);
  for (const name of names) {
    const provenance = JSON.parse(await readFile(`fixtures/${name}`, 'utf8'));
    const bytes = await readFile(provenance.file);
    assert.equal(bytes.length, provenance.bytes, `${name} size drifted from its provenance record`);
    assert.equal(
      createHash('sha256').update(bytes).digest('hex'),
      provenance.sha256,
      `${name} sha256 drifted - regenerate with ${provenance.generator.reproduce}`,
    );
    if (!String(provenance.file).endsWith('.pdf')) continue;
    const opened = engine.openDocument(toArrayBuffer(bytes));
    try {
      assert.equal(opened.doc.countPages(), provenance.pages, name);
      assert.ok(Number.isFinite(opened.openMs), name);
    } finally {
      engine.closeDocument(opened.doc);
    }
  }
});

test('info is all-false/0 for plain fixtures and reports form and encryption facts', async () => {
  const empty = {
    encrypted: false, needsPassword: false, algorithm: 'none', repaired: false, signatures: 0, certified: false,
    xfa: false, attachments: 0, openAction: false, tagged: false, versions: 0, printable: true,
    permissions: { print: true, copy: true, edit: true, annotate: true, form: true },
  };
  for (const id of ['hello', 'large', 'rotated', 'cjk', 'rtl']) {
    const { doc } = await openFixture(id);
    try {
      assert.deepEqual(engine.detect(doc), empty, id);
    } finally {
      engine.closeDocument(doc);
    }
  }
  const form = await openFixture('form');
  try {
    const info = engine.detect(form.doc);
    assert.equal(info.xfa, false);
    assert.equal(info.signatures, 0);
    assert.equal(info.certified, false);
    assert.deepEqual(engine.detect(form.doc), empty);
  } finally {
    engine.closeDocument(form.doc);
  }
  const encrypted = await openFixture('encrypted');
  try {
    const info = engine.detect(encrypted.doc);
    assert.equal(info.encrypted, true);
    assert.equal(info.needsPassword, true);
  } finally {
    engine.closeDocument(encrypted.doc);
  }
});

test('render paints the first and last page of large.pdf nonblank, and honours scale', async () => {
  const session = engine.createSession();
  const opened = engine.dispatch({ id: 1, op: 'open', name: 'large.pdf', pdf: toArrayBuffer(await readFile('fixtures/large.pdf')) }, session);
  assert.equal(opened.reply.ok, true, opened.reply.error);
  assert.equal(opened.reply.pageCount, 500);
  assert.equal(opened.reply.pages.length, 500);
  assert.deepEqual(opened.reply.pages[0], { width: 612, height: 792 });
  for (const page of [0, 499]) {
    const { reply } = engine.dispatch({ id: 10 + page, op: 'render', page, scale: 1 }, session);
    assert.equal(reply.ok, true, reply.error);
    assert.equal(reply.id, 10 + page, 'reply echoes the request id');
    assert.equal(reply.page, page);
    assert.equal(reply.pixelWidth, 612);
    assert.equal(reply.pixelHeight, 792);
    assert.equal(reply.pixels.byteLength, 612 * 792 * 4);
    assert.equal(reply.text.includes('PDF4All'), true, `page ${page} text`);
    assert.ok(reply.lines.length > 0, `page ${page} lines`);
    assert.ok(reply.nonWhite > 100 && reply.nonWhite < 612 * 792 / 2, `page ${page} nonWhite ${reply.nonWhite}`);
    assert.ok(Number.isFinite(reply.renderMs) && reply.renderMs >= 0);
  }
  const doubled = engine.dispatch({ id: 30, op: 'render', page: 0, scale: 2 }, session);
  assert.equal(doubled.reply.ok, true, doubled.reply.error);
  assert.equal(doubled.reply.pixelWidth, 1224);
  assert.equal(doubled.reply.pixelHeight, 1584);
  assert.equal(engine.dispatch({ id: 31, op: 'close' }, session).reply.ok, true);
  assert.equal(session.doc, null);
});

test('dispatch answers every bad request with an error reply, never a throw', async () => {
  const session = engine.createSession();
  const cases = [
    ['open: 5 junk bytes', { id: 1, op: 'open', name: 'junk.pdf', pdf: toArrayBuffer(randomBytes(5)) }, false],
    ['open: empty bytes', { id: 2, op: 'open', name: 'empty.pdf', pdf: new ArrayBuffer(0) }, false],
    ['render: nothing open', { id: 3, op: 'render', page: 0, scale: 1 }, false, /no document open/],
    ['thumbnail: nothing open', { id: 4, op: 'thumbnail', page: 0, maxEdge: THUMB_MAX_EDGE }, false, /no document open/],
    ['close: nothing open', { id: 5, op: 'close' }, true],
    ['unknown op', { id: 6, op: 'shred' }, false],
  ];
  for (const [label, request, expectedOk, pattern] of cases) {
    const { reply } = engine.dispatch(request, session);
    assert.equal(reply.ok, expectedOk, label);
    assert.equal(reply.id, request.id, label);
    if (reply.ok === false) {
      assert.equal(typeof reply.error, 'string', label);
      if (pattern) assert.match(reply.error, pattern, label);
    }
  }

  // A document that is too large for the ceiling is rejected without being parsed.
  const huge = new Proxy(new ArrayBuffer(4), {
    get: (target, key) => (key === 'byteLength' ? engine.maxPdfBytes + 1 : Reflect.get(target, key)),
  });
  const oversized = engine.dispatch({ id: 7, op: 'open', name: 'huge.pdf', pdf: huge }, session);
  assert.equal(oversized.reply.ok, false);
  assert.match(oversized.reply.error, /ceiling/);
  assert.equal(engine.maxPdfBytes, policy.MAX_PDF_BYTES, 'engine ceiling drifted from src/policy.ts');

  const { reply: opened } = engine.dispatch({ id: 8, op: 'open', name: 'large.pdf', pdf: toArrayBuffer(await readFile('fixtures/large.pdf')) }, session);
  assert.equal(opened.ok, true, opened.error);
  const rows = [
    ['render: page 500 of 500', { id: 9, op: 'render', page: 500, scale: 1 }],
    ['render: page -1', { id: 10, op: 'render', page: -1, scale: 1 }],
    ['render: fractional page', { id: 11, op: 'render', page: 1.5, scale: 1 }],
    ['render: scale 5', { id: 12, op: 'render', page: 0, scale: 5 }],
    ['render: scale 0.1', { id: 13, op: 'render', page: 0, scale: 0.1 }],
    ['render: scale NaN', { id: 14, op: 'render', page: 0, scale: Number.NaN }],
    ['open: missing name', { id: 15, op: 'open', pdf: toArrayBuffer(await readFile('fixtures/hello.pdf')) }],
    ['thumbnail: page 500 of 500', { id: 16, op: 'thumbnail', page: 500, maxEdge: THUMB_MAX_EDGE }, /page/],
    ['thumbnail: page -1', { id: 17, op: 'thumbnail', page: -1, maxEdge: THUMB_MAX_EDGE }, /page/],
    ['thumbnail: fractional page', { id: 18, op: 'thumbnail', page: 0.5, maxEdge: THUMB_MAX_EDGE }, /page/],
    ['thumbnail: page NaN', { id: 19, op: 'thumbnail', page: Number.NaN, maxEdge: THUMB_MAX_EDGE }, /page/],
    ['search: empty query', { id: 20, op: 'search', query: '' }],
    ['search: whitespace query', { id: 21, op: 'search', query: '   ' }],
    ['search: 300-char query', { id: 22, op: 'search', query: 'a'.repeat(300) }],
    ['search: missing query', { id: 23, op: 'search' }],
  ];
  for (const [index, maxEdge] of [31, 257, 32.5, 0, -256, Number.NaN, Infinity, '256', null, undefined].entries()) {
    rows.push([
      `thumbnail: maxEdge ${String(maxEdge)}`,
      { id: 30 + index, op: 'thumbnail', page: 0, maxEdge },
      /maxEdge must be an integer/,
    ]);
  }
  for (const [label, request, pattern] of rows) {
    const { reply } = engine.dispatch(request, session);
    assert.equal(reply.ok, false, label);
    assert.equal(reply.id, request.id, label);
    assert.equal(typeof reply.error, 'string', label);
    if (pattern) assert.match(reply.error, pattern, label);
  }
  // A failed open must not drop the document that was already open.
  assert.equal(session.doc !== null, true, 'previous document survived a failed open');

  // An encrypted document refuses all three read ops until a password arrives.
  const locked = engine.dispatch({ id: 60, op: 'open', name: 'encrypted.pdf', pdf: toArrayBuffer(await readFile('fixtures/encrypted.pdf')) }, session);
  assert.equal(locked.reply.ok, true, locked.reply.error);
  for (const [label, request] of [
    ['render', { id: 61, op: 'render', page: 0, scale: 1 }],
    ['thumbnail', { id: 62, op: 'thumbnail', page: 0, maxEdge: THUMB_MAX_EDGE }],
    ['search', { id: 63, op: 'search', query: 'x' }],
  ]) {
    const { reply } = engine.dispatch(request, session);
    assert.equal(reply.ok, false, label);
    assert.match(reply.error, /encrypted/, label);
  }
  engine.closeDocument(session.doc);
});

/** Minimal one-page PDF with a single /FT /Sig widget, optionally signed (/V present). */
function pdfWithSignatureField(signed) {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>',
    '<< /Length 2 >>\nstream\nq \nendstream',
    '<< /Fields [6 0 R] /SigFlags 3 >>',
    `<< /Type /Annot /Subtype /Widget /FT /Sig /T (sig1) /Rect [72 72 300 120] /P 3 0 R${signed ? ' /V 7 0 R' : ''} >>`,
    ...(signed ? ['<< /Type /Sig /Filter /Adobe.PPKLite >>'] : []),
  ]);
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

test('signatures counts signed fields only', () => {
  for (const [signed, expected] of [[false, 0], [true, 1]]) {
    const { doc } = engine.openDocument(pdfWithSignatureField(signed));
    try {
      const info = engine.detect(doc);
      assert.equal(info.signatures, expected, `signed=${signed}`);
      assert.equal(info.certified, false);
      // A blank signature field must not make the document read-only.
      assert.equal(info.signatures > 0 || info.certified || info.needsPassword, signed);
    } finally {
      engine.closeDocument(doc);
    }
  }
});

test('open drops the request bytes once mupdf has copied them into the WASM heap', async () => {
  const session = engine.createSession();
  const request = { id: 1, op: 'open', name: 'hello.pdf', pdf: toArrayBuffer(await readFile('fixtures/hello.pdf')) };
  const { reply } = engine.dispatch(request, session);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(request.pdf, null, 'the worker must not retain a JS copy of the document');
  assert.ok(session.doc, 'the document itself is still open');
  engine.closeDocument(session.doc);
});

test('search finds text in page order and honours the hit budget', async () => {
  const session = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'search', query: 'PDF4All' }, session).reply.ok, false, 'search needs a document open');

  const hello = engine.dispatch({ id: 2, op: 'open', name: 'hello.pdf', pdf: toArrayBuffer(await readFile('fixtures/hello.pdf')) }, session);
  assert.equal(hello.reply.ok, true, hello.reply.error);
  // hello.pdf carries "PDF4All" only; the P0-era fixture never contained "Hello".
  const helloHits = engine.dispatch({ id: 3, op: 'search', query: 'PDF4All' }, session).reply;
  assert.equal(helloHits.ok, true, helloHits.error);
  assert.ok(helloHits.total >= 1, 'hello.pdf finds PDF4All');
  assert.equal(helloHits.hits[0].page, 0);
  assert.equal(helloHits.hits[0].quads.length, 1);
  assert.equal(helloHits.hits[0].quads[0].length, 8);
  for (const value of helloHits.hits[0].quads[0]) assert.ok(Number.isFinite(value), 'quad entries are finite numbers');
  assert.equal(helloHits.truncated, false);
  assert.ok(Number.isFinite(helloHits.searchMs) && helloHits.searchMs >= 0);
  assert.equal(engine.dispatch({ id: 4, op: 'search', query: 'Hello' }, session).reply.total, 0, 'no "Hello" in the fixture');

  const cjk = engine.dispatch({ id: 5, op: 'open', name: 'cjk.pdf', pdf: toArrayBuffer(await readFile('fixtures/cjk.pdf')) }, session);
  assert.equal(cjk.reply.ok, true, cjk.reply.error);
  const cjkHits = engine.dispatch({ id: 6, op: 'search', query: '日本語テスト' }, session).reply;
  assert.equal(cjkHits.ok, true, cjkHits.error);
  assert.equal(cjkHits.hits.length, 1, 'cjk needle found exactly once');
  assert.equal(cjkHits.hits[0].page, 0);

  const large = engine.dispatch({ id: 7, op: 'open', name: 'large.pdf', pdf: toArrayBuffer(await readFile('fixtures/large.pdf')) }, session);
  assert.equal(large.reply.ok, true, large.reply.error);
  const budget = engine.dispatch({ id: 8, op: 'search', query: 'PDF4All', maxHits: 3 }, session).reply;
  assert.equal(budget.ok, true, budget.error);
  assert.equal(budget.hits.length, 3);
  assert.equal(budget.total, 3);
  assert.equal(budget.truncated, true);
  assert.deepEqual(budget.hits.map(hit => hit.page), [0, 0, 1], 'hits come back in page order');
  console.log(JSON.stringify({ suite: 'p1-search-budget3', searchMs: budget.searchMs }));
  engine.closeDocument(session.doc);
});

test('thumbnail renders a bounded, aspect-correct pixmap and transfers only pixels', async () => {
  for (const id of ['hello', 'large', 'rotated']) {
    const session = engine.createSession();
    const opened = engine.dispatch({ id: 1, op: 'open', name: `${id}.pdf`, pdf: toArrayBuffer(await readFile(`fixtures/${id}.pdf`)) }, session);
    assert.equal(opened.reply.ok, true, opened.reply.error);
    const page = id === 'large' ? 499 : 0;
    const size = opened.reply.pages[page];
    // The two bounds themselves are legal; 32 and 256 come from the constants.
    const result = engine.dispatch({ id: 2, op: 'thumbnail', page, maxEdge: THUMB_MAX_EDGE }, session);
    const reply = result.reply;
    assert.equal(reply.ok, true, `${id}: ${reply.error}`);
    assert.equal(reply.page, page, `${id} echoes the page`);
    assert.deepEqual([reply.width, reply.height], [size.width, size.height], `${id} keeps the page size`);
    assert.ok(reply.pixelWidth <= 256 && reply.pixelHeight <= 256, `${id} stays inside maxEdge`);
    assert.ok(reply.pixelWidth >= 250 || reply.pixelHeight >= 250, `${id} uses the maxEdge it was given`);
    assert.equal(Math.max(reply.pixelWidth, reply.pixelHeight), 256, `${id} pins the long edge to maxEdge`);
    assert.ok(
      Math.abs(reply.pixelWidth / reply.pixelHeight - size.width / size.height) < 0.02,
      `${id} keeps the aspect ratio (${reply.pixelWidth}x${reply.pixelHeight} vs ${size.width}x${size.height})`,
    );
    assert.equal(reply.pixels.byteLength, reply.pixelWidth * reply.pixelHeight * 4, `${id} one RGBA pixel each`);
    assert.deepEqual(result.transfer, [reply.pixels], `${id} transfers the pixmap, not a copy of the document`);
    assert.equal('text' in reply, false, `${id} carries no structured text`);
    assert.equal('lines' in reply, false, `${id} carries no text lines`);
    // A thumbnail must never disturb the page-render cache or the session document.
    assert.equal(session.doc.countPages(), opened.reply.pageCount, `${id} document untouched`);
    assert.equal(engine.dispatch({ id: 3, op: 'thumbnail', page, maxEdge: THUMB_MIN_EDGE }, session).reply.pixelHeight, THUMB_MIN_EDGE, `${id} honours the lower bound`);
    engine.closeDocument(session.doc);
  }
});

test('thumbnail ignores MIN_SCALE while the full page render still enforces it', () => {
  // 2000 x 1200 pt: the 256 px thumbnail needs scale 0.128, under MIN_SCALE (0.25).
  const wide = [
    '%PDF-1.4',
    '1 0 obj <</Type/Catalog/Pages 2 0 R>> endobj',
    '2 0 obj <</Type/Pages/Kids[3 0 R]/Count 1>> endobj',
    '3 0 obj <</Type/Page/Parent 2 0 R/MediaBox[0 0 2000 1200]/Contents 4 0 R>> endobj',
    '4 0 obj <</Length 0>> stream\nendstream endobj',
    'trailer <</Root 1 0 R/Size 5>>',
    '%%EOF',
    '',
  ].join('\n');
  const session = engine.createSession();
  const opened = engine.dispatch({ id: 1, op: 'open', name: 'wide.pdf', pdf: toArrayBuffer(Buffer.from(wide, 'latin1')) }, session);
  assert.equal(opened.reply.ok, true, opened.reply.error);
  assert.deepEqual(opened.reply.pages[0], { width: 2000, height: 1200 });

  const small = engine.dispatch({ id: 2, op: 'render', page: 0, scale: 0.128 }, session).reply;
  assert.equal(small.ok, false, 'a full page render may not go under MIN_SCALE');
  assert.match(small.error, /scale must be between/);

  const thumb = engine.dispatch({ id: 3, op: 'thumbnail', page: 0, maxEdge: 256 }, session).reply;
  assert.equal(thumb.ok, true, thumb.error);
  assert.deepEqual([thumb.pixelWidth, thumb.pixelHeight], [256, 154], 'the thumbnail still fits inside maxEdge');
  engine.closeDocument(session.doc);
});

test('visibleRange covers the viewport plus buffer and clamps at both ends', () => {
  const tops = Array.from({ length: 10 }, (_, i) => i * 100);
  const heights = tops.map(() => 100);

  assert.deepEqual(visibleRange(0, 300, [], [], 4), { start: 0, end: 0 }, 'empty list');
  assert.deepEqual(visibleRange(0, 300, tops, heights, 0), { start: 0, end: 3 }, 'exactly the viewport');
  assert.deepEqual(visibleRange(0, 300, tops, heights, 4), { start: 0, end: 7 }, 'the buffer clamps at the first page');
  assert.deepEqual(visibleRange(250, 300, tops, heights, 1), { start: 1, end: 7 }, 'a partly scrolled viewport');
  assert.deepEqual(visibleRange(900, 300, tops, heights, 4), { start: 5, end: 10 }, 'the buffer clamps at the last page');
  assert.deepEqual(visibleRange(2000, 300, tops, heights, 8), { start: 0, end: 0 }, 'scrolled past the end is empty');
  assert.deepEqual(visibleRange(0, 5000, tops, heights, 2), { start: 0, end: 10 }, 'a viewport taller than the list');
  assert.deepEqual(visibleRange(0, 300, tops, heights, -5), { start: 0, end: 3 }, 'a negative buffer is zero');
  assert.deepEqual(visibleRange(NaN, NaN, tops, heights, 1), { start: 0, end: 0 }, 'garbage offsets stay in range');
  assert.deepEqual(visibleRange(0, 300, tops.slice(0, 3), [100, 100], 4), { start: 0, end: 2 }, 'the shorter array wins');

  // A 500-page list: one call per scroll frame, index range, never the whole list.
  const tall = Array.from({ length: 500 }, (_, i) => i * 88);
  const tallHeights = tall.map(() => 88);
  const middle = visibleRange(20_000, 704, tall, tallHeights, 4);
  assert.ok(middle.start <= 227 && middle.end >= 236, `the window covers the pages on screen: ${middle.start}..${middle.end}`);
  assert.ok(middle.end - middle.start <= 9 + 2 * 4, 'visible rows plus both buffers');
  assert.ok(middle.start > 200 && middle.end < 300, `a scrolled list resolves to a small window: ${middle.start}..${middle.end}`);
});

test('the thumbnail cache stays inside its entry and byte bounds', () => {
  const MiB = 1024 * 1024;
  assert.equal(THUMB_CACHE_ENTRIES, 64);
  assert.equal(THUMB_CACHE_BYTES, 24 * MiB);
  const cache = createRenderCache(THUMB_CACHE_ENTRIES, THUMB_CACHE_BYTES);
  const thumb = (pixels) => ({ pixels: new ArrayBuffer(pixels) });

  // 256x256 RGBA thumbs are 256 KiB each: 64 fit in 24 MiB, so the entry cap binds.
  for (let i = 0; i < THUMB_CACHE_ENTRIES + 1; i++) cache.set(`p${i}`, thumb(256 * 1024));
  assert.equal(cache.size(), THUMB_CACHE_ENTRIES);
  assert.equal(cache.get('p0'), undefined, 'the oldest thumbnail is evicted');
  assert.ok(cache.get(`p${THUMB_CACHE_ENTRIES}`), 'the newest thumbnail stays');

  cache.clear();
  for (let i = 0; i < 25; i++) cache.set(`big${i}`, thumb(MiB));
  assert.equal(cache.size(), 24, 'the byte budget binds before the entry cap');
  assert.equal(cache.totalBytes(), 24 * MiB);
  assert.equal(cache.get('big0'), undefined);
  assert.ok(cache.get('big24'));
});

test('render cache evicts oldest by byte budget and by entry count, and keeps the newest', () => {
  const MiB = 1024 * 1024;
  const entry = (bytes) => ({ pixels: new ArrayBuffer(bytes) });
  const cache = createRenderCache(8, 96 * MiB);

  for (let i = 0; i < 9; i++) cache.set(`p${i}`, entry(1024));
  assert.equal(cache.size(), 8, 'entry cap holds');
  assert.equal(cache.totalBytes(), 8 * 1024);
  assert.equal(cache.get('p0'), undefined, 'oldest entry evicted');
  assert.ok(cache.get('p8'), 'newest entry survives');

  cache.clear();
  for (let i = 0; i < 4; i++) cache.set(`big${i}`, entry(40 * MiB));
  assert.equal(cache.size(), 2, '96 MiB holds two 40 MiB pages');
  assert.equal(cache.totalBytes(), 80 * MiB);
  assert.equal(cache.get('big0'), undefined);
  assert.ok(cache.get('big3'), 'the page being painted is never evicted');

  cache.clear();
  cache.set('huge', entry(200 * MiB));
  assert.equal(cache.size(), 1, 'a single over-budget page still stays');
  assert.equal(cache.totalBytes(), 200 * MiB);
});

test('latest-wins guard discards a slow hand-off superseded by a newer one', async () => {
  const guard = createLatestGuard();
  const applied = [];
  const handOff = async (name, delay) => {
    const token = guard.begin();
    await new Promise(resolve => setTimeout(resolve, delay));
    if (!guard.isCurrent(token)) return;
    applied.push(name);
  };
  // A starts first but resolves last; B starts second and resolves first.
  await Promise.all([handOff('A', 30), handOff('B', 1)]);
  assert.deepEqual(applied, ['B'], 'the newest open wins even when the older fetch resolves later');
  assert.equal(guard.isCurrent(guard.begin()), true, 'a fresh token is current');
});

test('gate holds queued worker work until the save releases it', async () => {
  const gate = createGate();
  const order = [];
  // A save takes the gate first; work arriving behind it must wait.
  const release = gate.hold();
  const queued = (async () => {
    await gate.wait();
    order.push('render');
  })();
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual(order, [], 'queued work must not post while a save holds the gate');
  release();
  let released = false;
  const second = (async () => {
    await gate.wait();
    released = true;
  })();
  await Promise.all([queued, second]);
  assert.deepEqual(order, ['render'], 'queued work resumes once the save releases the gate');
  assert.equal(released, true);
  release(); // a double release must be a no-op, not a wedge
  await gate.wait();
});

test('parseLines keeps the geometry the text layer needs', () => {
  const json = JSON.stringify({
    blocks: [
      { type: 'text', lines: [{ text: 'Hi', y: 12, bbox: { x: 1, y: 2, w: 3, h: 4 }, font: { size: 9, family: 'serif', weight: 'bold', style: 'italic' } }] },
      { type: 'image' },
      { type: 'text', lines: [{ text: 'No font' }] },
    ],
  });
  assert.deepEqual(engine.parseLines(json), [
    { text: 'Hi', x: 1, y: 12, w: 3, h: 4, size: 9, family: 'serif', weight: 'bold', style: 'italic' },
    { text: 'No font', x: 0, y: 0, w: 0, h: 0, size: 12, family: 'sans-serif', weight: 'normal', style: 'normal' },
  ]);
  assert.deepEqual(engine.parseLines('not json'), []);
});

// ----------------------------------------------------------------- save a copy

/**
 * Canonical digest of the object graph reachable from /Root: object identity is
 * the visit order (so an unchanged graph hashes equal even when object numbers
 * move), stream content is hashed decoded, and only unstable plumbing keys are
 * dropped. This is how a no-op save is checked structurally instead of by bytes.
 */
const DIGEST_SKIP = new Set(['Length', 'Filter', 'DecodeParms']);

function canonicalDigest(doc) {
  const seen = new Map();
  const parts = [];
  let next = 1;

  function mark(obj) {
    if (!obj.isIndirect()) return false;
    const ref = obj.asIndirect();
    if (seen.has(ref)) {
      parts.push(`@${seen.get(ref)}`);
      return true;
    }
    seen.set(ref, next);
    parts.push(`#${next++}{`);
    return false;
  }

  function walkDict(obj) {
    const entries = [];
    obj.forEach((value, key) => entries.push([key, value]));
    entries.sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
    for (const [key, value] of entries) {
      if (DIGEST_SKIP.has(key)) continue;
      parts.push(`${key}:`);
      walk(value);
      parts.push(',');
    }
  }

  function walk(obj) {
    const visited = mark(obj);
    if (visited) return;
    if (obj.isStream()) {
      parts.push('S[');
      walkDict(obj);
      const stream = obj.readStream();
      try {
        parts.push(`data=${createHash('sha256').update(stream.asUint8Array()).digest('hex')}]`);
      } finally {
        stream.destroy();
      }
    } else if (obj.isArray()) {
      parts.push('[');
      for (let i = 0; i < obj.length; i++) walk(obj.get(i));
      parts.push(']');
    } else if (obj.isDictionary()) {
      parts.push('{');
      walkDict(obj);
      parts.push('}');
    } else if (obj.isName()) {
      parts.push(`n:${obj.asName()}`);
    } else if (obj.isString()) {
      parts.push(`s:${Buffer.from(obj.asByteString()).toString('hex')}`);
    } else if (obj.isBoolean()) {
      parts.push(`b:${obj.asBoolean() ? 1 : 0}`);
    } else if (obj.isNumber()) {
      parts.push(`d:${obj.asNumber().toPrecision(12)}`);
    } else {
      parts.push('?');
    }
    if (obj.isIndirect()) parts.push('}');
  }

  walk(doc.getTrailer().get('Root'));
  return createHash('sha256').update(parts.join('')).digest('hex');
}

/** One page carrying a marked-content tag tree, an outline item and an attachment. */
function pdfWithFeatureGraph() {
  const content = 'BT /F1 24 Tf 72 700 Td (PDF4All tagged) Tj ET\n';
  const attachment = 'PDF4All attachment payload\n';
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R /StructTreeRoot 8 0 R'
      + ' /MarkInfo << /Marked true >> /Names << /EmbeddedFiles 10 0 R >> >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R'
      + ' /Resources << /Font << /F1 5 0 R >> >> /StructParents 0 >>',
    `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}endstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    '<< /Type /Outlines /First 7 0 R /Last 7 0 R /Count 1 >>',
    '<< /Title (Chapter 1) /Parent 6 0 R /Dest [3 0 R /Fit] >>',
    '<< /Type /StructTreeRoot /K [9 0 R] >>',
    '<< /Type /StructElem /S /P /P 8 0 R /Pg 3 0 R /K 0 >>',
    '<< /Names [(note.txt) 11 0 R] >>',
    '<< /Type /Filespec /F (note.txt) /UF (note.txt) /EF << /F 12 0 R >> >>',
    `<< /Type /EmbeddedFile /Length ${Buffer.byteLength(attachment)} >>\nstream\n${attachment}endstream`,
  ]);
}

/** A one-page /Perms /DocMDP certified-looking document. */
function pdfCertified() {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /Perms << /DocMDP 3 0 R >> >>',
    '<< /Type /Pages /Kids [4 0 R] /Count 1 >>',
    '<< /Type /Sig /Filter /Adobe.PPKLite >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R >>',
    '<< /Length 2 >>\nstream\nq \nendstream',
  ]);
}

test('save refuses signed and certified documents, and names the reason', async () => {
  const refusals = [];
  let session = engine.createSession();
  assert.equal(engine.dispatch({ id: 0, op: 'save' }, session).reply.reason, 'no-document');

  const opened = engine.dispatch({ id: 1, op: 'open', name: 'signed.pdf', pdf: pdfWithSignatureField(true) }, session);
  assert.equal(opened.reply.ok, true, opened.reply.error);
  const signed = engine.dispatch({ id: 2, op: 'save' }, session);
  assert.equal(signed.reply.ok, false);
  assert.equal(signed.reply.reason, 'read-only');
  assert.equal(signed.transfer, undefined, 'nothing transferred when the save is refused');
  refusals.push(['signed', signed.reply.reason]);
  engine.closeDocument(session.doc);

  // A blank signature widget is not a signature: the document still saves.
  session = engine.createSession();
  assert.equal(engine.dispatch({ id: 3, op: 'open', name: 'blank-sig.pdf', pdf: pdfWithSignatureField(false) }, session).reply.ok, true);
  assert.equal(engine.dispatch({ id: 4, op: 'save' }, session).reply.ok, true, 'blank sig field is not read-only');
  engine.closeDocument(session.doc);

  session = engine.createSession();
  assert.equal(engine.dispatch({ id: 5, op: 'open', name: 'certified.pdf', pdf: pdfCertified() }, session).reply.ok, true);
  assert.equal(engine.detect(session.doc).certified, true);
  const certified = engine.dispatch({ id: 6, op: 'save' }, session);
  assert.equal(certified.reply.ok, false);
  assert.equal(certified.reply.reason, 'read-only');
  refusals.push(['certified', certified.reply.reason]);
  engine.closeDocument(session.doc);

  session = engine.createSession();
  const encrypted = engine.dispatch({ id: 7, op: 'open', name: 'encrypted.pdf', pdf: toArrayBuffer(await readFile('fixtures/encrypted.pdf')) }, session);
  assert.equal(encrypted.reply.ok, true, encrypted.reply.error);
  assert.equal(engine.detect(session.doc).needsPassword, true);
  const locked = engine.dispatch({ id: 8, op: 'save' }, session);
  assert.equal(locked.reply.ok, false);
  assert.equal(locked.reply.reason, 'encrypted');
  refusals.push(['password-protected', locked.reply.reason]);
  engine.closeDocument(session.doc);

  // Encrypted with an empty user password: it opens without prompting, and P2 unit 10
  // makes it editable and saveable - the copy must keep its own encryption.
  const plain = mupdf.Document.openDocument(toArrayBuffer(await readFile('fixtures/hello.pdf')), 'application/pdf').asPDF();
  const buffer = plain.saveToBuffer('incremental=no,encrypt=aes-256,user-password=,owner-password=owner');
  const emptyPassword = toArrayBuffer(buffer.asUint8Array());
  buffer.destroy();
  plain.destroy();
  session = engine.createSession();
  const silent = engine.dispatch({ id: 9, op: 'open', name: 'empty-password.pdf', pdf: emptyPassword }, session);
  assert.equal(silent.reply.ok, true, silent.reply.error);
  const silentInfo = engine.detect(session.doc);
  assert.equal(silentInfo.encrypted, true);
  assert.equal(silentInfo.needsPassword, false, 'the empty user password opens without prompting');
  const kept = engine.dispatch({ id: 10, op: 'save' }, session);
  assert.equal(kept.reply.ok, true, kept.reply.error);
  const keptInfo = engine.detect(mupdf.Document.openDocument(kept.reply.bytes, 'application/pdf'));
  assert.equal(keptInfo.encrypted, true, 'the saved copy is still encrypted');
  refusals.push(['empty-password-encrypted', 'saved-encrypted']);
  engine.closeDocument(session.doc);

  assert.deepEqual(refusals.map(([, reason]) => reason), ['read-only', 'read-only', 'encrypted', 'saved-encrypted']);
});

test('save serializes a fixture and validate hands the callers own bytes straight back', async () => {
  const session = engine.createSession();
  const opened = engine.dispatch({ id: 1, op: 'open', name: 'hello.pdf', pdf: toArrayBuffer(await readFile('fixtures/hello.pdf')) }, session);
  assert.equal(opened.reply.ok, true, opened.reply.error);

  const saved = engine.dispatch({ id: 2, op: 'save' }, session);
  assert.equal(saved.reply.ok, true, saved.reply.error);
  assert.equal(saved.reply.id, 2);
  assert.ok(saved.reply.bytes instanceof ArrayBuffer);
  assert.ok(saved.reply.bytes.byteLength > 100 && saved.reply.bytes.byteLength < 5 * 1024, `saved ${saved.reply.bytes.byteLength} bytes`);
  assert.ok(Number.isFinite(saved.reply.saveMs) && saved.reply.saveMs >= 0);
  assert.deepEqual(saved.transfer, [saved.reply.bytes], 'the saved bytes are transferred, not cloned');

  const request = { id: 3, op: 'validate', pdf: saved.reply.bytes };
  const validated = engine.dispatch(request, session);
  assert.equal(validated.reply.ok, true, validated.reply.error);
  assert.equal(validated.reply.pageCount, 1);
  assert.equal(validated.reply.bytes, saved.reply.bytes, 'validate passes the original buffer through');
  assert.deepEqual(validated.transfer, [saved.reply.bytes]);
  assert.equal(request.pdf, null, 'the worker must not retain the validated bytes');
  assert.ok(session.doc, 'validate never replaces the session document');
  engine.closeDocument(session.doc);
});

test('every fixture saves, validates and reopens with an unchanged reachable object digest', async () => {
  for (const id of ['hello', 'form', 'large', 'rotated', 'cjk', 'rtl']) {
    const session = engine.createSession();
    const opened = engine.dispatch({ id: 1, op: 'open', name: `${id}.pdf`, pdf: toArrayBuffer(await readFile(`fixtures/${id}.pdf`)) }, session);
    assert.equal(opened.reply.ok, true, `${id} open: ${opened.reply.error}`);
    const before = canonicalDigest(session.doc);
    const declared = engine.detect(session.doc);

    const saved = engine.dispatch({ id: 2, op: 'save' }, session);
    assert.equal(saved.reply.ok, true, `${id} save: ${saved.reply.error}`);
    const validated = engine.dispatch({ id: 3, op: 'validate', pdf: saved.reply.bytes }, session);
    assert.equal(validated.reply.ok, true, `${id} validate: ${validated.reply.error}`);

    const reopened = engine.openDocument(saved.reply.bytes);
    try {
      assert.equal(reopened.doc.countPages(), opened.reply.pageCount, `${id} page count`);
      assert.equal(canonicalDigest(reopened.doc), before, `${id} reachable object graph drifted`);
      assert.deepEqual(engine.detect(reopened.doc), declared, `${id} declared features drifted`);
      assert.equal(reopened.doc.asPDF().countVersions(), 1, `${id} save normalizes to a single revision`);
    } finally {
      engine.closeDocument(reopened.doc);
    }
    engine.closeDocument(session.doc);
  }
});

test('a no-op save preserves tags, outlines and attachments structurally', async () => {
  const session = engine.createSession();
  const opened = engine.dispatch({ id: 1, op: 'open', name: 'featured.pdf', pdf: pdfWithFeatureGraph() }, session);
  assert.equal(opened.reply.ok, true, opened.reply.error);
  const info = engine.detect(session.doc);
  assert.equal(info.tagged, true, 'fixture is tagged');
  assert.equal(info.attachments, 1, 'fixture has one attachment');
  const before = canonicalDigest(session.doc);

  const saved = engine.dispatch({ id: 2, op: 'save' }, session);
  assert.equal(saved.reply.ok, true, saved.reply.error);
  assert.equal(engine.dispatch({ id: 3, op: 'validate', pdf: saved.reply.bytes }, session).reply.ok, true);

  const reopened = engine.openDocument(saved.reply.bytes);
  try {
    assert.equal(canonicalDigest(reopened.doc), before, 'reachable object set/content changed');
    assert.equal(engine.detect(reopened.doc).tagged, true);
    assert.equal(engine.detect(reopened.doc).attachments, 1);
    const pdf = reopened.doc.asPDF();
    const root = pdf.getTrailer().get('Root');
    const outline = root.get('Outlines');
    assert.equal(outline.get('Count').asNumber(), 1, 'outline count kept');
    assert.equal(outline.get('First').get('Title').asString(), 'Chapter 1', 'outline title kept');
    const struct = root.get('StructTreeRoot');
    assert.equal(struct.get('K').length, 1, 'tag tree kept');
    assert.equal(struct.get('K').get(0).get('S').asName(), 'P', 'structure element kind kept');
    const files = pdf.getEmbeddedFiles();
    assert.deepEqual(Object.keys(files), ['note.txt'], 'attachment name kept');
    const contents = pdf.getEmbeddedFileContents(files['note.txt']);
    try {
      assert.equal(Buffer.from(contents.asUint8Array()).toString(), 'PDF4All attachment payload\n', 'attachment bytes kept');
    } finally {
      contents.destroy();
    }
    const page = reopened.doc.loadPage(0);
    try {
      assert.ok(page.toStructuredText('preserve-spans').asText().includes('PDF4All tagged'), 'page text kept');
    } finally {
      page.destroy();
    }
  } finally {
    engine.closeDocument(reopened.doc);
  }
  engine.closeDocument(session.doc);
});

test('save enforces the 50 MiB save ceiling on the source before serialization', async () => {
  const fixtureBytes = (await readFile('fixtures/hello.pdf')).length;
  const session = engine.createSession();
  const opened = engine.dispatch({ id: 1, op: 'open', name: 'hello.pdf', pdf: toArrayBuffer(await readFile('fixtures/hello.pdf')) }, session);
  assert.equal(opened.reply.ok, true, opened.reply.error);
  assert.equal(engine.maxSaveBytes, 50 * 1024 * 1024, 'the chosen save ceiling');
  assert.equal(engine.maxSaveBytes, policy.MAX_SAVE_BYTES, 'engine save ceiling drifted from src/policy.ts');
  assert.equal(policy.saveWithinCeiling(engine.maxSaveBytes), true, 'the save ceiling is inclusive');
  assert.equal(policy.saveWithinCeiling(engine.maxSaveBytes + 1), false);
  assert.equal(policy.pdfWithinCeiling(engine.maxSaveBytes), true, 'opening stays looser than saving');
  assert.ok(engine.maxSaveBytes < engine.maxPdfBytes, 'saving is the tighter direction');
  assert.equal(session.sourceBytes, fixtureBytes, 'the source size is recorded on open');

  // Exactly at the ceiling the save reaches serialization (a spy proves it).
  let serialized = false;
  session.doc.saveToBuffer = () => {
    serialized = true;
    throw new Error('SERIALIZED');
  };
  session.sourceBytes = engine.maxSaveBytes;
  const at = engine.dispatch({ id: 2, op: 'save' }, session);
  assert.equal(at.reply.ok, false);
  assert.match(at.reply.error, /SERIALIZED/, 'a source exactly at the ceiling is not refused');
  assert.equal(serialized, true);

  // One byte over is refused before any serialization work happens.
  serialized = false;
  session.sourceBytes = engine.maxSaveBytes + 1;
  const over = engine.dispatch({ id: 3, op: 'save' }, session);
  assert.equal(over.reply.ok, false);
  assert.equal(over.reply.reason, 'too-large');
  assert.match(over.reply.error, /save ceiling/);
  assert.equal(serialized, false, 'an oversized source is refused before serialization');
  engine.closeDocument(session.doc);
});

test('save rejects an oversized rewrite before copying it out of the heap', async () => {
  const session = engine.createSession();
  engine.dispatch({ id: 1, op: 'open', name: 'hello.pdf', pdf: toArrayBuffer(await readFile('fixtures/hello.pdf')) }, session);
  // A lazy view only reports the length: nothing is ever copied out of the heap.
  session.doc.saveToBuffer = () => ({
    asUint8Array: () => new Proxy(new Uint8Array(0), {
      get: (target, key) => (key === 'byteLength' ? engine.maxSaveBytes + 1 : Reflect.get(target, key)),
    }),
    destroy() {},
  });
  const reply = engine.dispatch({ id: 2, op: 'save' }, session).reply;
  assert.equal(reply.ok, false);
  assert.equal(reply.reason, 'too-large');
  assert.match(reply.error, /save ceiling/);
  engine.closeDocument(session.doc);
});

test('validate rejects garbage, oversized, mismatched and repaired copies without touching the session', async () => {
  const session = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'validate', pdf: new ArrayBuffer(3) }, session).reply.reason, 'no-document');

  const opened = engine.dispatch({ id: 2, op: 'open', name: 'hello.pdf', pdf: toArrayBuffer(await readFile('fixtures/hello.pdf')) }, session);
  assert.equal(opened.reply.ok, true, opened.reply.error);
  const before = canonicalDigest(session.doc);
  const original = session.doc;

  for (const [label, pdf, reason] of [
    ['junk bytes', toArrayBuffer(randomBytes(64)), 'invalid'],
    ['too few bytes', new ArrayBuffer(3), 'invalid'],
    ['not bytes at all', 'not a PDF', 'invalid'],
  ]) {
    const { reply } = engine.dispatch({ id: 3, op: 'validate', pdf }, session);
    assert.equal(reply.ok, false, label);
    assert.equal(reply.reason, reason, label);
    assert.equal(typeof reply.error, 'string', label);
  }

  // The oversized check is the same save ceiling the rewrite output is held to; a
  // real 50 MiB worker output is not worth building just to re-test a `>` compare.
  const huge = new Proxy(new ArrayBuffer(4), {
    get: (target, key) => (key === 'byteLength' ? engine.maxSaveBytes + 1 : Reflect.get(target, key)),
  });
  const oversized = engine.dispatch({ id: 4, op: 'validate', pdf: huge }, session);
  assert.equal(oversized.reply.ok, false);
  assert.equal(oversized.reply.reason, 'too-large');

  // Same page count, different declared features.
  const featured = engine.dispatch({ id: 5, op: 'validate', pdf: pdfWithFeatureGraph() }, session);
  assert.equal(featured.reply.ok, false);
  assert.equal(featured.reply.reason, 'invalid', 'a tagged/attachment document is not this one');

  // Different page count.
  const mismatch = engine.dispatch({ id: 6, op: 'validate', pdf: toArrayBuffer(await readFile('fixtures/large.pdf')) }, session);
  assert.equal(mismatch.reply.ok, false);
  assert.equal(mismatch.reply.reason, 'invalid');

  // A broken startxref that mupdf rescues by scanning: opens, but was repaired.
  const good = (await readFile('fixtures/hello.pdf')).toString('latin1');
  const broken = good.replace(/startxref\s+\d+/, `startxref\n${Number(good.match(/startxref\s+(\d+)/)[1]) - 5}`);
  const repaired = engine.dispatch({ id: 7, op: 'validate', pdf: toArrayBuffer(Buffer.from(broken, 'latin1')) }, session);
  assert.equal(repaired.reply.ok, false);
  assert.equal(repaired.reply.reason, 'invalid', 'a repaired copy is not accepted');

  // None of that touched the open session.
  assert.equal(session.doc, original);
  assert.equal(session.doc.countPages(), 1);
  assert.equal(canonicalDigest(session.doc), before);
  engine.closeDocument(session.doc);
});

test('parsePageRange reads the print range grammar, or refuses it', () => {
  const pages = 5;
  assert.deepEqual(parsePageRange('', pages), [0, 1, 2, 3, 4], 'blank means every page');
  assert.deepEqual(parsePageRange('   ', pages), [0, 1, 2, 3, 4]);
  assert.deepEqual(parsePageRange('1', pages), [0]);
  assert.deepEqual(parsePageRange('1-3', pages), [0, 1, 2]);
  assert.deepEqual(parsePageRange('3-1', pages), [0, 1, 2], 'a reversed range is read forwards');
  assert.deepEqual(parsePageRange('-2', pages), [0, 1]);
  assert.deepEqual(parsePageRange('4-', pages), [3, 4]);
  assert.deepEqual(parsePageRange('1,1,2', pages), [0, 1], 'duplicates collapse');
  assert.deepEqual(parsePageRange(' 3 , 1 - 2 ', pages), [0, 1, 2], 'spaces are allowed');
  assert.deepEqual(parsePageRange('2,1,2', pages), [0, 1], 'the result is ascending');
  assert.deepEqual(parsePageRange('5', pages), [4], 'the last page is in range');
  assert.deepEqual(parsePageRange('1-5,3,4', pages), [0, 1, 2, 3, 4]);
  for (const bad of ['0', '6', '1-6', 'a', '1--2', '1-2-3', '1,,2', ',', '1.5', '-', '--', ' 2 ']) {
    if (bad === ' 2 ') continue;
    assert.equal(parsePageRange(bad, pages), null, `${JSON.stringify(bad)} is refused`);
  }
  assert.deepEqual(parsePageRange(' 2 ', pages), [1], 'a padded single page is still a page');
  assert.deepEqual(parsePageRange('1 - 2', pages), [0, 1], 'spaces around the dash are allowed');
  assert.equal(parsePageRange('1 2', pages), null, 'two pages need a comma, not a space');
  assert.equal(parsePageRange('1', 0), null, 'a document with no pages has no page 1');
  assert.deepEqual(parsePageRange('', 0), []);
});

test('authenticate unlocks an encrypted document without leaking the password or the lock', async () => {
  const session = engine.createSession();
  const nothing = engine.dispatch({ id: 1, op: 'authenticate', password: 'x' }, session).reply;
  assert.equal(nothing.ok, false);
  assert.match(nothing.error, /no document open/);

  const opened = engine.dispatch({ id: 2, op: 'open', name: 'encrypted.pdf', pdf: toArrayBuffer(await readFile('fixtures/encrypted.pdf')) }, session);
  assert.equal(opened.reply.ok, true, opened.reply.error);
  assert.equal(opened.reply.info.needsPassword, true);
  assert.equal(opened.reply.info.encrypted, true);
  assert.equal(opened.reply.info.printable, true, 'this fixture grants the print permission');
  assert.equal(engine.dispatch({ id: 3, op: 'render', page: 0, scale: 1 }, session).reply.error.includes('password'), true);

  for (const [label, password] of [
    ['missing', undefined], ['null', null], ['number', 12345], ['object', { password: 'pdf4all-user' }],
  ]) {
    const reply = engine.dispatch({ id: 4, op: 'authenticate', password }, session).reply;
    assert.equal(reply.ok, false, `${label} password rejected`);
    assert.match(reply.error, /password must be a string/);
  }
  const long = engine.dispatch({ id: 5, op: 'authenticate', password: 'a'.repeat(MAX_PASSWORD_CHARS + 1) }, session).reply;
  assert.equal(long.ok, false, 'an over-long password is refused before it reaches the document');
  assert.match(long.error, new RegExp(String(MAX_PASSWORD_CHARS)));

  const wrong = engine.dispatch({ id: 6, op: 'authenticate', password: 'wrong' }, session).reply;
  assert.equal(wrong.ok, true, 'a wrong password is an answer, not an error');
  assert.equal(wrong.authenticated, false);
  assert.equal(wrong.pageCount, undefined);
  assert.equal(wrong.info, undefined, 'a failed unlock reports nothing about the document');
  assert.equal(session.locked, true, 'a wrong password leaves the document locked');
  assert.match(engine.dispatch({ id: 7, op: 'thumbnail', page: 0, maxEdge: THUMB_MAX_EDGE }, session).reply.error, /password/);

  // Objects parsed before the password (page boxes for the open reply) are cached
  // with undecrypted strings; the unlock must not hand those back.
  const form = engine.createSession();
  engine.dispatch({ id: 1, op: 'open', name: 'e.pdf', pdf: toArrayBuffer(await readFile('fixtures/p3/encrypted-edit-noannot.pdf')) }, form);
  assert.equal(engine.dispatch({ id: 2, op: 'authenticate', password: 'pdf4all-user' }, form).reply.authenticated, true);
  assert.equal(form.lockedBytes, null, 'the source bytes are dropped once unlocked');
  assert.deepEqual(engine.dispatch({ id: 3, op: 'fields', page: 0 }, form).reply.items.map(item => item.name), ['fillable'],
    'a field name read after unlock is the decrypted string, not the pre-auth cache');

  const right = engine.dispatch({ id: 8, op: 'authenticate', password: 'pdf4all-user' }, session).reply;
  assert.equal(right.ok, true, right.error);
  assert.equal(right.authenticated, true);
  assert.equal(right.pageCount, 1);
  assert.deepEqual(right.pages, [{ width: 612, height: 792 }]);
  assert.equal(right.info.needsPassword, false);
  assert.equal(right.info.encrypted, true, 'unlocked, still encrypted');
  assert.equal(right.info.printable, true);
  assert.equal('password' in right, false, 'the password never comes back in a reply');

  // The unlocked document is a normal read-only session document.
  const render = engine.dispatch({ id: 9, op: 'render', page: 0, scale: 1 }, session).reply;
  assert.equal(render.ok, true, render.error);
  assert.ok(render.text.includes('PDF4All encrypted'), 'the unlocked page decrypts');
  assert.equal(engine.dispatch({ id: 10, op: 'search', query: 'PDF4All' }, session).reply.total, 1);
  assert.equal(engine.dispatch({ id: 11, op: 'thumbnail', page: 0, maxEdge: THUMB_MAX_EDGE }, session).reply.ok, true);

  // P2 unit 10: an unlocked encrypted document saves again, keeping its encryption.
  // A refused save must not re-lock the open document (mupdf's needsPassword() would).
  const save = engine.dispatch({ id: 12, op: 'save' }, session).reply;
  assert.equal(save.ok, true, save.error);
  assert.equal(save.security.mode, 'keep');
  assert.equal(save.security.algorithm, 'rc4-128');
  const copy = mupdf.Document.openDocument(save.bytes, 'application/pdf');
  try {
    assert.equal(copy.authenticatePassword('pdf4all-user'), 2, 'the copy keeps the same password');
  } finally {
    copy.destroy();
  }
  assert.equal(engine.dispatch({ id: 13, op: 'render', page: 0, scale: 1 }, session).reply.ok, true, 'the session is still unlocked');
  assert.equal(session.locked, false);

  const again = engine.dispatch({ id: 14, op: 'authenticate', password: 'pdf4all-user' }, session).reply;
  assert.equal(again.ok, false, 'an unlocked document is not locked');
  assert.match(again.error, /not locked/);
  assert.equal(engine.dispatch({ id: 15, op: 'render', page: 0, scale: 1 }, session).reply.ok, true, 'a refused authenticate does not re-lock either');

  // Reloading the same bytes gives a fresh locked session again.
  const second = engine.createSession();
  engine.dispatch({ id: 16, op: 'open', name: 'encrypted.pdf', pdf: toArrayBuffer(await readFile('fixtures/encrypted.pdf')) }, second);
  assert.equal(second.locked, true);
  assert.equal(engine.dispatch({ id: 17, op: 'authenticate', password: 'pdf4all-user' }, second).reply.authenticated, true);
  engine.closeDocument(second.doc);
  engine.closeDocument(session.doc);
});

test('authenticate is refused on plain documents and printable follows the print permission', async () => {
  const session = engine.createSession();
  for (const id of ['hello', 'large', 'form']) {
    const opened = engine.dispatch({ id: 1, op: 'open', name: `${id}.pdf`, pdf: toArrayBuffer(await readFile(`fixtures/${id}.pdf`)) }, session);
    assert.equal(opened.reply.ok, true, opened.reply.error);
    assert.equal(opened.reply.info.printable, true, `${id} prints`);
    const reply = engine.dispatch({ id: 2, op: 'authenticate', password: 'anything' }, session).reply;
    assert.equal(reply.ok, false, id);
    assert.equal(reply.error, 'authenticate: document is not locked', id);
  }

  // Encrypted with the print bit cleared: it unlocks, and then it still refuses to print.
  const opened = engine.dispatch({ id: 3, op: 'open', name: 'encrypted-noprint.pdf', pdf: toArrayBuffer(await readFile('fixtures/encrypted-noprint.pdf')) }, session);
  assert.equal(opened.reply.ok, true, opened.reply.error);
  assert.equal(opened.reply.info.printable, false, 'locked and print-forbidden');
  const unlocked = engine.dispatch({ id: 4, op: 'authenticate', password: 'pdf4all-user' }, session).reply;
  assert.equal(unlocked.ok, true, unlocked.error);
  assert.equal(unlocked.authenticated, true);
  assert.equal(unlocked.info.printable, false, 'the print permission survives the unlock');
  assert.equal(unlocked.info.encrypted, true);
  assert.equal(engine.detect(session.doc, false).printable, false, 'detect keeps the same answer');
  const render = engine.dispatch({ id: 5, op: 'render', page: 0, scale: 1 }, session).reply;
  assert.equal(render.ok, true, render.error);
  assert.ok(render.text.includes('PDF4All encrypted no-print'), 'the document itself is readable');
  engine.closeDocument(session.doc);
});
