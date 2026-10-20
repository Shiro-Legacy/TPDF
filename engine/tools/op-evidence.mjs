#!/usr/bin/env node
// SPDX-License-Identifier: MIT
// Op-to-test evidence for the Rust app engine (completion plan M0 and M9): which
// SessionRequest ops each unit test and each drive script exercises, and how
// every test fares on each engine. Run each phase under the memory cap, from the
// repository root, into one raw directory, then `report`
// (engine/reference/app-acceptance-20261016/README.md has the exact sequence):
//
//   node engine/tools/op-evidence.mjs unit   --engine=rust|mupdf --raw=<dir> [--concurrency=3] [test files]
//   node engine/tools/op-evidence.mjs suite  --engine=rust|mupdf --raw=<dir> [--concurrency=2] [test files]
//   node engine/tools/op-evidence.mjs rerun  --engine=rust|mupdf --mode=unit|suite --raw=<dir> --file=<test file> [--name=<test>] [--times=3]
//   node engine/tools/op-evidence.mjs report --raw=<dir> --out=<dir> [--ledger]
//
// unit: the unit suite without the app (TANDEM_NO_APP=1), on Rust exactly as
//   tests/run-rust.mjs runs it (its --import and RUST_SKIPS patterns), on MuPDF
//   as the default suite runs it (engine.ts) with only engine-select.mjs added.
//   Both set TPDF_OP_LOG, so engine-select.mjs logs every engine dispatch
//   against the node:test test that made it (tests/op-log.mjs). Uses build-test/
//   as it is.
// (The MuPDF app build was removed in the MIT plan's Phase 0, so only --engine=rust
// runs now; the mupdf branches below are kept for reporting earlier raw runs.)
// suite: `npm run build:test`, then the whole suite with the app
//   (tests/app-bin.mjs), `node --test tests/*.test.mjs` as npm test runs it, with
//   --test-concurrency (npm's default, 7 files at once on this 8-core Mac, does
//   not fit the 3000 MB memory cap). No op log.
// drive: ran the test files that run tests/drive scripts with a hook preloaded
//   into each Electron app process, logging the ops its worker received. The
//   Rust app runs the engine in-process, so the phase is gone with Electron;
//   `report` still reads earlier drive-<engine>/ raw runs.
// rerun: one test file (or one test by exact name) alone, N times, as unit or
//   suite runs it, to tell a load-sensitive failure from a steady one.
// report: joins the raw runs into op-evidence.json, suite-rust.json and
//   suite-mupdf.json under --out; --ledger also refreshes the two M0 columns of
//   docs/rust-engine-operation-ledger.csv.
import { spawn, spawnSync } from 'node:child_process';
import { createWriteStream, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import { join, relative, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('../..', import.meta.url));
const { RUST_SKIPS, rustTestArgs } = await import(pathToFileURL(join(root, 'tests/run-rust.mjs')).href);
const REPORTER = './engine/tools/op-evidence-reporter.mjs';

const [phase, ...rest] = process.argv.slice(2);
const option = (name, fallback) => rest.find(arg => arg.startsWith(`--${name}=`))?.slice(name.length + 3) ?? fallback;
const positional = rest.filter(arg => !arg.startsWith('--'));

function usage(message) {
  console.error(`op-evidence: ${message}\nsee the header of engine/tools/op-evidence.mjs`);
  process.exit(2);
}

function engineOption() {
  const engine = option('engine');
  if (engine !== 'rust' && engine !== 'mupdf') usage('--engine=rust|mupdf is required');
  // The MuPDF app build and its engine switch are gone (docs/mit-clean-room-plan.md Phase 0): the unit
  // suite runs only on Rust now. Earlier mupdf raw runs can still be reported.
  if (engine === 'mupdf') usage('--engine=mupdf needs the MuPDF app build, which was removed; only --engine=rust runs');
  return engine;
}

function rawOption() {
  const raw = option('raw');
  if (!raw) usage('--raw=<dir> is required');
  return resolve(raw);
}

function git(...args) {
  return spawnSync('git', args, { cwd: root, encoding: 'utf8' }).stdout.trim();
}

/** The engine build-test/ holds (`npm run build:test`): the Rust adapter, or none. */
function builtEngine() {
  return existsSync(join(root, 'build-test/engine-rust.js')) ? 'rust' : null;
}

function provenance() {
  return {
    commit: git('rev-parse', 'HEAD'),
    branch: git('rev-parse', '--abbrev-ref', 'HEAD'),
    dirty: spawnSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' }).stdout.split('\n').filter(Boolean),
    build: builtEngine(),
    node: process.version,
    machine: {
      platform: `${os.platform()} ${os.release()}`,
      arch: os.arch(),
      cpu: os.cpus()[0]?.model,
      cpus: os.cpus().length,
      memoryGB: Math.round(os.totalmem() / 2 ** 30),
      model: spawnSync('sysctl', ['-n', 'hw.model'], { encoding: 'utf8' }).stdout?.trim() || undefined,
    },
  };
}

/** Runs node (or npm) with stdout and stderr appended to `log`; resolves {code, signal, ms}. */
function run(command, args, { env, log }) {
  return new Promise(done => {
    const started = Date.now();
    const stream = createWriteStream(log, { flags: 'a' });
    const child = spawn(command, args, { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] });
    child.stdout.pipe(stream, { end: false });
    child.stderr.pipe(stream, { end: false });
    child.on('close', (code, signal) => {
      stream.end();
      done({ code, signal, ms: Date.now() - started });
    });
  });
}

function cleanEnv(extra) {
  const env = { ...process.env };
  for (const name of ['TPDF_ENGINE', 'TPDF_OP_LOG', 'TPDF_OP_LOG_TAG', 'TANDEM_NO_APP']) delete env[name];
  return { ...env, ...extra };
}

function reporterFlags(results, concurrency) {
  return [
    '--test-reporter=spec', '--test-reporter-destination=stdout',
    `--test-reporter=${REPORTER}`, `--test-reporter-destination=${results}`,
    `--test-concurrency=${concurrency}`,
  ];
}

function fresh(dir) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  return dir;
}

function quote(args) {
  return args.map(arg => (/^[\w./=:@^$-]+$/.test(arg) ? arg : `'${arg.replace(/'/g, `'\\''`)}'`)).join(' ');
}

async function unitPhase() {
  const engine = engineOption();
  const out = fresh(join(rawOption(), `unit-${engine}`));
  const concurrency = Number(option('concurrency', '3'));
  const flags = reporterFlags(join(out, 'results.jsonl'), concurrency);
  const files = positional.length ? positional : ['tests/*.test.mjs'];
  const args = engine === 'rust' ? rustTestArgs(files, flags) : ['--import', './tests/engine-select.mjs', '--test', ...flags, ...files];
  const env = cleanEnv({ TANDEM_NO_APP: '1', TPDF_OP_LOG: join(out, 'ops'), ...(engine === 'rust' ? { TPDF_ENGINE: 'rust' } : {}) });
  const meta = {
    phase: 'unit', engine, ...provenance(), concurrency,
    command: `TANDEM_NO_APP=1 TPDF_OP_LOG=${join(out, 'ops')}${engine === 'rust' ? ' TPDF_ENGINE=rust' : ''} node ${quote(args)}`,
    started: new Date().toISOString(),
  };
  const result = await run(process.execPath, args, { env, log: join(out, 'spec.log') });
  writeFileSync(join(out, 'meta.json'), JSON.stringify({ ...meta, finished: new Date().toISOString(), exitCode: result.code, signal: result.signal, ms: result.ms }, null, 2) + '\n');
  console.log(`unit ${engine}: exit ${result.code} in ${Math.round(result.ms / 1000)} s; ${summary(join(out, 'results.jsonl'))}`);
}

function summary(results) {
  const tally = { pass: 0, fail: 0, skip: 0, todo: 0 };
  for (const entry of readJsonl(results)) if (entry.kind === 'test' && entry.type !== 'suite') tally[entry.outcome]++;
  return Object.entries(tally).map(([key, value]) => `${key} ${value}`).join(', ');
}

function drivePhase() {
  usage('the drive phase logged ops inside the Electron app, which is retired (the Rust app runs the engine in-process); report still reads earlier drive runs');
}

async function suitePhase() {
  const engine = engineOption();
  const out = fresh(join(rawOption(), `suite-${engine}`));
  const concurrency = Number(option('concurrency', '2'));
  const env = cleanEnv({});
  const buildScript = 'build:test';
  const build = await run('npm', ['run', buildScript], { env, log: join(out, 'build.log') });
  if (build.code !== 0) {
    writeFileSync(join(out, 'meta.json'), JSON.stringify({ phase: 'suite', engine, build: buildScript, buildExit: build.code }, null, 2) + '\n');
    console.error(`suite ${engine}: npm run ${buildScript} failed (exit ${build.code}); see ${join(out, 'build.log')}`);
    process.exit(1);
  }
  const files = positional.length ? positional : ['tests/*.test.mjs'];
  const args = ['--test', ...reporterFlags(join(out, 'results.jsonl'), concurrency), ...files];
  const meta = {
    phase: 'suite', engine, ...provenance(), concurrency,
    command: `npm run ${buildScript} && node ${quote(args)}`,
    equivalent: `npm test, at --test-concurrency=${concurrency}`,
    started: new Date().toISOString(),
  };
  const result = await run(process.execPath, args, { env, log: join(out, 'spec.log') });
  writeFileSync(join(out, 'meta.json'), JSON.stringify({ ...meta, finished: new Date().toISOString(), exitCode: result.code, signal: result.signal, ms: result.ms }, null, 2) + '\n');
  console.log(`suite ${engine}: exit ${result.code} in ${Math.round(result.ms / 1000)} s; ${summary(join(out, 'results.jsonl'))}`);
}

async function rerunPhase() {
  const engine = engineOption();
  const mode = option('mode', 'unit');
  const file = option('file');
  const name = option('name');
  const times = Number(option('times', '3'));
  if (!file) usage('--file=<test file> is required');
  if (mode === 'suite' && builtEngine() !== engine) usage('build-test/ has no Rust adapter; run npm run build:test first');
  const dir = join(rawOption(), 'reruns');
  mkdirSync(dir, { recursive: true });
  const pattern = name ? [`--test-name-pattern=^${name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`] : [];
  for (let attempt = 1; attempt <= times; attempt++) {
    const label = `${engine}-${mode}-${file.replace(/\W+/g, '_')}${name ? `-${name.replace(/\W+/g, '_').slice(0, 60)}` : ''}-${attempt}`;
    const results = join(dir, `${label}.jsonl`);
    rmSync(results, { force: true });
    const flags = [...reporterFlags(results, 1), ...pattern];
    let args;
    let env;
    if (mode === 'unit') {
      args = engine === 'rust' ? rustTestArgs([file], flags) : ['--test', ...flags, file];
      env = cleanEnv({ TANDEM_NO_APP: '1', ...(engine === 'rust' ? { TPDF_ENGINE: 'rust' } : {}) });
    } else {
      args = ['--test', ...flags, file];
      env = cleanEnv({});
    }
    const result = await run(process.execPath, args, { env, log: join(dir, `${label}.log`) });
    const tests = readJsonl(results).filter(entry => entry.kind === 'test' && entry.type !== 'suite');
    const entry = {
      engine, mode, file, name: name ?? null, attempt, exitCode: result.code, ms: result.ms, build: builtEngine(),
      command: `${mode === 'unit' ? `TANDEM_NO_APP=1${engine === 'rust' ? ' TPDF_ENGINE=rust' : ''} ` : ''}node ${quote(args)}`,
      outcomes: tests.map(test => ({ name: test.name, outcome: test.outcome, durationMs: test.durationMs, error: test.error?.slice(0, 600) })),
    };
    writeFileSync(join(dir, 'reruns.jsonl'), JSON.stringify(entry) + '\n', { flag: 'a' });
    const failed = tests.filter(test => test.outcome === 'fail').map(test => test.name);
    console.log(`rerun ${label}: exit ${result.code}; ${summary(results)}${failed.length ? `; failed: ${failed.join(' | ')}` : ''}`);
  }
}

function readJsonl(path) {
  if (!existsSync(path)) return [];
  return readFileSync(path, 'utf8').split('\n').filter(Boolean).flatMap(line => {
    try {
      return [JSON.parse(line)];
    } catch {
      return [];
    }
  });
}

function readJson(path) {
  return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')) : null;
}

// ---------------------------------------------------------------- report

/** The SessionRequest union's ops in declaration order, with their request types (as engine/tools/operation-ledger.py reads them). */
function contractOps() {
  const text = readFileSync(join(root, 'web/protocol.ts'), 'utf8');
  const union = /export type SessionRequest\s*=([^;]+);/.exec(text)[1];
  return [...union.matchAll(/\b(\w+Request(?:Message)?)\b/g)].map(([, type]) => {
    const declaration = new RegExp(`export type ${type}\\s*=`).exec(text);
    const op = /\bop:\s*"([^"]+)"/.exec(text.slice(declaration.index + declaration[0].length))[1];
    return { op, requestType: type };
  });
}

const keyOf = (file, name) => `${file}\u0000${name}`;

/** Per-test outcomes of one run: Map key -> {file, name, outcome, reason, durationMs, error}, plus file-level failures. */
function outcomes(results) {
  const tests = new Map();
  const fileFailures = [];
  for (const entry of readJsonl(results)) {
    if (entry.kind !== 'test' || entry.type === 'suite') continue;
    // A test file that failed to load or exited early reports itself, by path, at nesting 0.
    if (entry.name === entry.file || /\.test\.mjs$/.test(entry.name ?? '')) {
      fileFailures.push(entry);
      continue;
    }
    let key = keyOf(entry.file, entry.name);
    for (let n = 2; tests.has(key); n++) key = keyOf(entry.file, `${entry.name} #${n}`);
    tests.set(key, entry);
  }
  return { tests, fileFailures, diagnostics: readJsonl(results).filter(entry => entry.kind === 'diagnostic').map(entry => entry.message) };
}

function tally(list) {
  const counts = { pass: 0, fail: 0, skip: 0, todo: 0, total: 0 };
  for (const entry of list) {
    counts[entry.outcome] = (counts[entry.outcome] ?? 0) + 1;
    counts.total++;
  }
  return counts;
}

function suiteReport(engine, raw) {
  const dir = join(raw, `suite-${engine}`);
  const meta = readJson(join(dir, 'meta.json'));
  if (!meta) return null;
  const { tests, fileFailures, diagnostics } = outcomes(join(dir, 'results.jsonl'));
  const list = [...tests.values()];
  const files = {};
  for (const entry of list) (files[entry.file] ??= []).push(entry);
  return {
    schema: 1,
    engine,
    meta,
    totals: tally(list),
    nodeDiagnostics: diagnostics,
    fileFailures: fileFailures.map(entry => ({ file: entry.file, outcome: entry.outcome, error: entry.error })),
    files: Object.fromEntries(Object.keys(files).sort().map(file => [file, tally(files[file])])),
    tests: list.sort((a, b) => a.file.localeCompare(b.file) || (a.line ?? 0) - (b.line ?? 0)).map(entry => ({
      file: entry.file, name: entry.name, line: entry.line, outcome: entry.outcome, reason: entry.reason, durationMs: entry.durationMs,
      ...(entry.outcome === 'fail' ? { failureType: entry.failureType, error: entry.error } : {}),
    })),
  };
}

/** Test name -> [{file, name}] for each drive script a test names (literally or through a file constant). */
function driveReferences() {
  const refs = new Map();
  for (const name of readdirSync(join(root, 'tests')).filter(entry => entry.endsWith('.test.mjs')).sort()) {
    const file = `tests/${name}`;
    const body = readFileSync(join(root, file), 'utf8');
    const constants = new Map();
    for (const match of body.matchAll(/\bconst\s+(\w+)\s*=\s*'(tests\/drive\/[\w.-]+\.jsonl)'/g)) constants.set(match[1], match[2]);
    const starts = [...body.matchAll(/\btest\(\s*(['"`])((?:\\.|(?!\1).)+)\1/g)];
    starts.forEach((start, index) => {
      const block = body.slice(start.index, starts[index + 1]?.index ?? body.length);
      const testName = start[2].replace(/\\(.)/g, '$1');
      const scripts = new Set(block.match(/tests\/drive\/[\w.-]+\.jsonl/g) ?? []);
      for (const [constant, script] of constants) if (new RegExp(`\\b${constant}\\b`).test(block)) scripts.add(script);
      for (const script of scripts) {
        if (!refs.has(script)) refs.set(script, []);
        refs.get(script).push({ file, name: testName });
      }
    });
  }
  return refs;
}

/** Unit op log of one run: Map test key -> Map op -> {engine -> {calls, ok, refused, threw, other, reasons}}; also edit-batch items. */
function unitOps(engine, raw) {
  const dir = join(raw, `unit-${engine}`, 'ops');
  const perTest = new Map();
  const unattributed = [];
  if (!existsSync(dir)) return { perTest, unattributed };
  for (const name of readdirSync(dir).filter(entry => entry.endsWith('.jsonl'))) {
    for (const entry of readJsonl(join(dir, name))) {
      // A test body or its own t.after/t.before hooks; file-level hooks and module code are kept apart.
      if (!entry.test || entry.kind === 'hook' || entry.kind === 'none') {
        unattributed.push(entry);
        continue;
      }
      const key = keyOf(entry.file, entry.test);
      if (!perTest.has(key)) perTest.set(key, { direct: new Map(), nested: new Map(), via: new Set() });
      const record = perTest.get(key);
      record.via.add(entry.kind);
      const bump = (map, op) => {
        if (!map.has(op)) map.set(op, {});
        const byEngine = map.get(op);
        const counts = (byEngine[entry.engine] ??= { calls: 0, ok: 0, refused: 0, threw: 0, other: 0, reasons: {} });
        counts.calls += entry.count;
        counts[entry.outcome] += entry.count;
        if (entry.reason) counts.reasons[entry.reason] = (counts.reasons[entry.reason] ?? 0) + entry.count;
      };
      bump(record.direct, entry.op ?? '(no op)');
      for (const item of entry.nested ?? []) bump(record.nested, item ?? '(no op)');
    }
  }
  return { perTest, unattributed };
}

/** sha256 of packDrive(script) -> script, for each tests/drive script: what scripts/drive.mjs hands Electron. */
async function scriptHashes() {
  const { packDrive } = await import(pathToFileURL(join(root, 'scripts/session.mjs')).href);
  const hashes = new Map();
  for (const name of readdirSync(join(root, 'tests/drive')).filter(entry => entry.endsWith('.jsonl'))) {
    const packed = packDrive(readFileSync(join(root, 'tests/drive', name), 'utf8'));
    hashes.set(createHash('sha256').update(packed).digest('hex'), `tests/drive/${name}`);
  }
  return hashes;
}

/** Electron runs of one engine's drive phase (recorded before Electron was retired), one per process: script, exit, ops. */
function driveRuns(engine, raw, hashes) {
  const dir = join(raw, `drive-${engine}`, 'ops');
  const runs = new Map();
  const problems = [];
  if (!existsSync(dir)) return { runs: [], problems };
  for (const name of readdirSync(dir).filter(entry => entry.endsWith('.jsonl'))) {
    for (const entry of readJsonl(join(dir, name))) {
      if (!runs.has(entry.pid)) runs.set(entry.pid, { pid: entry.pid, ops: new Map(), nested: new Map(), exit: null });
      const run = runs.get(entry.pid);
      if (entry.t === 'start') {
        run.drive = entry.drive;
        run.smoke = entry.smoke;
        run.script = entry.drive === '-' ? '(stdin session)' : hashes.get(entry.scriptSha) ?? (entry.drive ? `(unmatched script ${entry.scriptSha?.slice(0, 12)})` : entry.smoke ? '(smoke)' : '(app)');
      } else if (entry.t === 'exit') {
        run.exit = entry;
      } else if (entry.t === 'error') {
        problems.push(entry);
      } else if (entry.t === 'sent' || entry.t === 'reply') {
        if (!run.ops.has(entry.op)) run.ops.set(entry.op, { sent: 0, ok: 0, refused: 0, other: 0, reasons: {}, errors: {} });
        const counts = run.ops.get(entry.op);
        if (entry.t === 'sent') {
          counts.sent++;
          for (const item of entry.nested ?? []) run.nested.set(item, (run.nested.get(item) ?? 0) + 1);
        } else {
          counts[entry.outcome]++;
          if (entry.reason) counts.reasons[entry.reason] = (counts.reasons[entry.reason] ?? 0) + 1;
          if (entry.outcome === 'refused' && entry.error) counts.errors[entry.error] = (counts.errors[entry.error] ?? 0) + 1;
        }
      }
    }
  }
  return { runs: [...runs.values()], problems };
}

/** The ops of several runs added up. */
function mergeOps(runs) {
  const merged = new Map();
  const nested = new Map();
  for (const run of runs) {
    for (const [op, counts] of run.ops) {
      const into = merged.get(op) ?? { sent: 0, ok: 0, refused: 0, other: 0, reasons: {}, errors: {} };
      for (const field of ['sent', 'ok', 'refused', 'other']) into[field] += counts[field];
      for (const key of ['reasons', 'errors']) for (const [text, count] of Object.entries(counts[key])) into[key][text] = (into[key][text] ?? 0) + count;
      merged.set(op, into);
    }
    for (const [op, count] of run.nested) nested.set(op, (nested.get(op) ?? 0) + count);
  }
  return {
    ops: Object.fromEntries([...merged].sort(([a], [b]) => a.localeCompare(b)).map(([op, counts]) => [op, {
      sent: counts.sent, ok: counts.ok, refused: counts.refused, other: counts.other,
      ...(Object.keys(counts.reasons).length ? { reasons: counts.reasons } : {}),
      ...(Object.keys(counts.errors).length ? { errors: counts.errors } : {}),
    }])),
    editBatchItems: Object.fromEntries(nested),
  };
}

const RUST_ENGINES = ['rust', 'rust-direct'];

function sumEngines(byEngine, engines) {
  const total = { calls: 0, ok: 0, refused: 0, threw: 0, other: 0, reasons: {} };
  for (const engine of engines) {
    const counts = byEngine?.[engine];
    if (!counts) continue;
    for (const field of ['calls', 'ok', 'refused', 'threw', 'other']) total[field] += counts[field];
    for (const [reason, count] of Object.entries(counts.reasons)) total.reasons[reason] = (total.reasons[reason] ?? 0) + count;
  }
  return total.calls ? total : null;
}

async function report() {
  const raw = rawOption();
  const out = resolve(option('out') ?? usage('--out=<dir> is required'));
  mkdirSync(out, { recursive: true });
  const ops = contractOps();
  const skipReasons = new Map(RUST_SKIPS.map(skip => [skip.test, skip.reason]));

  const unit = {};
  for (const engine of ['rust', 'mupdf']) {
    unit[engine] = {
      meta: readJson(join(raw, `unit-${engine}`, 'meta.json')),
      ...outcomes(join(raw, `unit-${engine}`, 'results.jsonl')),
      ...unitOps(engine, raw),
    };
  }
  // Tests skipped by RUST_SKIPS never start on Rust, so node:test does not report them there: add them as skips.
  for (const [key, entry] of unit.mupdf.tests) {
    if (skipReasons.has(entry.name) && !unit.rust.tests.has(key)) {
      unit.rust.tests.set(key, { ...entry, outcome: 'skip', reason: `RUST_SKIPS: ${skipReasons.get(entry.name)}`, durationMs: undefined, error: undefined, rustSkip: true });
    }
  }
  const hashes = await scriptHashes();
  const drive = {};
  for (const engine of ['rust', 'mupdf']) {
    drive[engine] = { meta: readJson(join(raw, `drive-${engine}`, 'meta.json')), ...outcomes(join(raw, `drive-${engine}`, 'results.jsonl')), ...driveRuns(engine, raw, hashes) };
  }
  const suites = { rust: suiteReport('rust', raw), mupdf: suiteReport('mupdf', raw) };
  const refs = driveReferences();
  const suiteOutcome = (engine, file, name) => suites[engine]?.tests.find(test => test.file === file && test.name === name)?.outcome ?? null;
  const scripts = readdirSync(join(root, 'tests/drive')).filter(name => name.endsWith('.jsonl')).sort().map(name => `tests/drive/${name}`);

  // Per test: both engines' outcomes and ops.
  const keys = new Set([...unit.rust.tests.keys(), ...unit.mupdf.tests.keys()]);
  const tests = [...keys].map(key => {
    const [file, name] = key.split('\u0000');
    const rust = unit.rust.tests.get(key);
    const mupdf = unit.mupdf.tests.get(key);
    const rustOps = unit.rust.perTest.get(key);
    const mupdfOps = unit.mupdf.perTest.get(key);
    const opsOf = (record, engines) => record ? Object.fromEntries([...record.direct].flatMap(([op, byEngine]) => {
      const counts = sumEngines(byEngine, engines);
      return counts ? [[op, { calls: counts.calls, ok: counts.ok, refused: counts.refused, threw: counts.threw }]] : [];
    }).sort(([a], [b]) => a.localeCompare(b))) : {};
    const nestedOf = (record, engines) => record ? Object.fromEntries([...record.nested].flatMap(([op, byEngine]) => {
      const counts = sumEngines(byEngine, engines);
      return counts ? [[op, counts.calls]] : [];
    })) : {};
    return {
      file, name, line: (rust ?? mupdf)?.line,
      rust: rust ? {
        outcome: rust.outcome, reason: rust.reason, durationMs: rust.durationMs, error: rust.outcome === 'fail' ? rust.error : undefined,
        ops: opsOf(rustOps, RUST_ENGINES), editBatchItems: nestedOf(rustOps, RUST_ENGINES),
        fakeBridgeOps: opsOf(rustOps, ['rust-fake']), mupdfOracleOps: opsOf(rustOps, ['mupdf']),
        attribution: rustOps ? [...rustOps.via].sort() : [],
      } : null,
      mupdf: mupdf ? {
        outcome: mupdf.outcome, reason: mupdf.reason, durationMs: mupdf.durationMs, error: mupdf.outcome === 'fail' ? mupdf.error : undefined,
        ops: opsOf(mupdfOps, ['mupdf']), editBatchItems: nestedOf(mupdfOps, ['mupdf']),
        rustDirectOps: opsOf(mupdfOps, ['rust-direct']), fakeBridgeOps: opsOf(mupdfOps, ['rust-fake']),
        attribution: mupdfOps ? [...mupdfOps.via].sort() : [],
      } : null,
    };
  }).sort((a, b) => a.file.localeCompare(b.file) || (a.line ?? 0) - (b.line ?? 0));

  const driveScripts = scripts.map(script => {
    const perEngine = engine => {
      if (!drive[engine].meta) return null;
      const runs = drive[engine].runs.filter(run => run.script === script);
      return {
        // A script passes when every run of it (as its tests ran it) exited 0: every expect matched.
        outcome: !runs.length ? 'not run' : runs.every(run => run.exit?.code === 0) ? 'pass' : 'fail',
        runs: runs.map(run => ({
          exitCode: run.exit?.code ?? null, steps: run.exit?.steps, expects: run.exit?.expects, error: run.exit?.error,
          ...(run.exit?.failed?.length ? { failedSteps: run.exit.failed } : {}),
        })),
        ...mergeOps(runs),
      };
    };
    return {
      script,
      tests: (refs.get(script) ?? []).map(ref => ({
        ...ref,
        driveRunRust: drive.rust.tests.get(keyOf(ref.file, ref.name))?.outcome ?? null,
        suiteRust: suiteOutcome('rust', ref.file, ref.name),
        suiteMupdf: suiteOutcome('mupdf', ref.file, ref.name),
      })),
      rust: perEngine('rust'),
      mupdf: perEngine('mupdf'),
    };
  });
  const otherRuns = engine => {
    const groups = new Map();
    for (const run of drive[engine].runs.filter(item => !scripts.includes(item.script))) {
      if (!groups.has(run.script)) groups.set(run.script, []);
      groups.get(run.script).push(run);
    }
    return Object.fromEntries([...groups].map(([label, runs]) => [label, {
      runs: runs.length, exitCodes: runs.map(run => run.exit?.code ?? null), ...mergeOps(runs),
    }]));
  };

  const operations = {};
  for (const { op, requestType } of ops) {
    const rustTests = [];
    const mupdfTests = [];
    const rustBatch = [];
    const mupdfBatch = [];
    for (const test of tests) {
      const rustCalls = test.rust?.ops[op];
      if (rustCalls) rustTests.push({ file: test.file, name: test.name, outcome: test.rust.outcome, ...rustCalls });
      else if (test.rust?.outcome === 'skip' && test.mupdf?.ops[op]) {
        rustTests.push({ file: test.file, name: test.name, outcome: 'skip', reason: test.rust.reason, opsFrom: 'mupdf run', ...test.mupdf.ops[op] });
      }
      if (test.mupdf?.ops[op]) mupdfTests.push({ file: test.file, name: test.name, outcome: test.mupdf.outcome, ...test.mupdf.ops[op] });
      if (test.rust?.editBatchItems[op]) rustBatch.push({ file: test.file, name: test.name, outcome: test.rust.outcome, items: test.rust.editBatchItems[op] });
      if (test.mupdf?.editBatchItems[op]) mupdfBatch.push({ file: test.file, name: test.name, outcome: test.mupdf.outcome, items: test.mupdf.editBatchItems[op] });
    }
    const driveEntries = engine => driveScripts.flatMap(entry => {
      const counts = entry[engine]?.ops[op];
      return counts ? [{ script: entry.script, outcome: entry[engine].outcome, ...counts }] : [];
    });
    const driveRust = driveEntries('rust');
    const driveMupdf = driveEntries('mupdf');
    const count = list => ({ ...tally(list), withOkReply: list.filter(entry => entry.ok > 0).length });
    const rustCount = count(rustTests);
    const flags = [];
    if (rustCount.pass === 0) flags.push('no passing Rust unit test');
    if (rustTests.length && rustTests.every(entry => entry.outcome === 'skip')) flags.push('unit tests only skipped on Rust');
    if (!driveRust.length) flags.push('no drive coverage');
    if (rustTests.length && !rustTests.some(entry => entry.outcome === 'pass' && entry.ok > 0)) flags.push('no passing Rust unit test with an ok reply');
    if (driveRust.length && !driveRust.some(entry => entry.ok > 0)) flags.push('no ok reply in any Rust drive');
    operations[op] = {
      requestType,
      unit: {
        rust: { ...rustCount, tests: rustTests },
        mupdf: { ...count(mupdfTests), tests: mupdfTests },
        viaEditBatchItems: { rust: rustBatch, mupdf: mupdfBatch },
      },
      drive: {
        rust: { ...tally(driveRust), withOkReply: driveRust.filter(entry => entry.ok > 0).length, scripts: driveRust },
        mupdf: { ...tally(driveMupdf), withOkReply: driveMupdf.filter(entry => entry.ok > 0).length, scripts: driveMupdf },
      },
      flags,
    };
  }
  // Parity: for each test passing on both engines, the same ops with the same ok/refused/threw counts;
  // for each drive script, the same requests with the same replies (and refusal reasons).
  const sameCounts = (a = {}, b = {}, fields) => fields.every(field => JSON.stringify(a[field] ?? null) === JSON.stringify(b[field] ?? null));
  const unitParity = tests.filter(test => test.rust?.outcome === 'pass' && test.mupdf?.outcome === 'pass' && (Object.keys(test.rust.ops).length || Object.keys(test.mupdf.ops).length)).map(test => {
    const differing = [...new Set([...Object.keys(test.rust.ops), ...Object.keys(test.mupdf.ops)])].sort()
      .filter(op => !sameCounts(test.rust.ops[op], test.mupdf.ops[op], ['calls', 'ok', 'refused', 'threw']))
      .map(op => ({ op, rust: test.rust.ops[op] ?? null, mupdf: test.mupdf.ops[op] ?? null }));
    return { file: test.file, name: test.name, differing };
  });
  const driveParity = driveScripts.filter(entry => entry.rust?.runs.length && entry.mupdf?.runs.length).map(entry => {
    const differing = [...new Set([...Object.keys(entry.rust.ops), ...Object.keys(entry.mupdf.ops)])].sort()
      .filter(op => !sameCounts(entry.rust.ops[op], entry.mupdf.ops[op], ['sent', 'ok', 'refused', 'other', 'reasons']))
      .map(op => ({ op, rust: entry.rust.ops[op] ?? null, mupdf: entry.mupdf.ops[op] ?? null }));
    return { script: entry.script, differing };
  });
  const parity = {
    unitTestsCompared: unitParity.length,
    unitTestsIdentical: unitParity.filter(entry => !entry.differing.length).length,
    unitDifferences: unitParity.filter(entry => entry.differing.length),
    driveScriptsCompared: driveParity.length,
    driveScriptsIdentical: driveParity.filter(entry => !entry.differing.length).length,
    driveDifferences: driveParity.filter(entry => entry.differing.length),
  };
  const flagged = name => Object.entries(operations).filter(([, entry]) => entry.flags.includes(name)).map(([op]) => op);
  const unknownOps = new Set();
  for (const test of tests) for (const side of [test.rust, test.mupdf]) for (const op of Object.keys(side?.ops ?? {})) if (!operations[op]) unknownOps.add(op);
  for (const entry of driveScripts) for (const side of [entry.rust, entry.mupdf]) for (const op of Object.keys(side?.ops ?? {})) if (!operations[op]) unknownOps.add(op);

  const evidence = {
    schema: 1,
    generated: new Date().toISOString(),
    tool: 'engine/tools/op-evidence.mjs',
    contract: { source: 'web/protocol.ts SessionRequest', operations: ops.length },
    runs: {
      unitRust: { ...unit.rust.meta, totals: tally([...unit.rust.tests.values()]), fileFailures: unit.rust.fileFailures.map(entry => ({ file: entry.file, error: entry.error })), unattributedDispatches: unit.rust.unattributed.reduce((sum, entry) => sum + entry.count, 0) },
      unitMupdf: { ...unit.mupdf.meta, totals: tally([...unit.mupdf.tests.values()]), fileFailures: unit.mupdf.fileFailures.map(entry => ({ file: entry.file, error: entry.error })), unattributedDispatches: unit.mupdf.unattributed.reduce((sum, entry) => sum + entry.count, 0) },
      driveRust: {
        ...drive.rust.meta, testTotals: tally([...drive.rust.tests.values()]),
        scriptTotals: tally(driveScripts.filter(entry => entry.rust && entry.rust.outcome !== 'not run').map(entry => entry.rust)),
        scriptsNotRun: driveScripts.filter(entry => entry.rust?.outcome === 'not run').map(entry => entry.script),
        electronRuns: drive.rust.runs.length, otherRuns: otherRuns('rust'), hookProblems: drive.rust.problems,
      },
      driveMupdf: {
        ...drive.mupdf.meta, testTotals: tally([...drive.mupdf.tests.values()]),
        scriptTotals: tally(driveScripts.filter(entry => entry.mupdf && entry.mupdf.outcome !== 'not run').map(entry => entry.mupdf)),
        scriptsNotRun: driveScripts.filter(entry => entry.mupdf?.outcome === 'not run').map(entry => entry.script),
        electronRuns: drive.mupdf.runs.length, otherRuns: otherRuns('mupdf'), hookProblems: drive.mupdf.problems,
      },
      reruns: readJsonl(join(raw, 'reruns', 'reruns.jsonl')),
    },
    flags: {
      noPassingRustUnitTest: flagged('no passing Rust unit test'),
      unitTestsOnlySkippedOnRust: flagged('unit tests only skipped on Rust'),
      noDriveCoverage: flagged('no drive coverage'),
      noPassingRustUnitTestWithOkReply: flagged('no passing Rust unit test with an ok reply'),
      noOkReplyInAnyRustDrive: flagged('no ok reply in any Rust drive'),
    },
    opsOutsideContract: [...unknownOps].sort(),
    parity,
    table: Object.fromEntries(Object.entries(operations).map(([op, entry]) => [op, {
      rustUnit: `${entry.unit.rust.pass}/${entry.unit.rust.total}`,
      rustUnitSkipped: entry.unit.rust.skip,
      mupdfUnit: `${entry.unit.mupdf.pass}/${entry.unit.mupdf.total}`,
      rustDrive: `${entry.drive.rust.pass}/${entry.drive.rust.total}`,
      mupdfDrive: `${entry.drive.mupdf.pass}/${entry.drive.mupdf.total}`,
      flags: entry.flags,
    }])),
    operations,
    unattributed: {
      rust: unit.rust.unattributed,
      mupdf: unit.mupdf.unattributed,
    },
    tests,
    driveScripts,
  };
  writeFileSync(join(out, 'op-evidence.json'), JSON.stringify(evidence, null, 1) + '\n');
  if (rest.includes('--ledger')) updateLedger(operations, out);
  for (const engine of ['rust', 'mupdf']) if (suites[engine]) writeFileSync(join(out, `suite-${engine}.json`), JSON.stringify(suites[engine], null, 1) + '\n');
  console.log(JSON.stringify({ table: evidence.table, flags: evidence.flags, parity: { ...parity, unitDifferences: parity.unitDifferences.length, driveDifferences: parity.driveDifferences },  runs: Object.fromEntries(Object.entries(evidence.runs).map(([name, value]) => [name, value.totals])), suites: Object.fromEntries(Object.entries(suites).map(([name, value]) => [name, value?.totals])), opsOutsideContract: evidence.opsOutsideContract }, null, 1));
}

/** RFC 4180 rows, as Python's csv module reads them. */
function parseCsv(text) {
  const rows = [];
  let row = [];
  let field = '';
  let quoted = false;
  for (let i = 0; i < text.length; i++) {
    const char = text[i];
    if (quoted) {
      if (char === '"' && text[i + 1] === '"') {
        field += '"';
        i++;
      } else if (char === '"') quoted = false;
      else field += char;
    } else if (char === '"') quoted = true;
    else if (char === ',') {
      row.push(field);
      field = '';
    } else if (char === '\n') {
      row.push(field);
      rows.push(row);
      row = [];
      field = '';
    } else field += char;
  }
  if (field || row.length) rows.push([...row, field]);
  return rows;
}

/** Rows written as Python's csv.writer does (QUOTE_MINIMAL, "\n" line ends), which the ledger uses. */
function formatCsv(rows) {
  const cell = value => (/[",\r\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value);
  return rows.map(row => row.map(cell).join(',')).join('\n') + '\n';
}

/**
 * --ledger: the two columns M0 left stale in docs/rust-engine-operation-ledger.csv.
 * candidate_legacy_tests: the drive scripts, then the unit test files, whose runs here dispatched the op
 * (on Rust, or on MuPDF for a test skipped on Rust); remaining_verification: the counts and flags.
 * Other columns are kept byte for byte.
 */
function updateLedger(operations, out) {
  const path = join(root, 'docs/rust-engine-operation-ledger.csv');
  const text = readFileSync(path, 'utf8');
  const rows = parseCsv(text);
  if (formatCsv(rows) !== text) throw new Error('the ledger does not round-trip through the CSV writer; not touching it');
  const [header, ...body] = rows;
  const column = name => {
    const index = header.indexOf(name);
    if (index < 0) throw new Error(`the ledger has no ${name} column`);
    return index;
  };
  const [opColumn, candidates, remaining] = [column('operation'), column('candidate_legacy_tests'), column('remaining_verification')];
  const evidence = relative(root, join(out, 'op-evidence.json')).split('\\').join('/');
  const date = /app-acceptance-(\d{8})/.exec(evidence)?.[1] ?? new Date().toISOString().slice(0, 10).replace(/-/g, '');
  for (const row of body) {
    const entry = operations[row[opColumn]];
    if (!entry) throw new Error(`ledger operation ${row[opColumn]} is not in the contract`);
    const scripts = [...new Set(entry.drive.rust.scripts.map(script => script.script))].sort();
    const files = [...new Set([...entry.unit.rust.tests, ...entry.unit.mupdf.tests].map(test => test.file))].sort();
    // engine/tools/operation-ledger.py needs files here: with no recorded test, the old candidates stay.
    if (scripts.length || files.length) row[candidates] = [...scripts, ...files].join('; ');
    const rust = entry.unit.rust;
    const parts = [
      `M0 mapped ${date.slice(0, 4)}-${date.slice(4, 6)}-${date.slice(6)} in ${evidence}`,
      `Rust unit tests ${rust.pass}/${rust.total} pass${rust.fail ? ` (${rust.fail} fail)` : ''}${rust.skip ? ` (${rust.skip} skipped on Rust)` : ''}`,
      `MuPDF unit tests ${entry.unit.mupdf.pass}/${entry.unit.mupdf.total} pass`,
      `Rust drives ${entry.drive.rust.pass}/${entry.drive.rust.total} pass`,
      ...(scripts.length || files.length ? [] : ['no recorded test dispatches it (candidates above are the old static ones)']),
      ...entry.flags.map(flag => `flag: ${flag}`),
      'M10 final acceptance',
    ];
    row[remaining] = parts.join('; ');
  }
  writeFileSync(path, formatCsv([header, ...body]));
  console.error(`updated ${relative(root, path)}: candidate_legacy_tests and remaining_verification for ${body.length} operations`);
}

const phases = { unit: unitPhase, drive: drivePhase, suite: suitePhase, rerun: rerunPhase, report };
if (!phases[phase]) usage(`unknown phase ${JSON.stringify(phase)}`);
await phases[phase]();
