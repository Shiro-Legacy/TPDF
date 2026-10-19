import { createHash } from 'node:crypto';
import { copyFile, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { CJK_FONT, readCjkFont } from './cjk-font.mjs';

// The worker's PDF engine is the Rust engine's WASM package (engine/web/pkg); src/engine-build.ts names it,
// and src/policy.ts serves only its assets. MuPDF is a test-only oracle (engine/oracle) and ships nowhere.
const engine = 'rust';
if (process.argv.some(arg => arg.startsWith('--engine=')) || (process.env.TPDF_ENGINE && process.env.TPDF_ENGINE !== 'rust')) {
  throw new Error('the app builds only the Rust engine; the MuPDF build was removed (engine/oracle holds the MuPDF test oracle)');
}
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
  // The patched crates the package is built from (engine/Cargo.toml's [patch.crates-io]).
  if (await readdir('engine/vendor').then(() => true, () => false)) await walk('vendor');
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

// The signing module (crates/tandem-sign) Electron's main process loads in place of node:crypto
// (src/sign/native.ts). engine/tools/build-workflow.sh builds it into crates/tandem-sign/pkg with the
// sources it was built from; a missing or stale module fails the build like a stale engine package.
const SIGN_PKG = 'crates/tandem-sign/pkg';
async function signSources() {
  const files = ['Cargo.toml', 'Cargo.lock'];
  async function walk(dir) {
    for (const entry of await readdir(`crates/tandem-sign/${dir}`, { withFileTypes: true })) {
      if (entry.isDirectory()) await walk(`${dir}/${entry.name}`);
      else if (entry.isFile()) files.push(`${dir}/${entry.name}`);
    }
  }
  await walk('src');
  files.sort();
  const lines = [];
  for (const path of files) lines.push(`${createHash('sha256').update(await readFile(`crates/tandem-sign/${path}`)).digest('hex')}  ${path}`);
  return lines;
}
async function checkSignPackage() {
  try {
    const glue = await readFile(`${SIGN_PKG}/tandem_sign.js`, 'utf8');
    if (!glue.includes('verifySignatures')) throw new Error(`${SIGN_PKG}/tandem_sign.js has no verifySignatures`);
    await readFile(`${SIGN_PKG}/tandem_sign_bg.wasm`);
  } catch (err) {
    throw new Error(`the app needs the signing module in ${SIGN_PKG}; build it with engine/tools/build-workflow.sh (${err.message})`);
  }
  const built = await readFile(`${SIGN_PKG}/sources.sha256`, 'utf8').then(text => text.split('\n').filter(Boolean), () => null);
  const now = await signSources();
  const changed = built ? [...new Set([...built.filter(line => !now.includes(line)), ...now.filter(line => !built.includes(line))]
    .map(line => line.slice(line.indexOf('  ') + 2)))] : null;
  const problem = !built ? `${SIGN_PKG} has no sources.sha256, so the sources it was built from are unknown`
    : changed.length ? `${SIGN_PKG} is stale: crates/tandem-sign/${changed.slice(0, 5).join(', crates/tandem-sign/')} changed since it was built` : '';
  if (!problem) return;
  if (process.env.TPDF_ALLOW_STALE_ENGINE === '1') console.warn(`WARNING: ${problem}; building with it anyway (TPDF_ALLOW_STALE_ENGINE=1).`);
  else throw new Error(`${problem}; ${REBUILD}, or set TPDF_ALLOW_STALE_ENGINE=1 to build with it anyway`);
}

// The Rust engine draws CJK text in fonts a PDF does not embed with the committed, sha256-pinned font: a
// build without it fails here, before build/ changes.
const cjkFont = readCjkFont(process.cwd());
await checkRustPackage();
await checkSignPackage();
// A previous build must not leave other assets behind.
await rm('build/renderer/vendor', { recursive: true, force: true });
await rm('build/renderer/fonts', { recursive: true, force: true });
await mkdir('build/renderer/vendor', { recursive: true });
await mkdir('build/renderer/fixtures', { recursive: true });
for (const name of ['index.html', 'styles.css']) {
  await copyFile(`src/renderer/${name}`, `build/renderer/${name}`);
}
for (const name of RUST_ASSETS) await copyFile(`${RUST_PKG}/${name}`, `build/renderer/vendor/${name}`);
// The font the worker serves itself (src/policy.ts) and its Apache-2.0 licence beside it.
await mkdir('build/renderer/fonts', { recursive: true });
await writeFile(`build/renderer/${CJK_FONT.served}`, cjkFont);
await copyFile(CJK_FONT.notice, 'build/renderer/fonts/DroidSansFallback-NOTICE.txt');
// The worker imports ./worker-engine.js, which tsc emits empty: the engine's loader goes over it. The
// emitted loader is then removed, so a rerun without tsc fails here instead of keeping a stale loader.
try {
  await copyFile(`build/renderer/worker-engine-${engine}.js`, 'build/renderer/worker-engine.js');
} catch (err) {
  throw new Error(`build/renderer/worker-engine-${engine}.js is missing; run tsc before scripts/build.mjs (${err.message})`);
}
await rm(`build/renderer/worker-engine-${engine}.js`, { force: true });
// The signing module: wasm-bindgen's Node.js glue is CommonJS, and build/ is an ES module package, hence .cjs.
await rm('build/sign/vendor', { recursive: true, force: true });
await mkdir('build/sign/vendor', { recursive: true });
await copyFile(`${SIGN_PKG}/tandem_sign.js`, 'build/sign/vendor/tandem_sign.cjs');
await copyFile(`${SIGN_PKG}/tandem_sign_bg.wasm`, 'build/sign/vendor/tandem_sign_bg.wasm');
await copyFile('assets/icon.png', 'build/icon.png'); // Linux window/About icon (src/main.ts LINUX_ICON)
await copyFile('fixtures/hello.pdf', 'build/renderer/fixtures/hello.pdf');
await copyFile('fixtures/form.pdf', 'build/renderer/fixtures/form.pdf');
await copyFile('fixtures/large.pdf', 'build/renderer/fixtures/large.pdf');
