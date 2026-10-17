// The Rust engine the mobile app ships, for the Node tests: the WASM package (engine/web/pkg) behind the
// desktop build's adapter (build/renderer/engine-rust.js), with the committed CJK fallback font.
import { readFileSync } from 'node:fs';
import { createRustEngine } from '../../build/renderer/engine-rust.js';
import { readCjkFont } from '../../scripts/cjk-font.mjs';

const pkg = new URL('../../engine/web/pkg/', import.meta.url);
const wasm = await import(new URL('tandem_rust_workflow.js', pkg).href);
wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });

export function rustEngine() {
  const engine = createRustEngine(wasm);
  engine.useCjkFont(wasm, () => readCjkFont());
  return engine;
}

/** A session on its own engine: `send(request)` returns the reply and fails the test on a refusal. */
export function rustClient(assert) {
  const engine = rustEngine(), session = engine.createSession();
  let id = 0;
  return request => {
    const { reply } = engine.dispatch({ ...request, id: ++id }, session);
    assert.equal(reply.ok, true, JSON.stringify(reply));
    return reply;
  };
}
