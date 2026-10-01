import { copyFile, mkdir } from 'node:fs/promises';

await mkdir('build/renderer/vendor', { recursive: true });
await mkdir('build/renderer/fixtures', { recursive: true });
for (const name of ['index.html', 'styles.css']) {
  await copyFile(`src/renderer/${name}`, `build/renderer/${name}`);
}
for (const name of ['mupdf.js', 'mupdf-wasm.js', 'mupdf-wasm.wasm']) {
  await copyFile(`node_modules/mupdf/dist/${name}`, `build/renderer/vendor/${name}`);
}
await copyFile('fixtures/hello.pdf', 'build/renderer/fixtures/hello.pdf');
await copyFile('fixtures/form.pdf', 'build/renderer/fixtures/form.pdf');
await copyFile('fixtures/large.pdf', 'build/renderer/fixtures/large.pdf');
