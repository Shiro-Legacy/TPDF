#!/usr/bin/env node
/**
 * P6 U8 GUI evidence (contract-p6-unit8.md §9.11): duplicate text added with REAL macOS ⌘Return stays editable per run.
 * Launch/session()/pagePoint()/click()/shot() machinery copied from shot-direct8.mjs:1-200 (no --inspect, no CDP chord).
 * direct3 p0, Edit text: CDP click (300,400) → add editor, CDP Input.insertText "Twin", osascript ⌘Return (addressed to the spawned
 * Electron by unix id; refused unless it is frontmost) → active null, revision 1; same at (300,300) → revision 2; CDP clicks at the
 * twins' centres (312,404) and (312,304) → kind text, value "Twin", distinct targetIds; shot review-out/u8-a-twins.png.
 * Exit 2 + OSA-UNAVAILABLE when osascript cannot drive System Events (-1712 / 1002); exit 1 + "blocked: frontmost <name>" when
 * another app is frontmost (nothing is typed). Not part of npm test. Run after `npm run build`, under .quad/run.lock:
 * node .quad/shared/p6/checks/shot-direct9.mjs
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
  const udata = join(out, `u8-review-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `u8-review-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `u8-review-out-${tag}`)}`, `--report=${join(out, `u8-review-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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

// ---- review body (renderer + real-mouse lens, U8) ----
const R = {};
const rec = (k, v) => { R[k] = v; log('REC', k, q(v)); };
log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
try {
  await session('r', 'fixtures/p6/direct3.pdf', async ({ state, shot, click, key, pagePoint, mode, cdp }) => {
    await mode('edit');
    await wait(async () => (await state()).direct?.targets === 5);
    const editorBoxPt = async () => cdp.eval(`(() => { const e = document.activeElement, c = document.getElementById('canvas'); const r = c.getBoundingClientRect(), s = r.width / 612, b = e.getBoundingClientRect(); return { tag: e.tagName, cls: e.className, box: [(b.left - r.left) / s, (b.top - r.top) / s, (b.right - r.left) / s, (b.bottom - r.top) / s].map(v => Math.round(v * 100) / 100) }; })()`);
    const status = () => cdp.eval(`({ edit: document.getElementById('edit-status')?.textContent ?? null, error: document.getElementById('error')?.textContent ?? null })`);
    const realClickEl = async id => { const p = await cdp.eval(`(() => { const b = document.getElementById(${q(id)}).getBoundingClientRect(); return { x: b.left + b.width / 2, y: b.top + b.height / 2 }; })()`); await click(p.x, p.y); };
    const add = async (x, y, text, first) => {
      const p = await pagePoint(x, y, first);
      await click(p.x, p.y);
      const o = await wait(async () => { const v = await state(); return v.direct.active?.kind === 'add' ? v : null; }, 10_000);
      await cdp.send('Input.insertText', { text });
      await wait(async () => (await state()).direct.active?.draft === text);
      const rev = o.revision;
      await key('Enter', META);
      return wait(async () => { const v = await state(); return v.revision === rev + 1 && v.direct.active === null ? v : null; }, 10_000);
    };
    const open = async (x, y, label) => {
      const p = await pagePoint(x, y, false);
      await click(p.x, p.y);
      const v = await wait(async () => { const s = await state(); return s.direct.active ? s : null; }, 8_000);
      const a = v.direct.active;
      const eb = await editorBoxPt();
      rec(label, { kind: a.kind, value: a.value, targetId: a.targetId, rect: a.rect, editorBox: eb.box, editorTag: eb.tag });
      return v;
    };
    const s0 = await state(); rec('s0', { targets: s0.direct.targets, rev: s0.revision, dirty: s0.dirty, directKeys: Object.keys(s0.direct).sort(), contentKeys: Object.keys(s0.content ?? {}).sort() });
    const a1 = await add(300, 400, 'Twin', true); rec('add1', { rev: a1.revision, targets: a1.direct.targets, lastAdd: a1.content.lastAdd, status: await status() });
    const a2 = await add(300, 300, 'Twin', false); rec('add2', { rev: a2.revision, targets: a2.direct.targets, lastAdd: a2.content.lastAdd, status: await status() });
    rec('boundsTwins', a2.content.bounds.filter(b => [a1.content.lastAdd.targetId, a2.content.lastAdd.targetId].includes(b.targetId)));
    rec('keysAfter', { directKeys: Object.keys(a2.direct).sort(), contentKeys: Object.keys(a2.content).sort() });
    await shot('review-u8-r1-twins-added.png');
    const A = await open(312, 404, 'openA'); await shot('review-u8-r2-editorA.png'); await key('Escape');
    const B = await open(312, 304, 'openB'); await shot('review-u8-r3-editorB.png');
    // type "Twin 2" replacing the value
    await cdp.eval(`(() => { const e = document.activeElement; e.select?.(); })()`);
    await cdp.send('Input.insertText', { text: 'Twin 2' });
    const d = await state(); rec('draftB', d.direct.active);
    await key('Enter', META); await sleep(1500);
    const t2 = await state(); rec('commitTwin2', { rev: t2.revision, active: t2.direct.active, edited: t2.content.edited, refused: t2.content.refused, error: t2.error, status: await status() });
    await shot('review-u8-r4-after-twin2.png');
    if (t2.direct.active) { await key('Escape'); await sleep(500); }
    if (t2.revision === 2) {
      await open(312, 304, 'reopenB');
      await cdp.eval(`document.activeElement.select?.()`); await cdp.send('Input.insertText', { text: 'Pair' });
      await key('Enter', META); await sleep(1500);
      const p = await state(); rec('commitPair', { rev: p.revision, edited: p.content.edited, active: p.direct.active });
    }
    const B2 = await open(312, 304, 'clickB-after'); await key('Escape');
    const A2 = await open(312, 404, 'clickA-after'); await key('Escape');
    // (b) Move me
    const m = await add(300, 200, 'Move me', false); rec('addMoveMe', { rev: m.revision, targets: m.direct.targets, lastAdd: m.content.lastAdd, status: await status() });
    await open(100.78, 386.57, 'openOrigMoveMe'); await shot('review-u8-r5-orig-moveme.png'); await key('Escape');
    await open(324, 204, 'openCopyMoveMe'); await shot('review-u8-r6-copy-moveme.png'); await key('Escape');
    // undo all with the real #undo button
    let u = await state();
    while (u.revision > 0 && u.dirty && u.canUndo !== false) {
      const before = u.revision; await realClickEl('undo'); await sleep(900); u = await state();
      rec('undo@' + before, { rev: u.revision, targets: u.direct.targets, dirty: u.dirty });
      if (u.revision === before) break;
      if (Object.keys(R).filter(k => k.startsWith('undo@')).length > 8) break;
    }
    await shot('review-u8-r7-after-undos.png');
    // (d) overprint via the Text panel (same text, same default point twice)
    const tabVisible = await cdp.eval(`!!document.getElementById('tab-text')?.getClientRects().length`);
    if (!tabVisible) await realClickEl('inspector-toggle');
    await realClickEl('tab-text'); await sleep(500);
    for (const i of [1, 2]) {
      await realClickEl('content-add-text');
      await cdp.eval(`document.getElementById('content-add-text').select()`);
      await cdp.send('Input.insertText', { text: 'Solo' });
      const r0 = (await state()).revision;
      await realClickEl('content-add-go');
      const v = await wait(async () => { const s = await state(); return s.revision === r0 + 1 ? s : null; }, 10_000);
      rec('panelAdd' + i, { rev: v.revision, targets: v.direct.targets, lastAdd: v.content.lastAdd, status: await status(), announcement: v.announcement });
    }
    await shot('review-u8-r8-overprint.png');
    // Help text
    const help = await cdp.eval(`document.getElementById('help-dialog').textContent.replace(/\\s+/g, ' ')`);
    rec('help', {
      removedU7: help.includes('A pasted run whose text already appears as a whole line'),
      oldRepeats: help.includes('exactly repeats an existing line'),
      newSentence: help.includes('Text placed exactly on top of the same text, and text inside a form drawn more than once, cannot be edited.'),
      reason: help.includes('That text cannot be told apart from another run on the page.') || help.includes('Appears more than once; cannot be told apart'),
      hits: ['whole line', 'repeats', 'told apart', 'more than once', 'duplicate'].map(w => [w, [...help.matchAll(new RegExp('[^.]*' + w + '[^.]*\\\\.', 'g'))].map(x => x[0].trim())]),
    });
  });
  log('PASS all');
} catch (err) {
  console.error('FAIL', err?.stack ?? err); process.exitCode = 1;
} finally {
  await writeFile(join(out, 'review-u8-renderer-probe.json'), JSON.stringify(R, null, 1));
}
