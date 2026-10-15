import { createHash } from 'node:crypto';
import { copyFile, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';

// The worker's PDF engine is chosen here, at build time: --engine=rust|mupdf, else TPDF_ENGINE, else rust.
// rust ships the Rust engine's WASM package, mupdf ships MuPDF. A build carries one engine and never falls
// back to the other; build/engine-build.js names it, and src/policy.ts serves only that engine's assets.
const flag = process.argv.find(arg => arg.startsWith('--engine='))?.slice('--engine='.length);
const engine = flag || process.env.TPDF_ENGINE || 'rust';
if (engine !== 'rust' && engine !== 'mupdf') throw new Error(`the engine must be rust or mupdf, not ${JSON.stringify(engine)}`);
const RUST_PKG = 'engine/web/pkg';
const RUST_ASSETS = ['tandem_rust_workflow.js', 'tandem_rust_workflow_bg.wasm'];
const REBUILD = 'rebuild it with engine/tools/build-workflow.sh';

/**
 * `<sha256>  <path>` for each source the WASM package is built from, sorted, as
 * engine/tools/build-workflow.sh writes them to engine/web/pkg/sources.sha256.
 */
async function rustSources() {
  const files = ['Cargo.toml', 'bridge/Cargo.toml', 'bridge/Cargo.lock'];
  async function walk(dir) {
    for (const entry of await readdir(`engine/${dir}`, { withFileTypes: true })) {
      if (entry.isDirectory()) await walk(`${dir}/${entry.name}`);
      else if (entry.isFile()) files.push(`${dir}/${entry.name}`);
    }
  }
  await walk('src');
  await walk('bridge/src');
  files.sort();
  const lines = [];
  for (const path of files) lines.push(`${createHash('sha256').update(await readFile(`engine/${path}`)).digest('hex')}  ${path}`);
  return lines;
}

/** A package that is missing, has no RustApp or was built from other sources than engine/ holds now fails the build. */
async function checkRustPackage() {
  try {
    const glue = await readFile(`${RUST_PKG}/${RUST_ASSETS[0]}`, 'utf8');
    if (!glue.includes('export class RustApp')) throw new Error(`${RUST_PKG}/${RUST_ASSETS[0]} has no RustApp`);
    for (const name of RUST_ASSETS.slice(1)) await readFile(`${RUST_PKG}/${name}`);
  } catch (err) {
    throw new Error(`a Rust engine build needs ${RUST_PKG}; build it with engine/tools/build-workflow.sh (${err.message})`);
  }
  const built = await readFile(`${RUST_PKG}/sources.sha256`, 'utf8').then(text => text.split('\n').filter(Boolean), () => null);
  const now = await rustSources();
  let problem;
  if (!built) problem = `${RUST_PKG} has no sources.sha256, so the sources it was built from are unknown`;
  else {
    const before = new Set(built);
    const after = new Set(now);
    const changed = [...new Set([...built.filter(line => !after.has(line)), ...now.filter(line => !before.has(line))]
      .map(line => line.slice(line.indexOf('  ') + 2)))];
    if (changed.length) problem = `${RUST_PKG} is stale: engine/${changed.slice(0, 5).join(', engine/')}${changed.length > 5 ? ` and ${changed.length - 5} more` : ''} changed since it was built`;
  }
  if (!problem) return;
  // A deliberate build against a package of older sources, for example while engine/ is mid-edit.
  if (process.env.TPDF_ALLOW_STALE_ENGINE === '1') console.warn(`WARNING: ${problem}; building with it anyway (TPDF_ALLOW_STALE_ENGINE=1).`);
  else throw new Error(`${problem}; ${REBUILD}, or set TPDF_ALLOW_STALE_ENGINE=1 to build with it anyway`);
}

if (engine === 'rust') await checkRustPackage();
// A previous build of the other engine must not leave its assets behind.
await rm('build/renderer/vendor', { recursive: true, force: true });
await mkdir('build/renderer/vendor', { recursive: true });
await mkdir('build/renderer/fixtures', { recursive: true });
for (const name of ['index.html', 'styles.css']) {
  await copyFile(`src/renderer/${name}`, `build/renderer/${name}`);
}
if (engine === 'rust') {
  for (const name of RUST_ASSETS) await copyFile(`${RUST_PKG}/${name}`, `build/renderer/vendor/${name}`);
} else {
  for (const name of ['mupdf.js', 'mupdf-wasm.js', 'mupdf-wasm.wasm']) {
    await copyFile(`node_modules/mupdf/dist/${name}`, `build/renderer/vendor/${name}`);
  }
}
// The worker imports ./worker-engine.js, which tsc emits empty: the selected engine's loader goes over it.
// Both emitted loaders are then removed, so a rerun without tsc fails here instead of keeping a stale loader.
try {
  await copyFile(`build/renderer/worker-engine-${engine}.js`, 'build/renderer/worker-engine.js');
} catch (err) {
  throw new Error(`build/renderer/worker-engine-${engine}.js is missing; run tsc before scripts/build.mjs (${err.message})`);
}
for (const name of ['rust', 'mupdf']) await rm(`build/renderer/worker-engine-${name}.js`, { force: true });
await writeFile('build/engine-build.js', `/** Written by scripts/build.mjs: the worker engine this build ships. */\nexport const ENGINE = ${JSON.stringify(engine)};\n`);
await copyFile('assets/icon.png', 'build/icon.png'); // Linux window/About icon (src/main.ts LINUX_ICON)
await copyFile('fixtures/hello.pdf', 'build/renderer/fixtures/hello.pdf');
await copyFile('fixtures/form.pdf', 'build/renderer/fixtures/form.pdf');
await copyFile('fixtures/large.pdf', 'build/renderer/fixtures/large.pdf');
