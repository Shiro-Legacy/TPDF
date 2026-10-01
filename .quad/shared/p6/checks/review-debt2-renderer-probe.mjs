#!/usr/bin/env node
// Review debt2 renderer lens: real CDP mouse/keys (no osascript). Machinery copied from shot-debt2.mjs. Run under .quad/run.lock after npm run build.
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
const KEYS = { Enter: 13, Tab: 9, Escape: 27, ArrowRight: 39, ArrowDown: 40, c: 67, v: 86, x: 88, a: 65 };
const SHIFT = 8, ALT = 1, META = 4;

/** One Electron on one fixture: probe (open + hold), fresh udata, SIGTERM + hash check in finally. */
async function session(tag, fixture, body) {
  const udata = join(out, `debt2-review-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `debt2-review-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `debt2-review-out-${tag}`)}`, `--report=${join(out, `debt2-review-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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
    /** modifiers (Shift 8, Alt 1, Meta 4) ride on moved, pressed and released. */
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
    const key = async (name, modifiers = 0, commands = []) => {
      const code = KEYS[name];
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers, commands });
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
    await body({ state, shot, move, click, rightClick, drag, key, pagePoint, mode, cdp, child });
    log('PASS', tag);
  } finally {
    cdp?.close();
    child.kill('SIGTERM');
    log('electron closed', tag, await closed);
    log('fixtures after', tag, '\n' + hashes());
  }
}

const deq = (got, want, label) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) { fails.push(`${label}: ${q(got)} vs ${q(want)}`); log('FAIL', label, q(got), 'want', q(want)); }
  else log('ok', label, q(got));
};
const fails = [];
const round2 = v => Math.round(v * 100) / 100;
const TWO = /^-?\d+(\.\d{1,2})?$/;
const status = cdp => cdp.eval(`(() => { const e = document.getElementById('edit-status'); return { text: e.textContent, role: e.getAttribute('role'), live: e.getAttribute('aria-live') }; })()`);
const elCentre = (cdp, id) => cdp.eval(`(() => { const e = document.getElementById(${q(id)}); e.scrollIntoView({ block: 'nearest' }); const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width, disabled: e.disabled }; })()`);
const fields = cdp => cdp.eval(`['x','y','w','h'].map(k => document.getElementById('content-box-' + k).value)`);
const validity = cdp => cdp.eval(`['x','y','w','h'].map(k => { const v = document.getElementById('content-box-' + k).validity; return v.valid ? 'valid' : Object.keys(Object.getPrototypeOf(v)).filter(n => v[n]).join('|') || 'invalid'; })`);

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
try {
  await session('r', 'fixtures/p6/direct3.pdf', async ({ state, shot, click, drag, key, pagePoint, mode, cdp }) => {
    const undoBtn = async () => { const u = await elCentre(cdp, 'undo'); await click(u.x, u.y); await sleep(800); };
    await mode('edit');
    await wait(async () => (await state()).direct?.objects === 3, 20_000);
    await sleep(500);
    const run = await pagePoint(100.78, 386.57);
    let im1 = await pagePoint(122, 167, false);
    // (2a) mixed ⌘X via CDP chord (commands: cut → DOM cut event, the Edit ▸ Cut role path)
    await click(run.x, run.y, SHIFT);
    await click(im1.x, im1.y, SHIFT);
    deq((await state()).direct.selection.map(m => m.name), ['Move me', 'Im1'], 'A selection');
    const r0 = (await state()).revision;
    await key('x', META, ['cut']);
    await sleep(800);
    let s = await state();
    const st = await status(cdp);
    deq(st, { text: 'Cut 1 item; 1 text run stays (it cannot be deleted)', role: 'status', live: 'polite' }, 'A #edit-status (live region)');
    deq(s.announcement, 'Cut 1 item; 1 text run stays (it cannot be deleted)', 'A STATE announcement');
    deq([s.direct.objects, s.direct.clipboard?.count, s.revision, s.dirty], [2, 2, r0 + 1, true], 'A objects/clipboard/revision/dirty');
    deq(s.content.edited, { kind: 'selection', action: 'delete', name: 'Im1', count: 1 }, 'A content.edited');
    await shot('review-debt2-renderer-a-cut1.png');
    await undoBtn();
    s = await state();
    deq([s.direct.objects, s.revision, s.dirty], [3, r0 + 2, false], 'A undo (real click #undo)');

    // (2b) text-only ⌘X: refused cut never dirties
    await click(run.x, run.y, SHIFT);
    deq((await state()).direct.selection.map(m => m.name), ['Move me'], 'B selection');
    const rb = (await state()).revision;
    await key('x', META, ['cut']);
    await sleep(800);
    s = await state();
    deq((await status(cdp)).text, 'Copied 1 text run; text runs cannot be cut', 'B #edit-status');
    deq([s.revision, s.dirty, s.direct.clipboard?.count, s.direct.objects], [rb, false, 1, 3], 'B revision/dirty/clipboard/objects');
    await shot('review-debt2-renderer-b-textonly.png');

    // (2c) two runs + Im1: paste the copied run to get a second run
    const before = (await state()).content.bounds.map(b => b.targetId);
    await key('Escape');
    const empty = await pagePoint(300, 520, false);
        await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: empty.x, y: empty.y });
    await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: empty.x, y: empty.y, button: 'right', buttons: 2, clickCount: 1 });
    await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: empty.x, y: empty.y, button: 'right', buttons: 0, clickCount: 1 });
    await sleep(600);
    const pm = await cdp.eval(`(() => { const e = document.getElementById('object-menu-paste'); const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2, disabled: e.disabled, hidden: document.getElementById('object-menu').hidden }; })()`);
    log('C page menu paste', q(pm));
    await click(pm.x, pm.y);
    await sleep(1200);
    s = await state();
    log('C paste announcement', q(s.announcement), 'revision', s.revision);
    const added = s.content.bounds.find(b => !before.includes(b.targetId));
    log('C pasted run bounds', q(added));
    if (added) {
      const moveMe = s.content.bounds.find(b => before.includes(b.targetId) && Math.abs(b.bounds[0] - 72) < 1 && Math.abs(b.bounds[1] - 376.95) < 1);
      // click the pasted run at its far corner away from Move me, and Move me at its top-left
      const pp = await pagePoint((added.bounds[0] + added.bounds[2]) / 2, (added.bounds[1] + added.bounds[3]) / 2, false);
      const mp = await pagePoint(100.78, 386.57, false);
      const at = pt => cdp.eval(`(() => { const e = document.elementFromPoint(${pt.x}, ${pt.y}); return e ? e.id + '|' + e.className + '|' + JSON.stringify(e.dataset) : null; })()`);
      log('C hit mp', await at(mp), 'hit pp', await at(pp), 'selection before', q((await state()).direct.selection.map(m => m.targetId)));
      await key('Escape'); await key('Escape');
      await click(mp.x, mp.y, SHIFT);
      log('C after shift mp', q((await state()).direct.selection.map(m => m.targetId)));
      if (!(await state()).direct.selection.some(m => m.targetId === added.targetId)) await click(pp.x, pp.y, SHIFT);
      log('C after shift pp', q((await state()).direct.selection.map(m => m.targetId)));
      im1 = await pagePoint(122, 167, false);
      await click(im1.x, im1.y, SHIFT);
      const sel = (await state()).direct.selection;
      log('C selection', q(sel.map(m => [m.kind, m.name])));
      const rc = (await state()).revision;
      if (sel.length === 3) {
        await key('x', META, ['cut']);
        await sleep(800);
        s = await state();
        deq((await status(cdp)).text, 'Cut 1 item; 2 text runs stay (they cannot be deleted)', 'C #edit-status plural');
        deq([s.revision, s.direct.clipboard?.count], [rc + 1, 3], 'C revision/clipboard');
        await shot('review-debt2-renderer-c-cut2.png');
        await undoBtn();
      } else fails.push(`C could not build a 3-member selection: ${q(sel)}`);
      await undoBtn(); // the paste
    } else fails.push('C paste produced no new run');
    s = await state();
    deq([s.direct.objects, s.dirty], [3, false], 'C back to clean');

    // (1) Inspector rounding with a real drag at fit zoom (not 1)
    im1 = await pagePoint(122, 167, false);
    await click(im1.x, im1.y);
    const zoom = await cdp.eval(`(() => { const c = document.getElementById('canvas'); return c.getBoundingClientRect().width / 612; })()`);
    log('D css px per pt', zoom, 'dpr', await cdp.eval('devicePixelRatio'));
    const openFields = await fields(cdp);
    log('D fields before drag', q(openFields));
    const rd = (await state()).revision;
    await drag(im1.x, im1.y, 3, 7, META);
    s = await wait(async () => { const v = await state(); return v.revision === rd + 1 ? v : null; }, 5_000);
    const placement = s.direct.active?.placement ?? s.content.objects.find(o => o.name === 'Im1').box;
    log('D placement after drag', q(placement), 'active', q(s.direct.active?.name));
    if (!placement.some(v => round2(v) !== v)) fails.push(`D no float to round: ${q(placement)}`);
    const afterDragFields = await fields(cdp);
    const want = [placement[0], placement[1], placement[2] - placement[0], placement[3] - placement[1]].map(round2);
    log('D fields right after drag (form still open?)', q(afterDragFields), 'hidden', await cdp.eval(`document.getElementById('content-object-form').hidden`));
    deq(afterDragFields.map(Number), want, 'D fields refreshed after drag without reselect');
    await shot('review-debt2-renderer-d-inspector.png');
    afterDragFields.forEach((v, k) => { if (!TWO.test(v)) fails.push(`D field ${'xywh'[k]} ${v} > 2 decimals`); });
    const val = await validity(cdp);
    log('D validity', q(val));
    deq(val, ['valid', 'valid', 'valid', 'valid'], 'D fields valid for step=0.1 (submit not blocked)');
    // unchanged Apply with the real mouse
    const b0 = await state();
    let ap = await elCentre(cdp, 'content-apply-object');
    await click(ap.x, ap.y);
    await sleep(1200);
    let b1 = await state();
    deq([b1.revision, b1.dirty], [b0.revision, b0.dirty], 'D unchanged Apply (real click): revision/dirty');
    deq(b1.content.objects.find(o => o.name === 'Im1').box, placement, 'D unchanged Apply: box exact');
    log('D edit-status after unchanged Apply', q((await status(cdp)).text), 'form hidden', await cdp.eval(`document.getElementById('content-object-form').hidden`));
    await shot('review-debt2-renderer-d-apply-unchanged.png');
    // same unchanged Apply through the form's submit event (skips interactive validation): proves the §9.8 passthrough itself
    await cdp.eval(`document.getElementById('content-object-form').dispatchEvent(new Event('submit', { cancelable: true }))`);
    await sleep(1500);
    b1 = await state();
    deq([b1.revision, b1.dirty, b1.direct.active?.placement ?? null], [b0.revision, b0.dirty, placement], 'D unchanged Apply (submit event): revision/dirty/placement');
    log('D status after submit-event Apply', q((await status(cdp)).text));
    // edit ONE field (W) with a real click + select-all + insertText, real click Apply
    const wf = await elCentre(cdp, 'content-box-w');
    await click(wf.x, wf.y);
    await key('a', META, ['selectAll']);
    await cdp.send('Input.insertText', { text: '120' });
    await sleep(300);
    log('D fields after W edit', q(await fields(cdp)), 'validity', q(await validity(cdp)));
    ap = await elCentre(cdp, 'content-apply-object');
    await click(ap.x, ap.y);
    await sleep(1500);
    b1 = await state();
    const nb = b1.content.objects.find(o => o.name === 'Im1').box;
    log('D after W=120 Apply (real click): revision', b1.revision, 'box', q(nb), 'status', q((await status(cdp)).text));
    if (b1.revision !== b0.revision + 1) fails.push(`D W edit via real Apply click did not write (revision ${b1.revision}, want ${b0.revision + 1}); validity ${q(await validity(cdp))}`);
    else {
      const exp = [placement[0], placement[1], placement[0] + 120, placement[3]];
      const ok = nb.every((v, i) => Math.abs(v - exp[i]) < 2e-4);
      if (!ok) fails.push(`D W edit box ${q(nb)} vs ${q(exp)}`); else log('ok D W edit keeps unrounded x/y/top', q(nb));
      await undoBtn();
    }
    await shot('review-debt2-renderer-d-apply-w.png');
    // diagnosis: same edit via the form's submit event (bypasses interactive validation)
    const im1b = (await state()).content.objects.find(o => o.name === 'Im1');
    const cp = await pagePoint((im1b.view[0] + im1b.view[2]) / 2, (im1b.view[1] + im1b.view[3]) / 2, false);
    await click(cp.x, cp.y);
    const pre = await state();
    const pl2 = pre.direct.active?.placement ?? im1b.box;
    await cdp.eval(`(() => { const w = document.getElementById('content-box-w'); w.value = '120'; document.getElementById('content-object-form').dispatchEvent(new Event('submit', { cancelable: true })); })()`);
    await sleep(1500);
    let post = await state();
    const im1c = post.content.objects.find(o => o.name === 'Im1');
    const cp2 = await pagePoint((im1c.view[0] + im1c.view[2]) / 2, (im1c.view[1] + im1c.view[3]) / 2, false);
    await click(cp2.x, cp2.y);
    post = await state();
    const nb2 = post.direct.active?.placement;
    log('E submit-event W=120: revision', pre.revision, '→', post.revision, 'box', q(pl2), '→', q(nb2));
    const exp2 = [pl2[0], pl2[1], pl2[0] + 120, pl2[3]];
    if (post.revision !== pre.revision + 1 || !nb2.every((v, i) => Math.abs(v - exp2[i]) < 2e-4)) fails.push(`E submit-event W edit ${q(nb2)} vs ${q(exp2)}`);
    else log('ok E passthrough logic: other three edges unrounded');
    await undoBtn();
    await undoBtn();
    s = await state();
    log('final', q({ revision: s.revision, dirty: s.dirty, im1: s.content.objects.find(o => o.name === 'Im1').box }));
  });
  const ok = (hashes().match(/: OK$/gm) ?? []).length;
  if (ok !== 9) fails.push(`fixtures.sha256: ${ok} OK`);
} catch (err) { fails.push('ERROR ' + (err?.stack ?? err)); }
log(fails.length ? 'FAILS\n' + fails.join('\n') : 'ALL OK');
process.exitCode = fails.length ? 1 : 0;
