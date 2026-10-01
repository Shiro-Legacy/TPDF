#!/usr/bin/env node
/**
 * fill-anywhere GUI evidence (contract-fill-anywhere.md D9), template shot-direct3.mjs: real CDP mouse and keys on
 * fixtures/hello.pdf in Fill forms — a click on empty page space, typed text, Enter, then a real drag of the new
 * text box. Nothing is saved (the fixture is only opened). Screenshots → review-out/fill-anywhere-*.png.
 * Not part of npm test. Run after `npm run build`, under .quad/run.lock: node .quad/shared/p6/checks/shot-fill-anywhere.mjs
 */
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../../../../', import.meta.url));
const checksDir = fileURLToPath(new URL('./', import.meta.url));
const out = join(checksDir, 'review-out');
const udata = join(out, 'fill-anywhere-udata');
const electron = createRequire(join(root, 'package.json'))('electron');
const T0 = Date.now();
const sleep = ms => new Promise(r => setTimeout(r, ms));
const log = (...a) => console.log(...a);
const q = s => JSON.stringify(s);
function freePort() {
  return new Promise((resolve, reject) => {
    const s = createServer();
    s.listen(0, '127.0.0.1', () => { const port = s.address().port; s.close(err => err ? reject(err) : resolve(port)); });
    s.on('error', reject);
  });
}
async function wait(fn, ms = 25_000, step = 200) {
  const t0 = Date.now();
  let last;
  while (Date.now() - t0 < ms) {
    try { last = await fn(); } catch (err) { last = err; }
    if (last && last !== false && !(last instanceof Error)) return last;
    await sleep(step);
  }
  throw new Error(`timeout: ${last instanceof Error ? last.message : last}`);
}
class Cdp {
  constructor(url) { this.url = url; this.n = 0; this.pending = new Map(); }
  async connect() {
    this.ws = new WebSocket(this.url);
    await new Promise((resolve, reject) => {
      this.ws.addEventListener('open', resolve, { once: true });
      this.ws.addEventListener('error', reject, { once: true });
    });
    this.ws.addEventListener('message', ev => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) reject(new Error(msg.error.message));
        else resolve(msg.result);
      }
    });
  }
  send(method, params = {}) {
    const id = ++this.n;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`cdp timeout ${method}`)); }, 15_000);
      this.pending.set(id, { resolve: v => { clearTimeout(timer); resolve(v); }, reject: e => { clearTimeout(timer); reject(e); } });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }
  async eval(expression) {
    const r = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text || expression);
    return r.result?.value;
  }
  close() { try { this.ws.close(); } catch { /* already closed */ } }
}
async function attach(port) {
  const page = await wait(async () => {
    const res = await fetch(`http://127.0.0.1:${port}/json/list`);
    const list = await res.json();
    return list.find(t => t.webSocketDebuggerUrl && (t.type === 'page' || /index\.html/.test(t.url || '')));
  });
  const cdp = new Cdp(page.webSocketDebuggerUrl);
  await cdp.connect();
  await cdp.send('Runtime.enable');
  await cdp.send('Page.enable');
  return cdp;
}
const hold = (n, ms) => Array.from({ length: n }, () => ({ cmd: 'wait', ms }));
const near = (got, want, tol, label) => {
  if (!Array.isArray(got) || got.length !== want.length || got.some((v, i) => Math.abs(v - want[i]) > tol)) throw new Error(`${label}: ${q(got)} vs ${q(want)} ±${tol}`);
  log('ok', label, q(got));
};
const hashes = () => execFileSync('shasum', ['-a', '256', '-c', join(checksDir, 'fixtures.sha256')], { cwd: root, encoding: 'utf8' });

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
await rm(udata, { recursive: true, force: true });
const probe = join(out, 'fill-anywhere-probe.jsonl');
await writeFile(probe, [
  { cmd: 'open', path: join(root, 'fixtures/hello.pdf') },
  ...hold(40, 5000),
].map(l => JSON.stringify(l)).join('\n') + '\n');
const port = await freePort();
const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
  `--drive=${probe}`, '--gui', `--drive-out=${join(out, 'fill-anywhere-out')}`, `--report=${join(out, 'fill-anywhere-report.json')}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
const closed = new Promise(done => child.on('close', done));
let cdp;
try {
  cdp = await attach(port);
  try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch { /* headless window manager */ }
  const state = () => cdp.eval('window.pdf4allState()');
  const shot = async name => {
    const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
    await writeFile(join(out, name), Buffer.from(data, 'base64'));
    log('shot', name, Date.now() - T0);
  };
  const move = (x, y) => cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
  const click = async (x, y) => {
    await move(x, y); await sleep(100);
    await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
    await sleep(600);
  };
  const drag = async (x, y, dx, dy) => {
    await move(x, y); await sleep(150);
    await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
    for (let i = 1; i <= 8; i++) await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: x + dx * i / 8, y: y + dy * i / 8, button: 'left', buttons: 1 });
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: x + dx, y: y + dy, button: 'left', buttons: 0, clickCount: 1 });
    await sleep(1500);
  };
  const pagePoint = (x, y) => cdp.eval(`(() => {
    const v = document.getElementById('viewport'), c = document.getElementById('canvas');
    let r = c.getBoundingClientRect(); const s = r.width / 612;
    v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
    r = c.getBoundingClientRect();
    return { x: r.left + ${x} * s, y: r.top + ${y} * s, s };
  })()`);
  const key = async (k, code, vk) => {
    await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: k, code, windowsVirtualKeyCode: vk });
    await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: vk });
  };

  await wait(() => cdp.eval(`document.title.includes('hello')`), 60_000);
  await sleep(800);
  await cdp.eval(`document.getElementById('mode-forms').click()`);
  await wait(async () => (await state()).mode === 'forms', 20_000);

  // A. Real click on empty page space (72, 300) → text-box editor; real text; Enter adds one fitted FreeText.
  const empty = await pagePoint(72, 300);
  await click(empty.x, empty.y);
  const active = await wait(async () => { const a = (await state()).direct.active; return a?.kind === 'textbox' ? a : null; });
  near(active.point, [72, 300], 1, 'A editor at the click point');
  await cdp.send('Input.insertText', { text: 'Jane Doe' });
  await sleep(300);
  await shot('fill-anywhere-a0-editor.png');
  await key('Enter', 'Enter', 13);
  const added = await wait(async () => { const s = await state(); return s.fill.boxes.length === 1 && s.direct.active === null ? s : null; });
  const box = added.fill.boxes[0];
  if (Math.abs(box.rect[0] - 72) > 1 || Math.abs(box.rect[1] + 7.2 - 300) > 1 || !added.fill.saveAsCopy) throw new Error(`A box: ${q(added.fill)}`);
  log('ok A text box added', q(box.rect));
  await shot('fill-anywhere-a-added.png');

  // B. Real drag of the box by (100, 50) pt → one annot-move.
  const scale = empty.s;
  const target = await wait(() => cdp.eval(`(() => { const e = document.querySelector('#hit-layer .textbox-target'); if (!e) return null; const r = e.getBoundingClientRect(); return { cx: r.x + r.width / 2, cy: r.y + r.height / 2 }; })()`));
  await drag(target.cx, target.cy, 100 * scale, 50 * scale);
  const moved = await wait(async () => { const s = await state(); return s.fill.last?.op === 'move' ? s.fill.boxes[0].rect : null; });
  near(moved.slice(0, 2), [box.rect[0] + 100, box.rect[1] + 50], 1.5, 'B box moved by a real drag');
  await shot('fill-anywhere-b-moved.png');
  log('state', q((await state()).fill));
  log('PASS');
} catch (err) {
  console.error('FAIL', err?.stack ?? err);
  process.exitCode = 1;
} finally {
  cdp?.close();
  child.kill('SIGTERM');
  log('electron closed', await closed);
  log('fixtures after\n' + hashes());
}
