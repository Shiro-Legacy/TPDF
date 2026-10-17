// An image whose data is too large to hold, on the Rust engine's WASM build
// (the integration acceptance review, round 3): a 561 KB file holding one
// 24000 × 24000 grayscale Flate image (576 MB of samples). hayro-syntax
// holds an image's decoded data whole within 512 MiB, so Rust decoded
// 512 MiB, refused the page, and WASM memory stayed at 1,030 MB (linear
// memory never shrinks). MuPDF draws the page subsampled. The data is now
// decoded a strip at a time into the reduced image: the page draws and WASM
// memory stays under 256 MiB. Native cases: engine/tests/view_large_images.rs.
// Runs under npm run test:rust (TPDF_ENGINE=rust).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createDeflate } from 'node:zlib';

const skip = process.env.TPDF_ENGINE === 'rust' ? false : 'runs under npm run test:rust';
const pkg = new URL('../engine/web/pkg/', import.meta.url);

async function rustEngine() {
  const wasm = await import(new URL('tandem_rust_workflow.js', pkg).href);
  const exports = wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });
  const { createRustEngine } = await import('../build/renderer/engine-rust.js');
  return { engine: createRustEngine(wasm), memory: () => exports.memory.buffer.byteLength };
}

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
  const bytes = Buffer.isBuffer(data) ? data : Buffer.from(data, 'latin1');
  return Buffer.concat([Buffer.from(`<< ${dict} /Length ${bytes.length} >>\nstream\n`, 'latin1'), bytes, Buffer.from('\nendstream', 'latin1')]);
}

/** `rows` rows from `row(index)`, deflated a row at a time. */
async function deflateRows(rows, row) {
  return await new Promise((resolve, reject) => {
    const deflate = createDeflate({ level: 9 });
    const parts = [];
    deflate.on('data', part => parts.push(part));
    deflate.on('end', () => resolve(Buffer.concat(parts)));
    deflate.on('error', reject);
    let index = 0;
    const pump = () => {
      while (index < rows) {
        if (!deflate.write(row(index++))) return void deflate.once('drain', pump);
      }
      deflate.end();
    };
    pump();
  });
}

test('rust engine: an image too large to hold draws reduced within bounded WASM memory', { skip, timeout: 600_000 }, async () => {
  const { engine, memory } = await rustEngine();
  const megabytes = () => Math.round(memory() / 1048576);
  const side = 24000;
  const black = Buffer.alloc(side, 0);
  const white = Buffer.alloc(side, 255);
  const data = await deflateRows(side, row => (row < side / 2 ? black : white));
  const bytes = pdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>',
    stream('', 'q 100 0 0 100 0 0 cm /Im Do Q'),
    stream(`/Type /XObject /Subtype /Image /Width ${side} /Height ${side} /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode`, data),
  ]);
  assert.ok(bytes.byteLength < 1 << 20, `${bytes.byteLength} bytes`);
  const session = engine.createSession();
  const open = engine.dispatch({ id: 1, op: 'open', name: 'x.pdf', pdf: bytes }, session).reply;
  assert.equal(open.ok, true, JSON.stringify(open));
  for (const [id, scale] of [[2, 1], [3, 2]]) {
    const render = engine.dispatch({ id, op: 'render', page: 0, scale }, session).reply;
    assert.equal(render.ok, true, `scale ${scale}: ${render.error}`);
    // Half the page black.
    const pixels = (100 * scale) ** 2;
    assert.ok(Math.abs(render.nonWhite - pixels / 2) < pixels / 20, `scale ${scale}: ${render.nonWhite} of ${pixels}`);
  }
  const thumbnail = engine.dispatch({ id: 4, op: 'thumbnail', page: 0, maxEdge: 256 }, session).reply;
  assert.equal(thumbnail.ok, true, thumbnail.error);
  engine.dispatch({ id: 5, op: 'close' }, session);
  assert.ok(megabytes() < 256, `WASM memory ${megabytes()} MB`);
  // The same image drawn 2 points tall: each output row is 12,000 rows of
  // data, which the first version gathered as one strip (1,030 MB of WASM).
  const tiny = pdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>',
    stream('', 'q 100 0 0 2 0 0 cm /Im Do Q'),
    stream(`/Type /XObject /Subtype /Image /Width ${side} /Height ${side} /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode`, data),
  ]);
  const small = engine.createSession();
  assert.equal(engine.dispatch({ id: 6, op: 'open', name: 'tiny.pdf', pdf: tiny }, small).reply.ok, true);
  const drawn = engine.dispatch({ id: 7, op: 'render', page: 0, scale: 1 }, small).reply;
  assert.equal(drawn.ok, true, drawn.error);
  engine.dispatch({ id: 8, op: 'close' }, small);
  assert.ok(megabytes() < 256, `WASM memory ${megabytes()} MB after the tiny draw`);
});
