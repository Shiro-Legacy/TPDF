#!/usr/bin/env node
/**
 * draw-signature GUI evidence (contract-draw-signature.md D10), template shot-fill-anywhere.mjs: real CDP mouse and keys
 * on fixtures/hello.pdf in Fill forms — open the pad ("Draw signature…"), draw two strokes with real pointer moves,
 * Save, press Sign, click the page, then drag the se handle. Nothing is saved (the fixture is only opened; a drive
 * keeps the drawing in memory, never in the user's profile). Screenshots → review-out/draw-signature-*.png.
 * Not part of npm test. Written, NOT run (the human is at the machine). Run after `npm run build`, under
 * .quad/run.lock: node .quad/shared/p6/checks/shot-draw-signature.mjs
 * sign-merge D8 (appended, written, NOT run): D. the image route — open the pad, "Use an image…" (a review-out copy of
 * fixtures/img-rgb.png, pre-answered through the probe's `images`), Save, Sign, a real click on the page.
 */
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { copyFile, mkdir, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../../../../', import.meta.url));
const checksDir = fileURLToPath(new URL('./', import.meta.url));
const out = join(checksDir, 'review-out');
const udata = join(out, 'draw-signature-udata');
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
await copyFile(join(root, 'fixtures/img-rgb.png'), join(out, 'sign-merge-signature.png')); // sign-merge D8: a copy, never the fixture
const probe = join(out, 'draw-signature-probe.jsonl');
await writeFile(probe, [
  { cmd: 'open', path: join(root, 'fixtures/hello.pdf'), images: [join(out, 'sign-merge-signature.png')] },
  ...hold(40, 5000),
].map(l => JSON.stringify(l)).join('\n') + '\n');
const port = await freePort();
const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
  `--drive=${probe}`, '--gui', `--drive-out=${join(out, 'draw-signature-out')}`, `--report=${join(out, 'draw-signature-report.json')}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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


  // A. The pad: two real strokes, Save.
  await cdp.eval(`document.getElementById('fill-sign-draw').click()`);
  await wait(async () => (await state()).fill.signature.pad === true);
  const pad = await cdp.eval(`(() => { const r = document.getElementById('sigpad-canvas').getBoundingClientRect(); return { x: r.x, y: r.y }; })()`);
  const stroke = async points => {
    await move(pad.x + points[0][0], pad.y + points[0][1]); await sleep(100);
    await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: pad.x + points[0][0], y: pad.y + points[0][1], button: 'left', buttons: 1, clickCount: 1 });
    for (const [x, y] of points.slice(1)) { await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: pad.x + x, y: pad.y + y, button: 'left', buttons: 1 }); await sleep(20); }
    const [lx, ly] = points[points.length - 1];
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: pad.x + lx, y: pad.y + ly, button: 'left', buttons: 0, clickCount: 1 });
    await sleep(200);
  };
  await stroke([[60, 120], [90, 60], [120, 120], [150, 70], [180, 120]]);
  await stroke([[220, 100], [300, 90], [380, 110], [440, 95]]);
  await shot('draw-signature-a0-pad.png');
  await cdp.eval(`document.getElementById('sigpad-save').click()`);
  const saved = await wait(async () => { const s = await state(); return s.fill.signature.saved && !s.fill.signature.pad ? s.fill.signature : null; });
  if (saved.strokes !== 2) throw new Error(`A strokes: ${q(saved)}`);
  log('ok A signature saved', q(saved));

  // B. Sign, then a real click on the page at (300, 650) places it.
  await cdp.eval(`document.getElementById('fill-sign').click()`);
  await wait(async () => (await state()).fill.signature.armed === true);
  const at = await pagePoint(300, 650);
  await click(at.x, at.y);
  const placed = await wait(async () => { const s = await state(); return s.fill.signatures.length === 1 ? s.fill.signatures[0] : null; });
  near([(placed.rect[0] + placed.rect[2]) / 2, (placed.rect[1] + placed.rect[3]) / 2], [300, 650], 1.5, 'B placed at the click');
  await shot('draw-signature-b-placed.png');

  // C. A real drag of the se handle grows it, aspect kept.
  const handle = await wait(() => cdp.eval(`(() => { const t = document.querySelector('#hit-layer .signature-target'); if (!t) return null; t.focus(); const h = t.querySelector('[data-handle="se"]'); const r = h.getBoundingClientRect(); return r.width ? { x: r.x + r.width / 2, y: r.y + r.height / 2 } : null; })()`));
  await drag(handle.x, handle.y, 40 * at.s, 10 * at.s);
  const grown = await wait(async () => { const s = await state(); return s.fill.lastSignature?.op === 'resize' ? s.fill.signatures[0].rect : null; });
  const ratio = (r => (r[3] - r[1]) / (r[2] - r[0]));
  if (Math.abs(ratio(grown) - ratio(placed.rect)) > 0.01 || grown[2] - grown[0] <= placed.rect[2] - placed.rect[0]) throw new Error(`C resize: ${q(grown)} vs ${q(placed.rect)}`);
  log('ok C resized by the handle', q(grown));
  await shot('draw-signature-c-resized.png');
  // D. sign-merge D8: the image route — open the pad, "Use an image…", Save, Sign, a real click on the page.
  await cdp.eval(`document.getElementById('fill-sign-draw').click()`);
  await wait(async () => (await state()).fill.signature.pad === true);
  await cdp.eval(`document.getElementById('sigpad-image').click()`);
  await wait(() => cdp.eval(`!document.getElementById('sigpad-save').disabled`));
  await shot('draw-signature-d0-image-pad.png');
  await cdp.eval(`document.getElementById('sigpad-save').click()`);
  const imaged = await wait(async () => { const s = await state(); return s.fill.signature.kind === 'image' && !s.fill.signature.pad ? s.fill.signature : null; });
  log('ok D image saved', q(imaged));
  await cdp.eval(`document.getElementById('fill-sign').click()`);
  await wait(async () => (await state()).fill.signature.armed === true);
  const atD = await pagePoint(300, 300);
  await click(atD.x, atD.y);
  const imagePlaced = await wait(async () => { const s = await state(); return s.fill.signatures.find(item => item.kind === 'image') ?? null; });
  near([(imagePlaced.rect[0] + imagePlaced.rect[2]) / 2, (imagePlaced.rect[1] + imagePlaced.rect[3]) / 2], [300, 300], 1.5, 'D image placed at the click');
  await shot('draw-signature-d-image-placed.png');
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
