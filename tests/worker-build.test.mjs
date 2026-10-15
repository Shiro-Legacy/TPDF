// The renderer build ships one worker engine (scripts/build.mjs, TPDF_ENGINE): every module the
// renderer and worker load is served by the asset allowlist, a Rust build carries no MuPDF, and a
// Rust engine that stops after an internal error restarts the worker.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { cp, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import test from 'node:test';
import { localAsset } from '../build/policy.js';
import { ENGINE } from '../build/engine-build.js';

const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;

const RENDERER = resolve('build/renderer');
const ENGINE_FILES = {
  rust: ['tandem_rust_workflow.js', 'tandem_rust_workflow_bg.wasm'],
  mupdf: ['mupdf-wasm.js', 'mupdf-wasm.wasm', 'mupdf.js'],
};
/** The MuPDF engine in TypeScript: the renderer imports its constants, the worker never loads it. */
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
  assert.ok(ENGINE === 'rust' || ENGINE === 'mupdf', ENGINE);
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
  if (ENGINE === 'rust') {
    for (const name of ENGINE_FILES.rust) assert.ok(worker.has(`vendor/${name}`), `the worker loads vendor/${name}`);
    for (const name of MUPDF_ENGINE) assert.ok(!worker.has(name), `the Rust worker never loads ${name}`);
  } else {
    assert.ok(worker.has('vendor/mupdf.js') && worker.has('engine.js'), [...worker].join(' '));
  }
});

test('a Rust build holds no MuPDF asset and no module that loads one', { skip: ENGINE !== 'rust' && 'MuPDF build' }, () => {
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

test('a Rust engine that stops (a WASM trap) restarts the worker; the next open works', { skip: CI_NO_ELECTRON || (ENGINE !== 'rust' && 'MuPDF build'), timeout: 120_000 }, async () => {
  // The real path: main checks that the preload lies inside the app's own path.
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'tpdf-worker-trap-')));
  try {
    // A copy of the app whose engine throws on its first search, as wasm-bindgen does after a trap.
    const app = join(dir, 'app');
    await cp('build', join(app, 'build'), { recursive: true });
    await writeFile(join(app, 'package.json'), JSON.stringify({ name: 'tpdf-worker-trap', type: 'module', main: 'build/main.js' }));
    const loader = join(app, 'build/renderer/worker-engine.js');
    const source = await readFile(loader, 'utf8');
    const trapping = 'class extends RustApp { dispatch(session, request) { if (JSON.parse(request).op === "search") throw new WebAssembly.RuntimeError("unreachable"); return super.dispatch(session, request); } }';
    const patched = source.replace('createRustEngine({ RustApp })', `createRustEngine({ RustApp: ${trapping} })`);
    assert.notEqual(patched, source, 'the loader still creates the engine from RustApp');
    await writeFile(loader, patched);
    const hello = resolve('fixtures/hello.pdf');
    const script = join(dir, 'script.jsonl');
    await writeFile(script, [
      { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
      { cmd: 'find', contents: { query: 'PDF4All' } },
      { cmd: 'wait', ms: 500, expect: { doc: null, error: 'The PDF worker stopped (the PDF engine hit an internal error). Restarted it (1 of 3); open the document again to continue.' } },
      { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf', pageCount: 1 }, error: '' } },
    ].map(line => JSON.stringify(line)).join('\n'));
    const report = join(dir, 'report.json');
    const electron = createRequire(import.meta.url)('electron');
    const child = spawn(electron, [`--user-data-dir=${join(dir, 'udata')}`, app, `--drive=${script}`, `--drive-out=${dir}`, `--report=${report}`], { stdio: 'ignore' });
    const code = await new Promise(done => child.on('close', done));
    const result = JSON.parse(await readFile(report, 'utf8'));
    assert.equal(result.ok, true, JSON.stringify(result.steps?.map(step => ({ cmd: step.cmd, ok: step.ok, doc: step.state?.doc, error: step.state?.error })) ?? result));
    assert.equal(code, 0);
  } finally {
    await rm(dir, { recursive: true, force: true, maxRetries: 5 });
  }
});
