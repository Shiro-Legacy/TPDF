// Test-only engine switch. With TPDF_ENGINE=rust, every import that resolves to
// build/renderer/engine.js gets tests/rust-engine.mjs instead; otherwise this
// does nothing. Use: node --import ./tests/engine-select.mjs --test tests/*.test.mjs
import { registerHooks } from 'node:module';

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
