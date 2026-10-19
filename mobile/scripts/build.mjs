import { build } from 'esbuild';
import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { relative, resolve, sep } from 'node:path';
import { CJK_FONT, readCjkFont } from './cjk-font.mjs';
import { checkRustPackage, RUST_ASSETS, RUST_PKG } from './engine-package.mjs';

// Mobile runs the Rust engine: the editor's worker (src/engine/worker.ts, served as engine/worker.js) and, in the
// scan worker, the same WASM package through engine/worker-engine.js. No MuPDF file reaches dist/. A missing or
// stale engine/web/pkg, or a CJK font that is not the pinned one, fails here, before dist/ changes.
await checkRustPackage();
const cjkFont = readCjkFont();
await rm('dist', { recursive: true, force: true });
await mkdir('dist/engine/vendor', { recursive: true });
await build({ entryPoints: ['src/app.ts'], bundle: true, format: 'esm', target: 'es2022', outfile: 'dist/app.js', sourcemap: true });
// The scan worker loads the engine (worker-engine.js: the Rust loader, glue and adapter) from engine/, as the
// editor's worker does.
const engineSource = resolve('src/engine');
await build({ entryPoints: ['src/scan-worker.ts'], bundle: true, format: 'esm', target: 'es2022', outfile: 'dist/scan-worker.js', sourcemap: true,
  plugins: [{ name: 'shared-engine', setup(builder) {
    builder.onResolve({ filter: /(vendor\/tandem_rust_workflow|engine-rust|engine-core|worker-engine)\.js$/ }, args => {
      const path = relative(engineSource, resolve(args.resolveDir, args.path));
      if (path.startsWith('..')) throw new Error(`${args.path} is not in src/engine`);
      return { path: `./engine/${path.split(sep).join('/')}`, external: true };
    });
  } }] });
for (const name of ['index.html', 'styles.css']) await cp(`src/${name}`, `dist/${name}`);
// The engine worker and what it loads, one module per file as the desktop app served them: worker.js,
// worker-engine.js, engine-rust.js and engine-core.js (src/engine), the package's glue and WASM in vendor/, and
// the CJK font in fonts/.
await build({ entryPoints: ['worker', 'worker-engine', 'engine-rust', 'engine-core'].map(name => `src/engine/${name}.ts`),
  bundle: false, format: 'esm', target: 'es2022', outdir: 'dist/engine' });
for (const name of RUST_ASSETS) await cp(`${RUST_PKG}${name}`, `dist/engine/vendor/${name}`);
await mkdir('dist/engine/fonts', { recursive: true });
await writeFile(`dist/engine/${CJK_FONT.served}`, cjkFont);
// The bundled samples, made with the adapter just written to dist/engine.
const { writeSamples } = await import('./sample.mjs');
await cp('../assets/icon.png', 'dist/icon.png');
await mkdir('dist/licenses', { recursive: true });
await cp('../LICENSE', 'dist/licenses/Tandem-LICENSE.txt');
// The app and its Rust engine are MIT (Tandem-LICENSE.txt). The engine's third-party notices and its crates'
// licence files, as the desktop packages ship them; the footer links the licence and notices.
await cp('../engine/NOTICE', 'dist/licenses/Engine-NOTICE.txt');
await cp('../engine/reference/licenses', 'dist/licenses/engine-licenses', { recursive: true });
await mkdir('dist/licenses/third-party', { recursive: true });
await cp(`../${CJK_FONT.notice}`, 'dist/licenses/third-party/droid-sans-fallback-NOTICE.txt');
for (const name of ['core', 'app', 'filesystem', 'share']) {
  await cp(`node_modules/@capacitor/${name}/LICENSE`, `dist/licenses/Capacitor-${name}-LICENSE.txt`);
}
await writeSamples('dist/welcome.pdf', 'dist/sample-scan.png');
const shipped = [];
for await (const name of (await import('node:fs/promises')).glob('dist/**/*')) shipped.push(name);
const mupdf = shipped.filter(name => /mupdf/i.test(name));
if (mupdf.length) throw new Error(`a MuPDF file reached the mobile bundle: ${mupdf.join(', ')}`);
// Nor does any script or page load one (an import, dynamic import, URL or script tag naming MuPDF), and no file
// carries MuPDF's code under another name (its WASM's C symbols and name, its glue's libmupdf calls), as the
// desktop packages were checked.
for (const name of shipped.filter(name => /\.(js|html|wasm)$/.test(name))) {
  const text = await readFile(name, 'latin1');
  if (/(?:\bfrom\s*|\bimport\s*\(\s*|\bnew URL\(\s*|\bsrc=)["'`][^"'`]*mupdf/i.test(text)) throw new Error(`${name} loads a MuPDF file`);
  if ((name.endsWith('.wasm') ? /fz_new_context|pdf_new_document|mupdf-wasm/ : /libmupdf/).test(text)) throw new Error(`${name} carries MuPDF code`);
}
console.log('Mobile assets built in mobile/dist.');
