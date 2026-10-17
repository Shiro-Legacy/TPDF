// The renderer build ships the Rust worker engine (scripts/build.mjs): every module the renderer and
// worker load is served by the asset allowlist, the build carries no MuPDF (MuPDF is a test-only oracle,
// engine/oracle), a stale package or loader fails the build, and a worker that dies or stops restarts.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { cp, mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import test from 'node:test';
import { localAsset } from '../build/policy.js';
import { ENGINE } from '../build/engine-build.js';
import { CJK_FONT } from '../scripts/cjk-font.mjs';

const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;

const RENDERER = resolve('build/renderer');
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const ENGINE_FILES = {
  rust: ['tandem_rust_workflow.js', 'tandem_rust_workflow_bg.wasm'],
};
/** The former TypeScript engine over MuPDF, now a test-only oracle: never loaded or served. */
const MUPDF_ENGINE = ['engine.js', 'content-edit.js', 'redact.js', 'sign-prepare.js'];

/** Static and dynamic import specifiers, and `new URL("...", import.meta.url)` loads, of a module. */
function references(text) {
  const found = [];
  const patterns = [
    /\b(?:import|export)\s[^'"`;]*?\bfrom\s*["']([^"']+)["']/g,
    /\bimport\s*["']([^"']+)["']/g,
    /\bimport\s*\(\s*["']([^"']+)["']\s*\)/g,
    /\bnew URL\(\s*["']([^"']+)["']\s*,\s*import\.meta\.url\s*\)/g,
  ];
  for (const pattern of patterns) for (const match of text.matchAll(pattern)) found.push(match[1]);
  return found;
}

/** Every file a module loads, transitively, as paths relative to build/renderer. */
function graph(entry) {
  const seen = new Set();
  const queue = [entry];
  while (queue.length) {
    const name = queue.shift();
    if (seen.has(name)) continue;
    seen.add(name);
    if (!name.endsWith('.js')) continue;
    const file = join(RENDERER, name);
    assert.ok(existsSync(file), `${name} is loaded but not built`);
    for (const specifier of references(readFileSync(file, 'utf8'))) {
      const relativeAsset = specifier.startsWith('./') || specifier.startsWith('../') || /^[\w.-]+\.(js|wasm)$/.test(specifier);
      // Engine glue keeps Node-only imports (node:fs, module) behind its environment checks.
      if (!relativeAsset && name.startsWith('vendor/')) continue;
      assert.ok(relativeAsset, `${name} loads ${specifier}, which is not a relative asset`);
      queue.push(relative(RENDERER, resolve(dirname(file), specifier)));
    }
  }
  return seen;
}

function walk(dir) {
  return readdirSync(dir).flatMap(name => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}

test('the renderer and its worker load only served assets of the engine this build ships', () => {
  assert.equal(ENGINE, 'rust');
  assert.deepEqual(readdirSync(join(RENDERER, 'vendor')).sort(), ENGINE_FILES[ENGINE]);
  for (const [engine, files] of Object.entries(ENGINE_FILES)) {
    for (const name of files) {
      const served = localAsset(`app://pdf4all/vendor/${name}`);
      assert.equal(!!served, engine === ENGINE, `vendor/${name} served`);
    }
  }
  const loaded = graph('renderer.js');
  assert.ok(loaded.has('worker.js') && loaded.has('worker-engine.js'), [...loaded].join(' '));
  for (const name of loaded) {
    assert.ok(localAsset(`app://pdf4all/${name}`), `${name} is loaded but not on the asset allowlist`);
    assert.ok(existsSync(join(RENDERER, name)), `${name} is loaded but not built`);
  }
  const worker = graph('worker.js');
  for (const name of ENGINE_FILES.rust) assert.ok(worker.has(`vendor/${name}`), `the worker loads vendor/${name}`);
  for (const name of MUPDF_ENGINE) {
    assert.equal(loaded.has(name), false, `${name} loaded`);
    assert.equal(!!localAsset(`app://pdf4all/${name}`), false, `${name} served`);
  }
  // scripts/build.mjs installs the Rust loader as worker-engine.js and keeps no emitted loader.
  for (const name of ['worker-engine-rust.js', 'worker-engine-mupdf.js']) assert.ok(!existsSync(join(RENDERER, name)), name);
});

test('a Rust build holds no MuPDF asset and no module that loads one', () => {
  for (const file of walk(resolve('build'))) {
    const name = relative(resolve('build'), file);
    assert.ok(!/mupdf/i.test(name), `${name} is a MuPDF file`);
    if (!/\.(js|html)$/.test(name)) continue;
    for (const specifier of references(readFileSync(file, 'utf8'))) {
      assert.ok(!/mupdf/i.test(specifier), `${name} loads ${specifier}`);
    }
  }
  assert.equal(localAsset('app://pdf4all/vendor/mupdf.js'), undefined);
  assert.equal(localAsset('app://pdf4all/worker-engine-mupdf.js'), undefined);
});

test('scripts/build.mjs installs the Rust loader, refuses a MuPDF build and refuses a stale Rust package', { skip: process.platform === 'win32' && 'engine/tools is sh' }, async () => {
  // A scratch tree with what build.mjs reads: a stand-in engine (sources and package) and tsc's two loaders.
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'tpdf-build-script-')));
  try {
    for (const file of ['scripts/build.mjs', 'engine/tools/workflow-sources.sh', 'src/renderer/index.html', 'src/renderer/styles.css', 'assets/icon.png', 'fixtures/hello.pdf', 'fixtures/form.pdf', 'fixtures/large.pdf']) {
      await cp(file, join(dir, file));
    }
    // A Rust build also needs the CJK font and its licence (scripts/cjk-font.mjs); the built policy says what is served.
    for (const file of ['scripts/cjk-font.mjs', CJK_FONT.source, CJK_FONT.notice, 'build/policy.js', 'build/engine-build.js', 'build/renderer/limits.js']) {
      await cp(file, join(dir, file));
    }
    await symlink(resolve('node_modules'), join(dir, 'node_modules'), 'dir');
    const sources = { 'Cargo.toml': '[package]\n', 'bridge/Cargo.toml': '[package]\n', 'bridge/Cargo.lock': '# lock\n', 'src/lib.rs': 'pub mod app;\n', 'src/app/mod.rs': '// app\n', 'bridge/src/lib.rs': '// bridge\n' };
    for (const [name, text] of Object.entries(sources)) {
      await mkdir(dirname(join(dir, 'engine', name)), { recursive: true });
      await writeFile(join(dir, 'engine', name), text);
    }
    const pkg = join(dir, 'engine/web/pkg');
    await mkdir(pkg, { recursive: true });
    await writeFile(join(pkg, 'tandem_rust_workflow.js'), 'export class RustApp {}\n');
    await writeFile(join(pkg, 'tandem_rust_workflow_bg.wasm'), 'wasm');
    // The listing build-workflow.sh stores; build.mjs computes its own and compares.
    const listing = spawnSync('sh', [join(dir, 'engine/tools/workflow-sources.sh')], { encoding: 'utf8' });
    assert.equal(listing.status, 0, listing.stderr);
    assert.equal(listing.stdout.trim().split('\n').length, Object.keys(sources).length, listing.stdout);
    await writeFile(join(pkg, 'sources.sha256'), listing.stdout);
    const loaders = async () => {
      await mkdir(join(dir, 'build/renderer'), { recursive: true });
      await writeFile(join(dir, 'build/renderer/worker-engine-rust.js'), '// rust loader\n');
    };
    const build = (args, env = {}) => spawnSync(process.execPath, ['scripts/build.mjs', ...args], { cwd: dir, encoding: 'utf8', env: { ...process.env, TPDF_ENGINE: '', TPDF_ALLOW_STALE_ENGINE: '', ...env } });
    const built = async () => ({
      loader: await readFile(join(dir, 'build/renderer/worker-engine.js'), 'utf8'),
      vendor: readdirSync(join(dir, 'build/renderer/vendor')).sort(),
      leftovers: readdirSync(join(dir, 'build/renderer')).filter(name => name.startsWith('worker-engine-')),
    });
    /** The CJK font as built and served: its files' sha256 by name, and the policy's type for it. */
    const cjkFont = () => {
      const fonts = join(dir, 'build/renderer/fonts');
      const files = existsSync(fonts) ? Object.fromEntries(readdirSync(fonts).sort().map(name => [name, sha256(readFileSync(join(fonts, name)))])) : null;
      const policy = spawnSync(process.execPath, ['--input-type=module', '-e', `const { localAsset } = await import('./build/policy.js'); process.stdout.write(String(localAsset('app://pdf4all/${CJK_FONT.served}')?.type))`], { cwd: dir, encoding: 'utf8' });
      assert.equal(policy.status, 0, policy.stderr);
      return { files, served: policy.stdout };
    };
    const shippedFont = {
      files: { 'DroidSansFallback-NOTICE.txt': sha256(readFileSync(CJK_FONT.notice)), 'DroidSansFallback.ttf': CJK_FONT.sha256 },
      served: 'font/ttf',
    };

    await loaders();
    let run = build([]);
    assert.equal(run.status, 0, run.stderr);
    assert.deepEqual(await built(), { loader: '// rust loader\n', vendor: ENGINE_FILES.rust, leftovers: [] });
    assert.deepEqual(cjkFont(), shippedFont);
    // A rerun without tsc has no loader to install: it fails rather than keep a stale one.
    run = build([]);
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /worker-engine-rust\.js is missing; run tsc/);
    // The MuPDF build is gone: asking for it fails before build/ changes (the loader tsc wrote stays).
    await loaders();
    for (const [args, env] of [[['--engine=mupdf'], {}], [[], { TPDF_ENGINE: 'mupdf' }]]) {
      run = build(args, env);
      assert.notEqual(run.status, 0);
      assert.match(run.stderr, /the app builds only the Rust engine/);
      assert.deepEqual((await built()).leftovers, ['worker-engine-rust.js']);
    }

    // A build without the pinned font fails before build/ changes, and builds again once it is back.
    const font = join(dir, CJK_FONT.source);
    const bytes = readFileSync(font);
    for (const damage of [() => rm(font), () => writeFile(font, bytes.subarray(1))]) {
      await damage();
      await loaders();
      run = build([]);
      assert.notEqual(run.status, 0);
      assert.match(run.stderr, /DroidSansFallback\.ttf(: sha256 \w+ is not the pinned| is missing)/);
      assert.deepEqual((await built()).leftovers, ['worker-engine-rust.js']);
    }
    await writeFile(font, bytes);
    run = build([]);
    assert.equal(run.status, 0, run.stderr);
    assert.deepEqual(cjkFont(), shippedFont);

    // A source changed since the package was built: refused before build/ changes, unless allowed.
    await loaders();
    await writeFile(join(dir, 'engine/src/app/mod.rs'), '// app, edited\n');
    run = build([]);
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /engine\/web\/pkg is stale: engine\/src\/app\/mod\.rs changed since it was built/);
    assert.deepEqual((await built()).leftovers, ['worker-engine-rust.js']);
    run = build([], { TPDF_ALLOW_STALE_ENGINE: '1' });
    assert.equal(run.status, 0, run.stderr);
    assert.match(run.stderr, /WARNING: engine\/web\/pkg is stale/);
    assert.deepEqual((await built()).leftovers, []);
    // So is a package with no record of its sources.
    await loaders();
    await rm(join(pkg, 'sources.sha256'));
    run = build([]);
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /has no sources\.sha256/);
  } finally {
    await rm(dir, { recursive: true, force: true, maxRetries: 5 });
  }
});

/**
 * Drives a copy of the built app whose worker-engine.js `patch` rewrote, and returns the
 * drive report. The copy keeps the real path checks: main wants the preload inside the app.
 */
async function driveCopy(prefix, patch, lines) {
  const dir = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  try {
    const app = join(dir, 'app');
    await cp('build', join(app, 'build'), { recursive: true });
    await writeFile(join(app, 'package.json'), JSON.stringify({ name: prefix.replace(/-$/, ''), type: 'module', main: 'build/main.js' }));
    const loader = join(app, 'build/renderer/worker-engine.js');
    const source = await readFile(loader, 'utf8');
    const patched = patch(source);
    assert.notEqual(patched, source, 'the patch applies to the built loader');
    await writeFile(loader, patched);
    const script = join(dir, 'script.jsonl');
    await writeFile(script, lines.map(line => JSON.stringify(line)).join('\n'));
    const report = join(dir, 'report.json');
    const electron = createRequire(import.meta.url)('electron');
    const child = spawn(electron, [`--user-data-dir=${join(dir, 'udata')}`, app, `--drive=${script}`, `--drive-out=${dir}`, `--report=${report}`], { stdio: 'ignore' });
    const code = await new Promise(done => child.on('close', done));
    return { code, result: JSON.parse(await readFile(report, 'utf8')) };
  } finally {
    await rm(dir, { recursive: true, force: true, maxRetries: 5 });
  }
}

const summary = result => JSON.stringify(result.steps?.map(step => ({ cmd: step.cmd, ok: step.ok, doc: step.state?.doc, error: step.state?.error })) ?? result);

test('a Rust engine that stops (a WASM trap) restarts the worker and restores the document (M9); the next open works', { skip: CI_NO_ELECTRON, timeout: 120_000 }, async () => {
  // The engine throws on its first search, as wasm-bindgen does after a trap.
  const trapping = 'class extends RustApp { dispatch(session, request) { if (JSON.parse(request).op === "search") throw new WebAssembly.RuntimeError("unreachable"); return super.dispatch(session, request); } }';
  const hello = resolve('fixtures/hello.pdf');
  const { code, result } = await driveCopy('tpdf-worker-trap-', source => source.replace('createRustEngine({ RustApp })', `createRustEngine({ RustApp: ${trapping} })`), [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
    { cmd: 'note', contents: 'kept across the trap', expect: { notes: 1, dirty: true } },
    { cmd: 'find', contents: { query: 'PDF4All' } },
    // M9: the session is rebuilt in the new worker from the opened bytes and the logged note.
    { cmd: 'wait', ms: 3000, expect: { doc: { name: 'hello.pdf' }, notes: 1, dirty: true, error: 'Searching failed inside the PDF engine, so it was stopped. Your document and changes were restored.', recovery: { last: { outcome: 'restored', dropped: 'search', cause: 'trap' } } } },
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf', pageCount: 1 }, notes: 0, error: '' } },
  ]);
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
});

test('a worker that fails before it is ready leaves no startup timer to stop its replacement', { skip: CI_NO_ELECTRON, timeout: 120_000 }, async () => {
  // A search kills the first worker; its replacement fails to load (before it is ready); the third
  // works. IndexedDB counts the loads, since each worker starts with fresh globals.
  const patch = source => `${source.replace('export async function loadEngine()', 'async function builtLoadEngine()')}
function countLoad() {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open("tpdf-worker-loads", 1);
    open.onupgradeneeded = () => open.result.createObjectStore("loads");
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const db = open.result;
      const tx = db.transaction("loads", "readwrite");
      const store = tx.objectStore("loads");
      let count = 0;
      const get = store.get("count");
      get.onsuccess = () => { count = (get.result ?? 0) + 1; store.put(count, "count"); };
      tx.oncomplete = () => { db.close(); resolve(count); };
      tx.onerror = () => reject(tx.error);
    };
  });
}
export async function loadEngine() {
  if (await countLoad() === 2) throw new Error("injected load failure");
  const engine = await builtLoadEngine();
  return { ...engine, dispatch(request, session) { if (request.op === "search") throw new Error("injected worker crash"); return engine.dispatch(request, session); } };
}
`;
  const hello = resolve('fixtures/hello.pdf');
  const { code, result } = await driveCopy('tpdf-worker-startup-', patch, [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
    { cmd: 'find', contents: { query: 'PDF4All' } },
    { cmd: 'wait', ms: 3000 },
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf', pageCount: 1 }, error: '' } },
    // Past the failed worker's 20 s startup timeout: the third worker and its document are still there.
    { cmd: 'wait', ms: 21_000, expect: { doc: { name: 'hello.pdf', pageCount: 1 }, error: '' } },
  ]);
  // M9: the restore's worker failed to load (the second restart), so the third reopened the document.
  const restarted = result.steps?.[2]?.state;
  assert.equal(restarted?.doc?.name, 'hello.pdf', summary(result));
  assert.equal(restarted?.error, 'Searching stopped the PDF engine unexpectedly. Your document and changes were restored.', summary(result));
  assert.equal(restarted?.recovery?.last?.outcome, 'restored', summary(result));
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
});
