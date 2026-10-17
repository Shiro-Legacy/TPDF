import { build } from 'esbuild';
import { cp, mkdir, readdir, readFile, rm } from 'node:fs/promises';
import { relative, resolve, sep } from 'node:path';
import { writeSamples } from './sample.mjs';

// Mobile runs the Rust engine: the desktop build's worker (engine/worker.js) and, in the scan worker, the same
// WASM package through engine-rust.js. A MuPDF desktop build is refused, and no MuPDF file reaches dist/.
const { ENGINE } = await import(new URL('../../build/engine-build.js', import.meta.url).href);
if (ENGINE !== 'rust') throw new Error(`mobile needs the Rust desktop build (npm --prefix .. run build), not ${ENGINE}`);
const { localAsset } = await import(new URL('../../build/policy.js', import.meta.url).href);
const renderer = resolve('../build/renderer');
await rm('dist', { recursive: true, force: true });
await mkdir('dist/engine', { recursive: true });
await build({ entryPoints: ['src/app.ts'], bundle: true, format: 'esm', target: 'es2022', outfile: 'dist/app.js', sourcemap: true });
// The scan worker loads the engine glue, adapter and shared helpers from engine/, as the engine worker does.
await build({ entryPoints: ['src/scan-worker.ts'], bundle: true, format: 'esm', target: 'es2022', outfile: 'dist/scan-worker.js', sourcemap: true,
  plugins: [{ name: 'shared-engine', setup(builder) {
    builder.onResolve({ filter: /(vendor\/tandem_rust_workflow|engine-rust|engine-core)\.js$/ }, args => {
      const path = relative(resolve('../src/renderer'), resolve(args.resolveDir, args.path));
      return { path: `./engine/${path.split(sep).join('/')}`, external: true };
    });
  } }] });
for (const name of ['index.html', 'styles.css']) await cp(`src/${name}`, `dist/${name}`);
// Of the desktop renderer, the worker and what it loads, exactly as the desktop app serves them (src/policy.ts):
// a Rust build serves no MuPDF module. Not the desktop page, its script, styles or fixtures, nor recovery.js (the
// desktop renderer's edit log for a worker restart, src/renderer/recovery.ts, which only renderer.js imports).
async function copyEngine(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = resolve(dir, entry.name), name = relative(renderer, path).split(sep).join('/');
    if (entry.isDirectory()) { await copyEngine(path); continue; }
    if (['index.html', 'renderer.js', 'recovery.js', 'styles.css'].includes(name) || name.startsWith('fixtures/')) continue;
    if (!localAsset(`app://pdf4all/${name}`)) continue;
    await mkdir(resolve('dist/engine', relative(renderer, dir)), { recursive: true });
    await cp(path, resolve('dist/engine', name));
  }
}
await copyEngine(renderer);
await cp('../assets/icon.png', 'dist/icon.png');
await mkdir('dist/licenses', { recursive: true });
await cp('../LICENSE', 'dist/licenses/Tandem-LICENSE.txt');
// The Rust engine's notices, as scripts/package.mjs ships them with a Rust desktop build.
await cp('../engine/NOTICE', 'dist/licenses/Engine-NOTICE.txt');
await cp('../engine/COPYING', 'dist/licenses/Engine-AGPL-3.0.txt');
await cp('../engine/reference/licenses', 'dist/licenses/engine-licenses', { recursive: true });
await mkdir('dist/licenses/third-party', { recursive: true });
await cp('../docs/third-party/droid-sans-fallback-NOTICE.txt', 'dist/licenses/third-party/droid-sans-fallback-NOTICE.txt');
for (const name of ['core', 'app', 'filesystem', 'share']) {
  await cp(`node_modules/@capacitor/${name}/LICENSE`, `dist/licenses/Capacitor-${name}-LICENSE.txt`);
}
await writeSamples('dist/welcome.pdf', 'dist/sample-scan.png');
const shipped = [];
for await (const name of (await import('node:fs/promises')).glob('dist/**/*')) shipped.push(name);
const mupdf = shipped.filter(name => /mupdf/i.test(name));
if (mupdf.length) throw new Error(`a MuPDF file reached the mobile bundle: ${mupdf.join(', ')}`);
// Nor does any script or page load one (an import, dynamic import, URL or script tag naming MuPDF), and no file
// carries MuPDF's code under another name (its WASM's C symbols and name, its glue's libmupdf calls), as
// scripts/check-package.mjs checks desktop packages.
for (const name of shipped.filter(name => /\.(js|html|wasm)$/.test(name))) {
  const text = await readFile(name, 'latin1');
  if (/(?:\bfrom\s*|\bimport\s*\(\s*|\bnew URL\(\s*|\bsrc=)["'`][^"'`]*mupdf/i.test(text)) throw new Error(`${name} loads a MuPDF file`);
  if ((name.endsWith('.wasm') ? /fz_new_context|pdf_new_document|mupdf-wasm/ : /libmupdf/).test(text)) throw new Error(`${name} carries MuPDF code`);
}
console.log('Mobile assets built in mobile/dist.');
