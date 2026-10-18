// The CJK fallback font on the Rust engine (scripts/cjk-font.mjs): the
// adapter reads it the first time a render draws CJK text without it, hands it to the engine and draws again,
// so that render's reply already shows the text. The fake-bridge tests always run; the WASM tests need the
// Rust package (a Rust build, or npm run test:rust); the Electron test needs a Rust build.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { inflateSync } from 'node:zlib';
import mupdf from 'mupdf';
import { createRustEngine } from '../build/renderer/engine-rust.js';
import { ENGINE } from '../build/engine-build.js';
import { readCjkFont } from '../scripts/cjk-font.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const PKG = new URL('../engine/web/pkg/', import.meta.url);
const rustBuild = ENGINE === 'rust' || process.env.TPDF_ENGINE === 'rust';
const skipWasm = rustBuild && existsSync(new URL('tandem_rust_workflow_bg.wasm', PKG)) ? false : 'needs the Rust package (a Rust build)';
const skipElectron = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : ENGINE !== 'rust' && 'MuPDF build';
const STATE = { revision: 0, dirty: false, canUndo: false, canRedo: false, generation: 1, savedPosition: 0, locked: false, auth: 'none', name: 'a.pdf', sourceBytes: 1 };

/**
 * A stand-in for the WASM package: a request with `cjk: true` draws CJK text, which raises the cue while no
 * font is installed (and none was refused), as the engine's renders do; its reply says whether the text drew.
 */
function fakePackage({ refuse = false, trap = false } = {}) {
  const events = [];
  const state = { font: null, wanted: false, refused: false };
  class FakeApp {
    outputs = [];
    create_session() { return 1; }
    drop_session() {}
    clear_inputs() {}
    push_input() {}
    dispatch(_handle, json) {
      const request = JSON.parse(json);
      events.push(request.op);
      if (request.cjk && !state.font && !state.refused) state.wanted = true;
      if (request.fail) return JSON.stringify({ id: request.id, ok: false, error: 'render: no such page' });
      this.outputs = [new Uint8Array([state.font ? 1 : 0])];
      return JSON.stringify({ id: request.id, ok: true, drawn: !!(request.cjk && state.font), pixels: { $buffer: 0 }, paintMs: 0, renderMs: 0 });
    }
    output_count() { return this.outputs.length; }
    take_output(index) { return this.outputs[index]; }
    session_state() { return JSON.stringify(STATE); }
    snapshot_bytes() { return undefined; }
  }
  const route = {
    set_cjk_font(bytes) {
      events.push(`install ${bytes.length}`);
      if (trap) throw new WebAssembly.RuntimeError('unreachable');
      state.wanted = false;
      state.refused = refuse;
      if (!refuse) state.font = bytes;
      return !refuse;
    },
    cjk_font_wanted: () => state.wanted,
    has_cjk_font: () => state.font !== null,
  };
  return { wasm: { RustApp: FakeApp }, route, events };
}

/** An adapter over a fake package whose font reads are counted; `read` gives the bytes. */
function fakeEngine(read = () => new Uint8Array(8), options) {
  const pkg = fakePackage(options);
  const engine = createRustEngine(pkg.wasm);
  const reads = { count: 0 };
  engine.useCjkFont(pkg.route, () => {
    reads.count += 1;
    return read();
  });
  return { ...pkg, engine, session: engine.createSession(), reads };
}

test('rust adapter: the first render that wants the CJK font reads it once, installs it and draws again', () => {
  const { engine, session, events, reads } = fakeEngine();
  // No CJK text: no read.
  assert.equal(engine.dispatch({ id: 1, op: 'render', page: 0, scale: 1 }, session).reply.ok, true);
  assert.equal(reads.count, 0);
  const { reply, transfer } = engine.dispatch({ id: 2, op: 'render', page: 1, scale: 1, cjk: true }, session);
  assert.equal(reply.id, 2);
  assert.equal(reply.drawn, true, 'the reply is the second draw');
  assert.deepEqual([...new Uint8Array(reply.pixels)], [1]);
  assert.deepEqual(transfer, [reply.pixels]);
  assert.deepEqual(events, ['render', 'render', 'install 8', 'render']);
  assert.equal(reads.count, 1);
  // Installed: later renders draw once and read nothing.
  assert.equal(engine.dispatch({ id: 3, op: 'thumbnail', page: 1, maxEdge: 100, cjk: true }, session).reply.drawn, true);
  assert.deepEqual(events.slice(4), ['thumbnail']);
  assert.equal(reads.count, 1);
});

test('rust adapter: thumbnails and compare renders draw again; other ops and failed renders are not repeated', () => {
  for (const op of ['thumbnail', 'compare-render']) {
    const { engine, session, events } = fakeEngine();
    assert.equal(engine.dispatch({ id: 1, op, page: 0, cjk: true }, session).reply.drawn, true, op);
    assert.deepEqual(events, [op, 'install 8', op]);
  }
  // Not a render: the font is installed for the next one, the op runs once.
  let fake = fakeEngine();
  assert.equal(fake.engine.dispatch({ id: 1, op: 'text-all', cjk: true }, fake.session).reply.ok, true);
  assert.deepEqual(fake.events, ['text-all', 'install 8']);
  // A failed render is not drawn again.
  fake = fakeEngine();
  const failed = fake.engine.dispatch({ id: 2, op: 'render', page: 9, cjk: true, fail: true }, fake.session).reply;
  assert.equal(failed.ok, false);
  assert.deepEqual(fake.events, ['render', 'install 8']);
});

test('rust adapter: a CJK font that cannot be read, or that the engine refuses, is not asked for again', () => {
  const unreadable = [() => null, () => { throw new Error('HTTP 404'); }];
  for (const read of unreadable) {
    const { engine, session, events, reads } = fakeEngine(read);
    for (const id of [1, 2]) assert.equal(engine.dispatch({ id, op: 'render', page: 0, scale: 1, cjk: true }, session).reply.drawn, false);
    assert.deepEqual(events, ['render', 'render'], 'nothing installed, nothing drawn twice');
    assert.equal(reads.count, 1);
  }
  const { engine, session, events, reads } = fakeEngine(undefined, { refuse: true });
  for (const id of [1, 2]) assert.equal(engine.dispatch({ id, op: 'render', page: 0, scale: 1, cjk: true }, session).reply.drawn, false);
  assert.deepEqual(events, ['render', 'install 8', 'render']);
  assert.equal(reads.count, 1);
});

test('rust adapter: redact-apply installs the CJK font before it runs, so its pixel check draws CJK text', () => {
  const { engine, session, events, reads } = fakeEngine();
  assert.equal(engine.dispatch({ id: 1, op: 'redact-apply' }, session).reply.ok, true);
  assert.equal(engine.dispatch({ id: 2, op: 'redact-apply' }, session).reply.ok, true);
  assert.equal(engine.dispatch({ id: 3, op: 'render', page: 0, scale: 1, cjk: true }, session).reply.drawn, true);
  assert.deepEqual(events, ['install 8', 'redact-apply', 'redact-apply', 'render']);
  assert.equal(reads.count, 1);
});

test('rust adapter: the time a render reports includes the font read and the first draw', () => {
  const { engine, session } = fakeEngine(() => {
    const until = performance.now() + 30;
    while (performance.now() < until) { /* a slow read */ }
    return new Uint8Array(8);
  });
  const { reply } = engine.dispatch({ id: 1, op: 'render', page: 0, scale: 1, cjk: true }, session);
  assert.ok(reply.renderMs >= 30 && reply.paintMs >= 30, JSON.stringify(reply));
});

test('rust adapter: a trap while the font is installed stops the engine as a trap in an op does', () => {
  const { engine, session, events } = fakeEngine(undefined, { trap: true });
  const { reply } = engine.dispatch({ id: 1, op: 'render', page: 0, scale: 1, cjk: true }, session);
  assert.equal(reply.ok, false);
  assert.equal(reply.reason, 'worker');
  assert.equal(engine.dispatch({ id: 2, op: 'render', page: 0, scale: 1 }, session).reply.reason, 'worker');
  assert.deepEqual(events, ['render', 'install 8']);
});

/** A PDF from 1-based indirect object bodies. */
function pdf(objects) {
  let out = '%PDF-1.7\n';
  const offsets = [];
  objects.forEach((body, index) => {
    offsets.push(out.length);
    out += `${index + 1} 0 obj\n${body}\nendobj\n`;
  });
  const xref = out.length;
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('')}`;
  out += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(out, 'latin1');
}

/** 日本 at 80 pt in a GB1 Type0 font without a program (a CIDFontType0 descendant, UTF-16 CMap) on a 200 x 120 page. */
function cjkPage() {
  const content = 'BT /C0 80 Tf 10 30 Td <65E5672C> Tj ET';
  return pdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 120] /Resources << /Font << /C0 5 0 R >> >> /Contents 4 0 R >>',
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    '<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UTF16-H /DescendantFonts [6 0 R] >>',
    '<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 5 >> /FontDescriptor 7 0 R >>',
    '<< /Type /FontDescriptor /FontName /STSong-Light /Flags 4 /FontBBox [0 -120 1000 880] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>',
  ]);
}

/** 日本語文 at 40 pt from x = 10 in the same font, clipped to x below 70: 日 inside, 本 across, 語 and 文 outside. */
function clippedPage() {
  const content = 'q 0 0 70 120 re W n BT /C0 40 Tf 10 40 Td <65E5672C8A9E6587> Tj ET Q';
  const objects = cjkPage().toString('latin1').match(/\d+ 0 obj\n[\s\S]*?\nendobj/g).map(object => object.replace(/^\d+ 0 obj\n|\nendobj$/g, ''));
  objects[3] = `<< /Length ${content.length} >>\nstream\n${content}\nendstream`;
  return pdf(objects);
}

/**
 * Cells of 60 pt, each in its own GB1 Type0 font without a program (Identity-H): code <0FA2> (CID 4002) with no
 * ToUnicode, then with a ToUnicode that lacks it, maps it to two characters, to U+FFFF and to a lone surrogate (hayro
 * finds no Unicode for these four and takes the glyph numbered like the CID); last, CID 40000, past the collection.
 */
function cidPage() {
  const toUnicode = [null, ['<FFFF> <0041>'], ['<0FA2> <51F60041>'], ['<0FA2> <FFFF>'], ['<0FA2> <D800>'], null];
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    null,
    null,
    '<< /Type /FontDescriptor /FontName /STSong-Light /Flags 4 /FontBBox [0 -120 1000 880] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>',
  ];
  const fonts = [];
  let content = 'BT';
  toUnicode.forEach((entries, cell) => {
    objects.push('<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 5 >> /FontDescriptor 5 0 R >>');
    const descendant = objects.length;
    let unicode = '';
    if (entries) {
      const cmap = `/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /U def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange ${entries.length} beginbfchar ${entries.join(' ')} endbfchar endcmap CMapName currentdict /CMap defineresource pop end end`;
      objects.push(`<< /Length ${cmap.length} >>\nstream\n${cmap}\nendstream`);
      unicode = ` /ToUnicode ${objects.length} 0 R`;
    }
    objects.push(`<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /Identity-H /DescendantFonts [${descendant} 0 R]${unicode} >>`);
    fonts.push(`/F${cell} ${objects.length} 0 R`);
    content += ` /F${cell} 60 Tf 1 0 0 1 ${10 + 70 * cell} 20 Tm <${cell === 5 ? '9C40' : '0FA2'}> Tj`;
  });
  content += ' ET';
  objects[2] = `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 440 100] /Resources << /Font << ${fonts.join(' ')} >> >> /Contents 4 0 R >>`;
  objects[3] = `<< /Length ${content.length} >>\nstream\n${content}\nendstream`;
  return pdf(objects);
}

/** A page with one empty text field (no border, no background) at [40 40 260 80]. */
function fieldPage() {
  return pdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 120] /Annots [4 0 R] >>',
    '<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /F 4 /P 3 0 R /Rect [40 40 260 80] /DA (/Helv 24 Tf 0 g) >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
  ]);
}

const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);

/** Pixels of a render whose darkest channel is below 128, inside `rect` (pixels: x0, y0, x1, y1) when given. */
function ink({ pixels, pixelWidth: width, pixelHeight: height }, rect = [0, 0, width, height]) {
  const data = new Uint8Array(pixels);
  let count = 0;
  for (let y = Math.max(0, rect[1]); y < Math.min(height, rect[3]); y++) {
    for (let x = Math.max(0, rect[0]); x < Math.min(width, rect[2]); x++) {
      const at = (y * width + x) * 4;
      if (Math.min(data[at], data[at + 1], data[at + 2]) < 128) count++;
    }
  }
  return count;
}

/** Share of the pixels either image inks (a channel below 192) that agree within 16 levels. */
function inkAgreement(a, b) {
  const pa = new Uint8Array(a.pixels);
  const pb = new Uint8Array(b.pixels);
  let inked = 0;
  let close = 0;
  for (let at = 0; at < pa.length; at += 4) {
    const dark = Math.min(pa[at], pa[at + 1], pa[at + 2], pb[at], pb[at + 1], pb[at + 2]) < 192;
    if (!dark) continue;
    inked++;
    if (Math.max(Math.abs(pa[at] - pb[at]), Math.abs(pa[at + 1] - pb[at + 1]), Math.abs(pa[at + 2] - pb[at + 2])) <= 16) close++;
  }
  return inked ? close / inked : 1;
}

/** Open `bytes` in a new session of `engine`, run `requests`, and return the last reply. */
function run(engine, bytes, ...requests) {
  const session = engine.createSession();
  let reply = engine.dispatch({ id: 1, op: 'open', name: 'cjk.pdf', pdf: buffer(bytes) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  for (const request of requests) {
    reply = engine.dispatch({ id: 2, ...request }, session).reply;
    assert.equal(reply.ok, true, `${request.op}: ${reply.error}`);
  }
  engine.dispatch({ id: 3, op: 'close' }, session);
  return reply;
}

test('rust engine: non-embedded CJK text and CJK field values draw once the adapter installs the font on the cue', { skip: skipWasm }, async () => {
  const wasm = await import(new URL('tandem_rust_workflow.js', PKG).href);
  wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', PKG)) });
  // The font is module-wide; this file's process starts without it.
  assert.equal(wasm.has_cjk_font(), false);
  const field = engine => {
    const session = engine.createSession();
    assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'field.pdf', pdf: buffer(fieldPage()) }, session).reply.ok, true);
    const widget = engine.dispatch({ id: 2, op: 'fields', page: 0 }, session).reply.items.find(item => item.name === 'name');
    const set = engine.dispatch({ id: 3, op: 'field-set', page: 0, widgetId: widget.id, value: '日本語' }, session).reply;
    assert.equal(set.ok, true, set.error);
    const render = engine.dispatch({ id: 4, op: 'render', page: 0, scale: 2 }, session).reply;
    assert.equal(render.ok, true, render.error);
    engine.dispatch({ id: 5, op: 'close' }, session);
    return ink(render, widget.rect.map(value => Math.round(value * 2)));
  };

  // Without a font route: the text extracts but draws nothing, and the render raises the cue.
  const bare = createRustEngine(wasm);
  const clipped = run(bare, clippedPage(), { op: 'text-all' }).text;
  assert.equal(clipped, '日本', 'kept by em boxes where the run meets the clip');
  const before = run(bare, cjkPage(), { op: 'render', page: 0, scale: 1 });
  assert.equal(ink(before), 0);
  assert.match(before.text, /日本/);
  assert.equal(wasm.cjk_font_wanted(), true);
  assert.equal(field(bare), 0, 'the CJK field value draws nothing');

  // With the app's route: the first render reads the font once and its reply has the glyphs.
  const engine = createRustEngine(wasm);
  let reads = 0;
  engine.useCjkFont(wasm, () => {
    reads += 1;
    return readCjkFont();
  });
  const after = run(engine, cjkPage(), { op: 'render', page: 0, scale: 1 });
  assert.equal(reads, 1);
  assert.equal(wasm.has_cjk_font(), true);
  assert.equal(wasm.cjk_font_wanted(), false);
  assert.ok(ink(after) > 1000, `ink ${ink(after)}`);
  assert.equal(after.text, before.text);
  const glyphs = field(engine);
  assert.ok(glyphs > 300, `field ink ${glyphs}`);
  assert.equal(reads, 1, 'read once');
  assert.equal(run(engine, clippedPage(), { op: 'text-all' }).text, clipped, 'the same text with the font installed');

  // MuPDF draws the same page with the same font (inside its WASM).
  const { createEngine } = await import('./mupdf-oracle.mjs');
  const oracle = run(createEngine(mupdf), cjkPage(), { op: 'render', page: 0, scale: 1 });
  assert.deepEqual([oracle.pixelWidth, oracle.pixelHeight], [after.pixelWidth, after.pixelHeight]);
  const agreement = inkAgreement(after, oracle);
  assert.ok(agreement > 0.85, `ink within 16 levels of MuPDF: ${agreement}`);
});

test('rust engine: a non-embedded CJK code hayro finds no Unicode for draws its CID\'s glyph', { skip: skipWasm }, async () => {
  const wasm = await import(new URL('tandem_rust_workflow.js', PKG).href);
  wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', PKG)) });
  const engine = createRustEngine(wasm);
  engine.useCjkFont(wasm, () => readCjkFont());
  const rust = run(engine, cidPage(), { op: 'render', page: 0, scale: 1 });
  const { createEngine } = await import('./mupdf-oracle.mjs');
  const oracle = run(createEngine(mupdf), cidPage(), { op: 'render', page: 0, scale: 1 });
  /** Inked pixels (a channel below 128) of each 70-pixel cell, as y * 1000 + x within the cell. */
  const cells = ({ pixels, pixelWidth: width, pixelHeight: height }) => {
    const data = new Uint8Array(pixels);
    return [0, 1, 2, 3, 4, 5].map(cell => {
      const inked = [];
      for (let y = 0; y < height; y++) {
        for (let x = 5 + 70 * cell; x < 75 + 70 * cell; x++) {
          const at = (y * width + x) * 4;
          if (Math.min(data[at], data[at + 1], data[at + 2]) < 128) inked.push(y * 1000 + x - 70 * cell);
        }
      }
      return inked;
    });
  };
  const [ours, theirs] = [cells(rust), cells(oracle)];
  for (const cell of [0, 1, 2, 3, 4]) {
    const shared = new Set(theirs[cell]);
    const both = ours[cell].filter(at => shared.has(at)).length;
    const either = new Set([...ours[cell], ...theirs[cell]]).size;
    assert.ok(theirs[cell].length > 500, `MuPDF draws cell ${cell}`);
    assert.ok(both / either > 0.9, `cell ${cell}: ink overlap with MuPDF ${both / either}`);
  }
  assert.deepEqual([ours[5].length, theirs[5].length], [0, 0], 'a CID past the collection draws nothing');
});

/** RGBA pixels of an 8-bit, non-interlaced RGB or RGBA PNG. */
function decodePng(bytes) {
  const width = bytes.readUInt32BE(16);
  const height = bytes.readUInt32BE(20);
  const channels = { 2: 3, 6: 4 }[bytes[25]];
  assert.ok(bytes[24] === 8 && channels && bytes[28] === 0, 'an 8-bit RGB or RGBA PNG');
  const chunks = [];
  for (let at = 8; at < bytes.length;) {
    const length = bytes.readUInt32BE(at);
    if (bytes.toString('latin1', at + 4, at + 8) === 'IDAT') chunks.push(bytes.subarray(at + 8, at + 8 + length));
    at += 12 + length;
  }
  const raw = inflateSync(Buffer.concat(chunks));
  const stride = width * channels;
  const rows = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    for (let x = 0; x < stride; x++) {
      const value = raw[y * (stride + 1) + 1 + x];
      const left = x >= channels ? rows[y * stride + x - channels] : 0;
      const up = y > 0 ? rows[(y - 1) * stride + x] : 0;
      const corner = x >= channels && y > 0 ? rows[(y - 1) * stride + x - channels] : 0;
      const p = left + up - corner;
      const [a, b, c] = [Math.abs(p - left), Math.abs(p - up), Math.abs(p - corner)];
      const paeth = a <= b && a <= c ? left : b <= c ? up : corner;
      rows[y * stride + x] = (value + [0, left, up, (left + up) >> 1, paeth][filter]) & 0xff;
    }
  }
  const pixels = new Uint8Array(width * height * 4);
  for (let index = 0; index < width * height; index++) {
    pixels.set(rows.subarray(index * channels, index * channels + 3), index * 4);
    pixels[index * 4 + 3] = 255;
  }
  return { pixelWidth: width, pixelHeight: height, pixels: pixels.buffer };
}

test('app (Rust build): a page of non-embedded CJK text exports with its glyphs, the worker reading the font on demand', { skip: skipElectron, timeout: 120_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-cjk-font-'));
  const electron = createRequire(import.meta.url)('electron');
  const smoke = async (name, args) => {
    const report = join(dir, `${name}.json`);
    const child = spawn(electron, [`--user-data-dir=${join(dir, `udata-${name}`)}`, '.', '--smoke', `--report=${report}`, ...args], { cwd: root, stdio: 'ignore' });
    const code = await new Promise(done => child.on('close', done));
    const result = JSON.parse(await readFile(report, 'utf8'));
    assert.equal(result.ok, true, result.error);
    assert.equal(code, 0);
    return result;
  };
  try {
    // hello.pdf has no CJK text: the worker never reads the font.
    const hello = await smoke('hello', []);
    assert.ok(hello.protocolRequests.includes('vendor/tandem_rust_workflow_bg.wasm'), hello.protocolRequests.join(' '));
    assert.ok(!hello.protocolRequests.includes('fonts/DroidSansFallback.ttf'), hello.protocolRequests.join(' '));
    const page = join(dir, 'cjk.pdf');
    const png = join(dir, 'cjk.png');
    await writeFile(page, cjkPage());
    const cjk = await smoke('cjk', [`--open=${page}`, '--edit=export-png', `--save-to=${png}`]);
    assert.ok(cjk.protocolRequests.includes('fonts/DroidSansFallback.ttf'), cjk.protocolRequests.join(' '));
    const image = decodePng(await readFile(png));
    assert.ok(ink(image) > 4000, `ink ${ink(image)}`);
  } finally {
    await rm(dir, { recursive: true, force: true, maxRetries: 5 });
  }
});
