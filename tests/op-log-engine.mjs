// rust-engine.mjs as a test file sees it under TPDF_OP_LOG (see op-log.mjs):
// the same module, with each created engine's dispatch recorded.
// engine-select.mjs imports this as op-log-engine.mjs?file=<path>, one
// instance per test file.
import { createEngine as createRustEngine } from './rust-engine.mjs';
import { logged } from './op-log.mjs';

export * from './rust-engine.mjs';

const file = new URL(import.meta.url).searchParams.get('file');

export function createEngine(mupdf) {
  const engine = createRustEngine(mupdf);
  engine.dispatch = logged(engine.dispatch, 'rust', file);
  return engine;
}
