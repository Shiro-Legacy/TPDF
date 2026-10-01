#!/usr/bin/env node
/**
 * P6 U3 GUI evidence (contract-p6-unit3.md §6.3, §9.12): real CDP mouse gestures on fixtures/p6/direct3.pdf.
 * A grip drag of Im2, a corner resize of Im1 keeping 2:1, a click on empty page + insertText + ⌘Enter, and a
 * redaction-mark drag in Redact mode. Screenshots → review-out/u3-*.png. Not part of npm test.
 * Run after `npm run build`, under .quad/run.lock: node .quad/shared/p6/checks/shot-direct3.mjs
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
const udata = join(out, 'u3-udata');
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
const probe = join(out, 'u3-probe.jsonl');
// The mark is created by the probe before the GUI drags it (§9.12); the holds keep the app alive.
await writeFile(probe, [
  { cmd: 'open', path: join(root, 'fixtures/p6/direct3.pdf') },
  { cmd: 'redact', contents: { page: 0, rect: [300, 600, 400, 650] } },
  ...hold(40, 5000),
].map(l => JSON.stringify(l)).join('\n') + '\n');
const port = await freePort();
const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
  `--drive=${probe}`, '--gui', `--drive-out=${join(out, 'u3-out')}`, `--report=${join(out, 'u3-report.json')}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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
  /** Scroll the page point into view; return its client point and the px-per-pt scale. */
  const pagePoint = (x, y) => cdp.eval(`(() => {
    const v = document.getElementById('viewport'), c = document.getElementById('canvas');
    let r = c.getBoundingClientRect(); const s = r.width / 612;
    v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
    r = c.getBoundingClientRect();
    return { x: r.left + ${x} * s, y: r.top + ${y} * s, s };
  })()`);
  /** The target's rect after scrolling it into view; `sel` is evaluated in the page and must yield an element. */
  const rectOf = sel => cdp.eval(`(() => { const e = ${sel}; if (!e) return null; e.scrollIntoView({ block: 'center' }); const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2 }; })()`);
  const objectSel = name => `(() => { const o = window.pdf4allState().content.objects.find(i => i.name === ${q(name)}); return o && document.querySelector('#hit-layer .object-target[data-target-id="' + CSS.escape(String(o.targetId)) + '"]'); })()`;
  /** Hover the target first: handles are display:none until hover/focus (§9.12). */
  const handleOf = async (sel, name) => {
    const box = await wait(() => rectOf(sel));
    await move(box.cx, box.cy); await sleep(250);
    const h = await rectOf(`(${sel})?.querySelector('[data-handle="${name}"]')`);
    if (!h || h.w < 1) throw new Error(`handle ${name} not visible on hover: ${q(h)}`);
    return h;
  };
  const objectBox = async name => (await state()).content.objects.find(i => i.name === name)?.box;

  await wait(() => cdp.eval(`document.title.includes('direct3')`), 60_000);
  await wait(async () => (await state()).redact.marks === 1, 20_000);
  await sleep(800);
  await cdp.eval(`document.getElementById('mode-edit').click()`);
  await wait(async () => (await state()).direct.objects === 3, 20_000);
  await shot('u3-a0-edit-objects.png');

  // A. Real grip drag of Im2 by (50, 30) pt.
  const scale = (await pagePoint(430, 72)).s;
  const grip = await handleOf(objectSel('Im2'), 'grip');
  await drag(grip.cx, grip.cy, 50 * scale, 30 * scale);
  await wait(async () => (await state()).content.edited?.action === 'move');
  near(await objectBox('Im2'), [450, 670, 510, 710], 0.5, 'A Im2 moved by a real grip drag');
  await shot('u3-a-image-moved.png');

  // B. Real corner drag of Im1: se by (20, 20) pt keeps 2:1 (anchor top-left, y-up top 650).
  const se = await handleOf(objectSel('Im1'), 'se');
  await drag(se.cx, se.cy, 20 * scale, 20 * scale);
  await wait(async () => (await state()).content.edited?.action === 'resize');
  const im1 = await objectBox('Im1');
  if (Math.abs((im1[2] - im1[0]) - 2 * (im1[3] - im1[1])) > 0.1 || Math.abs(im1[0] - 72) > 0.01 || Math.abs(im1[3] - 650) > 0.01) throw new Error(`B Im1 ratio/anchor: ${q(im1)}`);
  near(im1, [72, 580, 212, 650], 0.5, 'B Im1 corner resize keeps 2:1');
  await shot('u3-b-image-resized.png');

  // C. Click empty page (300, 500) → add editor; real text; ⌘Enter commits one text-add.
  const revision = (await state()).revision;
  const empty = await pagePoint(300, 500);
  await click(empty.x, empty.y);
  const active = await wait(async () => { const a = (await state()).direct.active; return a?.kind === 'add' ? a : null; });
  near(active.point, [300, 500], 1, 'C add editor at the click point');
  await cdp.send('Input.insertText', { text: 'Hello GUI' });
  await sleep(300);
  await shot('u3-c0-add-editor.png');
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, modifiers: 4 });
  await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, modifiers: 4 });
  const added = await wait(async () => { const s = await state(); return s.revision === revision + 1 && s.direct.active === null && s.content.lastAdd?.targetId ? s : null; });
  const run = added.content.bounds.find(item => item.targetId === added.content.lastAdd.targetId);
  if (!run || Math.abs(run.bounds[0] - 300) > 1) throw new Error(`C new run not at the click: ${q(run)}`);
  await wait(() => cdp.eval(`!!document.querySelector('#hit-layer .text-target[aria-label="Hello GUI"]')`));
  log('ok C text added', q(run));
  await shot('u3-c-text-added.png');

  // D. Redact mode: real grip drag of the probe's area mark by 40 pt.
  await cdp.eval(`document.getElementById('mode-redact').click()`);
  await wait(async () => (await state()).direct.marks === 1, 20_000);
  const markSel = `document.querySelector('#hit-layer .redact-target')`;
  const before = (await state()).redact.rects[0].rect;
  near(before, [300, 600, 400, 650], 0.05, 'D mark before');
  const markGrip = await handleOf(markSel, 'grip');
  await drag(markGrip.cx, markGrip.cy, 40 * scale, 0);
  const moved = await wait(async () => { const r = (await state()).redact.rects[0]?.rect; return r && Math.abs(r[0] - before[0]) > 1 ? r : null; });
  near(moved, [340, 600, 440, 650], 0.5, 'D mark moved by a real drag');
  await shot('u3-d-mark-moved.png');
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
