// What vello keeps of a page's fills until the page is composed, on the Rust
// engine's WASM build (the redaction re-review, fix round 3): a page of
// letter-size fills grew WASM memory by about 32 KB a fill until the scene
// budget (then 1 GiB of estimate) refused it, 28,000 fills reaching 844 MB.
// The budget is now 384 MiB of the same estimate: 10,000 such fills draw,
// 11,000 are refused, and WASM memory stays under 512 MiB. Native case with
// the counter: engine/tests/view_scene_budget.rs. Runs under npm run
// test:rust (TPDF_ENGINE=rust).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';

const skip = process.env.TPDF_ENGINE === 'rust' ? false : 'runs under npm run test:rust';
const pkg = new URL('../engine/web/pkg/', import.meta.url);

async function rustEngine() {
  const wasm = await import(new URL('tandem_rust_workflow.js', pkg).href);
  const exports = wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });
  const { createRustEngine } = await import('../build-test/engine-rust.js');
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

function fills(count) {
  const data = deflateSync(Buffer.from('0.5 g 0 0 612 792 re f\n'.repeat(count), 'latin1'), { level: 9 });
  return pdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R >>',
    Buffer.concat([Buffer.from(`<< /Filter /FlateDecode /Length ${data.length} >>\nstream\n`, 'latin1'), data, Buffer.from('\nendstream', 'latin1')]),
  ]);
}

test('rust engine: page-size fills are kept within the scene budget', { skip, timeout: 600_000 }, async () => {
  const { engine, memory } = await rustEngine();
  const megabytes = () => Math.round(memory() / 1048576);
  for (const [count, drawn] of [[10000, true], [11000, false], [32000, false]]) {
    const session = engine.createSession();
    assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'f.pdf', pdf: fills(count) }, session).reply.ok, true);
    const reply = engine.dispatch({ id: 2, op: 'render', page: 0, scale: 1 }, session).reply;
    assert.notEqual(reply.reason, 'worker', `${count}: ${reply.error}`);
    assert.equal(reply.ok, drawn, `${count}: ${reply.error}`);
    if (!drawn) assert.match(reply.error, /too complex/, `${count}: ${reply.error}`);
    engine.dispatch({ id: 3, op: 'close' }, session);
    assert.ok(megabytes() < 512, `${count} fills: WASM memory ${megabytes()} MB`);
  }
});
