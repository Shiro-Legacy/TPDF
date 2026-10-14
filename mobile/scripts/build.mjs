import { build } from 'esbuild';
import { cp, mkdir, readdir, rm } from 'node:fs/promises';
import { writeSample } from './sample.mjs';

await rm('dist', { recursive: true, force: true });
await mkdir('dist/engine/vendor', { recursive: true });
await build({ entryPoints: ['src/app.ts'], bundle: true, format: 'esm', target: 'es2022', outfile: 'dist/app.js', sourcemap: true });
await build({ entryPoints: ['src/scan-worker.ts'], bundle: true, format: 'esm', target: 'es2022', outfile: 'dist/scan-worker.js', sourcemap: true,
  plugins: [{ name: 'shared-mupdf', setup(builder) { builder.onResolve({ filter: /vendor\/mupdf\.js$/ }, () => ({ path: './engine/vendor/mupdf.js', external: true })); } }] });
for (const name of ['index.html', 'styles.css']) await cp(`src/${name}`, `dist/${name}`);
for (const name of await readdir('../build/renderer')) {
  if (name.endsWith('.js') && name !== 'renderer.js') await cp(`../build/renderer/${name}`, `dist/engine/${name}`);
}
await cp('../build/renderer/vendor', 'dist/engine/vendor', { recursive: true });
await cp('../assets/icon.png', 'dist/icon.png');
await mkdir('dist/licenses', { recursive: true });
await cp('../LICENSE', 'dist/licenses/Tandem-LICENSE.txt');
await cp('../docs/third-party', 'dist/licenses/third-party', { recursive: true });
await cp('../node_modules/mupdf/LICENSE', 'dist/licenses/MuPDF-LICENSE.txt');
for (const name of ['core', 'app', 'filesystem', 'share']) {
  await cp(`node_modules/@capacitor/${name}/LICENSE`, `dist/licenses/Capacitor-${name}-LICENSE.txt`);
}
await writeSample('dist/welcome.pdf');
const { default: mupdf } = await import('mupdf');
const { readFile, writeFile } = await import('node:fs/promises');
const sample = mupdf.Document.openDocument(await readFile('dist/welcome.pdf'), 'application/pdf');
const page = sample.loadPage(0), pixmap = page.toPixmap(mupdf.Matrix.scale(2, 2), mupdf.ColorSpace.DeviceRGB, false);
await writeFile('dist/sample-scan.jpg', pixmap.asJPEG(90));
pixmap.destroy(); page.destroy(); sample.destroy();
console.log('Mobile assets built in mobile/dist.');
