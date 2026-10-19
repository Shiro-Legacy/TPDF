// The Rust engine the mobile app ships, for the Node tests: the WASM package (engine/web/pkg) behind the
// adapter the build ships (dist/engine/engine-rust.js; `npm test` builds first), with the committed CJK fallback font.
import { readFileSync } from 'node:fs';
import { createRustEngine } from '../dist/engine/engine-rust.js';
import { readCjkFont } from '../scripts/cjk-font.mjs';

const pkg = new URL('../../engine/web/pkg/', import.meta.url);
const wasm = await import(new URL('tandem_rust_workflow.js', pkg).href);
wasm.initSync({ module: readFileSync(new URL('tandem_rust_workflow_bg.wasm', pkg)) });

/** An engine on the package; `read` gives the CJK font when an op first needs it (module-wide, once). */
export function rustEngine(read = () => readCjkFont()) {
  const engine = createRustEngine(wasm);
  engine.useCjkFont(wasm, read);
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
