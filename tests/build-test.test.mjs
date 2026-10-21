// The test build (scripts/build-test.mjs): web/ compiled into build-test/ with the engine's WASM glue and the
// pinned CJK font beside it, no MuPDF in it, and a stale engine package or a wrong font refused before
// build-test/ changes. (Replaces worker-build.test.mjs, which checked the Electron renderer's build.)
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { cp, mkdir, mkdtemp, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import test from 'node:test';
import { CJK_FONT } from '../scripts/cjk-font.mjs';

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const VENDOR = ['tandem_rust_workflow.js', 'tandem_rust_workflow_bg.wasm'];
/** One module per web/*.ts file. */
const MODULES = readdirSync('web').filter(name => name.endsWith('.ts')).map(name => name.replace(/\.ts$/, '.js')).sort();

function walk(dir) {
  return readdirSync(dir).flatMap(name => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}

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

test('build-test/ holds web/ compiled, the engine glue and the pinned font, and every module it loads; no MuPDF', () => {
  const out = resolve('build-test');
  const files = walk(out).map(file => relative(out, file).split('\\').join('/')).sort();
  assert.deepEqual(files, [...MODULES, ...VENDOR.map(name => `vendor/${name}`), 'fonts/DroidSansFallback-NOTICE.txt', CJK_FONT.served].sort());
  assert.equal(sha256(readFileSync(join(out, CJK_FONT.served))), CJK_FONT.sha256);
  for (const name of VENDOR) assert.ok(readFileSync(join(out, 'vendor', name)).equals(readFileSync(join('engine/web/pkg', name))), `vendor/${name} is engine/web/pkg's`);
  for (const name of MODULES) {
    for (const specifier of references(readFileSync(join(out, name), 'utf8'))) {
      assert.ok(specifier.startsWith('./'), `${name} loads ${specifier}, which is not beside it`);
      assert.ok(existsSync(join(out, specifier)), `${name} loads ${specifier}, which the build lacks`);
      assert.ok(!/mupdf/i.test(specifier), `${name} loads ${specifier}`);
    }
  }
  assert.ok(!files.some(name => /mupdf/i.test(name)), files.join(' '));
});

test('scripts/build-test.mjs refuses a stale engine package and a font that is not the pinned one, before build-test/ changes', { skip: process.platform === 'win32' && 'engine/tools is sh', timeout: 120_000 }, async () => {
  // A scratch tree with what the build reads: the scripts, web/, the font and a stand-in engine (sources and package).
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'tpdf-build-test-')));
  try {
    for (const file of ['scripts/build-test.mjs', 'scripts/cjk-font.mjs', 'scripts/engine-package.mjs', 'engine/tools/workflow-sources.sh', CJK_FONT.source, CJK_FONT.notice]) {
      await cp(file, join(dir, file));
    }
    await cp('web', join(dir, 'web'), { recursive: true });
    await symlink(resolve('node_modules'), join(dir, 'node_modules'), 'dir');
    const sources = { 'Cargo.toml': '[package]\n', 'bridge/Cargo.toml': '[package]\n', '../crates/Cargo.lock': '# lock\n', 'src/lib.rs': 'pub mod app;\n', 'src/app/mod.rs': '// app\n', 'bridge/src/lib.rs': '// bridge\n' };
    for (const [name, text] of Object.entries(sources)) {
      await mkdir(dirname(join(dir, 'engine', name)), { recursive: true });
      await writeFile(join(dir, 'engine', name), text);
    }
    const pkg = join(dir, 'engine/web/pkg');
    await mkdir(pkg, { recursive: true });
    await writeFile(join(pkg, 'tandem_rust_workflow.js'), 'export class RustApp {}\n');
    await writeFile(join(pkg, 'tandem_rust_workflow_bg.wasm'), 'wasm');
    // The listing build-workflow.sh stores; the build computes its own and compares.
    const listing = () => {
      const run = spawnSync('sh', [join(dir, 'engine/tools/workflow-sources.sh')], { encoding: 'utf8' });
      assert.equal(run.status, 0, run.stderr);
      return run.stdout;
    };
    assert.equal(listing().trim().split('\n').length, Object.keys(sources).length);
    await writeFile(join(pkg, 'sources.sha256'), listing());
    const build = (env = {}) => spawnSync(process.execPath, ['scripts/build-test.mjs'], { cwd: dir, encoding: 'utf8', env: { ...process.env, TPDF_ALLOW_STALE_ENGINE: '', ...env } });
    const out = join(dir, 'build-test');
    const marker = join(out, 'kept');
    /** A previous build's output, so a refused build can be seen to leave it alone. */
    const previous = async () => {
      await mkdir(out, { recursive: true });
      await writeFile(marker, 'previous build');
    };

    let run = build();
    assert.equal(run.status, 0, run.stderr);
    assert.deepEqual(readdirSync(join(out, 'vendor')).sort(), VENDOR);
    assert.deepEqual(readdirSync(join(out, 'fonts')).sort(), ['DroidSansFallback-NOTICE.txt', 'DroidSansFallback.ttf']);
    assert.ok(MODULES.every(name => existsSync(join(out, name))), readdirSync(out).join(' '));

    // A missing or damaged font fails before build-test/ changes, and builds again once it is back.
    const font = join(dir, CJK_FONT.source);
    const bytes = readFileSync(font);
    for (const damage of [() => rm(font), () => writeFile(font, bytes.subarray(1))]) {
      await damage();
      await previous();
      run = build();
      assert.notEqual(run.status, 0);
      assert.match(run.stderr, /DroidSansFallback\.ttf(: sha256 \w+ is not the pinned| is missing)/);
      assert.ok(existsSync(marker), 'a refused build leaves build-test/ alone');
    }
    await writeFile(font, bytes);

    // A source changed since the package was built: refused before build-test/ changes, unless allowed.
    await writeFile(join(dir, 'engine/src/app/mod.rs'), '// app, edited\n');
    await previous();
    run = build();
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /engine\/web\/pkg is stale: engine\/src\/app\/mod\.rs changed since it was built/);
    assert.ok(existsSync(marker));
    run = build({ TPDF_ALLOW_STALE_ENGINE: '1' });
    assert.equal(run.status, 0, run.stderr);
    assert.match(run.stderr, /WARNING: engine\/web\/pkg is stale/);
    assert.ok(!existsSync(marker), 'a build replaces the previous output');

    // So is a package with no record of its sources, or with no RustApp.
    await rm(join(pkg, 'sources.sha256'));
    run = build();
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /has no sources\.sha256/);
    await writeFile(join(pkg, 'sources.sha256'), listing());
    await writeFile(join(pkg, 'tandem_rust_workflow.js'), '// no class\n');
    run = build();
    assert.notEqual(run.status, 0);
    assert.match(run.stderr, /has no RustApp/);
  } finally {
    await rm(dir, { recursive: true, force: true, maxRetries: 5 });
  }
});
