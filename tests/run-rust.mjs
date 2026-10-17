// npm run test:rust: the unit suite on the Rust engine, the app's only engine
// (every test imports it through rust-engine.mjs), with TPDF_ENGINE=rust so the
// Rust-only files under tests/rust-*.test.mjs run too. Tests that drove MuPDF
// internals of the removed TypeScript engine are skipped in their own files.
// RUST_SKIPS names tests to leave out by name (none now); engine/tools/op-evidence.mjs
// reads it. Usage: node tests/run-rust.mjs [test files or globs]
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

export const RUST_SKIPS = [];

const pattern = text => `^${text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`;

/** node's arguments for the Rust unit suite over `files` (engine/tools/op-evidence.mjs adds its own flags). */
export function rustTestArgs(files, flags = []) {
  return [
    '--import', './tests/engine-select.mjs', '--test', ...flags,
    ...RUST_SKIPS.map(skip => `--test-skip-pattern=${pattern(skip.test)}`),
    ...(files.length ? files : ['tests/*.test.mjs']),
  ];
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  // --test-skip-pattern drops these from the counts, so the list is printed instead.
  for (const skip of RUST_SKIPS) console.log(`# rust engine skips "${skip.test}": ${skip.reason}`);
  const files = process.argv.slice(2);
  const result = spawnSync(process.execPath, rustTestArgs(files), {
    stdio: 'inherit', env: { ...process.env, TPDF_ENGINE: 'rust', CI_NO_ELECTRON: '1' },
  });
  process.exitCode = result.status ?? 1;
}
