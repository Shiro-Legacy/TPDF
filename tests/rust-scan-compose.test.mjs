// The mobile scanner's ops on the Rust engine through the production adapter (engine-rust.ts):
// scan-measure and scan-compose (engine/src/app/ops/scan.rs, engine/design/app-m9-mobile-release.md).
// The adapter installs the CJK font when the op asks for it and dispatches again. MuPDF reads every output
// here as an independent oracle; engine.ts (MuPDF) refuses both ops as unsupported.
import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import mupdf from 'mupdf';
import { createRustEngine } from '../build/renderer/engine-rust.js';
import { createEngine } from './mupdf-oracle.mjs';
import { ENGINE } from '../build/engine-build.js';
import { readCjkFont } from '../scripts/cjk-font.mjs';

const PKG = new URL('../engine/web/pkg/', import.meta.url);
const rustBuild = ENGINE === 'rust' || process.env.TPDF_ENGINE === 'rust';
const skipWasm = rustBuild && existsSync(new URL('tandem_rust_workflow_bg.wasm', PKG)) ? false : 'needs the Rust package (a Rust build)';
// One instance per process: the package keeps its CJK font module-wide.
const wasm = skipWasm ? null : await import(new URL('tandem_rust_workflow.js', PKG).href);
wasm?.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', PKG)) });

/** A 200 x 260 px JPEG as the scanner's canvas makes them, encoded by the oracle. */
function jpeg() {
  const pixmap = new mupdf.Pixmap(mupdf.ColorSpace.DeviceRGB, [0, 0, 200, 260], false);
  pixmap.clear(230);
  const bytes = pixmap.asJPEG(90);
  pixmap.destroy();
  return bytes;
}

/** Each oracle text line on page 0: its text and bbox (y down). */
function oracleLines(bytes) {
  const doc = mupdf.Document.openDocument(new Uint8Array(bytes), 'application/pdf');
  const page = doc.loadPage(0), text = page.toStructuredText();
  const lines = JSON.parse(text.asJSON()).blocks.filter(block => block.type === 'text').flatMap(block => block.lines);
  text.destroy(); page.destroy(); doc.destroy();
  return lines.map(line => ({ text: line.text, bbox: line.bbox }));
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

  // The oracle: the text as given, at the positions given (MuPDF's JSON rounds them to whole points; the
  // native tests check 0.2 pt), the JPEG as it is, one image for both pages.
  const found = oracleLines(reply.bytes);
  assert.deepEqual(found.map(line => line.text), lines.map(line => line.text));
  assert.ok(Math.abs(found[0].bbox.x - 61.2) <= 0.5, JSON.stringify(found[0]));
  assert.ok(Math.abs(found[0].bbox.w - 306) <= 1, JSON.stringify(found[0]));
  assert.ok(Math.abs(found[1].bbox.x - 52) <= 0.5, JSON.stringify(found[1]));
  const doc = mupdf.Document.openDocument(new Uint8Array(reply.bytes), 'application/pdf');
  const pdf = doc.asPDF();
  assert.equal(pdf.wasRepaired(), false);
  assert.deepEqual(doc.loadPage(0).getBounds(), [0, 0, 612, 792]);
  const im0 = page => pdf.findPage(page).get('Resources').get('XObject').get('Im0');
  assert.deepEqual(Buffer.from(im0(0).readRawStream().asUint8Array()), Buffer.from(image));
  assert.equal(im0(0).asIndirect(), im0(1).asIndirect());
  assert.equal(pdf.getTrailer().get('Info').get('Producer').asString(), 'Tandem PDF');
  // The visible line's Greek and Cyrillic draw (embedded Nimbus Sans), not as boxes or blanks.
  const page = doc.loadPage(0);
  const pixmap = page.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  const pixels = pixmap.getPixels(), stride = pixmap.getStride();
  let dark = 0;
  for (let y = 290; y < 302; y++) for (let x = 52; x < 150; x++) if (pixels[y * stride + 3 * x] < 128) dark++;
  assert.ok(dark > 100, `${dark} dark pixels under the Greek word`);
  pixmap.destroy(); page.destroy(); doc.destroy();

  // The engine reads its own output back: text, search, and the strict check the scanner runs.
  const reopen = engine.createSession();
  const again = request => engine.dispatch({ id: ++id, ...request }, reopen).reply;
  assert.equal(again({ op: 'open', pdf: reply.bytes.slice(0), name: 'scan.pdf' }).pageCount, 2);
  const text = again({ op: 'text-all' }).text;
  for (const line of lines) assert.ok(text.includes(line.text), line.text);
  assert.ok(again({ op: 'search', query: 'Привет' }).hits.length);
  assert.equal(again({ op: 'validate', pdf: reply.bytes.slice(0) }).ok, true);
  // The engine's own view draws the embedded Nimbus Sans glyphs too (RGBA pixels at scale 1).
  const render = again({ op: 'render', page: 0, scale: 1 });
  const rgba = new Uint8Array(render.pixels);
  let drawn = 0;
  for (let y = 290; y < 302; y++) for (let x = 52; x < 150; x++) if (rgba[(y * render.pixelWidth + x) * 4] < 128) drawn++;
  assert.ok(drawn > 100, `${drawn} dark pixels under the Greek word (Rust)`);

  // Hebrew and Arabic are drawn in visual order, so MuPDF and the engine find the typed words; a visible
  // CJK line draws (the subset keeps the font's composite glyphs, renumbered).
  const rtl = send({ op: 'scan-compose', pages: [{ width: 612, height: 792, runs: [
    { text: 'שלום עולם', x: 50, y: 100, size: 20, invisible: true },
    { text: 'مرحبا بالعالم', x: 50, y: 150, size: 20, invisible: true },
    { text: 'מחיר 3.50 שקל 12/05/2026 סך 1,000 הנחה 50%', x: 50, y: 200, size: 12, invisible: true },
    { text: 'שָׁלוֹם מַרְחַבָּא بِالْعَالَمِ', x: 50, y: 230, size: 20 },
    { text: '中文日本語한국어', x: 50, y: 300, size: 40 },
  ] }] }).reply;
  assert.equal(rtl.ok, true, JSON.stringify(rtl));
  const rtlDoc = mupdf.Document.openDocument(new Uint8Array(rtl.bytes), 'application/pdf');
  const rtlPage = rtlDoc.loadPage(0);
  const typed = ['שלום', 'עולם', 'مرحبا', '3.50', '12/05/2026', '1,000', '50%', 'שָׁלוֹם', 'بِالْعَالَمِ'];
  for (const word of typed) assert.ok(rtlPage.search(word).length, `MuPDF finds ${word}`);
  const cjkPixmap = rtlPage.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  const cjkPixels = cjkPixmap.getPixels(), cjkStride = cjkPixmap.getStride();
  let cjkDark = 0;
  for (let y = 265; y < 305; y++) for (let x = 50; x < 400; x++) if (cjkPixels[y * cjkStride + 3 * x] < 128) cjkDark++;
  assert.ok(cjkDark > 1000, `${cjkDark} dark pixels under the CJK line`);
  cjkPixmap.destroy(); rtlPage.destroy(); rtlDoc.destroy();
  const rtlSession = engine.createSession();
  const rtlSend = request => engine.dispatch({ id: ++id, ...request }, rtlSession).reply;
  rtlSend({ op: 'open', pdf: rtl.bytes.slice(0), name: 'rtl.pdf' });
  for (const word of typed) assert.ok(rtlSend({ op: 'search', query: word }).hits.length, `the engine finds ${word}`);
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

test('engine.ts (MuPDF) refuses the scan ops as unsupported', () => {
  const engine = createEngine(mupdf);
  const session = engine.createSession();
  for (const op of ['scan-compose', 'scan-measure']) {
    const { reply } = engine.dispatch({ id: 1, op, pages: [], text: '' }, session);
    assert.deepEqual([reply.ok, reply.reason], [false, 'unsupported'], JSON.stringify(reply));
  }
});

