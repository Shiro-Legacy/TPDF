// Test-only engine switch. With TPDF_ENGINE=rust, every import that resolves to
// build/renderer/engine.js gets tests/rust-engine.mjs instead; otherwise this
// does nothing. Use: node --import ./tests/engine-select.mjs --test tests/*.test.mjs
import { registerHooks } from 'node:module';

// With TPDF_OP_LOG=<dir> (engine/tools/op-evidence.mjs), op-log.mjs is loaded
// here, before any test, and a test file's own imports of
// build/renderer/engine.js[?real] and build/renderer/engine-rust.js get
// op-log-engine.mjs and op-log-rust.mjs, which log every engine dispatch against
// the running test. Registered first, so it answers under the engine switch
// below; unset, none of this runs.
if (process.env.TPDF_OP_LOG) {
  await import('./op-log.mjs');
  const engine = new URL('../build/renderer/engine.js', import.meta.url).href;
  const adapter = new URL('../build/renderer/engine-rust.js', import.meta.url).href;
  const tests = new URL('./', import.meta.url).href;
  const logged = (url, parentURL) => {
    // Only a test file itself (no query: the stand-ins carry ?file=<test file>).
    if (!parentURL?.startsWith(tests) || !/^[\w.-]+\.test\.mjs$/.test(parentURL.slice(tests.length))) return null;
    const file = `?file=${encodeURIComponent(`tests/${parentURL.slice(tests.length)}`)}`;
    if (url === engine) return new URL(`./op-log-engine.mjs${file}`, import.meta.url).href;
    if (url === `${engine}?real`) return new URL(`./op-log-engine.mjs${file}&real=1`, import.meta.url).href;
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

if (process.env.TPDF_ENGINE === 'rust') {
  const engine = new URL('../build/renderer/engine.js', import.meta.url).href;
  const rust = new URL('./rust-engine.mjs', import.meta.url).href;
  registerHooks({
    resolve(specifier, context, nextResolve) {
      const resolved = nextResolve(specifier, context);
      return resolved.url === engine ? { ...resolved, url: rust } : resolved;
    },
  });
}
