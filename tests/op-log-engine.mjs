// build/renderer/engine.js as a test file sees it under TPDF_OP_LOG (see
// op-log.mjs): the module the test would otherwise get (rust-engine.mjs under
// TPDF_ENGINE=rust, else engine.js itself; engine.js?real always engine.js),
// with each created engine's dispatch recorded. engine-select.mjs imports this
// as op-log-engine.mjs?file=<path>[&real=1], one instance per test file.
import { createEngine as createMupdfEngine } from '../build/renderer/engine.js?real';
import { logged } from './op-log.mjs';

export * from '../build/renderer/engine.js?real';

const params = new URL(import.meta.url).searchParams;
const file = params.get('file');
const rust = process.env.TPDF_ENGINE === 'rust' && params.get('real') !== '1' ? await import('./rust-engine.mjs') : null;

export function createEngine(mupdf) {
  const engine = (rust ? rust.createEngine : createMupdfEngine)(mupdf);
  engine.dispatch = logged(engine.dispatch, rust ? 'rust' : 'mupdf', file);
  return engine;
}
