// Test-only op-log hook (tests import the Rust engine through rust-engine.mjs
// directly; there is no engine switch any more). Use:
// node --import ./tests/engine-select.mjs --test tests/*.test.mjs
import { registerHooks } from 'node:module';

// With TPDF_OP_LOG=<dir> (engine/tools/op-evidence.mjs), op-log.mjs is loaded
// here, before any test, and a test file's own imports of rust-engine.mjs,
// mupdf-oracle.mjs and build-test/engine-rust.js get op-log-engine.mjs
// (with real=1 for the oracle) and op-log-rust.mjs, which log every engine dispatch against
// the running test. Unset, none of this runs.
if (process.env.TPDF_OP_LOG) {
  await import('./op-log.mjs');
  const engine = new URL('./rust-engine.mjs', import.meta.url).href;
  const oracle = new URL('./mupdf-oracle.mjs', import.meta.url).href;
  const adapter = new URL('../build-test/engine-rust.js', import.meta.url).href;
  const tests = new URL('./', import.meta.url).href;
  const logged = (url, parentURL) => {
    // Only a test file itself (no query: the stand-ins carry ?file=<test file>).
    if (!parentURL?.startsWith(tests) || !/^[\w.-]+\.test\.mjs$/.test(parentURL.slice(tests.length))) return null;
    const file = `?file=${encodeURIComponent(`tests/${parentURL.slice(tests.length)}`)}`;
    if (url === engine) return new URL(`./op-log-engine.mjs${file}`, import.meta.url).href;
    if (url === oracle) return new URL(`./op-log-engine.mjs${file}&real=1`, import.meta.url).href;
    if (url === adapter) return new URL(`./op-log-rust.mjs${file}`, import.meta.url).href;
    return null;
  };
  registerHooks({
    resolve(specifier, context, nextResolve) {
      const resolved = nextResolve(specifier, context);
      const url = logged(resolved.url, context.parentURL);
      return url ? { url, format: 'module', shortCircuit: true } : resolved;
    },
  });
}
