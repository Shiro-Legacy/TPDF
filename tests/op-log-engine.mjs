// rust-engine.mjs as a test file sees it under TPDF_OP_LOG (see op-log.mjs):
// the module the test would otherwise get, with each created engine's dispatch
// recorded. engine-select.mjs imports this as op-log-engine.mjs?file=<path>,
// one instance per test file.
import { logged } from './op-log.mjs';
import * as rust from './rust-engine.mjs';

export * from './rust-engine.mjs';

const file = new URL(import.meta.url).searchParams.get('file');

export function createEngine() {
  const engine = rust.createEngine();
  engine.dispatch = logged(engine.dispatch, 'rust', file);
  return engine;
}
