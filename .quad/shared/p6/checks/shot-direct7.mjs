#!/usr/bin/env node
/**
 * P6 U6 GUI evidence (contract-p6-unit6.md §4.3 as amended by §9.19, §9.30, §9.33): real CDP input on fixtures/p6/objects6.pdf
 * in Edit text mode. Modifiers (Shift 8, Alt 1, Meta 4) ride on moved, pressed and released.
 * A: click Im1, Shift-click Im3 → one .group-box with 8 handles, no member handles shown; real drag of its east handle by 228 pt → resize.
 * B: real drag of Im1 by (326, 0) pt → direct.gesture.guides during the drag = [{x 400}, {y 142}]; with ⌘ held = [].
 * C: #content-crop → .cropping + .crop-full; real drag of Im1's east handle inward (−40 pt) → crop; #content-crop again leaves.
 * D: #content-rotate-cw → Im1 orient.swap true (STATE only). E: hover the Fm1 interior → not the Fm1 button; click its band → Fm1 (STATE only).
 * Screenshots → review-out/u6-{a,b,c}-*.png. Not part of npm test. Run after `npm run build`, under .quad/run.lock:
 * node .quad/shared/p6/checks/shot-direct7.mjs
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
const SHIFT = 8, ALT = 1, META = 4;

/** One Electron on one fixture: probe (open + hold), fresh udata, SIGTERM + hash check in finally. */
async function session(tag, fixture, body) {
  const udata = join(out, `u6-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `u6-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `u6-out-${tag}`)}`, `--report=${join(out, `u6-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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
    /** §9.19: modifiers (Shift 8, Alt 1, Meta 4) ride on moved, pressed and released. */
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

const deq = (got, want, label) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`${label}: ${q(got)} vs ${q(want)}`);
  log('ok', label, q(got));
};
const undo = async (cdp, state, rev) => {
  await cdp.eval(`document.getElementById('undo').click()`);
  await wait(async () => (await state()).revision === rev, 10_000);
  await sleep(400);
};
/** Client centre of the first element matching `selector` (null when absent or zero-sized); an Inspector control is scrolled into view inside the Inspector first. */
const centre = (cdp, selector) => cdp.eval(`(() => { const e = document.querySelector(${q(selector)}); if (!e) return null;
  if (e.closest('#inspector')) e.scrollIntoView({ block: 'nearest' }); const r = e.getBoundingClientRect();
  return r.width && r.height ? { x: r.left + r.width / 2, y: r.top + r.height / 2 } : null; })()`);
const IM1 = [72, 142, 172, 192];

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
try {
  await session('objects6', 'fixtures/p6/objects6.pdf', async ({ state, shot, move, click, drag, key, pagePoint, mode, cdp }) => {
    await mode('edit');
    await wait(async () => (await state()).direct.objects === 6, 20_000);
    await sleep(500);
    const im1 = await pagePoint(122, 167);
    const im3 = await pagePoint(250, 167, false);
    const s = im1.s;

    // A. Im1 + Shift Im3 → one .group-box with 8 handles, member handles hidden; real east-handle drag by 228 pt.
    await click(im1.x, im1.y);
    await wait(async () => names((await state()).direct.selection) === 'Im1');
    await click(im3.x, im3.y, SHIFT);
    await wait(async () => names((await state()).direct.selection) === 'Im1,Im3');
    await move(im3.x, im3.y); await sleep(300);
    const group = await cdp.eval(`(() => {
      const boxes = [...document.querySelectorAll('#hit-layer .group-box')];
      const handles = boxes.flatMap(b => [...b.querySelectorAll('.hit-handle')]);
      const member = [...document.querySelectorAll('#hit-layer .object-target.multi .hit-handle')];
      return { boxes: boxes.length, handles: handles.length, handlesShown: handles.filter(h => getComputedStyle(h).display !== 'none').length,
        memberShown: member.filter(h => getComputedStyle(h).display !== 'none').length, objectTargets: document.querySelectorAll('#hit-layer .object-target').length };
    })()`);
    if (group.boxes !== 1 || group.handles !== 8 || group.handlesShown !== 8 || group.memberShown !== 0 || group.objectTargets !== 6) throw new Error(`A group box: ${q(group)}`);
    log('ok A one .group-box, 8 handles, no member handles', q(group));
    await shot('u6-a-group-box.png');
    const east = await centre(cdp, '#hit-layer .group-box .hit-handle.e');
    if (!east) throw new Error('A: no group-box east handle');
    const r0 = (await state()).revision;
    await drag(east.x, east.y, 228 * s, 0);
    const a = await wait(async () => { const v = await state(); return v.revision === r0 + 1 ? v : null; });
    deq({ kind: a.content.edited?.kind, action: a.content.edited?.action, count: a.content.edited?.count }, { kind: 'objects', action: 'resize', count: 2 }, 'A group resize edited');
    near(sorted(a, 'Im1')[0], [72, 142, 272, 192], 1, 'A Im1 after group resize');
    near(sorted(a, 'Im3')[0], [328, 142, 528, 192], 1, 'A Im3 after group resize');
    await undo(cdp, state, r0 + 2);

    // B. real drag of Im1 by (326, 0) pt: guides during the drag; with ⌘ none and no snap.
    await key('Escape');
    await wait(async () => (await state()).direct.selection.length === 0);
    await click(im1.x, im1.y);
    const r1 = (await state()).revision;
    const guides = await drag(im1.x, im1.y, 326 * s, 0, 0, async () => {
      const g = (await state()).direct.gesture?.guides;
      const lines = await cdp.eval(`document.querySelectorAll('#hit-layer .guide').length`);
      await shot('u6-b-guides.png');
      return { g, lines };
    });
    deq(guides.g, [{ axis: 'x', at: 400 }, { axis: 'y', at: 142 }], 'B guides during the drag');
    if (guides.lines < 1) throw new Error(`B no .guide painted: ${q(guides)}`);
    const b = await wait(async () => { const v = await state(); return v.revision === r1 + 1 ? v : null; });
    near(sorted(b, 'Im1')[0], [400, 142, 500, 192], 0.5, 'B snapped to Im2 x0');
    if (await cdp.eval(`document.querySelectorAll('#hit-layer .guide').length`) !== 0) throw new Error('B guides left after the drag');
    await undo(cdp, state, r1 + 2);
    await click(im1.x, im1.y);
    const free = await drag(im1.x, im1.y, 326 * s, 0, META, async () => (await state()).direct.gesture?.guides);
    deq(free, [], 'B guides with ⌘ held');
    const b2 = await wait(async () => { const v = await state(); return v.revision === r1 + 3 ? v : null; });
    near(sorted(b2, 'Im1')[0], [398, 142, 498, 192], 1, 'B ⌘ drag unsnapped');
    await undo(cdp, state, r1 + 4);

    // C. #content-crop → crop mode; real drag of Im1's east handle −40 pt; #content-crop again leaves.
    await click(im1.x, im1.y);
    const cropBtn = await centre(cdp, '#content-crop');
    if (!cropBtn) throw new Error('C: #content-crop not visible');
    await click(cropBtn.x, cropBtn.y);
    const c0 = await wait(async () => { const v = await state(); return v.direct.crop?.name === 'Im1' ? v : null; }, 8_000).catch(async err => {
      const why = await cdp.eval(`(() => { const b = document.getElementById('content-crop'), r = b.getBoundingClientRect(); const top = document.elementFromPoint(${cropBtn.x}, ${cropBtn.y});
        return { disabled: b.disabled, hidden: !!b.closest('[hidden]'), rect: [r.left, r.top, r.width, r.height], top: top?.id || top?.className || top?.tagName, label: document.getElementById('content-object-label').textContent }; })()`);
      const v = await state();
      throw new Error(`C crop mode not entered: ${err.message} ${q({ why, point: cropBtn, crop: v.direct.crop, active: v.direct.active?.name, selection: names(v.direct.selection) })}`);
    });
    near(c0.direct.crop.full, IM1, 0.5, 'C direct.crop.full');
    const cropDom = await cdp.eval(`({ cropping: document.querySelectorAll('#hit-layer .object-target.cropping').length, full: document.querySelectorAll('#hit-layer .crop-full').length,
      pressed: document.getElementById('content-crop').getAttribute('aria-pressed') })`);
    deq(cropDom, { cropping: 1, full: 1, pressed: 'true' }, 'C crop-mode DOM');
    await shot('u6-c-crop-mode.png');
    const cropEast = await centre(cdp, '#hit-layer .object-target.cropping .hit-handle.e');
    if (!cropEast) throw new Error('C: no crop east handle');
    const r2 = c0.revision;
    const cropGesture = await drag(cropEast.x, cropEast.y, -40 * s, 0, 0, async () => (await state()).direct.gesture?.kind);
    deq(cropGesture, 'crop', 'C direct.gesture.kind during the crop drag');
    const c1 = await wait(async () => { const v = await state(); return v.revision === r2 + 1 ? v : null; });
    deq({ action: c1.content.edited?.action, name: c1.content.edited?.name }, { action: 'crop', name: 'Im1' }, 'C crop edited');
    const im1c = c1.content.objects.find(o => o.name === 'Im1');
    near(im1c.view, [72, 142, 132, 192], 1, 'C cropped view');
    near(im1c.full, IM1, 1, 'C full');
    if (c1.direct.crop?.name !== 'Im1') throw new Error(`C crop mode left after the drag: ${q(c1.direct.crop)}`);
    await shot('u6-c-cropped.png');
    const cropBtn2 = await centre(cdp, '#content-crop');
    await click(cropBtn2.x, cropBtn2.y);
    await wait(async () => (await state()).direct.crop === null);
    log('ok C #content-crop again leaves crop mode');
    await undo(cdp, state, r2 + 2);

    // D. #content-rotate-cw on Im1 → orient.swap (STATE only).
    await click(im1.x, im1.y);
    const rot = await centre(cdp, '#content-rotate-cw');
    if (!rot) throw new Error('D: #content-rotate-cw not visible');
    const r3 = (await state()).revision;
    await click(rot.x, rot.y);
    const d = await wait(async () => { const v = await state(); return v.revision === r3 + 1 ? v : null; });
    deq(d.content.objects.find(o => o.name === 'Im1')?.orient, { swap: true, sx: 1, sy: -1 }, 'D Im1 orient after #content-rotate-cw');
    await undo(cdp, state, r3 + 2);

    // E. Fm1 wraps Im5 and "Over": its interior is not the Fm1 button; a click on its border band selects it (STATE only).
    await key('Escape');
    const fm1Id = (await state()).content.objects.find(o => o.name === 'Fm1')?.targetId;
    const inner = await pagePoint(200, 600);
    const band = await pagePoint(74, 600, false);
    await move(inner.x, inner.y); await sleep(300);
    const hit = await cdp.eval(`document.elementFromPoint(${inner.x}, ${inner.y})?.closest('.object-target')?.dataset.targetId ?? null`);
    if (hit === fm1Id) throw new Error(`E interior hits the Fm1 button (${fm1Id})`);
    log('ok E interior elementFromPoint is not Fm1', q(hit));
    await click(band.x, band.y);
    const e = await wait(async () => { const v = await state(); return v.direct.active?.name === 'Fm1' ? v : null; });
    log('ok E band click selects Fm1', q(e.direct.active));
  });
  log('PASS all');
} catch (err) {
  console.error('FAIL', err?.stack ?? err);
  process.exitCode = 1;
}
