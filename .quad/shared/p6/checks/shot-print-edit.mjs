#!/usr/bin/env node
/**
 * print-edit GUI evidence (contract-print-edit.md D10): real CDP mouse gestures on fixtures/p6/quartz.pdf.
 * Click the "Postal code" glyph line, type "Post code", Enter; a real grip drag of "Floor 2" by (20, 10) pt.
 * Screenshots → review-out/print-edit-*.png. Not part of npm test. Written, not run (the human was at the machine).
 * Run after `npm run build`, under .quad/run.lock, when nobody is at the machine: node .quad/shared/p6/checks/shot-print-edit.mjs
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
const udata = join(out, 'print-edit-udata');
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
const probe = join(out, 'print-edit-probe.jsonl');
await writeFile(probe, [
  { cmd: 'open', path: join(root, 'fixtures/p6/quartz.pdf') },
  ...hold(40, 5000),
].map(l => JSON.stringify(l)).join('\n') + '\n');
const port = await freePort();
const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
  `--drive=${probe}`, '--gui', `--drive-out=${join(out, 'print-edit-out')}`, `--report=${join(out, 'print-edit-report.json')}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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
  };
  const drag = async (x, y, dx, dy) => {
    await move(x, y); await sleep(150);
    await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
    for (let i = 1; i <= 8; i++) await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: x + dx * i / 8, y: y + dy * i / 8, button: 'left', buttons: 1 });
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: x + dx, y: y + dy, button: 'left', buttons: 0, clickCount: 1 });
  };
  const rectOf = sel => cdp.eval(`(() => { const e = ${sel}; if (!e) return null; e.scrollIntoView({ block: 'center' }); const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2 }; })()`);
  const textSel = label => `document.querySelector('#hit-layer .text-target[aria-label=${q(label)}]')`;
  /** Hover the target first: handles are display:none until hover/focus. */
  const handleOf = async (sel, name) => {
    const box = await wait(() => rectOf(sel));
    await move(box.cx, box.cy); await sleep(250);
    const h = await rectOf(`(${sel})?.querySelector('[data-handle="${name}"]')`);
    if (!h || h.w < 1) throw new Error(`handle ${name} not visible on hover: ${q(h)}`);
    return h;
  };
  const boundsOf = async text => { const s = await state(); const id = (await cdp.eval(`${textSel(text)}?.dataset.targetId ?? null`)); return s.content.bounds.find(b => b.targetId === id)?.bounds; };

  await wait(() => cdp.eval(`document.title.includes('quartz')`), 60_000);
  await cdp.eval(`document.getElementById('mode-edit').click()`);
  await wait(async () => (await state()).content.glyphLines === 25, 20_000);
  await shot('print-edit-a0-lines.png');

  // A. Real click on the "Postal code" line, real typing, Enter commits one edit.
  const revision = (await state()).revision;
  const line = await wait(() => rectOf(textSel('Postal code')));
  await click(line.x + line.w / 3, line.cy);
  await wait(async () => (await state()).direct.active?.value === 'Postal code');
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'a', code: 'KeyA', windowsVirtualKeyCode: 65, modifiers: 4 });
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'a', code: 'KeyA', windowsVirtualKeyCode: 65, modifiers: 4 });
  await cdp.send('Input.insertText', { text: 'Post code' });
  await shot('print-edit-a1-typing.png');
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
  await wait(async () => { const s = await state(); return s.revision === revision + 1 && s.direct.active === null && s.content.edited?.to === 'Post code'; });
  log('ok A click + type + Enter');
  await shot('print-edit-a-edited.png');

  // B. Real grip drag of "Floor 2" by (20, 10) pt.
  const before = await wait(() => boundsOf('Floor 2'));
  const scale = line.w / (370.91 - 320);
  const grip = await handleOf(textSel('Floor 2'), 'grip');
  await drag(grip.cx, grip.cy, 20 * scale, 10 * scale);
  const after = await wait(async () => { const b = await boundsOf('Floor 2'); return b && Math.abs(b[0] - before[0]) > 1 ? b : null; });
  near(after, before.map((v, i) => v + (i % 2 ? 10 : 20)), 1, 'B Floor 2 moved by a real grip drag');
  await shot('print-edit-b-moved.png');
  log('state', q((await state()).direct));
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
