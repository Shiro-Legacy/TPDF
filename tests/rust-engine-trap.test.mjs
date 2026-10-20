// A real WASM trap through the production adapter (build-test/engine-rust.js): the
// worker-build Electron test injects a JavaScript throw, this is the engine's own abort. Runs under
// npm run test:rust (TPDF_ENGINE=rust), in a process of its own so the WASM memory starts small.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const skip = process.env.TPDF_ENGINE === 'rust' ? false : 'runs under npm run test:rust';

/** A one-page PDF filling [0 0 50 50]. */
function smallPdf() {
  const content = '0 0 50 50 re f';
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>',
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
  ];
  let text = '%PDF-1.7\n';
  const offsets = objects.map((body, index) => {
    const at = text.length;
    text += `${index + 1} 0 obj\n${body}\nendobj\n`;
    return at;
  });
  const xref = text.length;
  text += `xref\n0 5\n0000000000 65535 f \n${offsets.map(o => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}`;
  text += `trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return new TextEncoder().encode(text).buffer;
}

test('rust adapter: a real WASM trap stops its app; its sessions answer reason worker, a new session gets a fresh app', { skip }, async () => {
  const pkg = new URL('../engine/web/pkg/', import.meta.url);
  const wasm = await import(new URL('tandem_rust_workflow.js', pkg).href);
  const { memory } = wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });
  const { createRustEngine } = await import('../build-test/engine-rust.js');
  const engine = createRustEngine(wasm);
  const session = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'a.pdf', pdf: smallPdf() }, session).reply.ok, true);
  // wasm32 memory grown (untouched pages: no resident memory) until less than 32 MiB more can be
  // had; a request buffer larger than all the memory Rust had then fails its allocation in
  // push_input: Rust aborts, a trap.
  const before = memory.buffer.byteLength;
  for (;;) {
    try { memory.grow(512); } catch { break; }
  }
  const trapped = engine.dispatch({ id: 2, op: 'open', name: 'b.pdf', pdf: new ArrayBuffer(before + (64 << 20)) }, session).reply;
  assert.equal(trapped.ok, false);
  assert.equal(trapped.reason, 'worker', JSON.stringify(trapped));
  assert.match(trapped.error, /PDF engine stopped after an internal error/);
  const later = engine.dispatch({ id: 3, op: 'render', page: 0, scale: 1 }, session).reply;
  assert.equal(later.reason, 'worker', 'the trapped app stays stopped');
  const fresh = engine.createSession();
  assert.equal(engine.dispatch({ id: 4, op: 'open', name: 'c.pdf', pdf: smallPdf() }, fresh).reply.ok, true);
  const render = engine.dispatch({ id: 5, op: 'render', page: 0, scale: 1 }, fresh).reply;
  assert.equal(render.ok, true, render.error);
  assert.ok(render.nonWhite > 2000, String(render.nonWhite));
});
