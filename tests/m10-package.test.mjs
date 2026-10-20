// M10 package (engine/design/m10-macos-package.md): what a package relies on, checked without a package: the WASM
// modules' licence folders and build paths, drive.mjs's --app check, and the smoke's startup timing.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { appBinary } from './app-bin.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Absolute build-machine paths in a WASM module: a home folder, the checkout, any /Users/ or /home/ path. */
function wasmPathLeaks(bytes, { home = homedir(), checkout = root } = {}) {
  const text = Buffer.from(bytes).toString('latin1');
  return [home, checkout.replace(/[\\/]$/, ''), '/Users/', '/home/'].filter(needle => needle && text.includes(needle));
}
const NO_APP = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;
const cargo = spawnSync('cargo', ['--version'], { encoding: 'utf8' }).status === 0;

test('every crate compiled into the WASM, and every proc macro that builds it, has a licence folder the package ships', { skip: cargo ? false : 'no cargo', timeout: 60_000 }, () => {
  const tree = spawnSync('cargo', ['tree', '--offline', '--locked', '--target', 'wasm32-unknown-unknown', '-e', 'normal', '--prefix', 'none',
    '--manifest-path', join(root, 'engine/bridge/Cargo.toml')], { encoding: 'utf8' });
  assert.equal(tree.status, 0, tree.stderr);
  const crates = [...new Set(tree.stdout.split('\n').map(line => line.trim()).filter(Boolean)
    .map(line => line.replace(/ \(.*$/, '').replace(/ v(\S+).*/, '-$1')))]
    .filter(crate => !/^tandem-(pdf-engine|rust-workflow)-/.test(crate));
  assert.ok(crates.includes('wasm-bindgen-0.2.129'), crates.join(' '));
  const shipped = join(root, 'engine/reference/licenses');
  const missing = crates.filter(crate => !existsSync(join(shipped, crate)) || readdirSync(join(shipped, crate)).length === 0);
  assert.deepEqual(missing, [], 'engine/reference/licenses has a folder for every crate in the engine WASM (mobile ships it as licenses/engine-licenses)');
  const notice = readFileSync(join(root, 'engine/NOTICE'), 'utf8');
  for (const name of ['wasm-bindgen 0.2.129', 'wasm-bindgen-shared 0.2.129', 'bumpalo 3.20.3']) assert.ok(notice.includes(name), name);
});

test('every crate compiled into the signing WASM has a licence folder the package ships, named in its NOTICE', { skip: cargo ? false : 'no cargo', timeout: 60_000 }, () => {
  const tree = spawnSync('cargo', ['tree', '--offline', '--locked', '--target', 'wasm32-unknown-unknown', '-e', 'normal,no-proc-macro', '--features', 'wasm',
    '--prefix', 'none', '--manifest-path', join(root, 'crates/tandem-sign/Cargo.toml')], { encoding: 'utf8' });
  assert.equal(tree.status, 0, tree.stderr);
  const crates = [...new Set(tree.stdout.split('\n').map(line => line.trim()).filter(Boolean)
    .map(line => line.replace(/ \(.*$/, '').replace(/ v(\S+).*/, '-$1')))].filter(crate => !crate.startsWith('tandem-sign-'));
  assert.ok(crates.includes('rsa-0.9.10'), crates.join(' '));
  const shipped = join(root, 'crates/tandem-sign/licenses');
  const missing = crates.filter(crate => !existsSync(join(shipped, crate)) || readdirSync(join(shipped, crate)).length === 0);
  assert.deepEqual(missing, [], 'crates/tandem-sign/licenses.sh copies them');
  const extra = readdirSync(shipped).filter(name => name !== 'license-files.json' && !crates.includes(name));
  assert.deepEqual(extra, [], 'no licence folder for a crate the module no longer uses');
  const notice = readFileSync(join(root, 'crates/tandem-sign/NOTICE'), 'utf8').replace(/\s+/g, ' ');
  const unnamed = crates.filter(crate => !notice.includes(crate.replace(/-(\d[^-]*)$/, ' $1')));
  assert.deepEqual(unnamed, [], 'crates/tandem-sign/NOTICE names every crate with its version');
});

const signWasm = join(root, 'crates/tandem-sign/pkg/tandem_sign_bg.wasm');
test('the signing WASM embeds no absolute home or checkout path', { skip: existsSync(signWasm) ? false : 'no crates/tandem-sign/pkg' }, () => {
  assert.deepEqual(wasmPathLeaks(readFileSync(signWasm)), []);
});

const wasm = join(root, 'engine/web/pkg/tandem_rust_workflow_bg.wasm');
test('the WASM package embeds no absolute home or checkout path (build-workflow.sh remaps them)', { skip: existsSync(wasm) ? false : 'no engine/web/pkg' }, () => {
  const bytes = readFileSync(wasm);
  assert.deepEqual(wasmPathLeaks(bytes), []);
  const text = bytes.toString('latin1');
  assert.ok(text.includes('/tpdf/engine/src/'), 'engine sources under the fixed /tpdf prefix');
  assert.ok(text.includes('/cargo/registry/src/'), 'registry sources under the fixed /cargo prefix');
  // The check itself finds a leak.
  assert.ok(wasmPathLeaks(Buffer.from(`x${root}engine/src/lib.rs`), { home: '/nonexistent-home' }).includes(root.replace(/\/$/, '')));
});

test('drive.mjs --app and TANDEM_APP_BIN refuse an app that is not there', { timeout: 30_000 }, () => {
  const run = spawnSync(process.execPath, [join(root, 'scripts/drive.mjs'), 'tests/drive/save-in-place.jsonl', '--app=/nonexistent/Tandem PDF'], { cwd: root, encoding: 'utf8' });
  assert.equal(run.status, 2);
  assert.match(run.stderr, /--app: no such file/);
  const env = spawnSync(process.execPath, [join(root, 'scripts/drive.mjs'), 'tests/drive/save-in-place.jsonl'], { cwd: root, encoding: 'utf8', env: { ...process.env, TANDEM_APP_BIN: '/nonexistent/Tandem PDF' } });
  assert.equal(env.status, 2);
  // drive.mjs resolves TANDEM_APP_BIN from its cwd (root): on Windows /nonexistent/... becomes D:\nonexistent\...
  const missing = resolve(root, '/nonexistent/Tandem PDF').replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  assert.match(env.stderr, new RegExp(`no app at ${missing}`));
});

test('the smoke reports when the first page was drawn, for the packaged startup timing', { skip: NO_APP, timeout: 120_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-m10-startup-'));
  try {
    const report = join(dir, 'report.json');
    const spawned = Date.now();
    const child = spawn(appBinary, [`--user-data-dir=${join(dir, 'udata')}`, '--smoke', `--report=${report}`], { cwd: root, stdio: 'ignore' });
    const code = await new Promise(done => child.on('close', done));
    const exited = Date.now();
    const result = JSON.parse(await readFile(report, 'utf8'));
    assert.equal(code, 0, result.error);
    const { firstPageAtMs, firstPageEpochMs, mainStartedAtMs } = result.startup;
    assert.ok(firstPageEpochMs > spawned && firstPageEpochMs < exited, JSON.stringify(result.startup));
    assert.ok(mainStartedAtMs >= 0 && firstPageAtMs > mainStartedAtMs, JSON.stringify(result.startup));
  } finally {
    await rm(dir, { recursive: true, force: true, maxRetries: 5 });
  }
});
