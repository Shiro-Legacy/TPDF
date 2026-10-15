import { copyFile, mkdir, readFile, rm, writeFile } from 'node:fs/promises';

// The worker's PDF engine is chosen here, at build time: --engine=rust|mupdf, else TPDF_ENGINE, else rust.
// rust ships the Rust engine's WASM package, mupdf ships MuPDF. A build carries one engine and never falls
// back to the other; build/engine-build.js names it, and src/policy.ts serves only that engine's assets.
const flag = process.argv.find(arg => arg.startsWith('--engine='))?.slice('--engine='.length);
const engine = flag || process.env.TPDF_ENGINE || 'rust';
if (engine !== 'rust' && engine !== 'mupdf') throw new Error(`the engine must be rust or mupdf, not ${JSON.stringify(engine)}`);
const RUST_PKG = 'engine/web/pkg';
const RUST_ASSETS = ['tandem_rust_workflow.js', 'tandem_rust_workflow_bg.wasm'];

// A previous build of the other engine must not leave its assets behind.
await rm('build/renderer/vendor', { recursive: true, force: true });
await mkdir('build/renderer/vendor', { recursive: true });
await mkdir('build/renderer/fixtures', { recursive: true });
for (const name of ['index.html', 'styles.css']) {
  await copyFile(`src/renderer/${name}`, `build/renderer/${name}`);
}
if (engine === 'rust') {
  let glue;
  try {
    glue = await readFile(`${RUST_PKG}/${RUST_ASSETS[0]}`, 'utf8');
    for (const name of RUST_ASSETS) await copyFile(`${RUST_PKG}/${name}`, `build/renderer/vendor/${name}`);
  } catch (err) {
    throw new Error(`a Rust engine build needs ${RUST_PKG}; build it with engine/tools/build-workflow.sh (${err.message})`);
  }
  if (!glue.includes('export class RustApp')) throw new Error(`${RUST_PKG}/${RUST_ASSETS[0]} has no RustApp; rebuild it with engine/tools/build-workflow.sh`);
  // tsc also emits the MuPDF loader; a Rust build keeps no file that imports MuPDF.
  await rm('build/renderer/worker-engine-mupdf.js', { force: true });
} else {
  for (const name of ['mupdf.js', 'mupdf-wasm.js', 'mupdf-wasm.wasm']) {
    await copyFile(`node_modules/mupdf/dist/${name}`, `build/renderer/vendor/${name}`);
  }
  // The worker imports ./worker-engine.js, where tsc emitted the Rust loader.
  await copyFile('build/renderer/worker-engine-mupdf.js', 'build/renderer/worker-engine.js');
  await rm('build/renderer/worker-engine-mupdf.js');
}
await writeFile('build/engine-build.js', `/** Written by scripts/build.mjs: the worker engine this build ships. */\nexport const ENGINE = ${JSON.stringify(engine)};\n`);
await copyFile('assets/icon.png', 'build/icon.png'); // Linux window/About icon (src/main.ts LINUX_ICON)
await copyFile('fixtures/hello.pdf', 'build/renderer/fixtures/hello.pdf');
await copyFile('fixtures/form.pdf', 'build/renderer/fixtures/form.pdf');
await copyFile('fixtures/large.pdf', 'build/renderer/fixtures/large.pdf');
