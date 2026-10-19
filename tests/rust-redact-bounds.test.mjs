// redact-preview and redact-apply on the Rust engine's WASM build stay within
// memory and time on crafted files (redaction acceptance, round 2):
// - clip paths inside a touched Form under many mark rects: the preview's
//   Form-part step recorded an entry per clip and rect before any bound
//   (50,000 clips under 100 search hits held 973 MB; 4,000 clips under 2,000
//   marks 1,345 MB; a file of tens of KB would pass wasm32's 4 GiB, a trap
//   that loses the document), and apply held a copy of every rect per clip
//   before its cut budget refused;
// - 262,144 rects from the file (Redact annotations sharing one /QuadPoints
//   array): region queries cloned and sorted every wide rect (74-89 s), and the
//   pixel check walked every pixel of every rect (290 s).
// WASM memory never shrinks, so its size after each case is the peak so far.
// Native cases: engine/tests/app_redact_bounds.rs. Runs under npm run
// test:rust (TPDF_ENGINE=rust).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { deflateSync, inflateSync } from 'node:zlib';

const skip = process.env.TPDF_ENGINE === 'rust' ? false : 'runs under npm run test:rust';
const pkg = new URL('../engine/web/pkg/', import.meta.url);

/** The production adapter over the WASM package, and the package's memory. */
async function rustEngine() {
  const wasm = await import(new URL('tandem_rust_workflow.js', pkg).href);
  const exports = wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });
  const { createRustEngine } = await import('../build/renderer/engine-rust.js');
  return { engine: createRustEngine(wasm), memory: () => exports.memory.buffer.byteLength };
}

/** A classic-xref PDF from object bodies (strings or Buffers) numbered from 1. */
function pdf(objects) {
  const parts = [Buffer.from('%PDF-1.7\n', 'latin1')];
  let length = parts[0].length;
  const offsets = [];
  objects.forEach((body, index) => {
    offsets.push(length);
    const chunk = Buffer.concat([
      Buffer.from(`${index + 1} 0 obj\n`, 'latin1'),
      Buffer.isBuffer(body) ? body : Buffer.from(body, 'latin1'),
      Buffer.from('\nendobj\n', 'latin1'),
    ]);
    parts.push(chunk);
    length += chunk.length;
  });
  const size = objects.length + 1;
  const xref = [`xref\n0 ${size}\n0000000000 65535 f \n`, ...offsets.map(o => `${String(o).padStart(10, '0')} 00000 n \n`)].join('');
  parts.push(Buffer.from(`${xref}trailer\n<< /Size ${size} /Root 1 0 R >>\nstartxref\n${length}\n%%EOF\n`, 'latin1'));
  const bytes = Buffer.concat(parts);
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length);
}

function stream(dict, data) {
  const bytes = deflateSync(Buffer.from(data, 'latin1'));
  return Buffer.concat([Buffer.from(`<< ${dict} /Filter /FlateDecode /Length ${bytes.length} >>\nstream\n`, 'latin1'), bytes, Buffer.from('\nendstream', 'latin1')]);
}

const HELV = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>';

/** A 612 × 792 page drawing `page` with Helvetica and the page-size Form /F drawing `form`. */
function formPage(page, form, { xobjects = '', annots = '', more = [] } = {}) {
  return pdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /XObject << /F 6 0 R ${xobjects} >> >> /Contents 4 0 R ${annots} >>`,
    stream('', page),
    HELV,
    stream('/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>', form),
    ...more,
  ]);
}

const SANITIZE = Object.fromEntries(['info', 'xmp', 'attachments', 'outlines', 'annots', 'links', 'fields', 'layers', 'struct'].map(k => [k, false]));

/** The copy's bytes as Latin-1 text, each Flate stream also inflated. */
function readable(buffer) {
  const bytes = Buffer.from(buffer);
  const text = bytes.toString('latin1');
  const parts = [text];
  for (let at = text.indexOf('stream\n'); at >= 0; at = text.indexOf('stream\n', at + 1)) {
    const end = text.indexOf('\nendstream', at);
    if (end < 0) break;
    try {
      parts.push(inflateSync(bytes.subarray(at + 7, end)).toString('latin1'));
    } catch {
      // Not Flate.
    }
  }
  return parts.join('\n');
}

test('rust engine: redaction of clips, many marks and file-borne quads stays within bounded WASM memory and time', { skip, timeout: 600_000 }, async () => {
  const { engine, memory } = await rustEngine();
  const megabytes = () => Math.round(memory() / 1048576);
  const before = megabytes();
  let id = 1;
  const run = (session, request) => engine.dispatch({ id: id++, ...request }, session).reply;
  const timed = (session, request) => {
    const start = performance.now();
    const reply = run(session, request);
    return { reply, ms: performance.now() - start };
  };
  const open = bytes => {
    const session = engine.createSession();
    const reply = run(session, { op: 'open', name: 'r.pdf', pdf: bytes });
    assert.equal(reply.ok, true, JSON.stringify(reply));
    return session;
  };
  const report = [];

  // Review probe p46: a page-size Form of 50,000 clips and one search with 100 hits.
  {
    const words = Array.from({ length: 100 }, (_, i) => `1 0 0 1 ${20 + (i % 20) * 28} ${770 - Math.floor(i / 20) * 15} Tm (ZQ) Tj`).join(' ');
    const bytes = formPage(`/F Do BT /F1 6 Tf ${words} ET`, `${'q 0 0 612 792 re W n Q\n'.repeat(50000)}0.9 g 0 0 10 10 re f`);
    assert.ok(bytes.byteLength < 8192, `${bytes.byteLength} bytes`);
    const session = open(bytes);
    assert.equal(run(session, { op: 'redact-search', text: 'ZQ' }).added, 100);
    const preview = timed(session, { op: 'redact-preview' });
    assert.equal(preview.reply.ok, true, JSON.stringify(preview.reply));
    assert.deepEqual(preview.reply.shared, []);
    const apply = timed(session, { op: 'redact-apply', sanitize: SANITIZE });
    assert.notEqual(apply.reply.reason, 'worker', apply.reply.error);
    if (apply.reply.ok) assert.ok(!readable(apply.reply.bytes).includes('ZQ'));
    else assert.equal(apply.reply.reason, 'too-large', apply.reply.error);
    report.push(`p46: preview ${Math.round(preview.ms)} ms, apply ${apply.reply.ok ? 'ok' : apply.reply.reason} ${Math.round(apply.ms)} ms, ${megabytes()} MB`);
    run(session, { op: 'close' });
    assert.ok(megabytes() - before < 256, `p46: WASM memory ${megabytes()} MB (from ${before} MB)`);
  }

  // Review probe p40 at 2,000 clips under 2,000 marks (the mark cap).
  {
    const bytes = formPage('/F Do', `${'0 0 612 792 re W n\n'.repeat(2000)}BT /F1 12 Tf 100 700 Td (HELLO) Tj ET`);
    const session = open(bytes);
    for (let i = 0; i < 2000; i++) {
      const x = 20 + (i % 50) * 11, y = 20 + Math.floor(i / 50) * 11;
      assert.equal(run(session, { op: 'redact-add', page: 0, rect: [x, y, x + 10, y + 10] }).ok, true);
    }
    const preview = timed(session, { op: 'redact-preview' });
    assert.equal(preview.reply.ok, true, JSON.stringify(preview.reply));
    assert.equal(preview.reply.marks, 2000);
    const apply = timed(session, { op: 'redact-apply', sanitize: SANITIZE });
    assert.notEqual(apply.reply.reason, 'worker', apply.reply.error);
    if (apply.reply.ok) assert.ok(!readable(apply.reply.bytes).includes('HELLO'));
    else assert.equal(apply.reply.reason, 'too-large', apply.reply.error);
    report.push(`p40: preview ${Math.round(preview.ms)} ms, apply ${apply.reply.ok ? 'ok' : apply.reply.reason} ${Math.round(apply.ms)} ms, ${megabytes()} MB`);
    run(session, { op: 'close' });
    assert.ok(megabytes() - before < 256, `p40: WASM memory ${megabytes()} MB (from ${before} MB)`);
  }

  // Review probe p41: 256 Redact annotations sharing one /QuadPoints array of
  // 1,024 copies of one quad (262,144 rects), over a Form with page-size clips,
  // with and without an image under the quad; then page-size quads.
  const quadsFile = (quad, clips, image) => {
    const quads = Array.from({ length: 1024 }, (_, i) => quad(i)).join(' ');
    const annots = Array.from({ length: 256 }, (_, i) => `${9 + i} 0 R`).join(' ');
    return formPage(
      `/F Do ${image ? 'q 200 0 0 40 100 680 cm /Im Do Q ' : ''}0.5 g 0 0 612 30 re f 0 g BT /F1 12 Tf 120 700 Td (MARKEDWORD) Tj ET`,
      `${'q 0 0 612 792 re W n Q\n'.repeat(clips)}BT /F1 12 Tf 100 400 Td (FORMTEXT) Tj ET`,
      {
        xobjects: '/Im 8 0 R',
        annots: `/Annots [${annots}]`,
        more: [
          `[${quads}]`,
          stream('/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8', '@P`p'),
          ...Array.from({ length: 256 }, () => '<< /Type /Annot /Subtype /Redact /Rect [100 680 300 720] /QuadPoints 7 0 R /P 3 0 R >>'),
        ],
      },
    );
  };
  const word = () => '100 720 300 720 100 680 300 680';
  const page = () => '0 792 612 792 0 0 612 0';
  for (const [name, quad, clips, image, kept] of [
    ['p41, 10 clips', word, 10, false, 'FORMTEXT'],
    ['p41, 8 clips and an image', word, 8, true, 'FORMTEXT'],
    ['262,144 page-size quads', page, 2, false, null],
  ]) {
    const bytes = quadsFile(quad, clips, image);
    assert.ok(bytes.byteLength < 96 * 1024, `${name}: ${bytes.byteLength} bytes`);
    const session = open(bytes);
    const preview = timed(session, { op: 'redact-preview' });
    assert.equal(preview.reply.ok, true, `${name}: ${JSON.stringify(preview.reply)}`);
    assert.equal(preview.reply.marks, 256);
    assert.ok(preview.ms < 15_000, `${name}: preview ${preview.ms} ms`);
    const apply = timed(session, { op: 'redact-apply', sanitize: SANITIZE });
    assert.equal(apply.reply.ok, true, `${name}: ${JSON.stringify(apply.reply).slice(0, 400)}`);
    assert.equal(apply.reply.verified.pixels, 256);
    assert.ok(apply.ms < 30_000, `${name}: apply ${apply.ms} ms`);
    const copy = readable(apply.reply.bytes);
    assert.ok(!copy.includes('MARKEDWORD'), name);
    if (kept) assert.ok(copy.includes(kept), name);
    report.push(`${name}: preview ${Math.round(preview.ms)} ms, apply ${Math.round(apply.ms)} ms, ${megabytes()} MB`);
    run(session, { op: 'close' });
    assert.ok(megabytes() - before < 256, `${name}: WASM memory ${megabytes()} MB (from ${before} MB)`);
  }
  console.log(`WASM memory from ${before} MB:\n  ${report.join('\n  ')}`);
});

// Re-review of round 3: a Form without a /BBox decided by its box (5,000 clips
// put its parts past the budget) lost text past the page edge from its other
// placement; every run in its own page-size clip refused `too-large`; glyph
// queries in a crowded index cell were charged to nothing.
test('rust engine: a bbox-less Form, clips around every run and crowded rects redact as they should', { skip, timeout: 600_000 }, async () => {
  const { engine, memory } = await rustEngine();
  const megabytes = () => Math.round(memory() / 1048576);
  const before = megabytes();
  let id = 1;
  const run = (session, request) => engine.dispatch({ id: id++, ...request }, session).reply;
  const open = bytes => {
    const session = engine.createSession();
    const reply = run(session, { op: 'open', name: 'r.pdf', pdf: bytes });
    assert.equal(reply.ok, true, JSON.stringify(reply));
    return session;
  };

  // Probe r3-bboxless: LEFTPART and OFFPAGE in a Form without /BBox, placed
  // with OFFPAGE past page 1's right edge and on page 2.
  for (const clips of [0, 5000]) {
    const pageDict = c => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> /XObject << /F 8 0 R >> >> /Contents ${c} 0 R >>`;
    const bytes = pdf([
      '<< /Type /Catalog /Pages 2 0 R >>',
      '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
      pageDict(5),
      pageDict(7),
      stream('', 'q 1 0 0 1 300 600 cm /F Do Q BT /F1 12 Tf 72 300 Td (BODYTEXT) Tj ET'),
      HELV,
      stream('', 'q 1 0 0 1 50 300 cm /F Do Q BT /F1 12 Tf 72 200 Td (PAGETWO) Tj ET'),
      stream('/Type /XObject /Subtype /Form /Resources << /Font << /F1 6 0 R >> >>',
        `${'q 0 0 612 792 re W n Q\n'.repeat(clips)}BT /F1 20 Tf 10 40 Td (LEFTPART) Tj ET BT /F1 20 Tf 330 40 Td (OFFPAGE) Tj ET`),
    ]);
    const session = open(bytes);
    assert.equal(run(session, { op: 'redact-add', page: 0, rect: [550, 120, 612, 170] }).ok, true);
    assert.equal(run(session, { op: 'redact-add', page: 1, rect: [300, 420, 362, 470] }).ok, true);
    const preview = run(session, { op: 'redact-preview' });
    assert.equal(preview.ok, true, JSON.stringify(preview));
    assert.notDeepEqual(preview.shared, [], `${clips} clips`);
    // R3: the shared Form is copied for the boxed placements instead of refusing.
    const apply = run(session, { op: 'redact-apply', sanitize: SANITIZE });
    assert.equal(apply.ok, true, `${clips} clips: ${JSON.stringify(apply).slice(0, 300)}`);
    run(session, { op: 'close' });
  }

  // Probe r1-clipruns: 3 pages of 600 runs, each in a page-size clip; search "the".
  {
    const pages = [];
    for (let p = 0; p < 3; p++) {
      pages.push(Array.from({ length: 600 }, (_, i) => `q 0 0 612 792 re W n BT /F1 5 Tf ${20 + (i % 10) * 58} ${(780 - Math.floor(i / 10) * (760 / 60)).toFixed(2)} Td (the w${i}) Tj ET Q`).join('\n'));
    }
    const objects = ['<< /Type /Catalog /Pages 2 0 R >>', `<< /Type /Pages /Kids [${pages.map((_, p) => `${4 + 2 * p} 0 R`).join(' ')}] /Count 3 >>`, HELV];
    pages.forEach((content, p) => {
      objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents ${5 + 2 * p} 0 R >>`);
      objects.push(stream('', content));
    });
    const session = open(pdf(objects));
    assert.equal(run(session, { op: 'redact-search', text: 'the' }).added, 1800);
    const apply = run(session, { op: 'redact-apply', sanitize: SANITIZE });
    assert.equal(apply.ok, true, JSON.stringify(apply).slice(0, 300));
    run(session, { op: 'close' });
  }

  // Probe r2-cluster: 4,096 tiny rects in one index cell, 100,000 zero-width glyphs there.
  {
    let seed = 1;
    const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
    const quads = [[0, 0, 1, 1], [611, 791, 612, 792]];
    for (let i = 0; i < 4096; i++) { const x = 100 + rnd() * 10, y = 100 + rnd() * 10; quads.push([x, y, x + 0.01, y + 0.01]); }
    const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>', null,
      stream('', `BT /F1 1 Tf 0 Tz 112 105 Td (${'x'.repeat(100000)}) Tj ET`), HELV];
    const annots = [];
    for (let k = 0; k < quads.length; k += 1024) {
      const qs = quads.slice(k, k + 1024).map(([x0, y0, x1, y1]) => [x0, y1, x1, y1, x0, y0, x1, y0].map(v => v.toFixed(4)).join(' '));
      annots.push(`${objects.length + 1} 0 R`);
      objects.push(`<< /Type /Annot /Subtype /Redact /Rect [0 0 612 792] /QuadPoints [${qs.join(' ')}] >>`);
    }
    objects[2] = `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [${annots.join(' ')}] >>`;
    const session = open(pdf(objects));
    const start = performance.now();
    const apply = run(session, { op: 'redact-apply', sanitize: SANITIZE });
    const ms = performance.now() - start;
    assert.equal(apply.reason, 'scan-budget', JSON.stringify(apply).slice(0, 300));
    assert.ok(ms < 15_000, `${ms} ms`);
    console.log(`cluster: apply refused scan-budget in ${Math.round(ms)} ms`);
    run(session, { op: 'close' });
  }
  assert.ok(megabytes() - before < 256, `WASM memory ${megabytes()} MB (from ${before} MB)`);
  console.log(`WASM memory from ${before} MB to ${megabytes()} MB`);
});

// Second re-review of round 3: the shared region lists' keys were not charged
// (2,000 zero-width file-borne quads under fills each with its own matrix:
// about 62 KB of WASM a fill, 1,493 MB at 24,000 fills), and one run of a
// million glyphs in a crowded cell ran 13.7-25.3 s before the query budget was
// checked.
test('rust engine: fills under many zero-width quads and one huge run stay bounded', { skip, timeout: 600_000 }, async () => {
  const { engine, memory } = await rustEngine();
  const megabytes = () => memory() / 1048576;
  let id = 1;
  const run = (session, request) => engine.dispatch({ id: id++, ...request }, session).reply;
  const fills = count => {
    const quads = Array.from({ length: 2000 }, (_, i) => { const x = (100 + i * 0.2).toFixed(2); return `${x} 401 ${x} 401 ${x} 400 ${x} 400`; });
    const content = Array.from({ length: count }, (_, i) => `q 1 0 0 1 ${(i * 0.001).toFixed(3)} 0 cm 0.5 g 0 0 612 792 re f Q`).join('\n');
    return pdf([
      '<< /Type /Catalog /Pages 2 0 R >>',
      '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
      '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [5 0 R 6 0 R] >>',
      stream('', content),
      `<< /Type /Annot /Subtype /Redact /Rect [0 0 612 792] /QuadPoints [${quads.slice(0, 1000).join(' ')}] >>`,
      `<< /Type /Annot /Subtype /Redact /Rect [0 0 612 792] /QuadPoints [${quads.slice(1000).join(' ')}] >>`,
    ]);
  };
  const applyOn = bytes => {
    const session = engine.createSession();
    assert.equal(run(session, { op: 'open', name: 'f.pdf', pdf: bytes }).ok, true);
    const start = performance.now();
    const reply = run(session, { op: 'redact-apply', sanitize: SANITIZE });
    const ms = performance.now() - start;
    run(session, { op: 'close' });
    return { reply, ms };
  };
  // Warm up at 200 fills, then measure 2,000: WASM memory never shrinks, so
  // the growth is the peak the extra 1,800 fills add.
  assert.equal(applyOn(fills(200)).reply.ok, true);
  const before = megabytes();
  const big = applyOn(fills(2000));
  assert.equal(big.reply.ok, true, JSON.stringify(big.reply).slice(0, 300));
  const perFill = (megabytes() - before) * 1048576 / 1800;
  console.log(`2,000 fills: apply ${Math.round(big.ms)} ms, WASM +${(megabytes() - before).toFixed(1)} MB (${Math.round(perFill)} bytes a fill)`);
  assert.ok(perFill < 8192, `${perFill} bytes a fill`);

  // Probe r2c: 4,096 rects in one cell under one run of 1,000,000 zero-width glyphs, in a Form.
  let seed = 1;
  const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
  const quads = [[0, 0, 1, 1], [611, 791, 612, 792]];
  for (let i = 0; i < 4096; i++) { const x = 100 + rnd() * 10, y = 100 + rnd() * 10; quads.push([x, y, x + 0.01, y + 0.01]); }
  const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>', null, stream('', '/Fm Do'), HELV,
    stream('/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>', `BT /F1 1 Tf 0 Tz 112 105 Td (${'x'.repeat(1000000)}) Tj ET`)];
  const annots = [];
  for (let k = 0; k < quads.length; k += 1024) {
    const qs = quads.slice(k, k + 1024).map(([x0, y0, x1, y1]) => [x0, y1, x1, y1, x0, y0, x1, y0].map(v => v.toFixed(4)).join(' '));
    annots.push(`${objects.length + 1} 0 R`);
    objects.push(`<< /Type /Annot /Subtype /Redact /Rect [0 0 612 792] /QuadPoints [${qs.join(' ')}] >>`);
  }
  objects[2] = `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /XObject << /Fm 6 0 R >> >> /Contents 4 0 R /Annots [${annots.join(' ')}] >>`;
  const session = engine.createSession();
  assert.equal(run(session, { op: 'open', name: 'c.pdf', pdf: pdf(objects) }).ok, true);
  let start = performance.now();
  const preview = run(session, { op: 'redact-preview' });
  const previewMs = performance.now() - start;
  assert.equal(preview.reason, 'scan-budget', JSON.stringify(preview).slice(0, 300));
  start = performance.now();
  const apply = run(session, { op: 'redact-apply', sanitize: SANITIZE });
  const applyMs = performance.now() - start;
  assert.equal(apply.reason, 'scan-budget', JSON.stringify(apply).slice(0, 300));
  console.log(`one run of 1,000,000 glyphs in a Form: preview refused in ${Math.round(previewMs)} ms, apply in ${Math.round(applyMs)} ms`);
  assert.ok(previewMs < 8000 && applyMs < 8000, `${previewMs} ${applyMs}`);
});
