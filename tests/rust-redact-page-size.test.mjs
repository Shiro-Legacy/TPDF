// redact-apply on the Rust engine's WASM build returns promptly for pages whose size is not finite
// (final re-review, blocker): /UserUnit 1e36 made the page size infinite, the pixel check's full scale 0
// and its retry loop endless, so the worker never replied. Each case runs in a worker thread that is
// stopped after 30 s (a busy WASM loop would block this thread's own timers). Native cases:
// engine/tests/app_redact_pixel_scale.rs. Runs under npm run test:rust (TPDF_ENGINE=rust).
import test from 'node:test';
import assert from 'node:assert/strict';
import { Worker } from 'node:worker_threads';

const skip = process.env.TPDF_ENGINE === 'rust' ? false : 'runs under npm run test:rust';

const WORKER = `
const { parentPort, workerData } = require('node:worker_threads');
const { readFileSync } = require('node:fs');
(async () => {
  const pkg = workerData.pkg;
  const wasm = await import(pkg + 'tandem_rust_workflow.js');
  wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });
  const { createRustEngine } = await import(workerData.adapter);
  const engine = createRustEngine(wasm);
  const content = 'BT /F1 24 Tf 72 700 Td (SECRET TEXT) Tj ET';
  const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R ' + workerData.page + ' /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
    '<< /Length ' + content.length + ' >>\\nstream\\n' + content + '\\nendstream',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'];
  let body = '%PDF-1.7\\n'; const offsets = [];
  objects.forEach((o, i) => { offsets.push(body.length); body += (i + 1) + ' 0 obj\\n' + o + '\\nendobj\\n'; });
  const xref = body.length;
  body += 'xref\\n0 6\\n0000000000 65535 f \\n' + offsets.map(o => String(o).padStart(10, '0') + ' 00000 n \\n').join('');
  body += 'trailer\\n<< /Size 6 /Root 1 0 R >>\\nstartxref\\n' + xref + '\\n%%EOF\\n';
  const bytes = Buffer.from(body, 'latin1');
  const session = engine.createSession();
  let id = 1;
  const run = request => engine.dispatch({ id: id++, ...request }, session).reply;
  const opened = run({ op: 'open', name: 'x.pdf', pdf: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length) });
  if (!opened.ok) return parentPort.postMessage({ stage: 'open', reply: opened });
  run({ op: 'redact-add', page: 0, rect: [0, 0, 100, 100] });
  const sanitize = Object.fromEntries(['info', 'xmp', 'attachments', 'outlines', 'annots', 'links', 'fields', 'layers', 'struct'].map(k => [k, false]));
  const reply = run({ op: 'redact-apply', sanitize });
  parentPort.postMessage({ stage: 'apply', reply: { ok: reply.ok, reason: reply.reason, error: reply.error, verified: reply.verified } });
})();
`;

/** redact-apply of an area mark on a letter page with `page` entries, or null after 30 s. */
function applyOn(page) {
  const worker = new Worker(WORKER, {
    eval: true,
    workerData: {
      page,
      pkg: new URL('../engine/web/pkg/', import.meta.url).href,
      adapter: new URL('../build-test/engine-rust.js', import.meta.url).href,
    },
  });
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => { worker.terminate(); resolve(null); }, 30_000);
    worker.on('message', message => { clearTimeout(timer); worker.terminate(); resolve(message); });
    worker.on('error', error => { clearTimeout(timer); reject(error); });
  });
}

const zeros = n => '0'.repeat(n);

test('a /UserUnit past f32 refuses scan-budget promptly instead of never replying', { skip, timeout: 120_000 }, async () => {
  for (const unit of [`1${zeros(36)}.0`, `1${zeros(36)}`]) {
    const started = Date.now();
    const result = await applyOn(`/MediaBox [0 0 612 792] /UserUnit ${unit}`);
    assert.ok(result, `no reply in 30 s (${unit})`);
    assert.equal(result.stage, 'apply', JSON.stringify(result));
    assert.deepEqual([result.reply.ok, result.reply.reason], [false, 'scan-budget'], JSON.stringify(result.reply));
    assert.match(result.reply.error, /has no size that can be drawn at 36 dpi or more/);
    assert.ok(Date.now() - started < 10_000, `${Date.now() - started} ms`);
  }
});

test('any page size replies, and an apply that succeeds verified its pixels', { skip, timeout: 300_000 }, async () => {
  for (const page of [`/MediaBox [0 0 1${zeros(39)}.0 792]`, `/MediaBox [-3${zeros(38)} 0 3${zeros(38)} 792]`,
    '/MediaBox [0 0 612 792] /UserUnit 0', '/MediaBox [0 0 0 0]', '/MediaBox [0 0 612 0]', '/MediaBox [0 0 612 792]']) {
    const result = await applyOn(page);
    assert.ok(result, `no reply in 30 s (${page})`);
    assert.equal(result.stage, 'apply', JSON.stringify(result));
    if (result.reply.ok) assert.equal(result.reply.verified.pixels, 1, page);
    else assert.ok(['scan-budget', 'too-large', 'invalid'].includes(result.reply.reason), `${page}: ${JSON.stringify(result.reply)}`);
  }
});
