#!/usr/bin/env node
/**
 * P6 U7 GUI evidence (contract-p6-unit7.md §4.3 as amended by §9.11, §9.23, §9.41, §10.44): real CDP input, one Electron per session,
 * launched with --inspect so the main process can call webContents.copy()/paste() (the Edit-menu roles' path, §9.23).
 * A (objects6, Edit text): click Im1, real ⌘C (commands ['copy']) → direct.clipboard.count 1 and the copy event defaultPrevented;
 *   real ⌘V → objects 7, selection PA1 at Im1 + (10,10) (u7-a-pasted.png); undo; main-process wc.copy() on Fm1 → kinds.form 1,
 *   wc.paste() → objects 7; undo.
 * B (objects6): real right-click on empty space (500,450) → the page menu with Paste alone enabled (u7-b-page-menu.png).
 * D (objects6): real ⌘C with focus in the Inspector's W field → not prevented, direct.clipboard unchanged, the OS clipboard holds the
 *   field's text. THIS TOUCHES THE OS CLIPBOARD: pbpaste is saved before and restored with pbcopy in finally (run while the human is away).
 *   Ceiling: pbpaste/pbcopy save and restore the text flavour only; a rich OS clipboard is flattened to text by case D.
 * C (direct3): real Shift-click on "Move me" and on Im1 → one .group-box, handles hidden (u7-c-mixed-group.png); real ArrowRight → both +1.
 * E (field-defs, Fill forms): Shift-click plain, real ⌘C/⌘V → fields.count 6, "plain copy" (u7-e-field-paste.png).
 * Screenshots → review-out/u7-{a,b,c,e}-*.png. Not part of npm test. Run after `npm run build`, under .quad/run.lock:
 * node .quad/shared/p6/checks/shot-direct8.mjs   (U7_SKIP_D=1 leaves the OS clipboard alone and skips D)
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
const KEYS = { Enter: 13, Tab: 9, Escape: 27, ArrowRight: 39, ArrowDown: 40, c: 67, v: 86, x: 88, a: 65 };
const SHIFT = 8, ALT = 1, META = 4;

/** One Electron on one fixture: probe (open + hold), fresh udata, SIGTERM + hash check in finally. */
async function session(tag, fixture, body) {
  const udata = join(out, `u7-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `u7-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort(), iport = await freePort();
  const child = spawn(electron, [`--inspect=${iport}`, `--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `u7-out-${tag}`)}`, `--report=${join(out, `u7-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  const closed = new Promise(done => child.on('close', done));
  let cdp, mcdp;
  try {
    cdp = await attach(port);
    // §9.23: the main process (--inspect) runs webContents.copy()/paste(), what the Edit-menu roles call.
    const mt = await wait(async () => (await (await fetch(`http://127.0.0.1:${iport}/json/list`)).json())[0]);
    mcdp = new Cdp(mt.webSocketDebuggerUrl); await mcdp.connect(); await mcdp.send('Runtime.enable');
    const main = async expr => { const r = await mcdp.send('Runtime.evaluate', { includeCommandLineAPI: true, awaitPromise: true, returnByValue: true, expression: `(async () => { const E = (typeof process.getBuiltinModule === 'function' && process.getBuiltinModule('electron')) || require('electron'); const wc = E.BrowserWindow.getAllWindows()[0].webContents; ${expr} })()` });
      if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text); return r.result?.value; };
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
    const key = async (name, modifiers = 0) => {
      const code = KEYS[name];
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await sleep(500);
    };
    /** §9.11: a real ⌘-chord carries the editing command (a plain Meta key fires no DOM clipboard event). */
    const chord = async (letter, command) => {
      const code = KEYS[letter], k = { key: letter, code: `Key${letter.toUpperCase()}`, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers: META };
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', ...k, commands: [command] });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', ...k });
      await sleep(800);
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
    await body({ state, shot, move, click, rightClick, drag, key, chord, pagePoint, mode, cdp, main });
    log('PASS', tag);
  } finally {
    cdp?.close();
    mcdp?.close();
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
/** Client centre of the first element matching `selector` (null when absent or zero-sized); an Inspector control is scrolled into view first. */
const centre = (cdp, selector) => cdp.eval(`(() => { const e = document.querySelector(${q(selector)}); if (!e) return null;
  if (e.closest('#inspector')) e.scrollIntoView({ block: 'nearest' }); const r = e.getBoundingClientRect();
  return r.width && r.height ? { x: r.left + r.width / 2, y: r.top + r.height / 2 } : null; })()`);
/** §9.41: a window-level bubble listener (after the renderer's document listener) records whether the copy was taken. */
const LISTEN = `window.__copyPrevented = null; if (!window.__copyHooked) { window.__copyHooked = true; window.addEventListener('copy', e => { window.__copyPrevented = e.defaultPrevented; }); } 1`;
const IM1 = [72, 142, 172, 192];
const ITEMS_PAGE = { cut: false, copy: false, paste: true, replace: false, duplicate: false, rotateCw: false, rotateCcw: false, flipH: false, flipV: false, crop: false, delete: false };

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
let savedClipboard = null;
try {
  await session('objects6', 'fixtures/p6/objects6.pdf', async ({ state, shot, click, rightClick, key, chord, pagePoint, mode, cdp, main }) => {
    await mode('edit');
    await wait(async () => (await state()).direct.objects === 6, 20_000);
    await sleep(500);
    let im1 = await pagePoint(122, 167);
    await cdp.eval(LISTEN);

    // A. real ⌘C / ⌘V on Im1.
    await click(im1.x, im1.y);
    await wait(async () => names((await state()).direct.selection) === 'Im1');
    const r0 = (await state()).revision;
    await chord('c', 'copy');
    const a0 = await wait(async () => { const v = await state(); return v.direct.clipboard?.count === 1 ? v : null; });
    deq(a0.direct.clipboard.kinds.image, 1, 'A ⌘C clipboard kinds.image');
    deq(a0.revision, r0, 'A ⌘C writes nothing');
    deq(await cdp.eval('window.__copyPrevented'), true, 'A copy event defaultPrevented');
    await chord('v', 'paste');
    const a1 = await wait(async () => { const v = await state(); return v.revision === r0 + 1 ? v : null; });
    deq(a1.direct.objects, 7, 'A ⌘V objects');
    deq(names(a1.direct.selection), 'PA1', 'A ⌘V selection');
    near(sorted(a1, 'PA1')[0], [82, 152, 182, 202], 0.5, 'A PA1 view');
    await shot('u7-a-pasted.png');
    await undo(cdp, state, r0 + 2);

    // A'. the Edit-menu role path: main-process webContents.copy() on Fm1, webContents.paste().
    await key('Escape');
    const fm1 = await pagePoint(74, 600);
    await click(fm1.x, fm1.y);
    await wait(async () => names((await state()).direct.selection) === 'Fm1');
    await main('wc.copy(); return 1'); await sleep(800);
    const m0 = await wait(async () => { const v = await state(); return v.direct.clipboard?.kinds?.form === 1 ? v : null; });
    deq(m0.direct.clipboard.count, 1, "A' webContents.copy() clipboard");
    await main('wc.paste(); return 1');
    const m1 = await wait(async () => { const v = await state(); return v.revision === r0 + 3 ? v : null; });
    deq(m1.direct.objects, 7, "A' webContents.paste() objects");
    near(sorted(m1, 'PA1')[0], [82, 502, 382, 702], 0.5, "A' PA1 form view");
    await undo(cdp, state, r0 + 4);

    // B. real right-click on empty space → the page menu, Paste alone.
    await key('Escape');
    const empty = await pagePoint(500, 450);
    await rightClick(empty.x, empty.y);
    const b = await wait(async () => { const v = await state(); return v.direct.menu?.kind === 'page' ? v : null; });
    deq(b.direct.menu, { name: '', kind: 'page', count: 0, items: ITEMS_PAGE }, 'B page menu');
    await shot('u7-b-page-menu.png');
    await key('Escape');
    await wait(async () => (await state()).direct.menu === null);

    // D. ⌘C with focus in the Inspector's W field: the field's text is copied, the clipboard is not touched.
    // U7_SKIP_D=1 skips it (it writes the OS clipboard; §9.41: fable51 runs it while the human is away).
    if (process.env.U7_SKIP_D) { log('D skipped (U7_SKIP_D): the OS clipboard is not touched'); return; }
    im1 = await pagePoint(122, 167);
    await click(im1.x, im1.y);
    await wait(async () => names((await state()).direct.selection) === 'Im1');
    const before = (await state()).direct.clipboard;
    savedClipboard = execFileSync('pbpaste', { encoding: 'utf8' });
    log('D touches the OS clipboard (saved, restored in finally)');
    const w = await cdp.eval(`(() => { const e = document.getElementById('content-box-w'); e.scrollIntoView({ block: 'nearest' }); e.focus(); e.select(); return { value: e.value, active: document.activeElement.id }; })()`);
    deq(w.active, 'content-box-w', 'D focus');
    await cdp.eval(LISTEN);
    await chord('c', 'copy');
    deq(await cdp.eval('window.__copyPrevented'), false, 'D copy event not prevented');
    deq((await state()).direct.clipboard, before, 'D direct.clipboard unchanged');
    deq(execFileSync('pbpaste', { encoding: 'utf8' }), w.value, 'D OS clipboard holds the W text');
  });

  await session('direct3', 'fixtures/p6/direct3.pdf', async ({ state, shot, click, key, pagePoint, mode, cdp }) => {
    await mode('edit');
    await wait(async () => (await state()).direct.objects === 3, 20_000);
    await sleep(500);
    const run = await pagePoint(100.78, 386.57);
    const im1 = await pagePoint(122, 167, false);
    // C. Shift-click a run and an image → one group box, handles hidden.
    await click(run.x, run.y, SHIFT);
    await wait(async () => names((await state()).direct.selection) === 'Move me');
    await click(im1.x, im1.y, SHIFT);
    const c0 = await wait(async () => { const v = await state(); return names(v.direct.selection) === 'Move me,Im1' ? v : null; });
    deq(c0.direct.selection.map(m => m.kind), ['text', 'image'], 'C member kinds');
    const group = await cdp.eval(`(() => { const boxes = [...document.querySelectorAll('#hit-layer .group-box')];
      const handles = boxes.flatMap(b => [...b.querySelectorAll('.hit-handle')]);
      return { boxes: boxes.length, shown: handles.filter(h => getComputedStyle(h).display !== 'none' && getComputedStyle(h).visibility !== 'hidden').length,
        multi: document.querySelectorAll('#hit-layer .multi').length }; })()`);
    if (group.boxes !== 1 || group.shown !== 0 || group.multi < 2) throw new Error(`C group box: ${q(group)}`);
    log('ok C one .group-box, handles hidden, members .multi', q(group));
    await shot('u7-c-mixed-group.png');
    const r0 = c0.revision;
    await key('ArrowRight');
    const c1 = await wait(async () => { const v = await state(); return v.revision === r0 + 1 ? v : null; });
    deq({ kind: c1.content.edited?.kind, action: c1.content.edited?.action, count: c1.content.edited?.count }, { kind: 'selection', action: 'move', count: 2 }, 'C ArrowRight edited');
    near(sorted(c1, 'Im1')[0], [73, 142, 173, 192], 0.5, 'C Im1 +1');
    if (!c1.content.bounds.some(b => Math.abs(b.bounds[0] - 73) <= 0.5 && Math.abs(b.bounds[1] - 376.95) <= 0.5)) throw new Error(`C run not moved: ${q(c1.content.bounds)}`);
    log('ok C "Move me" +1');
    await undo(cdp, state, r0 + 2);
  });

  await session('fielddefs', 'fixtures/p3/field-defs.pdf', async ({ state, shot, click, chord, pagePoint, mode, cdp }) => {
    await mode('forms');
    await wait(async () => (await state()).fields?.count === 5, 20_000);
    await sleep(500);
    const plain = await pagePoint(132, 259.5);
    // E. Fill forms: Shift-click plain, real ⌘C / ⌘V.
    await click(plain.x, plain.y, SHIFT);
    await wait(async () => names((await state()).direct.selection) === 'plain');
    const r0 = (await state()).revision;
    await chord('c', 'copy');
    await wait(async () => (await state()).direct.clipboard?.kinds?.field === 1);
    await chord('v', 'paste');
    const e = await wait(async () => { const v = await state(); return v.revision === r0 + 1 ? v : null; });
    deq(e.fields.count, 6, 'E ⌘V fields.count');
    near(e.fields.defs['plain copy']?.rect, [82, 257, 202, 282], 0.5, 'E plain copy rect');
    await shot('u7-e-field-paste.png');
    await undo(cdp, state, r0 + 2);
  });
  log('PASS all');
} catch (err) {
  console.error('FAIL', err?.stack ?? err);
  process.exitCode = 1;
} finally {
  if (savedClipboard !== null) { execFileSync('pbcopy', { input: savedClipboard }); log('OS clipboard restored'); }
}
