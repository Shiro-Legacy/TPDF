#!/usr/bin/env node
/**
 * Debt sweep 2 GUI evidence (contract-debt2.md §3.6 as amended by §9.8, §9.9, §9.11): real input on direct3, Edit text.
 * Launch/session()/Cdp/pagePoint()/click()/shot() machinery copied from shot-direct9.mjs (itself shot-direct8.mjs:1-200).
 * A: real CDP Shift-click on "Move me" (100.78,386.57) and on Im1 (122,167); real osascript ⌘X (Edit ▸ Cut, role) → within 5 s
 *    direct.objects 2 (else FAIL), #edit-status exactly "Cut 1 item; 1 text run stays (it cannot be deleted)", direct.clipboard.count 2
 *    (review-out/debt2-a-cut.png); real osascript ⌘Z (Edit ▸ Undo, ⌘Z custom item) → direct.objects 3.
 * B: a real CDP drag of Im1 by (0, 13 px) with ⌘ held on the CDP events only (no snap) → a float placement (else FAIL "no float to round");
 *    click Im1; #content-box-x/-y/-w/-h each match /^-?\d+(\.\d{1,2})?$/ and equal round2 of placement[0], [1], [2]−[0], [3]−[1];
 *    form checkValidity() gate; click #content-apply-object unchanged → revision and dirty unchanged (§9.8 passthrough);
 *    real click #content-box-w, ⌘A, insertText 120, real Apply → direct.active.placement ≈ [p0,p1,p0+120,p3] ±2e-4 (never
 *    content.objects[].box, rounded by STATE); osascript ⌘Z → pre-edit placement; osascript ⌘Z again restores the drag.
 * Every osascript call addresses the Electron this script spawned by unix id and checks it is frontmost in the same call; otherwise
 * nothing is typed: "blocked: frontmost <name>", exit 1 (never retried). Exit 2 + OSA-UNAVAILABLE when System Events refuses (-1712/1002).
 * Touches no OS clipboard (⌘X of page objects goes to the worker clipboard). Not part of npm test. Run after `npm run build`, under
 * .quad/run.lock: node .quad/shared/p6/checks/shot-debt2.mjs
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
  const udata = join(out, `debt2-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `debt2-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `debt2-out-${tag}`)}`, `--report=${join(out, `debt2-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`${label}: ${q(got)} vs ${q(want)}`);
  log('ok', label, q(got));
};
class OsaUnavailable extends Error {}
/** One osascript call: bring the spawned Electron (by unix id) to the front, then ⌘<letter> only if it is frontmost. */
function commandKey(pid, letter) {
  const proc = `(first process whose unix id is ${pid})`;
  try {
    const script = [
      'tell application "System Events"',
      `tell ${proc} to set frontmost to true`,
      'delay 0.4',
      `if frontmost of ${proc} then`,
      `keystroke "${letter}" using command down`,
      'else',
      'error "not frontmost: " & (name of first process whose frontmost is true)',
      'end if',
      'end tell',
    ];
    execFileSync('osascript', script.flatMap(line => ['-e', line]), { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    log('osascript ⌘' + letter.toUpperCase(), 'pid', pid);
  } catch (err) {
    const text = String(err.stderr ?? err.message);
    if (/-1712|1002|not allowed assistive|not authori[sz]ed/i.test(text)) throw new OsaUnavailable(text.trim());
    const front = text.match(/not frontmost: ([^\n"]*)/)?.[1];
    throw new Error(front !== undefined ? `blocked: frontmost ${front}` : `osascript: ${text.trim()}`);
  }
}
const round2 = v => Math.round(v * 100) / 100;
const TWO = /^-?\d+(\.\d{1,2})?$/;
const CUT = 'Cut 1 item; 1 text run stays (it cannot be deleted)';

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
try {
  await session('a', 'fixtures/p6/direct3.pdf', async ({ state, shot, click, drag, pagePoint, mode, cdp, child }) => {
    await mode('edit');
    await wait(async () => (await state()).direct?.objects === 3, 20_000);
    await sleep(500);
    // A. Shift-click the run and Im1, real ⌘X, real ⌘Z.
    const run = await pagePoint(100.78, 386.57);
    let im1 = await pagePoint(122, 167, false);
    await click(run.x, run.y, SHIFT);
    await wait(async () => (await state()).direct.selection.map(m => m.name).join() === 'Move me');
    await click(im1.x, im1.y, SHIFT);
    await wait(async () => (await state()).direct.selection.map(m => m.name).join() === 'Move me,Im1');
    const r0 = (await state()).revision;
    commandKey(child.pid, 'x');
    let a;
    try { a = await wait(async () => { const v = await state(); return v.direct.objects === 2 ? v : null; }, 5_000); }
    catch { throw new Error(`A ⌘X: direct.objects stayed ${(await state()).direct.objects} (the Edit ▸ Cut role did not reach the page)`); }
    deq(await cdp.eval(`document.getElementById('edit-status').textContent`), CUT, 'A #edit-status');
    deq(a.direct.clipboard.count, 2, 'A direct.clipboard.count');
    deq(a.revision, r0 + 1, 'A ⌘X revision');
    await shot('debt2-a-cut.png');
    commandKey(child.pid, 'z');
    const undone = await wait(async () => { const v = await state(); return v.direct.objects === 3 ? v : null; }, 5_000);
    deq(undone.revision, r0 + 2, 'A ⌘Z revision');
    deq(undone.dirty, false, 'A ⌘Z dirty');

    // B. A real drag leaves a float placement; the Inspector shows it to 2 decimals; an unchanged Apply writes nothing.
    im1 = await pagePoint(122, 167, false);
    await drag(im1.x, im1.y, 0, 13, META);
    const moved = await wait(async () => { const v = await state(); return v.revision === r0 + 3 ? v : null; }, 5_000);
    const again = await pagePoint(122, (moved.content.objects.find(o => o.name === 'Im1').view[1] + moved.content.objects.find(o => o.name === 'Im1').view[3]) / 2, false);
    await click(again.x, again.y);
    const sel = await wait(async () => { const v = await state(); return v.direct.active?.name === 'Im1' ? v : null; }, 5_000);
    const placement = sel.direct.active.placement;
    // Unrounded source only: content.objects[].box is rounded to 2 decimals by STATE (renderer.ts contentSummary).
    log('B placement after drag', q(placement));
    if (!Array.isArray(placement) || !placement.some(v => round2(v) !== v)) throw new Error(`B no float to round: ${q(placement)}`);
    const shown = await cdp.eval(`['x','y','w','h'].map(k => document.getElementById('content-box-' + k).value)`);
    const want = [placement[0], placement[1], placement[2] - placement[0], placement[3] - placement[1]].map(round2);
    shown.forEach((value, k) => {
      if (!TWO.test(value)) throw new Error(`B #content-box-${'xywh'[k]} ${q(value)} fails ${TWO}`);
      if (Number(value) !== want[k]) throw new Error(`B #content-box-${'xywh'[k]} ${q(value)} !== round2 ${want[k]}`);
    });
    log('ok B Inspector rounded', q(shown), 'placement', q(placement));
    const invalid = await cdp.eval(`(() => { const f = document.getElementById('content-object-form');
      return f.checkValidity() ? null : [...f.elements].filter(e => e.willValidate && !e.checkValidity()).map(e => e.id + ': ' + e.validationMessage).join('; ') || 'form invalid'; })()`);
    if (invalid !== null) throw new Error(`B #content-object-form checkValidity false: ${invalid}`);
    const before = await state();
    const applyAt = () => cdp.eval(`(() => { const e = document.getElementById('content-apply-object'); e.scrollIntoView({ block: 'nearest' });
      const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width, disabled: e.disabled }; })()`);
    let apply = await applyAt();
    if (apply.w > 0 && !apply.disabled) await click(apply.x, apply.y);
    else throw new Error(`B #content-apply-object not clickable ${q(apply)}`);
    await sleep(1500);
    const after = await state();
    deq([after.revision, after.dirty], [before.revision, before.dirty], 'B unchanged Apply: revision and dirty');
    // The unchanged Apply submits and closes the object form (R-B1 fix): reselect Im1 so the form is visible again.
    const formShown = () => cdp.eval(`(() => { const r = document.getElementById('content-box-w').getBoundingClientRect(); return r.width > 0 && r.height > 0; })()`);
    if (!(await formShown())) {
      await click(again.x, again.y);
      await wait(async () => { const v = await state(); return v.direct.active?.name === 'Im1' && (await formShown()) ? v : null; }, 5_000);
    }
    near((await state()).direct.active.placement, placement, 2e-4, 'B reselected Im1 keeps the unrounded placement');
    // §9.8 edited field: real click on W, ⌘A, type 120, real Apply → only the right edge moves; x/y/h pass through unrounded.
    const w = await cdp.eval(`(() => { const r = document.getElementById('content-box-w').getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
    await click(w.x, w.y);
    for (const type of ['rawKeyDown', 'keyUp']) await cdp.send('Input.dispatchKeyEvent', { type, key: 'a', code: 'KeyA', windowsVirtualKeyCode: 65, nativeVirtualKeyCode: 65, modifiers: META, ...(type === 'rawKeyDown' ? { commands: ['selectAll'] } : {}) });
    await cdp.send('Input.insertText', { text: '120' });
    deq(await cdp.eval(`document.getElementById('content-box-w').value`), '120', 'B #content-box-w after ⌘A + type');
    apply = await applyAt();
    if (apply.w > 0 && !apply.disabled) await click(apply.x, apply.y);
    else throw new Error(`B #content-apply-object not clickable after W edit ${q(apply)}`);
    const edited = await wait(async () => { const v = await state(); return v.revision === before.revision + 1 && v.direct.active?.name === 'Im1' ? v : null; }, 5_000);
    near(edited.direct.active.placement, [placement[0], placement[1], placement[0] + 120, placement[3]], 2e-4, 'B W=120 direct.active.placement');
    commandKey(child.pid, 'z');
    let unedited = await wait(async () => { const v = await state(); return v.revision === before.revision + 2 ? v : null; }, 5_000);
    if (unedited.direct.active?.name !== 'Im1') {
      await click(again.x, again.y);
      unedited = await wait(async () => { const v = await state(); return v.direct.active?.name === 'Im1' ? v : null; }, 5_000);
    }
    near(unedited.direct.active.placement, placement, 2e-4, 'B ⌘Z restores the pre-edit placement');
    commandKey(child.pid, 'z');
    const restored = await wait(async () => { const v = await state(); return v.revision === before.revision + 3 ? v : null; }, 5_000);
    deq(restored.content.objects.find(o => o.name === 'Im1').box, [72, 600, 172, 650], 'B ⌘Z restores Im1');
  });
  const ok = (hashes().match(/: OK$/gm) ?? []).length;
  if (ok !== 9) throw new Error(`fixtures.sha256: ${ok} OK, want 9`);
  log('fixtures 9 OK');
  log('PASS all');
} catch (err) {
  if (err instanceof OsaUnavailable) { console.error('OSA-UNAVAILABLE', err.message); process.exitCode = 2; }
  else { console.error('FAIL', err?.stack ?? err); process.exitCode = 1; }
}
