/**
 * Phase 1 UX quick wins, checked in the real window over the Chrome DevTools protocol: what the drive
 * protocol cannot see (computed styles, real key presses on the page, the Sign chooser). One Electron
 * at a time; the session is a stdin drive (`--drive=-`), so it starts on hello.pdf and keeps nothing.
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = fileURLToPath(new URL('..', import.meta.url));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

/** A drive session with a DevTools port: `step(line)` sends a drive line and resolves with its STATE; `js(expr)` evaluates in the page. */
async function session() {
  const electron = createRequire(import.meta.url)('electron');
  const port = await new Promise(resolve => { const server = createServer(); server.listen(0, '127.0.0.1', () => { const { port } = server.address(); server.close(() => resolve(port)); }); });
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-p1-gui-'));
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${join(dir, 'udata')}`, '.', '--drive=-', `--drive-out=${dir}`], { cwd: root, stdio: ['pipe', 'pipe', 'pipe'] });
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
  const key = async (name, code) => { for (const type of ['keyDown', 'keyUp']) await send('Input.dispatchKeyEvent', { type, key: name, code: name, windowsVirtualKeyCode: code }); await sleep(150); };
  const text = async value => { await send('Input.insertText', { text: value }); await sleep(100); };
  // Wait for the startup sample before the first step.
  for (let i = 0; i < 100 && !(await js('document.title').catch(() => '')).startsWith('hello.pdf'); i++) await sleep(100);
  const close = async () => { ws.close(); child.stdin.end(); child.kill('SIGTERM'); await new Promise(resolve => child.once('exit', resolve)); await rm(dir, { recursive: true, force: true, maxRetries: 5 }); };
  return { step, js, key, text, close, dir };
}

test('p1-ux GUI: chrome is not selectable but content is; Esc keeps a typed note; the inspector Sign opens the chooser', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const s = await session();
  try {
    // Review fix 4 (X12): user-select none only on the app's own controls.
    const select = await s.js(`Object.fromEntries(['#open', '.toolbar', '.tabs', '.mode', '#thumbs', '#note-list', '#verify-list', '#help-matrix', '#banner-text', '#status', '#link-list', '#error', '#text-layer']
      .map(q => [q, getComputedStyle(document.querySelector(q)).userSelect]))`);
    for (const q of ['#open', '.toolbar', '.tabs', '.mode', '#thumbs']) assert.equal(select[q], 'none', `${q}: ${JSON.stringify(select)}`);
    for (const q of ['#note-list', '#verify-list', '#help-matrix', '#banner-text', '#status', '#link-list', '#error', '#text-layer']) assert.notEqual(select[q], 'none', `${q}: ${JSON.stringify(select)}`);
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
