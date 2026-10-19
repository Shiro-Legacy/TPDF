/**
 * Phase 1 UX quick wins, checked in the real window over the Chrome DevTools protocol: what the drive
 * protocol cannot see (computed styles, real key presses on the page, the Sign chooser). One Electron
 * at a time; the session is a stdin drive (`--drive=-`), so it starts on hello.pdf and keeps nothing.
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import mupdf from 'mupdf';
import { createRequire } from 'node:module';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { electronOnly } from './app-bin.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

/**
 * A window with a DevTools port: `step(line)` sends a drive line and resolves with its STATE; `js(expr)` evaluates
 * in the page. `normal: true` starts the app as a person does (no drive: the start screen) with `files` written
 * into its userData folder first.
 */
async function session({ normal = false, files = {} } = {}) {
  const electron = createRequire(import.meta.url)('electron');
  const port = await new Promise(resolve => { const server = createServer(); server.listen(0, '127.0.0.1', () => { const { port } = server.address(); server.close(() => resolve(port)); }); });
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-p1-gui-'));
  await mkdir(join(dir, 'udata'), { recursive: true });
  for (const [name, body] of Object.entries(files)) await writeFile(join(dir, 'udata', name), body);
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${join(dir, 'udata')}`, '.', ...(normal ? [] : ['--drive=-', `--drive-out=${dir}`])], { cwd: root, stdio: ['pipe', 'pipe', 'pipe'] });
  const waiting = [];
  let buffer = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => {
    buffer += chunk;
    for (let at = buffer.indexOf('\n'); at >= 0; at = buffer.indexOf('\n')) {
      const line = buffer.slice(0, at); buffer = buffer.slice(at + 1);
      if (line.startsWith('STEP ')) waiting.shift()?.(JSON.parse(line.slice(5)));
    }
  });
  let target;
  for (let i = 0; i < 150 && !target; i++) {
    try { target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(item => item.url.startsWith('app://pdf4all/index.html')); } catch { /* not up yet */ }
    if (!target) await sleep(200);
  }
  assert.ok(target, 'no DevTools target');
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let n = 0;
  const pending = new Map();
  ws.onmessage = event => { const msg = JSON.parse(String(event.data)); const p = pending.get(msg.id); if (p) { pending.delete(msg.id); if (msg.error) p.reject(new Error(msg.error.message)); else p.resolve(msg.result); } };
  const send = (method, params = {}) => new Promise((resolve, reject) => { const id = ++n; pending.set(id, { resolve, reject }); ws.send(JSON.stringify({ id, method, params })); });
  const step = async line => { const got = new Promise(resolve => waiting.push(resolve)); child.stdin.write(`${JSON.stringify(line)}\n`); return (await got).state; };
  const js = async expression => {
    const reply = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (reply.exceptionDetails) throw new Error(JSON.stringify(reply.exceptionDetails).slice(0, 300));
    return reply.result.value;
  };
  const key = async (name, code, modifiers = 0, pause = 150) => { for (const type of ['keyDown', 'keyUp']) await send('Input.dispatchKeyEvent', { type, key: name, code: name, windowsVirtualKeyCode: code, modifiers }); await sleep(pause); };
  const text = async value => { await send('Input.insertText', { text: value }); await sleep(100); };
  const mouse = async (type, x, y, buttons = type === 'mouseReleased' ? 0 : 1) => { await send('Input.dispatchMouseEvent', { type, x, y, button: buttons || type !== 'mouseMoved' ? 'left' : 'none', buttons, clickCount: 1 }); await sleep(60); };
  // Wait for the startup sample (or the start screen) before the first step.
  const ready = normal ? 'document.getElementById("status").textContent === "No document open"' : 'document.title.startsWith("hello.pdf")';
  for (let i = 0; i < 100 && !(await js(ready).catch(() => false)); i++) await sleep(100);
  const close = async () => { ws.close(); child.stdin.end(); child.kill('SIGTERM'); await new Promise(resolve => child.once('exit', resolve)); await rm(dir, { recursive: true, force: true, maxRetries: 5 }); };
  return { step, js, key, text, mouse, close, dir };
}

test('p1-ux GUI: chrome is not selectable but content is; Esc keeps a typed note; the inspector Sign opens the chooser', { skip: electronOnly, timeout: 180_000 }, async () => {
  const s = await session();
  try {
    // Review fix 4 (X12): user-select none only on the app's own controls.
    const select = await s.js(`Object.fromEntries(['#open', '.toolbar', '.tabs', '.mode', '#thumbs', '#note-list', '#verify-list', '#help-matrix', '#banner-text', '#status', '#link-list', '#error', '#text-layer', '#search-status']
      .map(q => [q, getComputedStyle(document.querySelector(q)).userSelect]))`);
    for (const q of ['#open', '.toolbar', '.tabs', '.mode', '#thumbs']) assert.equal(select[q], 'none', `${q}: ${JSON.stringify(select)}`);
    for (const q of ['#note-list', '#verify-list', '#help-matrix', '#banner-text', '#status', '#link-list', '#error', '#text-layer', '#search-status']) assert.notEqual(select[q], 'none', `${q}: ${JSON.stringify(select)}`);
    // Review fix 10: the real Escape behaves as the drive's: with the Add text tool armed and the text just
    // added focused, one Escape stops the tool (p1-ux.jsonl asserts the same of the drive key).
    await s.step({ cmd: 'new' });
    await s.step({ cmd: 'mode', contents: { mode: 'edit', tool: 'add-text' } });
    let state = await s.step({ cmd: 'click', contents: { page: 0, x: 300, y: 400 } });
    assert.equal(state.direct.active?.kind, 'add');
    await s.js('document.querySelector(".add-text").focus(), 1');
    await s.text('Typed for real');
    state = await s.step({ cmd: 'key', contents: { key: 'Meta+Enter' } });
    assert.equal(state.revision, 1);
    assert.equal(state.tool, 'Adding text: click empty space on the page');
    await s.key('Escape', 27);
    state = await s.step({ cmd: 'state' });
    assert.equal(state.tool, null, 'one real Escape stopped the tool');

    // Review fix 8: a real Escape keeps a note that has text, and discards an empty one.
    await s.step({ cmd: 'mode', contents: { mode: 'annotate' } });
    await s.step({ cmd: 'arm', contents: { tool: 'note' } });
    state = await s.step({ cmd: 'click', contents: { page: 0, x: 300, y: 400 } });
    assert.equal(state.direct.active?.kind, 'note');
    await s.js('document.querySelector(".note-inline-editor").focus(), 1');
    await s.text('Typed, then Esc');
    await s.key('Escape', 27);
    state = await s.step({ cmd: 'state' });
    assert.equal(state.annots.Text, 1, JSON.stringify(state.direct));
    assert.equal(state.comments[0].contents, 'Typed, then Esc');
    await s.step({ cmd: 'arm', contents: { tool: 'note' } });
    await s.step({ cmd: 'click', contents: { page: 0, x: 300, y: 500 } });
    await s.js('document.querySelector(".note-inline-editor").focus(), 1');
    await s.key('Escape', 27);
    state = await s.step({ cmd: 'state' });
    assert.equal(state.annots.Text, 1, 'the empty note was discarded');
    assert.equal(state.direct.active, null);
    // Review fix 9: the inspector's Sign… opens the one chooser; there is one Add text box (the Fill forms tool).
    await s.step({ cmd: 'mode', contents: { mode: 'forms' } });
    assert.equal(await s.js('document.querySelectorAll("#fill-textbox-add").length'), 0);
    assert.equal(await s.js('[...document.querySelectorAll("button")].filter(b => b.textContent.trim() === "Add text box" && b.closest("#context-forms")).length'), 1);
    await s.js('document.getElementById("fill-sign").click(), 1');
    assert.equal(await s.js('document.getElementById("sign-choice-dialog").open'), true);
    await s.js('document.getElementById("sign-choice-certificate").click(), 1');
    state = await s.step({ cmd: 'state' });
    assert.deepEqual(state.sign.certificateDialog, { ready: false, label: 'Sign and save as…' });
    await s.js('document.getElementById("cert-sign-cancel").click(), 1');
  } finally { await s.close(); }
});

test('p1-ux GUI: the start screen lists recent files, redraws after a missing one, and clears them', { skip: electronOnly, timeout: 120_000 }, async () => {
  const listed = [join(root, 'fixtures/nav.pdf'), join(root, 'fixtures/no-such-file.pdf'), join(root, 'fixtures/form.pdf')];
  const s = await session({ normal: true, files: { 'recent-files.json': JSON.stringify(listed) } });
  try {
    const names = () => s.js('[...document.querySelectorAll("#start-recent-list button")].map(b => b.firstChild.textContent)');
    assert.deepEqual(await names(), ['nav.pdf', 'no-such-file.pdf', 'form.pdf']);
    // Review fix 3: the missing file says so, leaves the list, and the list on screen is redrawn.
    await s.js('document.querySelectorAll("#start-recent-list button")[1].click(), 1');
    for (let i = 0; i < 50 && (await names()).length === 3; i++) await sleep(100);
    assert.deepEqual(await names(), ['nav.pdf', 'form.pdf']);
    assert.match(await s.js('document.getElementById("error").textContent'), /could not be opened/);
    // Its neighbour still opens its own file (an index would now name form.pdf).
    await s.js('document.querySelectorAll("#start-recent-list button")[1].click(), 1');
    for (let i = 0; i < 50 && !(await s.js('document.title')).startsWith('form.pdf'); i++) await sleep(100);
    assert.match(await s.js('document.title'), /^form\.pdf/);
  } finally { await s.close(); }
  // Review fix 15: Clear recent files empties the list (and the section hides).
  const again = await session({ normal: true, files: { 'recent-files.json': JSON.stringify([join(root, 'fixtures/nav.pdf')]) } });
  try {
    assert.equal(await again.js('document.getElementById("start-recent").hidden'), false);
    await again.js('document.getElementById("start-recent-clear").click(), 1');
    for (let i = 0; i < 50 && !(await again.js('document.getElementById("start-recent").hidden')); i++) await sleep(100);
    assert.equal(await again.js('document.querySelectorAll("#start-recent-list button").length'), 0);
  } finally { await again.close(); }
});

test('p1-ux GUI: a signed file\'s notice says it opens for reading', { skip: electronOnly, timeout: 120_000 }, async () => {
  const s = await session();
  try {
    await s.step({ cmd: 'open', path: join(root, 'fixtures/p0b/signed-intact.pdf') });
    assert.equal(await s.js('document.getElementById("banner-text").textContent'),
      'This file is signed. Editing it would break the signature, so it opens for reading. Use Verify to check the signatures.');
  } finally { await s.close(); }
});

test('p1-ux GUI: Escape during a text drag cancels the drag, even with a tool armed (re-review fix 2)', { skip: electronOnly, timeout: 120_000 }, async () => {
  const s = await session();
  try {
    await s.step({ cmd: 'new' });
    let state = await s.step({ cmd: 'text-add', contents: { page: 0, point: [72, 120], text: 'Drag me', font: 'Helvetica', size: 18, color: [0, 0, 0] } });
    assert.equal(state.revision, 1);
    state = await s.step({ cmd: 'mode', contents: { mode: 'edit', tool: 'add-text' } });
    assert.equal(state.tool, 'Adding text: click empty space on the page', JSON.stringify({ mode: state.mode, rev: state.revision, err: state.error, doc: state.doc }));
    // Hover the run so its handles show (gotchas: hover before reading handle rects), then read the grip.
    const centre = await s.js(`(() => { const target = [...document.querySelectorAll('#hit-layer .text-target')].find(node => node.querySelector('.hit-grip'));
      const box = target.getBoundingClientRect(); return [box.left + box.width / 2, box.top + box.height / 2]; })()`);
    await s.mouse('mouseMoved', centre[0], centre[1], 0);
    const grip = await s.js(`(() => { const box = [...document.querySelectorAll('#hit-layer .text-target .hit-grip')].map(node => node.getBoundingClientRect()).find(rect => rect.width > 0);
      return [box.left + box.width / 2, box.top + box.height / 2]; })()`);
    const hit = await s.js(`document.elementFromPoint(${grip[0]}, ${grip[1]})?.className ?? ''`);
    assert.match(String(hit), /hit-grip/, `the press lands on the grip: ${hit} ${grip}`);
    await s.mouse('mousePressed', grip[0], grip[1]);
    for (let i = 1; i <= 6; i++) await s.mouse('mouseMoved', grip[0] + i * 10, grip[1] + i * 5);
    await s.key('Escape', 27);
    await s.mouse('mouseReleased', grip[0] + 60, grip[1] + 30);
    await sleep(400);
    state = await s.step({ cmd: 'state' });
    assert.equal(state.revision, 1, 'Escape cancelled the drag: nothing moved');
    assert.equal(state.tool, 'Adding text: click empty space on the page', 'the drag took the Escape, not the tool');
    assert.equal(state.direct.active, null, 'releasing the cancelled drag is not a click: no add-text editor opened');
    // The pressed run is selected: Escape peels the selection first, then stops the tool.
    if (state.direct.selection.length) {
      await s.key('Escape', 27);
      state = await s.step({ cmd: 'state' });
      assert.deepEqual(state.direct.selection, []);
      assert.equal(state.tool, 'Adding text: click empty space on the page');
    }
    // An inspector form (and a dialog, the compare view) takes its own Escape: the tool stays armed.
    assert.deepEqual(state.direct.selection, []);
    const escapeAt = id => s.js(`document.getElementById('${id}').dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })), 1`);
    for (const id of ['content-new-text', 'content-box-x']) {
      await escapeAt(id);
      state = await s.step({ cmd: 'state' });
      assert.equal(state.tool, 'Adding text: click empty space on the page', id);
    }
    // Final check 2: inputs that do not handle Escape themselves (the field designer, find and replace, the
    // compare view's page number) still let Escape stop the tool.
    for (const id of ['field-new-name', 'content-find', 'compare-page-b']) {
      await escapeAt(id);
      state = await s.step({ cmd: 'state' });
      assert.equal(state.tool, null, id);
      state = await s.step({ cmd: 'arm', contents: { tool: 'add-text' } });
      assert.equal(state.tool, 'Adding text: click empty space on the page');
    }
    await s.key('Escape', 27);
    state = await s.step({ cmd: 'state' });
    assert.equal(state.tool, null, 'the next Escape stops the tool');
  } finally { await s.close(); }
});

test('p1-ux GUI: Save right after ⌘Enter waits for that commit (re-review fix 3)', { skip: electronOnly, timeout: 120_000 }, async () => {
  const s = await session();
  try {
    await s.step({ cmd: 'new' });
    const path = join(s.dir, 'commit-then-save.pdf');
    await s.step({ cmd: 'save-to', path });
    let state = await s.step({ cmd: 'open', path });
    assert.equal(state.doc.name, 'commit-then-save.pdf');
    await s.step({ cmd: 'mode', contents: { mode: 'edit', tool: 'add-text' } });
    state = await s.step({ cmd: 'click', contents: { page: 0, x: 100, y: 200 } });
    assert.equal(state.direct.active?.kind, 'add');
    await s.js('document.querySelector(".add-text").focus(), 1');
    await s.text('Committed before the save');
    // The real ⌘Enter starts the commit; Save is sent at once, while the commit is still in flight.
    const save = s.key('Enter', 13, 4, 0).then(() => s.step({ cmd: 'save-in-place' }));
    state = await save;
    assert.equal(state.save?.ok, true, JSON.stringify(state.save));
    assert.equal(state.dirty, false);
    const text = mupdf.Document.openDocument(await readFile(path), 'application/pdf').loadPage(0).toStructuredText().asText();
    assert.match(text, /Committed before the save/);
  } finally { await s.close(); }
});

test('p1-ux GUI: an open that arrives while another is asking is answered "cancelled" (re-review fix 4)', { skip: electronOnly, timeout: 120_000 }, async () => {
  const s = await session({ normal: true, files: { 'recent-files.json': JSON.stringify([join(root, 'fixtures/nav.pdf')]) } });
  try {
    await s.js('window.__openErrors = [], window.pdf4all.onOpenError(error => window.__openErrors.push(error)), 1');
    // The Open dialog stays up (nothing answers it); a recent file opened meanwhile must not open silently later.
    await s.js('void window.pdf4all.openFile(), 1');
    await sleep(500);
    await s.js('document.querySelector("#start-recent-list button").click(), 1');
    for (let i = 0; i < 30 && !(await s.js('window.__openErrors.length')); i++) await sleep(100);
    assert.deepEqual(await s.js('window.__openErrors'), [{ reason: 'cancelled' }]);
    assert.equal(await s.js('document.title'), 'Tandem PDF', 'nothing opened');
  } finally { await s.close(); }
});

test('p1-ux GUI: the toolbar Save clicked during a ⌘Enter commit waits for it (final check 1)', { skip: electronOnly, timeout: 120_000 }, async () => {
  const s = await session();
  try {
    await s.step({ cmd: 'new' });
    const path = join(s.dir, 'button-save.pdf');
    await s.step({ cmd: 'save-to', path });
    await s.step({ cmd: 'open', path });
    let state = await s.step({ cmd: 'note', contents: 'already edited' });
    assert.equal(state.dirty, true, 'Save is enabled');
    await s.step({ cmd: 'mode', contents: { mode: 'edit', tool: 'add-text' } });
    state = await s.step({ cmd: 'click', contents: { page: 0, x: 100, y: 300 } });
    assert.equal(state.direct.active?.kind, 'add');
    await s.js('document.querySelector(".add-text").focus(), 1');
    await s.text('Kept by the Save button');
    const save = await s.js('(() => { const box = document.getElementById("save").getBoundingClientRect(); return [box.left + box.width / 2, box.top + box.height / 2]; })()');
    // ⌘Enter starts the commit; the real click on Save (which also blurs the editor) follows at once.
    await s.key('Enter', 13, 4, 0);
    await s.mouse('mousePressed', save[0], save[1]);
    await s.mouse('mouseReleased', save[0], save[1]);
    for (let i = 0; i < 100; i++) { state = await s.step({ cmd: 'state' }); if (state.save?.ok && !state.dirty) break; await sleep(100); }
    assert.equal(state.save?.ok, true, JSON.stringify(state.save));
    const text = mupdf.Document.openDocument(await readFile(path), 'application/pdf').loadPage(0).toStructuredText().asText();
    assert.match(text, /Kept by the Save button/);
  } finally { await s.close(); }
});
