// The mobile scanner's ops on the Rust engine through the production adapter (engine-rust.ts):
// scan-measure and scan-compose (engine/src/app/ops/scan.rs, engine/design/app-m9-mobile-release.md).
// The adapter installs the CJK font when the op asks for it and dispatches again. The outputs are read back
// by the engine (tests/inspect.mjs over a reopened copy) and its render, text and search ops.
import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { createRustEngine } from '../build-test/engine-rust.js';
import { createEngine, inherited, sameId } from './rust-engine.mjs';
import { readCjkFont } from '../scripts/cjk-font.mjs';

const PKG = new URL('../engine/web/pkg/', import.meta.url);
const skipWasm = existsSync(new URL('tandem_rust_workflow_bg.wasm', PKG)) ? false : 'needs the Rust package (engine/web/pkg)';
// One instance per process: the package keeps its CJK font module-wide.
const wasm = skipWasm ? null : await import(new URL('tandem_rust_workflow.js', PKG).href);
wasm?.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', PKG)) });

/** A 200 x 260 px JPEG as the scanner's canvas makes them: grey 230, made for this test with cjpeg -quality 90 -optimize. */
function jpeg() {
  return new Uint8Array(Buffer.from(
    '/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMDAsKCwsNDhIQDQ4RDgsLEBYQERMU' +
    'FRUVDA8XGBYUGBIUFRT/2wBDAQMEBAUEBQkFBQkUDQsNFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU' +
    'FBQUFBQUFBT/wAARCAEEAMgDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAn/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QA' +
    'FAEBAAAAAAAAAAAAAAAAAAAAAP/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/AKIAAAAAAAAAAAAAAAAAAAAAAAAA' +
    'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' +
    'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' +
    'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' +
    'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' +
    'AAAAAAAAAAAAAAAAA//Z',
    'base64'));
}

test('rust engine: scan-measure and scan-compose through the adapter, the CJK font installed on the cue', { skip: skipWasm }, () => {
  const engine = createRustEngine(wasm);
  let reads = 0;
  engine.useCjkFont(wasm, () => { reads++; return readCjkFont(); });
  const session = engine.createSession();
  let id = 0;
  const send = request => engine.dispatch({ id: ++id, ...request }, session);

  // Latin, Greek and Cyrillic need no CJK font.
  const measured = send({ op: 'scan-measure', text: 'Aż Ω Ж' }).reply;
  assert.equal(measured.ok, true, JSON.stringify(measured));
  assert.deepEqual(measured.chars, ['A', 'ż', ' ', 'Ω', 'Ж']);
  assert.deepEqual(measured.fonts, ['helvetica', 'latin', 'helvetica', 'nimbus', 'nimbus']);
  assert.equal(measured.regular[0], 667);
  assert.equal(reads, 0);

  const image = jpeg();
  const lines = [
    { text: 'Hello scan résumé 中文', x: 61.2, y: 177.44, size: 23.76, invisible: true, width: 306 },
    { text: 'Ωμέγα Привет Zażółć ≤ →', x: 52, y: 300, size: 14, bold: true, color: [0.12, 0.16, 0.15] },
  ];
  const { reply } = send({
    op: 'scan-compose', producer: 'Tandem PDF', images: [image.buffer.slice(0)],
    pages: [{ width: 612, height: 792, image: 0, runs: lines, boxes: [{ x: 52, y: 400, width: 100, height: 20, lineWidth: 0.5 }] },
      { width: 612, height: 792, image: 0 }],
  });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.equal(reads, 1, 'the CJK font is read once, when the op asks for it');
  assert.equal(reply.pageCount, 2);
  assert.ok(reply.bytes instanceof ArrayBuffer);

  // Read back: the text as given, at the positions given (the engine's runs are whole points, one per font;
  // a line's runs share its baseline; the native tests check 0.2 pt), the JPEG as it is, one image for both pages.
  const reader = createEngine();
  const doc = reader.reopen(reply.bytes.slice(0));
  try {
    assert.deepEqual(doc.text(0).split('\n').filter(Boolean), lines.map(line => line.text));
    const runs = doc.lines(0);
    const found = [...new Set(runs.map(run => run.y))].map(y => {
      const on = runs.filter(run => run.y === y);
      return { x: on[0].x, w: on.at(-1).x + on.at(-1).w - on[0].x };
    });
    assert.equal(found.length, 2, JSON.stringify(runs));
    assert.ok(Math.abs(found[0].x - 61.2) <= 0.5, JSON.stringify(found[0]));
    assert.ok(Math.abs(found[0].w - 306) <= 1, JSON.stringify(found[0]));
    assert.ok(Math.abs(found[1].x - 52) <= 0.5, JSON.stringify(found[1]));
    assert.equal(doc.repaired, false);
    assert.deepEqual(inherited(doc.page(0), 'MediaBox').numbers(), [0, 0, 612, 792]);
    const xobjects = page => inherited(doc.page(page), 'Resources').get('XObject');
    assert.deepEqual(Buffer.from(xobjects(0).get('Im0').rawStream()), Buffer.from(image));
    assert.ok(sameId(xobjects(0).ref('Im0'), xobjects(1).ref('Im0')), 'one image for both pages');
    assert.equal(doc.trailer.get('Info').string('Producer'), 'Tandem PDF');
  } finally {
    reader.closeDocument(doc);
  }

  // The engine reads its own output back: text, search, and the strict check the scanner runs.
  const reopen = engine.createSession();
  const again = request => engine.dispatch({ id: ++id, ...request }, reopen).reply;
  assert.equal(again({ op: 'open', pdf: reply.bytes.slice(0), name: 'scan.pdf' }).pageCount, 2);
  const text = again({ op: 'text-all' }).text;
  for (const line of lines) assert.ok(text.includes(line.text), line.text);
  assert.ok(again({ op: 'search', query: 'Привет' }).hits.length);
  assert.equal(again({ op: 'validate', pdf: reply.bytes.slice(0) }).ok, true);
  // The visible line's Greek and Cyrillic draw (embedded Nimbus Sans), not as boxes or blanks (RGBA pixels at scale 1).
  const render = again({ op: 'render', page: 0, scale: 1 });
  const rgba = new Uint8Array(render.pixels);
  let drawn = 0;
  for (let y = 290; y < 302; y++) for (let x = 52; x < 150; x++) if (rgba[(y * render.pixelWidth + x) * 4] < 128) drawn++;
  assert.ok(drawn > 100, `${drawn} dark pixels under the Greek word (Rust)`);

  // Hebrew and Arabic are drawn in visual order, so the engine finds the typed words; a visible
  // CJK line draws (the subset keeps the font's composite glyphs, renumbered).
  const rtl = send({ op: 'scan-compose', pages: [{ width: 612, height: 792, runs: [
    { text: 'שלום עולם', x: 50, y: 100, size: 20, invisible: true },
    { text: 'مرحبا بالعالم', x: 50, y: 150, size: 20, invisible: true },
    { text: 'מחיר 3.50 שקל 12/05/2026 סך 1,000 הנחה 50%', x: 50, y: 200, size: 12, invisible: true },
    { text: 'שָׁלוֹם מַרְחַבָּא بِالْعَالَمِ', x: 50, y: 230, size: 20 },
    { text: '中文日本語한국어', x: 50, y: 300, size: 40 },
  ] }] }).reply;
  assert.equal(rtl.ok, true, JSON.stringify(rtl));
  const typed = ['שלום', 'עולם', 'مرحبا', '3.50', '12/05/2026', '1,000', '50%', 'שָׁלוֹם', 'بِالْعَالَمِ'];
  const rtlSession = engine.createSession();
  const rtlSend = request => engine.dispatch({ id: ++id, ...request }, rtlSession).reply;
  rtlSend({ op: 'open', pdf: rtl.bytes.slice(0), name: 'rtl.pdf' });
  for (const word of typed) assert.ok(rtlSend({ op: 'search', query: word }).hits.length, `the engine finds ${word}`);
  const cjkRender = rtlSend({ op: 'render', page: 0, scale: 1 });
  const cjkPixels = new Uint8Array(cjkRender.pixels);
  let cjkDark = 0;
  for (let y = 265; y < 305; y++) for (let x = 50; x < 400; x++) if (cjkPixels[(y * cjkRender.pixelWidth + x) * 4] < 128) cjkDark++;
  assert.ok(cjkDark > 1000, `${cjkDark} dark pixels under the CJK line`);
  assert.match(rtlSend({ op: 'text-all' }).text, /שלום עולם/);

  // Past 40,000 runs and boxes the adapter refuses before encoding the request (it took 33 s in the engine).
  const boxes = Array.from({ length: 4000 }, () => ({ x: 1, y: 1, width: 1, height: 1 }));
  const started = performance.now();
  const crowded = send({ op: 'scan-compose', pages: Array.from({ length: 100 }, () => ({ width: 9, height: 9, boxes })) }).reply;
  assert.deepEqual([crowded.ok, crowded.reason, crowded.detail], [false, 'too-large', 'runs']);
  assert.equal(crowded.error, 'scan-compose: a scan has at most 40000 runs and boxes in all');
  assert.ok(performance.now() - started < 1000, `${Math.round(performance.now() - started)} ms`);

  // Refusals: not a JPEG, too many pages.
  const bad = send({ op: 'scan-compose', images: [new Uint8Array([1, 2, 3]).buffer], pages: [{ width: 1, height: 1, image: 0 }] }).reply;
  assert.deepEqual([bad.ok, bad.reason], [false, 'invalid']);
  const many = send({ op: 'scan-compose', pages: Array(101).fill({ width: 1, height: 1 }) }).reply;
  assert.deepEqual([many.ok, many.reason], [false, 'too-large']);
});

test('adapter: a CJK font that cannot be read leaves the scan op refused (no-font), not retried', () => {
  // The package above already holds the font, so a stand-in package plays one that asks for it.
  const calls = [];
  class App {
    outputs = [];
    create_session() { return 1; }
    drop_session() {}
    clear_inputs() {}
    push_input() {}
    dispatch(_handle, json) {
      calls.push(JSON.parse(json).op);
      return JSON.stringify({ id: 1, ok: false, reason: 'no-font', error: 'scan-compose: the text needs the CJK fallback font, which is not installed' });
    }
    output_count() { return 0; }
    take_output() { return new Uint8Array(); }
    session_state() { return 'null'; }
  }
  let wanted = true;
  const engine = createRustEngine({ RustApp: App });
  engine.useCjkFont({ set_cjk_font: () => false, cjk_font_wanted: () => wanted, has_cjk_font: () => false }, () => { wanted = false; return null; });
  const { reply } = engine.dispatch({ id: 1, op: 'scan-compose', pages: [] }, engine.createSession());
  assert.deepEqual([reply.ok, reply.reason], [false, 'no-font']);
  assert.deepEqual(calls, ['scan-compose'], 'an unreadable font is not retried');
});

// The test that the TypeScript engine refused both scan ops went with it (removed with the oracle).
