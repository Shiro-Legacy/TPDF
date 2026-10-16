// build/renderer/engine-rust.js as a test file sees it under TPDF_OP_LOG (see
// op-log.mjs): the same adapter, with each engine's dispatch recorded as
// "rust-direct" when its RustApp is engine/web/pkg's (or a subclass of it), else
// "rust-fake" (a stand-in bridge that runs no engine). rust-engine.mjs imports
// the adapter itself, unwrapped. engine-select.mjs imports this as
// op-log-rust.mjs?file=<path>, one instance per test file.
import { createRustEngine as createRealRustEngine } from '../build/renderer/engine-rust.js';
import { logged } from './op-log.mjs';

export * from '../build/renderer/engine-rust.js';

const file = new URL(import.meta.url).searchParams.get('file');
const pkg = await import('../engine/web/pkg/tandem_rust_workflow.js').catch(() => null);

function real(wasm) {
  const app = wasm?.RustApp;
  return Boolean(pkg && typeof app === 'function' && (app === pkg.RustApp || app.prototype instanceof pkg.RustApp));
}

export function createRustEngine(wasm, ...rest) {
  const engine = createRealRustEngine(wasm, ...rest);
  engine.dispatch = logged(engine.dispatch, real(wasm) ? 'rust-direct' : 'rust-fake', file);
  return engine;
}
