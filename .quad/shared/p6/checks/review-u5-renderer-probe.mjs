#!/usr/bin/env node
/**
 * P6 U5 review, renderer + human UX lens: real CDP mouse/keys on fixtures/p6/direct3.pdf (Edit text) and fixtures/form2.pdf.
 * Harness copied from shot-direct6.mjs. Every check logs `CHECK <id> ok|FAIL <detail>`; it never throws on a failed check,
 * so one run reports everything. Shots → review-out/review-u5-*.png. Run after `npm run build`, under .quad/run.lock:
 *   node .quad/shared/p6/checks/review-u5-renderer-probe.mjs      (ONLY=sel,save,form2,fix,esc,d0,d6,d8,d9,d10 selects sessions)
 * P6 U6 §9.32: d0, d6, d8, d9 and d10 each run in their own session (fresh Electron, mode edit, 3 objects) and are the default.
 */
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { existsSync } from 'node:fs';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../../../../', import.meta.url));
const checksDir = fileURLToPath(new URL('./', import.meta.url));
const out = join(checksDir, 'review-out');
const scratch = join(root, '.quad/scratch/p6u5/review-renderer');
const electron = createRequire(join(root, 'package.json'))('electron');
const ONLY = (process.env.ONLY ?? 'd0,d6,d8,d9,d10').split(',');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const log = (...a) => console.log(...a);
const q = s => JSON.stringify(s);
let fails = 0;
const check = (id, ok, detail) => { if (!ok) fails++; log('CHECK', id, ok ? 'ok' : 'FAIL', typeof detail === 'string' ? detail : q(detail)); return ok; };
function freePort() {
  return new Promise((resolve, reject) => {
    const s = createServer();
    s.listen(0, '127.0.0.1', () => { const port = s.address().port; s.close(err => err ? reject(err) : resolve(port)); });
    s.on('error', reject);
  });
}
async function wait(fn, ms = 15_000, step = 150) {
  const t0 = Date.now(); let last;
  while (Date.now() - t0 < ms) {
    try { last = await fn(); } catch (err) { last = err; }
    if (last && last !== false && !(last instanceof Error)) return last;
    await sleep(step);
  }
  return null;
}
class Cdp {
  constructor(url) { this.url = url; this.n = 0; this.pending = new Map(); }
  async connect() {
    this.ws = new WebSocket(this.url);
    await new Promise((resolve, reject) => { this.ws.addEventListener('open', resolve, { once: true }); this.ws.addEventListener('error', reject, { once: true }); });
    this.ws.addEventListener('message', ev => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && this.pending.has(msg.id)) { const { resolve, reject } = this.pending.get(msg.id); this.pending.delete(msg.id); if (msg.error) reject(new Error(msg.error.message)); else resolve(msg.result); }
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
  close() { try { this.ws.close(); } catch { /* closed */ } }
}
async function attach(port) {
  const page = await wait(async () => {
    const res = await fetch(`http://127.0.0.1:${port}/json/list`); const list = await res.json();
    return list.find(t => t.webSocketDebuggerUrl && (t.type === 'page' || /index\.html/.test(t.url || '')));
  }, 30_000);
  const cdp = new Cdp(page.webSocketDebuggerUrl); await cdp.connect();
  await cdp.send('Runtime.enable'); await cdp.send('Page.enable');
  return cdp;
}
const hold = (n, ms) => Array.from({ length: n }, () => ({ cmd: 'wait', ms }));
const hashes = () => execFileSync('shasum', ['-a', '256', '-c', join(checksDir, 'fixtures.sha256')], { cwd: root, encoding: 'utf8' });
const KEYS = { Enter: 13, Tab: 9, Escape: 27, ArrowRight: 39, ArrowDown: 40, ArrowUp: 38, ArrowLeft: 37, Home: 36, End: 35, Delete: 46, F10: 121, PageDown: 34, ' ': 32, '=': 187 };
const CODES = { ' ': 'Space', '=': 'Equal' };
const SHIFT = 8, ALT = 1, CTRL = 2, META = 4;
const near = (a, b, tol = 0.6) => Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((v, i) => Math.abs(v - b[i]) <= tol);

async function session(tag, fixture, extraOpen, body) {
  const udata = join(out, `u5-review-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `review-u5-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture), ...extraOpen }, ...hold(80, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `review-u5-out-${tag}`)}`, `--report=${join(out, `review-u5-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  const closed = new Promise(done => child.on('close', done));
  let cdp;
  try {
    cdp = await attach(port);
    try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch { /* ignore */ }
    const state = () => cdp.eval('window.pdf4allState()');
    const shot = async name => { const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }); await writeFile(join(out, name), Buffer.from(data, 'base64')); log('shot', name); };
    const mouse = (type, x, y, extra = {}) => cdp.send('Input.dispatchMouseEvent', { type, x, y, ...extra });
    const move = (x, y, modifiers = 0) => mouse('mouseMoved', x, y, { modifiers });
    const click = async (x, y, modifiers = 0, ms = 600) => {
      await move(x, y, modifiers); await sleep(80);
      await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1, modifiers });
      await mouse('mouseReleased', x, y, { button: 'left', buttons: 0, clickCount: 1, modifiers });
      await sleep(ms);
    };
    const rightClick = async (x, y) => {
      await move(x, y); await sleep(80);
      await mouse('mousePressed', x, y, { button: 'right', buttons: 2, clickCount: 1 });
      await mouse('mouseReleased', x, y, { button: 'right', buttons: 0, clickCount: 1 });
      await sleep(500);
    };
    const drag = async (x, y, dx, dy, modifiers = 0, during, beforeRelease) => {
      await move(x, y, modifiers); await sleep(120);
      await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1, modifiers });
      for (let i = 1; i <= 8; i++) await mouse('mouseMoved', x + dx * i / 8, y + dy * i / 8, { button: 'left', buttons: 1, modifiers });
      const seen = during ? await during() : undefined;
      if (beforeRelease) await beforeRelease();
      await mouse('mouseReleased', x + dx, y + dy, { button: 'left', buttons: 0, clickCount: 1, modifiers });
      await sleep(1500);
      return seen;
    };
    const key = async (name, modifiers = 0, ms = 500) => {
      const code = KEYS[name];
      const text = name.length === 1 ? name : undefined;
      await cdp.send('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', key: name, code: CODES[name] ?? name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers, ...(text && !modifiers ? { text } : {}) });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code: CODES[name] ?? name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await sleep(ms);
    };
    /** Page pt (y-down) → client px through the canvas box; `scroll` centres the point first. */
    const pagePoint = (x, y, scroll = true, widthPt = 612) => cdp.eval(`(() => {
      const v = document.getElementById('viewport'), c = document.getElementById('canvas');
      let r = c.getBoundingClientRect(); const s = r.width / ${widthPt};
      if (${scroll}) v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
      r = c.getBoundingClientRect();
      return { x: r.left + ${x} * s, y: r.top + ${y} * s, s };
    })()`);
    const elCenter = id => cdp.eval(`(() => { const r = document.getElementById(${q(id)}).getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
    const clickEl = async id => { const p = await elCenter(id); await click(p.x, p.y, 0, 1200); };
    const mode = async name => cdp.eval(`document.getElementById('mode-${name}').click()`);
    await wait(() => cdp.eval(`document.title.includes(${q(fixture.split('/').pop().replace('.pdf', ''))})`), 60_000);
    await sleep(800);
    await body({ state, shot, move, mouse, click, rightClick, drag, key, pagePoint, clickEl, mode, cdp });
    log('DONE', tag);
  } catch (err) {
    fails++; console.error('SESSION ERROR', tag, err?.stack ?? err);
  } finally {
    cdp?.close();
    child.kill('SIGTERM');
    log('electron closed', tag, await closed);
    log('fixtures after', tag, '\n' + hashes());
  }
}

const names = sel => (sel ?? []).map(m => m.name).join(',');
const all = (s, name) => s.content.objects.filter(o => o.name === name).map(o => o.view).sort((a, b) => a[0] - b[0] || a[1] - b[1]);
const DOMQ = `(() => {
  const btns = [...document.querySelectorAll('#hit-layer .object-target')];
  const box = b => [parseFloat(b.style.left), parseFloat(b.style.top), parseFloat(b.style.width), parseFloat(b.style.height)];
  return { btns: btns.map(b => ({ id: b.dataset.targetId, sel: b.classList.contains('selected'), multi: b.classList.contains('multi'), box: box(b) })),
    active: document.activeElement?.dataset?.targetId ?? document.activeElement?.id ?? document.activeElement?.tagName,
    ghosts: document.querySelectorAll('#hit-layer .ghost, .ghost').length,
    menuHidden: document.getElementById('object-menu').hidden,
    live: document.getElementById('edit-status').textContent, error: document.getElementById('error')?.textContent ?? null,
    inspector: { label: document.getElementById('content-object-label').textContent, x: document.getElementById('content-box-x').value, y: document.getElementById('content-box-y').value },
    editors: document.querySelectorAll('.inline-editor').length, page: document.getElementById('page-status')?.textContent };
})()`;
const idOf = (s, name, view) => s.content.objects.find(o => o.name === name && near(o.view, view))?.targetId;

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true }); await mkdir(scratch, { recursive: true });

if (ONLY.includes('sel')) await session('sel', 'fixtures/p6/direct3.pdf', {}, async ({ state, shot, move, mouse, click, rightClick, drag, key, pagePoint, clickEl, mode, cdp }) => {
  const dom = () => cdp.eval(DOMQ);
  await mode('edit');
  await wait(async () => (await state()).direct.objects === 3, 20_000); await sleep(500);
  const s0 = await state();
  const IM1 = [72, 142, 172, 192], IM2 = [400, 52, 460, 92], FM1 = [400, 262, 500, 292];
  const id0 = { Im1: idOf(s0, 'Im1', IM1), Im2: idOf(s0, 'Im2', IM2), Fm1: idOf(s0, 'Fm1', FM1) };
  log('ids', q(id0));

  // (a) click Im1, real Shift-click Im2.
  let p = await pagePoint(122, 167); await sleep(200); await click(p.x, p.y);
  p = await pagePoint(430, 72); await sleep(200); await click(p.x, p.y, SHIFT);
  let s = await state(), d = await dom();
  check('a1 selection Im1,Im2', names(s.direct.selection) === 'Im1,Im2', s.direct.selection);
  check('a1 anchor Im2', s.direct.active?.name === 'Im2', s.direct.active);
  check('a1 targetIds match content.objects', s.direct.selection.every(m => s.content.objects.some(o => o.targetId === m.targetId && o.name === m.name)), s.direct.selection);
  check('a1 two .selected.multi', d.btns.filter(b => b.sel && b.multi).length === 2, d.btns);
  await move(p.x, p.y); await sleep(300);
  const hidden = await cdp.eval(`(() => { const hs = [...document.querySelectorAll('#hit-layer .object-target.multi .hit-handle')]; return { n: hs.length, grips: hs.filter(h => h.classList.contains('hit-grip')).length, shown: hs.filter(h => getComputedStyle(h).display !== 'none').length }; })()`);
  check('a1 handles+grip display:none (hover Im2)', hidden.n === 18 && hidden.grips === 2 && hidden.shown === 0, hidden);
  check('K2 Inspector follows the anchor Im2 (x 400)', d.inspector.x === '400', d.inspector);
  await shot('review-u5-a1-multi.png');
  // Shift-click Im2 again: removes it; focus on Im1's button; Delete deletes Im1 only.
  await click(p.x, p.y, SHIFT);
  s = await state(); d = await dom();
  check('a2 selection Im1', names(s.direct.selection) === 'Im1' && s.direct.active?.name === 'Im1', s.direct.selection);
  check('a2 focus on Im1 button', d.active === s.direct.selection[0]?.targetId, d.active);
  check('K2b Inspector shows Im1 after removal', d.inspector.x === '72', d.inspector);
  const r0 = s.revision;
  await key('Delete', 0, 1500);
  s = await state(); d = await dom();
  check('a3 Delete removes Im1 only', s.direct.objects === 2 && all(s, 'Im1').length === 0 && all(s, 'Im2').length === 1 && s.revision === r0 + 1, { objects: s.direct.objects, edited: s.content.edited, rev: s.revision });
  check('a3 announcement', /Deleted image/.test(d.live), d.live);
  await clickEl('undo'); s = await state();
  check('a3 undo restores 3', s.direct.objects === 3 && s.revision === r0 + 2, { objects: s.direct.objects, rev: s.revision });

  // (b) group: CSS boxes after a 2 px press-release; body drag moves both; Escape mid-drag; edge clamp.
  p = await pagePoint(122, 167); await sleep(200); await click(p.x, p.y);
  const p2 = await pagePoint(430, 72, false); await click(p2.x, p2.y, SHIFT);
  const boxesBefore = (await dom()).btns;
  const rb = (await state()).revision;
  await move(p2.x, p2.y); await sleep(100);
  await mouse('mousePressed', p2.x, p2.y, { button: 'left', buttons: 1, clickCount: 1 });
  await mouse('mouseMoved', p2.x + 2, p2.y, { button: 'left', buttons: 1 });
  await mouse('mouseReleased', p2.x + 2, p2.y, { button: 'left', buttons: 0, clickCount: 1 }); await sleep(800);
  s = await state(); d = await dom();
  check('b1 2px press: no request, no union resize', s.revision === rb && d.btns.every((b, i) => near(b.box, boxesBefore[i].box, 0.01)), { rev: s.revision, before: boxesBefore.map(b => b.box), after: d.btns.map(b => b.box) });
  log('b1 selection after 2px plain press', names(s.direct.selection));
  // body drag 30,20 pt on a group.
  p = await pagePoint(122, 167); await sleep(200); await click(p.x, p.y);
  let q2 = await pagePoint(430, 72, false); await click(q2.x, q2.y, SHIFT);
  const rDrag = (await state()).revision;
  await drag(q2.x, q2.y, 30 * q2.s, 20 * q2.s);
  s = await state(); d = await dom();
  check('b2 one revision, edited objects move count 2', s.revision === rDrag + 1 && s.content.edited?.kind === 'objects' && s.content.edited?.action === 'move' && s.content.edited?.count === 2 && s.content.edited?.name === 'Im1,Im2', { rev: s.revision, edited: s.content.edited });
  check('b2 both moved', near(all(s, 'Im1')[0], [102, 162, 202, 212]) && near(all(s, 'Im2')[0], [430, 72, 490, 112]), { Im1: all(s, 'Im1'), Im2: all(s, 'Im2') });
  check('b2 selection still 2, ids match scan', s.direct.selection.length === 2 && s.direct.selection.every(m => s.content.objects.some(o => o.targetId === m.targetId)), s.direct.selection);
  check('b2 announcement Moved 2 objects', d.live === 'Moved 2 objects', d.live);
  await shot('review-u5-b2-group-drag.png');
  await clickEl('undo'); s = await state();
  check('K1 after undo the selection re-resolves (length 2)', s.direct.selection.length === 2, s.direct.selection);
  check('b2 undo restores', near(all(s, 'Im1')[0], IM1) && near(all(s, 'Im2')[0], IM2), { Im1: all(s, 'Im1'), Im2: all(s, 'Im2') });
  // Escape mid-drag.
  p = await pagePoint(122, 167); await sleep(200); await click(p.x, p.y);
  q2 = await pagePoint(430, 72, false); await click(q2.x, q2.y, SHIFT);
  const bEsc = (await dom()).btns, rEsc = (await state()).revision;
  await drag(q2.x, q2.y, 40 * q2.s, 40 * q2.s, 0, async () => (await dom()).btns.map(b => b.box), async () => { await key('Escape', 0, 200); });
  s = await state(); d = await dom();
  check('b3 Escape mid-drag: no request, members back', s.revision === rEsc && d.btns.every((b, i) => near(b.box, bEsc[i].box, 0.01)) && s.direct.gesture === null, { rev: s.revision, after: d.btns.map(b => b.box) });
  log('b3 selection after Escape mid-drag', names(s.direct.selection));
  // Edge clamp: drag +300,-200 → Im2 at the top-right corner [552,0,612,40].
  p = await pagePoint(122, 167); await sleep(200); await click(p.x, p.y);
  q2 = await pagePoint(430, 72, false); await click(q2.x, q2.y, SHIFT);
  const rEdge = (await state()).revision, refusedBefore = (await state()).content.refused.length;
  await drag(q2.x, q2.y, 300 * q2.s, -200 * q2.s);
  s = await state();
  check('b4 clamp at the page edge', s.revision === rEdge + 1 && near(all(s, 'Im2')[0], [552, 0, 612, 40]) && near(all(s, 'Im1')[0], [224, 90, 324, 140]) && s.content.refused.length === refusedBefore, { rev: s.revision, Im1: all(s, 'Im1'), Im2: all(s, 'Im2'), refused: s.content.refused });
  await clickEl('undo');
  s = await state();
  check('b4 undo pristine', s.revision === rEdge + 2 && near(all(s, 'Im1')[0], IM1) && near(all(s, 'Im2')[0], IM2) && all(s, 'Im2').length === 1, { rev: s.revision, Im1: all(s, 'Im1'), Im2: all(s, 'Im2') });

  // (c) ⌥-drag Im2 single.
  await key('Escape'); await sleep(300);
  q2 = await pagePoint(430, 72); await sleep(200);
  const before = new Set((await state()).content.objects.map(o => o.targetId));
  const mid = await drag(q2.x, q2.y, 60 * q2.s, 40 * q2.s, ALT, async () => ({ g: (await state()).direct.gesture, ghosts: (await dom()).ghosts }));
  s = await state(); d = await dom();
  check('c1 mid-drag gesture duplicate + one ghost', mid.g?.kind === 'duplicate' && mid.ghosts === 1, mid);
  const copyId = idOf(s, 'Im2', [460, 92, 520, 132]);
  check('c1 copy + original, 4 objects, ghost gone', s.direct.objects === 4 && !!copyId && !!idOf(s, 'Im2', IM2) && d.ghosts === 0, { objects: s.direct.objects, Im2: all(s, 'Im2'), ghosts: d.ghosts });
  check('c1 selection is the copy (new id §9.18)', s.direct.selection.length === 1 && s.direct.selection[0].targetId === copyId && !before.has(copyId) && s.direct.active?.targetId === copyId, { sel: s.direct.selection, active: s.direct.active, copyId });
  check('c1 edited single duplicate, no count', s.content.edited?.kind === 'image' && s.content.edited?.action === 'duplicate' && !('count' in (s.content.edited ?? {})), s.content.edited);
  check('c1 announcement', d.live === 'Duplicated image', d.live);
  await shot('review-u5-c1-alt-duplicate.png');
  await clickEl('undo'); s = await state();
  check('c1 undo → 3', s.direct.objects === 3, s.direct.objects);
  log('c1 selection after undo', q(s.direct.selection));
  // ⌥-drag on a group.
  p = await pagePoint(122, 167); await sleep(200); await click(p.x, p.y);
  q2 = await pagePoint(430, 72, false); await click(q2.x, q2.y, SHIFT);
  const beforeG = new Set((await state()).content.objects.map(o => o.targetId));
  await drag(p.x, p.y, 20 * p.s, 20 * p.s, ALT);
  s = await state(); d = await dom();
  check('c2 group ⌥-drag duplicates both', s.direct.objects === 5 && s.content.edited?.kind === 'objects' && s.content.edited?.action === 'duplicate' && s.content.edited?.count === 2 && all(s, 'Im1').some(v => near(v, [92, 162, 192, 212])) && all(s, 'Im2').some(v => near(v, [420, 72, 480, 112])), { objects: s.direct.objects, edited: s.content.edited, Im1: all(s, 'Im1'), Im2: all(s, 'Im2') });
  check('c2 selection = the two copies', s.direct.selection.length === 2 && s.direct.selection.every(m => !beforeG.has(m.targetId)) && d.ghosts === 0, s.direct.selection);
  check('c2 announcement', d.live === 'Duplicated 2 objects', d.live);
  await clickEl('undo'); s = await state();
  check('c2 undo → 3', s.direct.objects === 3, s.direct.objects);

  // (d) context menu.
  await key('Escape');
  const empty = await pagePoint(300, 500); await sleep(300);
  const fm = await pagePoint(450, 277, false), i1 = await pagePoint(122, 167, false), i2 = await pagePoint(430, 72, false);
  log('points', q({ empty, fm, i1, i2 }));
  await rightClick(fm.x, fm.y);
  s = await state(); d = await dom();
  const menuBox = await cdp.eval(`(() => { const m = document.getElementById('object-menu').getBoundingClientRect(), v = document.getElementById('viewport').getBoundingClientRect(); return { m: [m.left, m.top, m.right, m.bottom], v: [v.left, v.top, v.right, v.bottom], replace: [document.getElementById('object-menu-replace').disabled, document.getElementById('object-menu-replace').getAttribute('aria-disabled')], focus: document.activeElement.id }; })()`);
  check('d1 menu open at pointer inside viewport', !d.menuHidden && Math.abs(menuBox.m[0] - fm.x) < 2 && Math.abs(menuBox.m[1] - fm.y) < 2 && menuBox.m[2] <= menuBox.v[2] && menuBox.m[3] <= menuBox.v[3], menuBox);
  check('d1 Replace disabled DOM + STATE; focus Duplicate', menuBox.replace[0] === true && menuBox.replace[1] === 'true' && s.direct.menu?.items?.replace === false && menuBox.focus === 'object-menu-duplicate', { menuBox, menu: s.direct.menu });
  await shot('review-u5-d1-menu-fm1.png');
  const focusId = () => cdp.eval('document.activeElement.id');
  await key('ArrowDown', 0, 200); const f1 = await focusId();
  await key('ArrowDown', 0, 200); const f2 = await focusId();
  await key('ArrowUp', 0, 200); const f3 = await focusId();
  check('d2 arrows cycle enabled items only', f1 === 'object-menu-delete' && f2 === 'object-menu-duplicate' && f3 === 'object-menu-delete', [f1, f2, f3]);
  await key('Home', 0, 300); const f4 = await focusId(); await key('End', 0, 300); const f5 = await focusId();
  s = await state();
  check('d2 Home/End stay in the menu, page stays 0', f4 === 'object-menu-duplicate' && f5 === 'object-menu-delete' && s.page === 0 && s.direct.menu !== null, { f4, f5, page: s.page, menu: s.direct.menu, pageStatus: (await dom()).page });
  await key('Escape', 0, 300);
  s = await state(); d = await dom();
  check('d3 Escape closes, focus Fm1', s.direct.menu === null && d.active === s.direct.active?.targetId && s.direct.active?.name === 'Fm1', { menu: s.direct.menu, active: d.active });
  // Enter activates once.
  await rightClick(fm.x, fm.y);
  const rEnter = (await state()).revision;
  await key('Enter', 0, 1800);
  s = await state(); d = await dom();
  check('d4 Enter duplicates once (+1 revision)', s.revision === rEnter + 1 && s.direct.objects === 4 && all(s, 'Fm1').some(v => near(v, [410, 272, 510, 302])), { rev: s.revision, Fm1: all(s, 'Fm1') });
  check('d4 edited form duplicate + announcement', s.content.edited?.kind === 'form' && s.content.edited?.action === 'duplicate' && d.live === 'Duplicated Form Fm1', { edited: s.content.edited, live: d.live });
  await clickEl('undo');
  // Space activates once (Delete).
  await rightClick(fm.x, fm.y); await key('ArrowDown', 0, 200);
  const rSpace = (await state()).revision;
  await key(' ', 0, 1800);
  s = await state();
  check('d5 Space deletes once', s.revision === rSpace + 1 && s.direct.objects === 2, { rev: s.revision, objects: s.direct.objects });
  await clickEl('undo');
  // Right-click a non-member while a group is selected → single selection.
  await click(i1.x, i1.y); await click(i2.x, i2.y, SHIFT);
  await rightClick(fm.x, fm.y);
  s = await state();
  check('d7 non-member right-click → single Fm1', names(s.direct.selection) === 'Fm1' && s.direct.menu?.count === 1, { sel: s.direct.selection, menu: s.direct.menu });
  await key('Escape');
  // Scroll closes (d11 opens its own menu, §9.32).
  await rightClick(i1.x, i1.y);
  check('d11 menu open on Im1', (await state()).direct.menu?.name === 'Im1', (await state()).direct.menu);
  await mouse('mouseWheel', empty.x, empty.y, { deltaX: 0, deltaY: 120 }); await sleep(600);
  s = await state();
  check('d11 wheel scroll closes', s.direct.menu === null, s.direct.menu);
  // Zoom closes (⌘= from the menu).
  const i1b = await pagePoint(122, 167); await sleep(300);
  await rightClick(i1b.x, i1b.y);
  await key('=', META, 1200);
  s = await state();
  check('d12 zoom closes', s.direct.menu === null, { menu: s.direct.menu, zoom: s.zoom });
  await cdp.eval(`document.getElementById('zoom-out')?.click()`); await sleep(800);
  // Page change closes (PageDown from the menu).
  const i1c = await pagePoint(122, 167); await sleep(300);
  await rightClick(i1c.x, i1c.y);
  await key('PageDown', 0, 1200);
  s = await state();
  check('d13 page change closes', s.direct.menu === null, { menu: s.direct.menu, page: s.page });
  await shot('review-u5-d13-after-pagedown.png');
  await cdp.eval(`document.getElementById('prev')?.click()`); await sleep(1200);

  // (g) single-object regression: body drag and se handle on Im1.
  const g1 = await pagePoint(122, 167); await sleep(300);
  await click(g1.x, g1.y);
  const rG = (await state()).revision;
  await drag(g1.x, g1.y, 20 * g1.s, 10 * g1.s);
  s = await state();
  check('g1 single drag', s.revision === rG + 1 && s.content.edited?.kind === 'image' && s.content.edited?.action === 'move' && near(all(s, 'Im1')[0], [92, 152, 192, 202]) && !('count' in s.content.edited), { rev: s.revision, edited: s.content.edited, Im1: all(s, 'Im1') });
  const se = await cdp.eval(`(() => { const b = document.querySelector('#hit-layer .object-target.selected .hit-handle.se'); if (!b) return null; const r = b.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2, shown: getComputedStyle(b).display }; })()`);
  if (check('g2 se handle present', !!se && se.shown !== 'none', se)) {
    await drag(se.x, se.y, 50 * g1.s, 25 * g1.s);
    s = await state();
    check('g2 single resize', s.revision === rG + 2 && s.content.edited?.action === 'resize', { rev: s.revision, edited: s.content.edited, Im1: all(s, 'Im1') });
  }
  await shot('review-u5-g-single.png');
});


if (ONLY.includes('fix')) await session('fix', 'fixtures/p6/direct3.pdf', {}, async ({ state, shot, move, mouse, click, rightClick, drag, key, pagePoint, clickEl, mode, cdp }) => {
  const dom = () => cdp.eval(DOMQ);
  await mode('edit');
  await wait(async () => (await state()).direct.objects === 3, 20_000); await sleep(500);
  const IM1 = [72, 142, 172, 192];
  // b3 again: what does the release after an Escape mid-drag do?
  let p = await pagePoint(122, 167); await sleep(200); await click(p.x, p.y);
  let q2 = await pagePoint(430, 72, false); await click(q2.x, q2.y, SHIFT);
  await drag(q2.x, q2.y, 40 * q2.s, 40 * q2.s, 0, undefined, async () => { await key('Escape', 0, 200); });
  let s = await state(), d = await dom();
  check('b3x after Escape mid-drag + release', s.direct.active?.kind !== 'add' && d.editors === 0, { active: s.direct.active, sel: s.direct.selection, editors: d.editors, rev: s.revision });
  await key('Escape'); await sleep(300);
  // b4 again with logging.
  p = await pagePoint(122, 167); await sleep(200); await click(p.x, p.y);
  q2 = await pagePoint(430, 72, false); await click(q2.x, q2.y, SHIFT);
  s = await state();
  log('b4x selection before drag', names(s.direct.selection), q(s.direct.active?.name), 'q2', q(q2));
  const rEdge = s.revision;
  await drag(q2.x, q2.y, 300 * q2.s, -200 * q2.s);
  s = await state();
  check('b4x clamp at the page edge', s.revision === rEdge + 1 && near(all(s, 'Im2')[0], [552, 0, 612, 40]) && near(all(s, 'Im1')[0], [224, 90, 324, 140]) && s.content.refused.length === 0, { rev: s.revision, Im1: all(s, 'Im1'), Im2: all(s, 'Im2'), edited: s.content.edited, sel: s.direct.selection });
  await shot('review-u5-b4-clamp.png');
  await clickEl('undo'); await key('Escape');
  await pagePoint(250, 180); await sleep(400);
  const i1 = await pagePoint(122, 167, false);
  // Tab from the open menu: closes and refocuses the anchor.
  await rightClick(i1.x, i1.y); await key('Tab', 0, 300);
  s = await state(); d = await dom();
  check('d14 Tab closes, focus anchor', s.direct.menu === null && d.active === s.direct.active?.targetId, { menu: s.direct.menu, active: d.active });
});


if (ONLY.includes('esc')) await session('esc', 'fixtures/p6/direct3.pdf', {}, async ({ state, shot, move, mouse, click, rightClick, drag, key, pagePoint, clickEl, mode, cdp }) => {
  const dom = () => cdp.eval(DOMQ);
  await mode('edit');
  await wait(async () => (await state()).direct.objects === 3, 20_000); await sleep(500);
  // Single-object Escape mid-drag (U3/U4 path, unchanged by U5): does the release open an add-text editor?
  let q2 = await pagePoint(430, 72); await sleep(300); await click(q2.x, q2.y);
  await drag(q2.x, q2.y, 40 * q2.s, 40 * q2.s, 0, undefined, async () => { await key('Escape', 0, 200); });
  let s = await state(), d = await dom();
  check('e1 single Escape mid-drag + release: no add editor', s.direct.active?.kind !== 'add' && d.editors === 0, { active: s.direct.active, editors: d.editors, rev: s.revision });
  await shot('review-u5-e1-escape-release.png');
  await key('Escape'); await sleep(300);
  // Group drag moved Im2 to the corner, then the corner single menu Duplicate (ArrowDown from Replace).
  let p = await pagePoint(122, 167); await sleep(200); await click(p.x, p.y);
  q2 = await pagePoint(430, 72, false); await click(q2.x, q2.y, SHIFT);
  await drag(q2.x, q2.y, 300 * q2.s, -200 * q2.s);
  const c2 = await pagePoint(582, 20); await sleep(400);
  await click(c2.x, c2.y); await rightClick(c2.x, c2.y);
  await key('ArrowDown', 0, 200);
  log('d0y focus', await cdp.eval('document.activeElement.id'));
  const rMenu = (await state()).revision;
  await key('Enter', 0, 1800);
  s = await state(); d = await dom();
  const copy = s.content.objects.find(o => o.name === 'Im2' && near(o.view, [542, 10, 602, 50]));
  check('d0y corner menu duplicate at [542,10,602,50] (−10 x, +10 y), selected', s.revision === rMenu + 1 && !!copy && s.direct.selection[0]?.targetId === copy?.targetId, { rev: s.revision, Im2: all(s, 'Im2'), sel: s.direct.selection, live: d.live });
  await shot('review-u5-d0-corner-dup.png');
  await clickEl('undo'); await clickEl('undo');
  // Group menu Delete (ArrowDown from Duplicate, Enter).
  await pagePoint(250, 180); await sleep(400);
  const i1 = await pagePoint(122, 167, false), i2 = await pagePoint(430, 72, false);
  await click(i1.x, i1.y); await click(i2.x, i2.y, SHIFT);
  await rightClick(i2.x, i2.y); await key('ArrowDown', 0, 200);
  const rDel = (await state()).revision;
  await key('Enter', 0, 1800);
  s = await state(); d = await dom();
  check('d8y group menu delete', s.revision === rDel + 1 && s.direct.objects === 1 && s.content.edited?.kind === 'objects' && s.content.edited?.action === 'delete' && s.content.edited?.count === 2 && d.live === 'Deleted 2 objects' && s.direct.selection.length === 0, { rev: s.revision, objects: s.direct.objects, edited: s.content.edited, live: d.live, sel: s.direct.selection });
  await clickEl('undo');
  // Group arrows: ⌥→ then plain ↓ (focused member).
  await click(i1.x, i1.y); await click(i2.x, i2.y, SHIFT);
  const rA = (await state()).revision;
  await key('ArrowDown', 0, 1500);
  s = await state(); d = await dom();
  check('h1 plain ↓ on a focused member moves the group 1 pt', s.revision === rA + 1 && s.content.edited?.count === 2 && near(all(s, 'Im1')[0], [72, 143, 172, 193]) && near(all(s, 'Im2')[0], [400, 53, 460, 93]) && s.direct.selection.length === 2 && d.active === s.direct.active?.targetId, { rev: s.revision, Im1: all(s, 'Im1'), Im2: all(s, 'Im2'), sel: s.direct.selection, focus: d.active });
  await key('ArrowRight', SHIFT, 1500);
  s = await state();
  check('h2 Shift+→ group 10 pt', s.revision === rA + 2 && near(all(s, 'Im1')[0], [82, 143, 182, 193]), { rev: s.revision, Im1: all(s, 'Im1') });
  // Enter on a group does nothing (no native dialog).
  await key('Enter', 0, 800);
  s = await state();
  check('h3 group Enter is inert', s.revision === rA + 2 && s.direct.menu === null, { rev: s.revision });
});

/** §9.32: one session per U5 menu check, on direct3.pdf in Edit text with 3 objects; points on screen after one centring scroll. */
async function menuSession(tag, body) {
  if (!ONLY.includes(tag)) return;
  await session(tag, 'fixtures/p6/direct3.pdf', {}, async h => {
    await h.mode('edit');
    await wait(async () => (await h.state()).direct.objects === 3, 20_000); await sleep(500);
    await h.pagePoint(250, 180); await sleep(400);
    const pts = { i1: await h.pagePoint(122, 167, false), i2: await h.pagePoint(430, 72, false), fm: await h.pagePoint(450, 277, false), empty: await h.pagePoint(250, 330, false) };
    log('points', tag, q(pts));
    await body({ ...h, ...pts, dom: () => h.cdp.eval(DOMQ) });
  });
}

// d0: group drag to the top-right corner, then the single-object menu on the corner Im2; ArrowDown (Replace → Duplicate) and Enter.
await menuSession('d0', async ({ state, click, rightClick, drag, key, pagePoint, i1, i2 }) => {
  await click(i1.x, i1.y); await click(i2.x, i2.y, SHIFT);
  await drag(i2.x, i2.y, 300 * i2.s, -200 * i2.s);
  let s = await state();
  check('d0 group clamped at the corner', s.revision === 1 && near(all(s, 'Im2')[0], [552, 0, 612, 40]), { rev: s.revision, Im2: all(s, 'Im2') });
  const c2 = await pagePoint(582, 20); await sleep(400);
  await click(c2.x, c2.y); await rightClick(c2.x, c2.y);
  s = await state();
  check('d0 menu on corner Im2 single', s.direct.menu?.name === 'Im2' && s.direct.menu?.count === 1, s.direct.menu);
  await key('ArrowDown', 0, 200);
  const rMenu = s.revision;
  await key('Enter', 0, 1800);
  s = await state();
  check('d0 menu duplicate −10 x, +10 y', s.revision === rMenu + 1 && all(s, 'Im2').some(v => near(v, [542, 10, 602, 50])), { rev: s.revision, Im2: all(s, 'Im2') });
});

// d6: a click on empty space closes the menu and opens no add-text editor.
await menuSession('d6', async ({ state, click, rightClick, i1, empty, dom }) => {
  await rightClick(i1.x, i1.y);
  check('d6 menu open on Im1', (await state()).direct.menu?.name === 'Im1', (await state()).direct.menu);
  await click(empty.x, empty.y, 0, 800);
  const s = await state(), d = await dom();
  check('d6 empty click closes menu, no add editor', s.direct.menu === null && s.direct.active?.kind !== 'add' && d.editors === 0, { menu: s.direct.menu, active: s.direct.active, editors: d.editors });
});

// d8: a right-click on a member keeps the group.
await menuSession('d8', async ({ state, click, rightClick, i1, i2 }) => {
  await click(i1.x, i1.y); await click(i2.x, i2.y, SHIFT);
  await rightClick(i2.x, i2.y);
  const s = await state();
  check('d8 member right-click keeps group', s.direct.menu?.count === 2 && s.direct.menu?.items?.replace === false && names(s.direct.selection) === 'Im1,Im2', { menu: s.direct.menu, sel: s.direct.selection });
});

// d9: Control-click (macOS) opens the menu, never a drag.
await menuSession('d9', async ({ state, move, mouse, i1 }) => {
  const rCtrl = (await state()).revision;
  await move(i1.x, i1.y, CTRL); await sleep(80);
  await mouse('mousePressed', i1.x, i1.y, { button: 'left', buttons: 1, clickCount: 1, modifiers: CTRL });
  await sleep(250);
  const ctrlMenu = (await state()).direct.menu;
  for (let i = 1; i <= 6; i++) await mouse('mouseMoved', i1.x + 5 * i, i1.y + 5 * i, { button: 'left', buttons: 1, modifiers: CTRL });
  const ctrlGesture = (await state()).direct.gesture;
  await mouse('mouseReleased', i1.x + 30, i1.y + 30, { button: 'left', buttons: 0, clickCount: 1, modifiers: CTRL }); await sleep(1000);
  const s = await state();
  check('d9 Control-click opens the menu, no drag', ctrlMenu?.name === 'Im1' && ctrlGesture === null && s.revision === rCtrl && near(all(s, 'Im1')[0], [72, 142, 172, 192]), { ctrlMenu, ctrlGesture, rev: s.revision, Im1: all(s, 'Im1'), menuAfter: s.direct.menu });
});

// d10: Shift+F10 on the focused object opens the menu.
await menuSession('d10', async ({ state, click, key, cdp, i1 }) => {
  await click(i1.x, i1.y);
  await key('F10', SHIFT, 500);
  const s = await state();
  check('d10 Shift+F10 opens the menu', s.direct.menu?.name === 'Im1' && (await cdp.eval('document.activeElement.id')) === 'object-menu-replace', { menu: s.direct.menu, focus: await cdp.eval('document.activeElement.id') });
});

if (ONLY.includes('save')) {
  const dest = join(scratch, 'group-move.pdf');
  await rm(dest, { force: true });
  await session('save', 'fixtures/p6/direct3.pdf', { to: dest }, async ({ state, click, drag, pagePoint, mode, cdp }) => {
    await mode('edit');
    await wait(async () => (await state()).direct.objects === 3, 20_000); await sleep(500);
    const p = await pagePoint(122, 167); await sleep(200); await click(p.x, p.y);
    const p2 = await pagePoint(430, 72, false); await click(p2.x, p2.y, SHIFT);
    await drag(p2.x, p2.y, 30 * p2.s, 20 * p2.s);
    const s = await state();
    check('s1 group moved before save', s.revision === 1 && s.content.edited?.count === 2, { rev: s.revision, edited: s.content.edited });
    await cdp.eval(`document.getElementById('save-copy').click()`);
    const saved = await wait(() => existsSync(dest), 20_000);
    check('s1 save-copy wrote the scratch file', !!saved, dest);
    await wait(async () => (await state()).dirty === false, 5_000);
  });
  if (existsSync(dest)) {
    const mupdf = (await import(join(root, 'node_modules/mupdf/dist/mupdf.js'))).default;
    const { createEngine } = await import(join(root, 'build/renderer/engine.js'));
    const engine = createEngine(mupdf), { readFile } = await import('node:fs/promises');
    const bytes = await readFile(dest), sess = engine.createSession();
    const buf = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    engine.dispatch({ id: 1, op: 'open', name: 'g.pdf', pdf: buf }, sess);
    const sc = engine.dispatch({ id: 2, op: 'edit-targets', page: 0 }, sess).reply;
    const box = n => sc.items.find(i => i.objectName === n)?.placement?.box;
    check('s2 reopen: both moved (PDF y-up)', near(box('Im1'), [102, 580, 202, 630], 0.05) && near(box('Im2'), [430, 680, 490, 720], 0.05) && near(box('Fm1'), [400, 500, 500, 530], 0.05), { Im1: box('Im1'), Im2: box('Im2'), Fm1: box('Fm1') });
  }
}

if (ONLY.includes('form2')) await session('form2', 'fixtures/form2.pdf', {}, async ({ state, drag, click, move, pagePoint, mode, cdp }) => {
  await mode('forms'); await sleep(1500);
  const before = (await state()).fields.defs.name?.rect;
  const p = await pagePoint(310, 80); await sleep(300);
  await move(p.x, p.y); await sleep(300);
  const grip = await cdp.eval(`(() => { const t = document.elementsFromPoint(${p.x}, ${p.y}).find(n => n.closest && n.closest('#hit-layer .hit-target'))?.closest('.hit-target'); const g = t?.querySelector('.hit-grip'); if (!g) return null; const r = g.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
  log('f1 grip', q(grip));
  await drag(grip.x, grip.y, 40 * p.s, 20 * p.s);
  const s = await state();
  check('f1 real field drag (shared startRectGesture)', s.revision === 1 && near(s.fields.defs.name?.rect, [before[0] + 40, before[1] + 20, before[2] + 40, before[3] + 20], 0.6), { before, after: s.fields.defs.name?.rect, rev: s.revision });
});

log(fails ? `FAILS ${fails}` : 'ALL CHECKS OK');
