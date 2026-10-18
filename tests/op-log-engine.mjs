// rust-engine.mjs (or, with real=1, mupdf-oracle.mjs) as a test file sees it under
// TPDF_OP_LOG (see op-log.mjs): the module the test would otherwise get, with
// each created engine's dispatch recorded. engine-select.mjs imports this as
// op-log-engine.mjs?file=<path>[&real=1], one instance per test file.
import { createEngine as createMupdfEngine } from './mupdf-oracle.mjs';
import { logged } from './op-log.mjs';

export * from './mupdf-oracle.mjs';

const params = new URL(import.meta.url).searchParams;
const file = params.get('file');
const rust = params.get('real') !== '1' ? await import('./rust-engine.mjs') : null;

export function createEngine(mupdf) {
  const engine = (rust ? rust.createEngine : createMupdfEngine)(mupdf);
  engine.dispatch = logged(engine.dispatch, rust ? 'rust' : 'mupdf', file);
  return engine;
}
