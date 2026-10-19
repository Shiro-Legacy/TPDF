// M10 macOS package (engine/design/m10-macos-package.md): what the package checklist relies on, checked without a
// package, plus the checklist itself on a packaged app when TPDF_PACKAGE_CHECK names a work folder.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
// The package checker's own test: absolute build-machine paths in a WASM module (home, checkout, /Users/, /home/).
import { wasmPathLeaks } from '../scripts/check-package.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
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
  assert.deepEqual(missing, [], 'scripts/package.mjs ships engine/reference/licenses as licenses/engine-licenses');
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
  assert.deepEqual(missing, [], 'crates/tandem-sign/licenses.sh copies them; scripts/package.mjs ships them as licenses/sign-licenses');
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

test('drive.mjs --app refuses a packaged app that is not there', { timeout: 30_000 }, () => {
  const run = spawnSync(process.execPath, [join(root, 'scripts/drive.mjs'), 'tests/drive/save-in-place.jsonl', '--app=/nonexistent/Tandem PDF'], { cwd: root, encoding: 'utf8' });
  assert.equal(run.status, 2);
  assert.match(run.stderr, /--app: no such file/);
  const env = spawnSync(process.execPath, [join(root, 'scripts/drive.mjs'), 'tests/drive/save-in-place.jsonl'], { cwd: root, encoding: 'utf8', env: { ...process.env, TPDF_DRIVE_APP: '/nonexistent/Tandem PDF' } });
  assert.equal(env.status, 2);
  assert.match(env.stderr, /--app: no such file/);
});

test('the smoke reports when the first page was drawn, for the packaged startup timing', { skip: CI_NO_ELECTRON, timeout: 120_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-m10-startup-'));
  try {
    const report = join(dir, 'report.json');
    const spawned = Date.now();
    const child = spawn(createRequire(import.meta.url)('electron'), [`--user-data-dir=${join(dir, 'udata')}`, '.', '--smoke', `--report=${report}`], { cwd: root, stdio: 'ignore' });
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

// Opt-in: TPDF_PACKAGE_CHECK=<work folder> after `npm run package:rust` on darwin-arm64 (several minutes, ~25 launches).
const work = process.env.TPDF_PACKAGE_CHECK;
const packaged = join(root, 'out/Tandem PDF-darwin-arm64/Tandem PDF.app');
test('the macOS package checklist passes on the packaged app', {
  skip: !work ? 'set TPDF_PACKAGE_CHECK to a work folder' : !existsSync(packaged) ? 'no package: npm run package:rust' : CI_NO_ELECTRON, timeout: 1_200_000,
}, () => {
  const run = spawnSync(process.execPath, [join(root, 'scripts/check-macos-package.mjs'), `--work=${work}`], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
  const report = JSON.parse(run.stdout);
  const failed = Object.entries(report.items).filter(([, item]) => item.result === 'fail').map(([name]) => name);
  assert.deepEqual(failed, []);
  assert.equal(run.status, 0);
});
