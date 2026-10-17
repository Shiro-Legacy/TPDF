// M9 recovery for long operations, driven in Electron (engine/design/app-m9-recovery.md). Each test
// drives a copy of the built app whose watchdog is shortened (REQUEST_TIMEOUT_MS, SAVE_TIMEOUT_MS) and
// whose worker engine hangs on a marked request (a busy loop, never a real 20 s computation):
// - contents with "HANG": the request hangs in every worker;
// - contents with "SLOW-ON-REPLAY": it hangs only in a restarted worker, so its replay hangs;
// - `save` hangs in the first worker only (`hangSave`), `insert-pages` likewise (`hangInsert`);
// - a page render hangs while the page has an annotation whose contents include "HANGPAGE";
// - Rust only (`trapOnReplay`): a request with "TRAP-ON-REPLAY" traps the engine in a restarted worker.
// A worker counts its incarnation in IndexedDB, as worker-build.test.mjs does.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { cp, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';
import mupdf from 'mupdf';
import { ENGINE } from '../build/engine-build.js';
import { createEngine } from '../build/renderer/engine.js';

const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const WATCHDOG_MS = 3000;
const hello = resolve('fixtures/hello.pdf');

function hangPatch({ hangSave = false, hangInsert = false, trapOnReplay = false } = {}) {
  const trapping = 'class extends RustApp { dispatch(session, request) { if (globalThis.m9Born >= 2 && request.includes("TRAP-ON-REPLAY")) throw new WebAssembly.RuntimeError("unreachable"); return super.dispatch(session, request); } }';
  return source => `${(trapOnReplay ? source.replace('createRustEngine({ RustApp })', `createRustEngine({ RustApp: ${trapping} })`) : source).replace('export async function loadEngine()', 'async function builtLoadEngine()')}
function incarnation() {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open("tpdf-m9-loads", 1);
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
  const born = await incarnation();
  globalThis.m9Born = born;
  const engine = await builtLoadEngine();
  return { ...engine, dispatch(request, session) {
    const text = typeof request?.contents === "string" ? request.contents : "";
    let hang = (text.includes("HANG") && !text.includes("HANGPAGE")) || (text.includes("SLOW-ON-REPLAY") && born >= 2)
      || (${hangSave} && request?.op === "save" && born === 1) || (${hangInsert} && request?.op === "insert-pages" && born === 1);
    if (request?.op === "render") {
      const listed = engine.dispatch({ id: 0, op: "annots", page: request.page }, session).reply;
      if (listed?.ok !== false && (listed.items ?? []).some(item => String(item.contents).includes("HANGPAGE"))) hang = true;
    }
    if (hang) { const end = Date.now() + 120000; while (Date.now() < end) { /* hung */ } }
    return engine.dispatch(request, session);
  } };
}
`;
}

/** Drives a patched copy of the built app; returns the report and the drive's output folder files. */
async function driveCopy(prefix, lines, { engine = hangPatch(), budget = null, renderer = source => source, saveTimeoutMs = WATCHDOG_MS, main = null } = {}) {
  const dir = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  try {
    const app = join(dir, 'app');
    const out = join(dir, 'out');
    await mkdir(out);
    await cp('build', join(app, 'build'), { recursive: true });
    await writeFile(join(app, 'package.json'), JSON.stringify({ name: prefix.replace(/-$/, ''), type: 'module', main: 'build/main.js' }));
    const edit = async (file, change) => {
      const path = join(app, 'build', file);
      const source = await readFile(path, 'utf8');
      const patched = change(source);
      assert.notEqual(patched, source, `the patch applies to ${file}`);
      await writeFile(path, patched);
    };
    await edit('renderer/worker-engine.js', engine);
    await edit('renderer/renderer.js', source => renderer(source
      .replace('const REQUEST_TIMEOUT_MS = 20_000;', `const REQUEST_TIMEOUT_MS = ${WATCHDOG_MS};`)
      .replace('const SAVE_TIMEOUT_MS = 120_000;', `const SAVE_TIMEOUT_MS = ${saveTimeoutMs};`)));
    if (main) await edit('main.js', main);
    if (budget !== null) {
      await edit('renderer/recovery.js', source => source.replace('export const RECOVERY_BUDGET_BYTES = 256 * 1024 * 1024;', `export const RECOVERY_BUDGET_BYTES = ${budget};`));
    }
    const script = join(dir, 'script.jsonl');
    await writeFile(script, lines.map(line => JSON.stringify(line).replaceAll('${OUT}', out)).join('\n'));
    const report = join(dir, 'report.json');
    const electron = createRequire(import.meta.url)('electron');
    const child = spawn(electron, [`--user-data-dir=${join(dir, 'udata')}`, app, `--drive=${script}`, `--drive-out=${out}`, `--report=${report}`], { stdio: 'ignore' });
    const code = await new Promise(done => child.on('close', done));
    const result = JSON.parse(await readFile(report, 'utf8'));
    const files = {};
    for (const name of ['recovered.pdf', 'control.pdf', 'fb.pdf', 'saved.pdf']) {
      try { files[name] = await readFile(join(out, name)); } catch { /* not written by this run */ }
    }
    return { code, result, files };
  } finally {
    await rm(dir, { recursive: true, force: true, maxRetries: 5 });
  }
}

const summary = result => JSON.stringify(result.steps?.map(step => ({ cmd: step.cmd, ok: step.ok, doc: step.state?.doc?.name ?? null, notes: step.state?.notes, revision: step.state?.revision, error: step.state?.error, recovery: step.state?.recovery })) ?? result);

/** The annotations of a saved file, read with MuPDF: type, contents and rect (dates are not compared). */
function annotsOf(bytes) {
  const engine = createEngine(mupdf);
  const session = engine.createSession();
  try {
    const opened = engine.dispatch({ id: 1, op: 'open', name: 'x.pdf', pdf: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) }, session).reply;
    assert.equal(opened.ok, true, opened.error);
    const items = engine.dispatch({ id: 2, op: 'annots', page: 0 }, session).reply.items;
    return items.map(item => ({ type: item.type, contents: item.contents, rect: item.rect.map(value => Math.round(value * 100) / 100) }));
  } finally {
    engine.dispatch({ id: 3, op: 'close' }, session);
  }
}

const EDITS = [
  { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' }, error: '', recovery: { available: true, entries: 0 } } },
  { cmd: 'freetext', contents: 'M9 box one', expect: { annots: { FreeText: 1 }, dirty: true } },
  { cmd: 'note', contents: 'M9 note two', expect: { notes: 1 } },
  { cmd: 'undo', expect: { notes: 0, canRedo: true } },
  { cmd: 'redo', expect: { notes: 1 } },
  { cmd: 'note', contents: 'M9 note three', expect: { notes: 2, recovery: { available: true, entries: 5 } } },
];

test('a hung request is dropped and reported; the edits before it, their ids and undo history survive; the save matches', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const hung = await driveCopy('tpdf-m9-hang-', [
    ...EDITS,
    { cmd: 'note', contents: 'M9 HANG' },
    { cmd: 'wait', ms: WATCHDOG_MS + 3000, expect: {
      doc: { name: 'hello.pdf', pageCount: 1 }, notes: 2, annots: { FreeText: 1, Text: 2 }, dirty: true, canUndo: true, canRedo: false,
      error: 'Adding the annotation took too long, so it was stopped. That change was not applied; your other changes were restored.',
      recovery: { available: true, entries: 5, last: { outcome: 'restored', dropped: 'annot-add', cause: 'slow', lost: 0, steps: 5 } },
    } },
    { cmd: 'undo', expect: { notes: 1, canRedo: true } },
    { cmd: 'redo', expect: { notes: 2, canRedo: false } },
    { cmd: 'save-to', path: '${OUT}/recovered.pdf', expect: { save: { ok: true, name: 'recovered.pdf' } } },
  ]);
  assert.equal(hung.result.ok, true, summary(hung.result));
  assert.equal(hung.code, 0);
  const before = hung.result.steps[5].state;
  const after = hung.result.steps[7].state;
  assert.equal(after.revision, before.revision, 'the restored journal is the one the window showed');
  assert.deepEqual(after.fill.boxes, before.fill.boxes, 'annotation ids and rects survive the restart');
  if (before.pages.ids) assert.deepEqual(after.pages.ids, before.pages.ids, 'page ids survive the restart');

  const control = await driveCopy('tpdf-m9-control-', [
    ...EDITS,
    { cmd: 'undo', expect: { notes: 1 } },
    { cmd: 'redo', expect: { notes: 2 } },
    { cmd: 'save-to', path: '${OUT}/control.pdf', expect: { save: { ok: true, name: 'control.pdf' } } },
  ]);
  assert.equal(control.result.ok, true, summary(control.result));
  const recovered = hung.files['recovered.pdf'];
  const expected = control.files['control.pdf'];
  assert.ok(recovered && expected);
  assert.deepEqual(annotsOf(recovered), annotsOf(expected), 'the same annotations as the same edits without the hang');
  assert.ok(!annotsOf(recovered).some(item => item.contents.includes('HANG')));
  assert.deepEqual(recovered, expected, 'byte-identical to the same edits without the hang');
});

test('a replay that fails reopens the last saved state and says the unsaved changes are gone', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const { code, result, files } = await driveCopy('tpdf-m9-fallback-', [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
    { cmd: 'note', contents: 'M9 one', expect: { notes: 1 } },
    { cmd: 'save-to', path: '${OUT}/fb.pdf', expect: { save: { ok: true, name: 'fb.pdf' } } },
    { cmd: 'open', path: '${OUT}/fb.pdf', expect: { doc: { name: 'fb.pdf' }, notes: 1, dirty: false } },
    { cmd: 'note', contents: 'M9 SLOW-ON-REPLAY two', expect: { notes: 2 } },
    { cmd: 'save-in-place', expect: { save: { ok: true, name: 'fb.pdf' }, dirty: false, recovery: { available: true, saved: true, entries: 2 } } },
    { cmd: 'note', contents: 'M9 three, unsaved', expect: { notes: 3, dirty: true } },
    { cmd: 'note', contents: 'M9 HANG' },
    // The replay hangs on "two": a second restart, then the saved bytes are reopened.
    { cmd: 'wait', ms: 2 * WATCHDOG_MS + 4000, expect: {
      doc: { name: 'fb.pdf', pageCount: 1 }, notes: 2, dirty: false, canUndo: false,
      error: 'Adding the annotation took too long, so it was stopped. Your changes since the last save could not be restored, so the document was reopened as last saved.',
      recovery: { available: true, entries: 0, last: { outcome: 'reopened', dropped: 'annot-add', cause: 'slow' } },
    } },
    // The reopened document is a healthy session: edit and save in place again.
    { cmd: 'note', contents: 'M9 four', expect: { notes: 3, dirty: true } },
    { cmd: 'save-in-place', expect: { save: { ok: true, name: 'fb.pdf' }, dirty: false } },
  ]);
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
  assert.deepEqual(annotsOf(files['fb.pdf']).map(item => item.contents), ['M9 one', 'M9 SLOW-ON-REPLAY two', 'M9 four']);
});

test('a save past its time limit is dropped, never marks the document saved, and is not replayed', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const { code, result, files } = await driveCopy('tpdf-m9-save-', [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
    { cmd: 'note', contents: 'M9 kept', expect: { notes: 1, dirty: true } },
    { cmd: 'save-to', path: '${OUT}/saved.pdf', expect: { save: { ok: false, reason: 'worker' } } },
    { cmd: 'wait', ms: 3000, expect: {
      doc: { name: 'hello.pdf' }, notes: 1, dirty: true,
      error: 'Saving took too long, so it was stopped. Nothing was saved. Your document and changes were restored.',
      recovery: { available: true, entries: 1, last: { outcome: 'restored', dropped: 'save', steps: 1 } },
    } },
    { cmd: 'save-to', path: '${OUT}/saved.pdf', expect: { save: { ok: true, name: 'saved.pdf' }, dirty: true } },
  ], { engine: hangPatch({ hangSave: true }) });
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
  assert.deepEqual(annotsOf(files['saved.pdf']).map(item => item.contents), ['M9 kept']);
});

test('past the recovery budget a restart keeps the old behaviour and says the changes were too large', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const { code, result } = await driveCopy('tpdf-m9-budget-', [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' }, recovery: { available: true } } },
    { cmd: 'note', contents: 'M9 small', expect: { notes: 1, recovery: { available: true, entries: 1 } } },
    { cmd: 'note', contents: `M9 large ${'x'.repeat(3000)}`, expect: { notes: 2, recovery: { available: false, unavailable: 'budget', entries: 0, bytes: 0 } } },
    { cmd: 'note', contents: 'M9 HANG' },
    { cmd: 'wait', ms: WATCHDOG_MS + 1500, expect: {
      doc: null,
      error: 'The PDF worker stopped (worker stopped responding). Restarted it (1 of 3); open the document again to continue. Your unsaved changes could not be kept: they were too large to keep a recovery copy.',
      recovery: { available: false, entries: 0 },
    } },
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' }, notes: 0, error: '', recovery: { available: true } } },
  ], { budget: 4096 });
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
});

test('an unlocked document is restored with its unlock and its page change', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const { code, result } = await driveCopy('tpdf-m9-unlock-', [
    { cmd: 'open', path: resolve('fixtures/encrypted.pdf'), expect: { doc: { name: 'encrypted.pdf', needsPassword: true } } },
    { cmd: 'password', contents: 'pdf4all-user', expect: { doc: { needsPassword: false } } },
    { cmd: 'rotate-right', expect: { pages: { rotate: [90] }, dirty: true, recovery: { entries: 2 } } },
    { cmd: 'note', contents: 'M9 HANG' },
    { cmd: 'wait', ms: WATCHDOG_MS + 3000, expect: {
      doc: { name: 'encrypted.pdf', needsPassword: false }, pages: { rotate: [90] }, dirty: true, notes: 0,
      error: 'Adding the annotation took too long, so it was stopped. That change was not applied; your other changes were restored.',
      recovery: { available: true, entries: 2, last: { outcome: 'restored', steps: 2 } },
    } },
  ]);
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
});

test('drive script: the recovery log over opens, edits, saves and an unlock', { skip: CI_NO_ELECTRON, timeout: 120_000 }, async () => {
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'tpdf-m9-log-')));
  try {
    const run = spawnSync(process.execPath, ['scripts/drive.mjs', 'tests/drive/recovery-log.jsonl', `--out=${dir}`], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
    const report = JSON.parse(run.stdout);
    assert.equal(report.ok, true, summary(report));
    assert.equal(run.status, 0);
    assert.ok(!run.stdout.includes('pdf4all-user'), 'the password is in no STATE');
  } finally {
    await rm(dir, { recursive: true, force: true, maxRetries: 5 });
  }
});

/** Inserts `code` into the built renderer right after the first occurrence of `needle`. */
const after = (needle, code) => source => {
  const at = source.indexOf(needle);
  assert.ok(at >= 0, `renderer patch anchor: ${needle}`);
  return `${source.slice(0, at + needle.length)}${code}${source.slice(at + needle.length)}`;
};

test('an edit queued behind a hung save is applied after the restore, with no second restart', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  // The review's probe A: a note is made 300 ms into a Save a copy whose `save` hangs; it waits at the save's gate.
  const queue = after('setStatus(`Saving a copy of ${name}…`);', ' if (!window.m9Queued) { window.m9Queued = true; window.setTimeout(() => { void smokeNote("M9 queued during the save"); }, 300); }');
  const { code, result, files } = await driveCopy('tpdf-m9-gate-', [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
    { cmd: 'note', contents: 'M9 kept', expect: { notes: 1 } },
    { cmd: 'save-to', path: '${OUT}/queued.pdf', expect: { save: { ok: false, reason: 'worker' } } },
    { cmd: 'wait', ms: 3000, expect: {
      doc: { name: 'hello.pdf' }, notes: 2, dirty: true,
      error: 'Saving took too long, so it was stopped. Nothing was saved. Your document and changes were restored.',
      recovery: { available: true, entries: 2, last: { outcome: 'restored', dropped: 'save', steps: 1, lost: 0 } },
    } },
    // Past another watchdog: nothing hung in the restored worker, so there was no second restart.
    { cmd: 'wait', ms: WATCHDOG_MS + 1500, expect: { notes: 2, error: 'Saving took too long, so it was stopped. Nothing was saved. Your document and changes were restored.', recovery: { entries: 2, last: { dropped: 'save', steps: 1 } } } },
    { cmd: 'save-to', path: '${OUT}/saved.pdf', expect: { save: { ok: true, name: 'saved.pdf' } } },
  ], { engine: hangPatch({ hangSave: true }), renderer: queue });
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
  assert.deepEqual(annotsOf(files['saved.pdf']).map(item => item.contents).sort(), ['M9 kept', 'M9 queued during the save']);
});

test('a page whose drawing hangs is restored blank and not drawn again until the document changes', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const notice = 'Showing the page took too long, so it was stopped. Your document and changes were restored. Page 1 was not drawn again; edit the document or open it again to try again.';
  const { code, result } = await driveCopy('tpdf-m9-page-', [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
    { cmd: 'note', contents: 'M9 kept one', expect: { notes: 1 } },
    { cmd: 'note', contents: 'M9 kept two', expect: { notes: 2 } },
    { cmd: 'note', contents: 'M9 HANGPAGE' },
    { cmd: 'wait', ms: WATCHDOG_MS + 3000, expect: {
      doc: { name: 'hello.pdf' }, notes: 3, dirty: true, error: notice,
      status: 'Page 1 is not shown: drawing it took too long. Edit the document or open it again to try again.',
      recovery: { available: true, entries: 3, paused: ['render:0', 'thumbnail:0'], last: { outcome: 'restored', dropped: 'render', steps: 3 } },
    } },
    // No loop: two more watchdog periods later the same restore is the last one.
    { cmd: 'wait', ms: 2 * WATCHDOG_MS + 1000, expect: { doc: { name: 'hello.pdf' }, notes: 3, error: notice, recovery: { last: { dropped: 'render', steps: 3 } } } },
    // Undo changes the document: the page is drawn again, and draws.
    { cmd: 'undo', expect: { notes: 2, recovery: { paused: [] } } },
    { cmd: 'wait', ms: 1000, expect: { doc: { name: 'hello.pdf' }, notes: 2, recovery: { last: { dropped: 'render', steps: 3 } } } },
  ]);
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
  assert.match(result.steps.at(-1).state.status, /^Page 1 of 1 — rendered/);
});

test('a read queued behind a long job does not take the blame for it', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  // The review's minor 7: an annotation list is requested 300 ms into an insert-pages that hangs.
  const queue = after('async function insertPages(source, at, range) {', ' window.setTimeout(() => { void refreshNotes(); }, 300);');
  const { code, result } = await driveCopy('tpdf-m9-blame-', [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
    { cmd: 'note', contents: 'M9 kept', expect: { notes: 1 } },
    { cmd: 'insert', from: hello, contents: { after: 1, range: '1' } },
    { cmd: 'wait', ms: 4000, expect: {
      doc: { name: 'hello.pdf', pageCount: 1 }, notes: 1, pages: { count: 1 },
      error: 'Inserting the pages took too long, so it was stopped. That change was not applied; your other changes were restored.',
      recovery: { last: { outcome: 'restored', dropped: 'insert-pages', steps: 1 } },
    } },
  ], { engine: hangPatch({ hangInsert: true }), renderer: queue, saveTimeoutMs: 2 * WATCHDOG_MS });
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
});

test('a save in place written during a failed replay is what the window reopens, and main hears dirty meanwhile', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  // The review's minor 3: a note hangs while main writes a save in place; the replay then hangs too.
  const hangDuringWrite = after('const diskCopy = validated.bytes.slice(0);', ' void smokeNote("M9 HANG during the write");');
  const { code, result, files } = await driveCopy('tpdf-m9-write-', [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
    { cmd: 'note', contents: 'M9 one', expect: { notes: 1 } },
    { cmd: 'save-to', path: '${OUT}/fb.pdf', expect: { save: { ok: true, name: 'fb.pdf' } } },
    { cmd: 'open', path: '${OUT}/fb.pdf', expect: { doc: { name: 'fb.pdf' }, notes: 1, dirty: false } },
    { cmd: 'note', contents: 'M9 SLOW-ON-REPLAY two', expect: { notes: 2, dirty: true } },
    { cmd: 'save-in-place', expect: { save: { ok: false, written: true } } },
    // The replay is hung on "two": the window shows nothing yet, and main still hears unsaved edits.
    { cmd: 'wait', ms: 1000, expect: { doc: null, recovery: { restoring: true, dirtyReported: true } } },
    { cmd: 'wait', ms: WATCHDOG_MS + 3000, expect: {
      doc: { name: 'fb.pdf' }, notes: 2, dirty: false,
      error: 'Adding the annotation took too long, so it was stopped. Your changes since the last save could not be restored, so the document was reopened as last saved.',
      recovery: { available: true, entries: 0, restoring: false, dirtyReported: false, last: { outcome: 'reopened', dropped: 'annot-add' } },
    } },
  ], { renderer: hangDuringWrite });
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
  assert.deepEqual(annotsOf(files['fb.pdf']).map(item => item.contents), ['M9 one', 'M9 SLOW-ON-REPLAY two']);
});

test('an engine trap during the replay restarts first, then reopens the opened bytes', { skip: CI_NO_ELECTRON || (ENGINE !== 'rust' && 'MuPDF build'), timeout: 180_000 }, async () => {
  const { code, result } = await driveCopy('tpdf-m9-trap-', [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
    { cmd: 'note', contents: 'M9 TRAP-ON-REPLAY one', expect: { notes: 1 } },
    { cmd: 'note', contents: 'M9 HANG' },
    { cmd: 'wait', ms: WATCHDOG_MS + 3000, expect: {
      doc: { name: 'hello.pdf' }, notes: 0, dirty: false,
      error: 'Adding the annotation took too long, so it was stopped. Your changes since the last save could not be restored, so the document was reopened as it was opened.',
      recovery: { available: true, last: { outcome: 'reopened', dropped: 'annot-add' } },
    } },
  ], { engine: hangPatch({ trapOnReplay: true }) });
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
});

test('an edit made while main writes a save in place stays unsaved: the window is dirty, close asks, the next save has it', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  // main's write takes 1.5 s; 300 ms into it the renderer adds a note. Each STATE step also
  // carries main's own dirty flag, which is what its close prompt reads.
  const slowWrite = source => after("ipcMain.handle('save-file', async (event, raw, documentUrl, options) => {", ' await new Promise(resolve => setTimeout(resolve, 1500));')(source)
    .replace('step.state = last;', 'step.state = last; step.mainDirty = dirtyWindows.has(contents.id);');
  const editDuringWrite = after('const diskCopy = validated.bytes.slice(0);', ' if (!window.m9Edited) { window.m9Edited = true; window.setTimeout(() => { void smokeNote("M9 made during the write"); }, 300); }');
  const { code, result, files } = await driveCopy('tpdf-m9-ticket-', [
    { cmd: 'open', path: hello, expect: { doc: { name: 'hello.pdf' } } },
    { cmd: 'save-to', path: '${OUT}/fb.pdf', expect: { save: { ok: true, name: 'fb.pdf' } } },
    { cmd: 'open', path: '${OUT}/fb.pdf', expect: { doc: { name: 'fb.pdf' }, notes: 0, dirty: false } },
    { cmd: 'note', contents: 'M9 saved one', expect: { notes: 1, dirty: true } },
    { cmd: 'save-in-place', expect: { save: { ok: true, name: 'fb.pdf' }, notes: 2, dirty: true, canUndo: true, recovery: { dirtyReported: true } } },
    { cmd: 'wait', ms: 200, expect: { dirty: true } },
    // Undoing the late edit reaches the state on disk: clean.
    { cmd: 'undo', expect: { notes: 1, dirty: false } },
    { cmd: 'redo', expect: { notes: 2, dirty: true } },
    { cmd: 'save-in-place', expect: { save: { ok: true, name: 'fb.pdf' }, dirty: false } },
    { cmd: 'open', path: '${OUT}/fb.pdf', expect: { doc: { name: 'fb.pdf' }, notes: 2, dirty: false } },
  ], { main: slowWrite, renderer: editDuringWrite });
  assert.equal(result.ok, true, summary(result));
  assert.equal(code, 0);
  assert.equal(result.steps[4].mainDirty, true, 'main still holds the window dirty: a close asks');
  assert.equal(result.steps[5].mainDirty, true);
  assert.equal(result.steps[6].mainDirty, false);
  assert.equal(result.steps[8].mainDirty, false);
  assert.deepEqual(annotsOf(files['fb.pdf']).map(item => item.contents).sort(), ['M9 made during the write', 'M9 saved one']);
});
