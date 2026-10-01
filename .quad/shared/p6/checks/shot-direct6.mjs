#!/usr/bin/env node
/**
 * P6 U5 GUI evidence (contract-p6-unit5.md §4.3 as amended by §9.34): real CDP input on fixtures/p6/direct3.pdf in Edit text mode.
 * A: click Im1, Shift-click Im2 → two .selected.multi buttons, every .multi .hit-handle computed display:none while hovering a member.
 * B: real right-click on Fm1 → #object-menu open, Replace disabled (DOM + STATE), then Escape.
 * C: real ⌥-drag of Im2 by 60,40 pt → 4 .object-target, the copy at [460,92,520,132] (y-down), direct.gesture.kind 'duplicate' mid-drag.
 * Screenshots → review-out/u5-{a,b,c}-*.png. Not part of npm test. Run after `npm run build`, under .quad/run.lock:
 * node .quad/shared/p6/checks/shot-direct6.mjs
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
  throw new Error(`timeout: ${last instanceof Error ? last.message : q(last)}`);
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
const KEYS = { Enter: 13, Tab: 9, Escape: 27, ArrowRight: 39, ArrowDown: 40 };
const SHIFT = 8, ALT = 1;

/** One Electron on one fixture: probe (open + hold), fresh udata, SIGTERM + hash check in finally. */
async function session(tag, fixture, body) {
  const udata = join(out, `u5-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `u5-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `u5-out-${tag}`)}`, `--report=${join(out, `u5-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  const closed = new Promise(done => child.on('close', done));
  let cdp;
  try {
    cdp = await attach(port);
    try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch { /* headless window manager */ }
    const state = () => cdp.eval('window.pdf4allState()');
    /** §9.30 (P6 U6): the first pagePoint (Im1 row) scrolls and saves scrollTop; shot() only asserts it (a scroll would close the menu). */
    let saved = null;
    const shot = async name => {
      const top = await cdp.eval(`document.getElementById('viewport').scrollTop`);
      if (saved !== null && top !== saved) throw new Error(`${name}: viewport.scrollTop ${top} !== saved ${saved}`);
      const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
      await writeFile(join(out, name), Buffer.from(data, 'base64'));
      log('shot', name, Date.now() - T0);
    };
    const mouse = (type, x, y, extra = {}) => cdp.send('Input.dispatchMouseEvent', { type, x, y, ...extra });
    const move = (x, y, modifiers = 0) => mouse('mouseMoved', x, y, { modifiers });
    /** §9.34: modifiers (Shift 8, Alt 1) ride on moved, pressed and released. */
    const click = async (x, y, modifiers = 0) => {
      await move(x, y, modifiers); await sleep(100);
      await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1, modifiers });
      await mouse('mouseReleased', x, y, { button: 'left', buttons: 0, clickCount: 1, modifiers });
      await sleep(600);
    };
    const rightClick = async (x, y) => {
      await move(x, y); await sleep(100);
      await mouse('mousePressed', x, y, { button: 'right', buttons: 2, clickCount: 1 });
      await mouse('mouseReleased', x, y, { button: 'right', buttons: 0, clickCount: 1 });
      await sleep(600);
    };
    /** `during` runs after the last move, before the release (mid-gesture STATE). */
    const drag = async (x, y, dx, dy, modifiers = 0, during) => {
      await move(x, y, modifiers); await sleep(150);
      await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1, modifiers });
      for (let i = 1; i <= 8; i++) await mouse('mouseMoved', x + dx * i / 8, y + dy * i / 8, { button: 'left', buttons: 1, modifiers });
      const seen = during ? await during() : undefined;
      await mouse('mouseReleased', x + dx, y + dy, { button: 'left', buttons: 0, clickCount: 1, modifiers });
      await sleep(1500);
      return seen;
    };
    const key = async (name, modifiers = 0) => {
      const code = KEYS[name];
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await sleep(500);
    };
    /** Page pt (y-down) → client px; scale by the shown page's real CSS width over its width in pt; `scroll` centres the point first. */
    const pagePoint = async (x, y, scroll = true, widthPt = 612) => {
      const p = await cdp.eval(`(() => {
        const v = document.getElementById('viewport'), c = document.getElementById('canvas');
        let r = c.getBoundingClientRect(); const s = r.width / ${widthPt};
        if (${scroll}) v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
        r = c.getBoundingClientRect();
        return { x: r.left + ${x} * s, y: r.top + ${y} * s, s, top: v.scrollTop };
      })()`);
      if (scroll) { await sleep(300); saved = await cdp.eval(`document.getElementById('viewport').scrollTop`); }
      return p;
    };
    const mode = async name => cdp.eval(`document.getElementById('mode-${name}').click()`);
    await wait(() => cdp.eval(`document.title.includes(${q(fixture.split('/').pop().replace('.pdf', ''))})`), 60_000);
    await sleep(800);
    await body({ state, shot, move, click, rightClick, drag, key, pagePoint, mode, cdp });
    log('PASS', tag);
  } finally {
    cdp?.close();
    child.kill('SIGTERM');
    log('electron closed', tag, await closed);
    log('fixtures after', tag, '\n' + hashes());
  }
}

const names = sel => (sel ?? []).map(m => m.name).join(',');
const sorted = (s, name) => s.content.objects.filter(o => o.name === name).map(o => o.view).sort((a, b) => a[0] - b[0] || a[1] - b[1]);

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
try {
  await session('direct3', 'fixtures/p6/direct3.pdf', async ({ state, shot, move, click, rightClick, drag, key, pagePoint, mode, cdp }) => {
    await mode('edit');
    await wait(async () => (await state()).direct.objects === 3, 20_000);
    await sleep(500);

    // A. click Im1, Shift-click Im2 → two .selected.multi, no handles while hovering a member.
    const im1 = await pagePoint(122, 167);
    await click(im1.x, im1.y);
    await wait(async () => names((await state()).direct.selection) === 'Im1');
    const im2 = await pagePoint(430, 72, false);
    await click(im2.x, im2.y, SHIFT);
    const s = await wait(async () => { const v = await state(); return names(v.direct.selection) === 'Im1,Im2' ? v : null; });
    if (s.direct.active?.name !== 'Im2') throw new Error(`A anchor: ${q(s.direct.active)}`);
    await move(im2.x, im2.y); await sleep(300);
    const multi = await cdp.eval(`(() => {
      const buttons = [...document.querySelectorAll('#hit-layer .object-target.selected.multi')];
      const handles = buttons.flatMap(b => [...b.querySelectorAll('.hit-handle')]);
      return { buttons: buttons.length, handles: handles.length, shown: handles.filter(h => getComputedStyle(h).display !== 'none').length };
    })()`);
    if (multi.buttons !== 2 || multi.handles === 0 || multi.shown !== 0) throw new Error(`A multi paint: ${q(multi)}`);
    log('ok A two .selected.multi, handles hidden', q(multi));
    await shot('u5-a-multi-select.png');

    // B. real right-click on Fm1 → menu open, Replace disabled; Escape closes it.
    await key('Escape');
    await wait(async () => (await state()).direct.selection.length === 0);
    const fm1 = await pagePoint(450, 277, false);
    await rightClick(fm1.x, fm1.y);
    const m = await wait(async () => { const v = await state(); return v.direct.menu?.name === 'Fm1' ? v.direct.menu : null; });
    if (m.items.replace !== false || m.items.duplicate !== true || m.items.delete !== true || m.count !== 1) throw new Error(`B menu STATE: ${q(m)}`);
    const dom = await cdp.eval(`(() => {
      const menu = document.getElementById('object-menu'), r = document.getElementById('object-menu-replace');
      return { hidden: menu.hidden, display: getComputedStyle(menu).display, disabled: r.disabled, aria: r.getAttribute('aria-disabled') };
    })()`);
    if (dom.hidden || dom.display === 'none' || dom.disabled !== true || dom.aria !== 'true') throw new Error(`B menu DOM: ${q(dom)}`);
    log('ok B right-click menu on Fm1, Replace disabled', q(m), q(dom));
    await shot('u5-b-menu-fm1.png');
    await key('Escape');
    await wait(async () => (await state()).direct.menu === null);

    // C. real ⌥-drag of Im2 by 60,40 pt → a copy at the drop point, 4 targets.
    const at = await pagePoint(430, 72, false);
    const kind = await drag(at.x, at.y, 60 * at.s, 40 * at.s, ALT, async () => (await state()).direct.gesture?.kind);
    if (kind !== 'duplicate') throw new Error(`C gesture kind mid-drag: ${q(kind)}`);
    const c = await wait(async () => { const v = await state(); return v.direct.objects === 4 ? v : null; });
    const targets = await cdp.eval(`document.querySelectorAll('#hit-layer .object-target').length`);
    if (targets !== 4) throw new Error(`C targets: ${targets}`);
    const boxes = sorted(c, 'Im2');
    near(boxes[0], [400, 52, 460, 92], 1, 'C original Im2 stays');
    near(boxes[1], [460, 92, 520, 132], 1, 'C ⌥-drag copy');
    await shot('u5-c-alt-duplicate.png');
  });
  log('PASS all');
} catch (err) {
  console.error('FAIL', err?.stack ?? err);
  process.exitCode = 1;
}
