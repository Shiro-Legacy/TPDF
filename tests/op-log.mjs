// Test-only op log (engine/tools/op-evidence.mjs). With TPDF_OP_LOG=<dir>,
// engine-select.mjs preloads this module and routes each test file's imports of
// rust-engine.mjs, mupdf-oracle.mjs and build/renderer/engine-rust.js through
// op-log-engine.mjs and op-log-rust.mjs, whose engines' dispatch calls land in
// logged() below; with it unset nothing loads this file and the suite runs
// exactly as before.
//
// Attribution is node:test's own: each test, and each hook, is an AsyncResource
// of type "Test" whose body runs in its async scope. An init hook marks every
// async resource created inside a test with that test, so a dispatch made
// anywhere in a test body or its async continuations names the test; one made
// in a t.after() or file-level hook names the hook and its test (or the file).
// Lines are counted in memory and appended to <dir>/unit-<pid>.jsonl at exit.
import { createHook, executionAsyncResource } from 'node:async_hooks';
import { appendFileSync, mkdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const dir = process.env.TPDF_OP_LOG;
if (!dir) throw new Error('tests/op-log.mjs is loaded only under TPDF_OP_LOG');
const root = fileURLToPath(new URL('..', import.meta.url));
const OWNER = Symbol('tpdf op-log test');
const counts = new Map();

createHook({
  init(_asyncId, type, _triggerAsyncId, resource) {
    const owner = type === 'Test' ? resource : executionAsyncResource()?.[OWNER];
    if (owner && resource && typeof resource === 'object') resource[OWNER] = owner;
  },
}).enable();

process.on('exit', () => {
  if (!counts.size) return;
  mkdirSync(dir, { recursive: true });
  appendFileSync(join(dir, `unit-${process.pid}.jsonl`), [...counts.values()].map(entry => JSON.stringify(entry)).join('\n') + '\n');
});

function attempt(read) {
  try {
    return read();
  } catch {
    return undefined;
  }
}

/** {file, test, kind} of the running test or hook, from node:test's resource. */
function running() {
  const resource = executionAsyncResource();
  let owner = resource?.[OWNER] ?? (resource?.constructor?.name === 'Test' ? resource : undefined);
  if (!owner) return { file: null, test: null, kind: 'none' };
  let hook = null;
  if (owner.hookType) {
    hook = owner.hookType;
    owner = owner.parentTest;
  }
  const names = [];
  for (let at = owner; at?.parent; at = at.parent) names.unshift(at.name);
  const path = attempt(() => owner.loc?.file) ?? attempt(() => owner.entryFile);
  const file = typeof path === 'string' ? relative(root, path.startsWith('file:') ? fileURLToPath(path) : path).split('\\').join('/') : null;
  if (!names.length) return { file, test: hook ? `(file ${hook} hook)` : null, kind: hook ? 'hook' : 'none' };
  return { file, test: names.join(' > '), kind: hook ? `${hook} hook` : 'test' };
}

function record(engine, importer, request, outcome, reason) {
  const op = attempt(() => request?.op);
  const items = op === 'edit-batch' ? attempt(() => request.items) : undefined;
  const nested = Array.isArray(items) ? items.map(item => attempt(() => item?.op) ?? null) : undefined;
  const entry = {
    importer,
    ...running(),
    engine,
    op: typeof op === 'string' ? op : null,
    nested,
    outcome,
    reason: typeof reason === 'string' ? reason : undefined,
  };
  const key = JSON.stringify(entry);
  const known = counts.get(key);
  if (known) known.count++;
  else counts.set(key, { ...entry, count: 1 });
}

/**
 * The engine's dispatch, recording each call: the op, an edit-batch's item ops,
 * and the reply's ok (or the throw). The request and reply pass through untouched.
 */
export function logged(dispatch, engine, importer) {
  return function dispatchLogged(request, session) {
    let result;
    try {
      result = Reflect.apply(dispatch, this, arguments);
    } catch (err) {
      record(engine, importer, request, 'threw', attempt(() => String(err?.message).slice(0, 200)));
      throw err;
    }
    const reply = attempt(() => result?.reply);
    const ok = attempt(() => reply?.ok);
    record(engine, importer, request, ok === true ? 'ok' : ok === false ? 'refused' : 'other', ok === false ? attempt(() => reply.reason) : undefined);
    return result;
  };
}
