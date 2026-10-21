// The Rust engine's WASM package (engine/web/pkg, built by engine/tools/build-workflow.sh) that mobile ships
// (mobile/scripts/build.mjs) and the tests load (scripts/build-test.mjs). A package that is missing, has no RustApp
// or was built from other sources than engine/ holds now fails the build (TPDF_ALLOW_STALE_ENGINE=1 builds with it
// anyway, with a warning).
import { createHash } from 'node:crypto';
import { readdir, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const ENGINE = fileURLToPath(new URL('../engine/', import.meta.url));
export const RUST_PKG = fileURLToPath(new URL('../engine/web/pkg/', import.meta.url));
/** The glue and the WASM: what mobile's dist/engine/vendor and build-test/vendor hold. */
export const RUST_ASSETS = ['tandem_rust_workflow.js', 'tandem_rust_workflow_bg.wasm'];
const REBUILD = 'rebuild it with engine/tools/build-workflow.sh';

/**
 * `<sha256>  <path>` for each source the WASM package is built from, sorted, as
 * engine/tools/build-workflow.sh writes them to engine/web/pkg/sources.sha256.
 */
async function rustSources() {
  const files = ['Cargo.toml', 'bridge/Cargo.toml', '../crates/Cargo.lock'];
  async function walk(dir) {
    for (const entry of await readdir(`${ENGINE}${dir}`, { withFileTypes: true })) {
      if (entry.isDirectory()) await walk(`${dir}/${entry.name}`);
      else if (entry.isFile()) files.push(`${dir}/${entry.name}`);
    }
  }
  await walk('src');
  await walk('bridge/src');
  // The patched crates the package is built from (crates/Cargo.toml's [patch.crates-io]).
  if (await readdir(`${ENGINE}vendor`).then(() => true, () => false)) await walk('vendor');
  files.sort();
  const lines = [];
  for (const path of files) lines.push(`${createHash('sha256').update(await readFile(`${ENGINE}${path}`)).digest('hex')}  ${path}`);
  return lines;
}

export async function checkRustPackage() {
  try {
    const glue = await readFile(`${RUST_PKG}${RUST_ASSETS[0]}`, 'utf8');
    if (!glue.includes('export class RustApp')) throw new Error(`engine/web/pkg/${RUST_ASSETS[0]} has no RustApp`);
    for (const name of RUST_ASSETS.slice(1)) await readFile(`${RUST_PKG}${name}`);
  } catch (err) {
    throw new Error(`this build needs the Rust engine package engine/web/pkg; build it with engine/tools/build-workflow.sh (${err.message})`);
  }
  const built = await readFile(`${RUST_PKG}sources.sha256`, 'utf8').then(text => text.split('\n').filter(Boolean), () => null);
  const now = await rustSources();
  let problem;
  if (!built) problem = 'engine/web/pkg has no sources.sha256, so the sources it was built from are unknown';
  else {
    const before = new Set(built);
    const after = new Set(now);
    const changed = [...new Set([...built.filter(line => !after.has(line)), ...now.filter(line => !before.has(line))]
      .map(line => line.slice(line.indexOf('  ') + 2)))];
    if (changed.length) problem = `engine/web/pkg is stale: engine/${changed.slice(0, 5).join(', engine/')}${changed.length > 5 ? ` and ${changed.length - 5} more` : ''} changed since it was built`;
  }
  if (!problem) return;
  // A deliberate build against a package of older sources, for example while engine/ is mid-edit.
  if (process.env.TPDF_ALLOW_STALE_ENGINE === '1') console.warn(`WARNING: ${problem}; building with it anyway (TPDF_ALLOW_STALE_ENGINE=1).`);
  else throw new Error(`${problem}; ${REBUILD}, or set TPDF_ALLOW_STALE_ENGINE=1 to build with it anyway`);
}
