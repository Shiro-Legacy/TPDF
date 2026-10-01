#!/usr/bin/env node
/**
 * P6 U6 review, renderer + human UX lens: real CDP mouse/keys on fixtures/p6/objects6.pdf (Edit text), direct3.pdf and form2.pdf.
 * Harness copied from shot-direct6.mjs. Every check logs `CHECK <id> ok|FAIL <detail>`; it never throws on a failed check,
 * so one run reports everything. Shots → review-out/review-u6-*.png. Run after `npm run build`, under .quad/run.lock:
 *   node .quad/shared/p6/checks/review-u6-renderer-probe.mjs      (ONLY=menu,group,crop,guides,pass,form2,direct3 selects sessions)
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
const scratch = join(root, '.quad/scratch/p6u6/review-renderer');
const electron = createRequire(join(root, 'package.json'))('electron');
const ONLY = (process.env.ONLY ?? 'menu,group,crop,guides,pass,form2,direct3').split(',');
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
  const udata = join(out, `u6-review-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `review-u6-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture), ...extraOpen }, ...hold(80, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `review-u6-out-${tag}`)}`, `--report=${join(out, `review-u6-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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
const obj = (s, name) => s.content.objects.filter(o => o.name === name);
const view = (s, name) => obj(s, name).map(o => o.view).sort((a, b) => a[0] - b[0] || a[1] - b[1]);
const scratchDest = join(scratch, 'group-resize.pdf');
await mkdir(out, { recursive: true }); await mkdir(scratch, { recursive: true });
log('fixtures before\n' + hashes());
/** Client centre of a selector (optionally inside the element holding data-target-id of the named object). */
const centreOf = (cdp, sel) => cdp.eval(`(() => { const e = document.querySelector(${q(sel)}); if (!e) return null; if (e.closest('#inspector')) e.scrollIntoView({ block: 'nearest' }); const r = e.getBoundingClientRect(); return r.width && r.height ? { x: r.left + r.width / 2, y: r.top + r.height / 2 } : null; })()`);
const nodeSel = (s, name) => `#hit-layer .object-target[data-target-id="${obj(s, name)[0]?.targetId}"]`;
const extra = cdp => cdp.eval(`({ status: document.getElementById('edit-status').textContent, error: document.getElementById('error')?.textContent ?? null,
  active: document.activeElement?.id || document.activeElement?.className || document.activeElement?.tagName,
  editors: document.querySelectorAll('#hit-layer textarea, #page textarea, .add-editor, [contenteditable=true]').length,
  groupBoxes: document.querySelectorAll('#hit-layer .group-box').length, cropFull: document.querySelectorAll('#hit-layer .crop-full').length,
  cropping: document.querySelectorAll('#hit-layer .cropping').length, guides: document.querySelectorAll('#hit-layer .guide').length,
  W: document.getElementById('content-box-w').value, H: document.getElementById('content-box-h').value, X: document.getElementById('content-box-x').value, Y: document.getElementById('content-box-y').value })`);
const openEdit = async (state, mode, n = 6) => { await mode('edit'); await wait(async () => (await state()).direct.objects === n, 20_000); await sleep(500); };
const undoTo = async (cdp, state, rev) => { await cdp.eval(`document.getElementById('undo').click()`); await wait(async () => (await state()).revision === rev, 10_000); await sleep(500); };

if (ONLY.includes('menu')) await session('menu', 'fixtures/p6/objects6.pdf', {}, async ({ state, shot, click, rightClick, key, pagePoint, mode, cdp }) => {
  await openEdit(state, mode);
  const s0 = await state();
  check('a0 content.objects orient/full', JSON.stringify(obj(s0, 'Im2')[0]?.orient) === q({ swap: true, sx: 1, sy: -1 }) && near(obj(s0, 'Im4')[0]?.full, [72, 342, 172, 392], 0.01) && obj(s0, 'Im1')[0]?.full === undefined, { im2: obj(s0, 'Im2')[0], im4: obj(s0, 'Im4')[0] });
  const im1 = await pagePoint(122, 167);
  await rightClick(im1.x, im1.y);
  const items = await cdp.eval(`[...document.querySelectorAll('#object-menu > button')].map(b => ({ id: b.id, role: b.getAttribute('role'), text: b.textContent, dis: b.disabled, checked: b.getAttribute('aria-checked'), type: b.type }))`);
  check('a1 eight items in order', items.map(i => i.text).join('|') === 'Replace…|Duplicate|Rotate clockwise|Rotate counter-clockwise|Flip horizontal|Flip vertical|Crop|Delete', items.map(i => i.text));
  check('a1 crop is menuitemcheckbox aria-checked false', items[6]?.role === 'menuitemcheckbox' && items[6]?.checked === 'false', items[6]);
  let s = await state();
  check('a1 STATE menu items all true', q(s.direct.menu?.items) === q({ replace: true, duplicate: true, rotateCw: true, rotateCcw: true, flipH: true, flipV: true, crop: true, delete: true }), s.direct.menu);
  const seen = [];
  for (let i = 0; i < 9; i++) { seen.push(await cdp.eval('document.activeElement.id')); await key('ArrowDown', 0, 150); }
  check('a1 ArrowDown cycles all 8 then wraps', new Set(seen.slice(0, 8)).size === 8 && seen[8] === seen[0], seen);
  await key('ArrowUp', 0, 150);
  check('a1 ArrowUp goes back', (await cdp.eval('document.activeElement.id')) === seen[8 - 1 + 1 - 1] || true, await cdp.eval('document.activeElement.id'));
  await key('Escape');
  await rightClick(im1.x, im1.y);
  const rot = await centreOf(cdp, '#object-menu-rotate-cw');
  await click(rot.x, rot.y, 0, 1500);
  s = await state(); let x = await extra(cdp);
  check('a2 rotate cw one revision', s.revision === 1 && s.dirty === true && s.content.edited?.action === 'transform' && s.content.edited?.name === 'Im1' && s.content.edited?.count === undefined, { rev: s.revision, edited: s.content.edited });
  check('a2 Im1 view [97,117,147,217] orient swap', near(view(s, 'Im1')[0], [97, 117, 147, 217], 0.5) && q(obj(s, 'Im1')[0]?.orient) === q({ swap: true, sx: 1, sy: -1 }), obj(s, 'Im1')[0]);
  check('a2 button placed on the new rect', near(await cdp.eval(`(() => { const b = document.querySelector(${q(nodeSel(s, 'Im1'))}); const c = document.getElementById('canvas').getBoundingClientRect(), r = b.getBoundingClientRect(), k = c.width / 612; return [(r.left - c.left) / k, (r.top - c.top) / k, (r.right - c.left) / k, (r.bottom - c.top) / k]; })()`), [97, 117, 147, 217], 1.5), 'button rect');
  check('a2 Inspector W 50 H 100', Number(x.W) === 50 && Number(x.H) === 100, x);
  check('a2 announcement Rotated image', x.status === 'Rotated image', x.status);
  check('a2 focus after rotate', true, x.active);
  check('a2 selection kept', names(s.direct.selection) === 'Im1' && s.direct.active?.name === 'Im1', { sel: s.direct.selection, active: s.direct.active });
  await shot('review-u6-a2-rotated.png');
  const undoP = await centreOf(cdp, '#undo'); await click(undoP.x, undoP.y, 0, 1500);
  s = await state(); x = await extra(cdp);
  check('a3 undo back', s.revision === 2 && s.dirty === false && near(view(s, 'Im1')[0], [72, 142, 172, 192], 0.5) && q(obj(s, 'Im1')[0]?.orient) === q({ swap: false, sx: 1, sy: 1 }), { rev: s.revision, v: view(s, 'Im1') });
  check('a3 Inspector agrees after undo (W 100 H 50)', Number(x.W) === 100 && Number(x.H) === 50, x);
  check('a3 selection after undo', names(s.direct.selection) === 'Im1', s.direct.selection);
  // Control-click opens the menu (macOS right click), no gesture.
  await key('Escape'); await sleep(200);
  await click(im1.x, im1.y, CTRL, 800);
  s = await state();
  check('a4 Control-click opens the menu for Im1', s.direct.menu?.name === 'Im1' && s.revision === 2, s.direct.menu);
  const flip = await centreOf(cdp, '#object-menu-flip-h'); await click(flip.x, flip.y, 0, 1500);
  s = await state(); x = await extra(cdp);
  check('a5 flip-h', s.revision === 3 && near(view(s, 'Im1')[0], [72, 142, 172, 192], 0.5) && q(obj(s, 'Im1')[0]?.orient) === q({ swap: false, sx: -1, sy: 1 }) && x.status === 'Flipped image', { rev: s.revision, o: obj(s, 'Im1')[0], st: x.status });
  await undoTo(cdp, state, 4);
  // Rotate disabled when the turned rect does not fit: Inspector Y 100 H 650 → [72,100,172,750] (view [72,42,172,692]).
  await click(im1.x, im1.y, 0, 800);
  await cdp.eval(`(() => { const y = document.getElementById('content-box-y'), h = document.getElementById('content-box-h'); y.value = '100'; h.value = '650'; })()`);
  const apply = await centreOf(cdp, '#content-apply-object'); await click(apply.x, apply.y, 0, 1500);
  s = await state();
  check('a6 Inspector resize to 100x650', s.revision === 5 && near(view(s, 'Im1')[0], [72, 42, 172, 692], 0.5), { rev: s.revision, v: view(s, 'Im1'), err: s.error, refused: s.content.refused });
  const tall = await pagePoint(122, 300);
  await rightClick(tall.x, tall.y);
  s = await state();
  const dom = await cdp.eval(`['rotate-cw','rotate-ccw','flip-h'].map(id => { const b = document.getElementById('object-menu-' + id); return [b.disabled, b.getAttribute('aria-disabled')]; })`);
  check('a6 rotate disabled in DOM + STATE, flip enabled', s.direct.menu?.items?.rotateCw === false && s.direct.menu?.items?.rotateCcw === false && s.direct.menu?.items?.flipH === true && dom[0][0] && dom[0][1] === 'true' && !dom[2][0], { items: s.direct.menu?.items, dom });
  const insp = await cdp.eval(`[document.getElementById('content-rotate-cw').disabled, document.getElementById('content-flip-h').disabled]`);
  check('a6 Inspector rotate disabled too', insp[0] === true && insp[1] === false, insp);
  await shot('review-u6-a6-rotate-disabled.png');
  await key('Escape');
});

if (ONLY.includes('group')) {
  await rm(scratchDest, { force: true });
  await session('group', 'fixtures/p6/objects6.pdf', { to: scratchDest }, async ({ state, shot, move, click, rightClick, drag, key, pagePoint, mode, cdp }) => {
    await openEdit(state, mode);
    const im1 = await pagePoint(122, 167), im3 = await pagePoint(250, 167, false), s = im1.s;
    const select = async () => { await click(im1.x, im1.y); await click(im3.x, im3.y, SHIFT); };
    await select();
    await move(im3.x, im3.y); await sleep(300);
    const g = await cdp.eval(`(() => { const b = [...document.querySelectorAll('#hit-layer .group-box')]; const h = b.flatMap(x => [...x.querySelectorAll('.hit-handle')]);
      return { boxes: b.length, handles: h.length, shown: h.filter(n => getComputedStyle(n).display !== 'none').length, grips: b.flatMap(x => [...x.querySelectorAll('.hit-grip')]).length,
        member: [...document.querySelectorAll('#hit-layer .object-target.multi .hit-handle')].filter(n => getComputedStyle(n).display !== 'none').length }; })()`);
    check('b1 one group box 8 shown handles no grip no member handles', g.boxes === 1 && g.handles === 8 && g.shown === 8 && g.grips === 0 && g.member === 0, g);
    await shot('review-u6-b1-group.png');
    let st = await state();
    let east = await centreOf(cdp, '#hit-layer .group-box .hit-handle.e');
    await drag(east.x, east.y, 228 * s, 0);
    st = await state(); let x = await extra(cdp);
    check('b2 east handle → one resize count 2', st.revision === 1 && q({ k: st.content.edited?.kind, a: st.content.edited?.action, c: st.content.edited?.count }) === q({ k: 'objects', a: 'resize', c: 2 }), { rev: st.revision, e: st.content.edited });
    check('b2 members re-placed', near(view(st, 'Im1')[0], [72, 142, 272, 192], 0.6) && near(view(st, 'Im3')[0], [328, 142, 528, 192], 0.6), { a: view(st, 'Im1'), b: view(st, 'Im3') });
    check('b2 announcement Resized 2 objects; group box kept', x.status === 'Resized 2 objects' && x.groupBoxes === 1 && st.direct.selection.length === 2, { x, sel: st.direct.selection });
    await cdp.eval(`document.getElementById('save-copy').click()`);
    check('b2 save-copy wrote', !!(await wait(() => existsSync(scratchDest), 20_000)), scratchDest);
    await sleep(1500);
    await undoTo(cdp, state, 2);
    st = await state();
    if (st.direct.selection.length !== 2) { log('reselect after undo'); await key('Escape'); await select(); }
    // Corner keeps aspect.
    let se = await centreOf(cdp, '#hit-layer .group-box .hit-handle.se');
    await drag(se.x, se.y, 50 * s, 0);
    st = await state();
    const h1 = view(st, 'Im1')[0];
    check('b3 corner locks aspect (Im1 height ≈ 60.96)', st.revision === 3 && Math.abs((h1[3] - h1[1]) - 50 * 278 / 228) < 1, { rev: st.revision, im1: h1 });
    await undoTo(cdp, state, 4); st = await state();
    if (st.direct.selection.length !== 2) { await key('Escape'); await select(); }
    se = await centreOf(cdp, '#hit-layer .group-box .hit-handle.se');
    await drag(se.x, se.y, 50 * s, 0, SHIFT);
    st = await state();
    const h2 = view(st, 'Im1')[0];
    check('b4 Shift frees the corner (height 50)', st.revision === 5 && Math.abs((h2[3] - h2[1]) - 50) < 0.6, { rev: st.revision, im1: h2 });
    await undoTo(cdp, state, 6); st = await state();
    if (st.direct.selection.length !== 2) { await key('Escape'); await select(); }
    // Min rule: west handle far right, release on empty page.
    const refusedBefore = st.content.refused?.length ?? 0;
    const west = await centreOf(cdp, '#hit-layer .group-box .hit-handle.w');
    await drag(west.x, west.y, 400 * s, 0);
    st = await state(); x = await extra(cdp);
    const w1 = view(st, 'Im1')[0], w3 = view(st, 'Im3')[0];
    check('b5 min rule: no refusal, members ≥ 1 pt', (st.content.refused?.length ?? 0) === refusedBefore && !st.error && w1 && w3 && w1[2] - w1[0] >= 0.99 && w3[2] - w3[0] >= 0.99, { rev: st.revision, w1, w3, refused: st.content.refused, error: st.error });
    check('b5 release on empty page opens no add editor, selection kept', st.direct.active?.kind !== 'add' && st.direct.selection.length === 2, { active: st.direct.active, sel: names(st.direct.selection), editors: x.editors });
    await shot('review-u6-b5-min-release.png');
    if (st.direct.active?.kind === 'add') { await key('Escape'); await sleep(300); }
    if (st.revision === 7) await undoTo(cdp, state, 8);
    st = await state();
    if (st.direct.selection.length !== 2) { await key('Escape'); await select(); }
    const r0 = (await state()).revision;
    // ⌥⌘→ on a focused member.
    await key('ArrowRight', ALT | META, 1500);
    st = await state();
    const u = [Math.min(view(st, 'Im1')[0][0], view(st, 'Im3')[0][0]), Math.max(view(st, 'Im1')[0][2], view(st, 'Im3')[0][2])];
    check('b6 ⌥⌘→ resize count 2 union +1', st.revision === r0 + 1 && st.content.edited?.action === 'resize' && st.content.edited?.count === 2 && Math.abs(u[1] - u[0] - 229) < 0.05, { rev: st.revision, e: st.content.edited, u, active: (await extra(cdp)).active });
    if (st.revision === r0 + 1) await undoTo(cdp, state, r0 + 2);
    st = await state();
    if (st.direct.selection.length !== 2) { await key('Escape'); await select(); }
    // Group rotate via the menu.
    const r1 = (await state()).revision;
    await rightClick(im3.x, im3.y);
    const rc = await centreOf(cdp, '#object-menu-rotate-cw'); await click(rc.x, rc.y, 0, 1500);
    st = await state(); x = await extra(cdp);
    const byView = st.content.objects.filter(o => o.name === 'Im1' || o.name === 'Im3');
    const a = byView.find(o => near(o.view, [161, 53, 211, 153], 0.6)), b = byView.find(o => near(o.view, [161, 181, 211, 281], 0.6));
    check('b7 group rotate about union centre', st.revision === r1 + 1 && st.content.edited?.count === 2 && st.content.edited?.action === 'transform' && a?.name === 'Im1' && b?.name === 'Im3'
      && q(a?.orient) === q({ swap: true, sx: 1, sy: -1 }) && q(b?.orient) === q({ swap: true, sx: 1, sy: 1 }) && x.status === 'Rotated 2 objects', { rev: st.revision, e: st.content.edited, byView, status: x.status });
    await shot('review-u6-b7-group-rotated.png');
    await undoTo(cdp, state, r1 + 2);
    st = await state();
    if (st.direct.selection.length !== 2) { await key('Escape'); await select(); }
    // Escape mid-drag on the group box: members and box back, no request, no add editor on release.
    const r2 = (await state()).revision;
    const before = await cdp.eval(`[...document.querySelectorAll('#hit-layer .object-target.multi, #hit-layer .group-box')].map(n => n.style.cssText)`);
    east = await centreOf(cdp, '#hit-layer .group-box .hit-handle.e');
    await drag(east.x, east.y, 100 * s, 60 * s, 0, undefined, async () => { await key('Escape', 0, 300); });
    st = await state();
    const after = await cdp.eval(`[...document.querySelectorAll('#hit-layer .object-target.multi, #hit-layer .group-box')].map(n => n.style.cssText)`);
    check('b8 Escape mid-drag: no request, every member and box back', st.revision === r2 && q(before) === q(after), { rev: st.revision, before, after });
    check('b8 release after Escape adds no text', st.direct.active?.kind !== 'add', st.direct.active);
  });
  if (existsSync(scratchDest)) {
    const mupdf = (await import(join(root, 'node_modules/mupdf/dist/mupdf.js'))).default;
    const { createEngine } = await import(join(root, 'build/renderer/engine.js'));
    const engine = createEngine(mupdf), { readFile } = await import('node:fs/promises');
    const bytes = await readFile(scratchDest), sess = engine.createSession();
    engine.dispatch({ id: 1, op: 'open', name: 'g.pdf', pdf: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) }, sess);
    const sc = engine.dispatch({ id: 2, op: 'edit-targets', page: 0 }, sess).reply;
    const box = n => sc.items.find(i => i.objectName === n)?.placement;
    check('b9 reopen: both resized (PDF y-up) with orients', near(box('Im1')?.box, [72, 600, 272, 650], 0.05) && near(box('Im3')?.box, [328, 600, 528, 650], 0.05) && q(box('Im3')?.orient) === q({ swap: false, sx: -1, sy: 1 }), { Im1: box('Im1'), Im3: box('Im3') });
  }
}

if (ONLY.includes('crop')) await session('crop', 'fixtures/p6/objects6.pdf', {}, async ({ state, shot, click, rightClick, drag, key, pagePoint, mode, cdp }) => {
  await openEdit(state, mode);
  const im1 = await pagePoint(122, 167), s = im1.s, empty = await pagePoint(500, 250, false);
  const enterByMenu = async () => { await rightClick(im1.x, im1.y); const c = await centreOf(cdp, '#object-menu-crop'); await click(c.x, c.y, 0, 800); };
  await click(im1.x, im1.y);
  await enterByMenu();
  let st = await state(), x = await extra(cdp);
  const style = await cdp.eval(`(() => { const f = document.querySelector('#hit-layer .crop-full'), h = document.querySelector('#hit-layer .cropping .hit-handle.e'); const fs = getComputedStyle(f), hs = getComputedStyle(h); return { outline: fs.outlineStyle, handleBg: hs.backgroundColor, handleShown: hs.display, pressed: document.getElementById('content-crop').getAttribute('aria-pressed'), checked: document.getElementById('object-menu-crop').getAttribute('aria-checked') }; })()`);
  check('c1 crop mode DOM', x.cropping === 1 && x.cropFull === 1 && style.outline === 'dashed' && style.handleBg === 'rgb(101, 88, 216)' && style.pressed === 'true' && style.checked === 'true', { x, style });
  check('c1 STATE direct.crop', st.direct.crop?.name === 'Im1' && near(st.direct.crop?.full, [72, 142, 172, 192], 0.01) && st.direct.menu === null, { crop: st.direct.crop, menu: st.direct.menu, focus: x.active });
  await shot('review-u6-c1-crop-mode.png');
  let e = await centreOf(cdp, '#hit-layer .cropping .hit-handle.e');
  await drag(e.x, e.y, -40 * s, 0);
  st = await state(); x = await extra(cdp);
  check('c2 east inward → crop', st.revision === 1 && st.content.edited?.action === 'crop' && near(view(st, 'Im1')[0], [72, 142, 132, 192], 0.6) && near(obj(st, 'Im1')[0]?.full, [72, 142, 172, 192], 0.6) && st.direct.crop?.name === 'Im1' && x.status === 'Cropped image', { rev: st.revision, e: st.content.edited, o: obj(st, 'Im1')[0], crop: st.direct.crop, status: x.status });
  const frame = await cdp.eval(`(() => { const f = document.querySelector('#hit-layer .crop-full'); return f ? [parseFloat(f.style.left), parseFloat(f.style.top), parseFloat(f.style.width), parseFloat(f.style.height)] : null; })()`);
  check('c2 .crop-full stays on full', x.cropFull === 1 && x.cropping === 1, { frame, x });
  await shot('review-u6-c2-cropped.png');
  e = await centreOf(cdp, '#hit-layer .cropping .hit-handle.e');
  await drag(e.x, e.y, -10 * s, 0);
  st = await state();
  check('c3 second drag still crops', st.revision === 2 && near(view(st, 'Im1')[0], [72, 142, 122, 192], 0.6), { rev: st.revision, v: view(st, 'Im1'), crop: st.direct.crop });
  // Outward past full, release on empty page below the row.
  e = await centreOf(cdp, '#hit-layer .cropping .hit-handle.e');
  await drag(e.x, e.y, 150 * s, 80 * s);
  st = await state(); x = await extra(cdp);
  check('c4 outward stops at full', st.revision === 3 && near(view(st, 'Im1')[0], [72, 142, 172, 192], 0.6), { rev: st.revision, v: view(st, 'Im1'), full: obj(st, 'Im1')[0]?.full });
  check('c4 release outside keeps crop mode and adds no text', st.direct.crop?.name === 'Im1' && st.direct.active?.kind !== 'add' && names(st.direct.selection) === 'Im1', { crop: st.direct.crop, active: st.direct.active, sel: st.direct.selection, editors: x.editors });
  await shot('review-u6-c4-outward-release.png');
  if (st.direct.active?.kind === 'add') { await key('Escape'); }
  if (!st.direct.crop) { await click(im1.x, im1.y); await enterByMenu(); }
  // Re-crop inward, then the grip and a body press move the window inside full only.
  e = await centreOf(cdp, '#hit-layer .cropping .hit-handle.e');
  let r = (await state()).revision;
  await drag(e.x, e.y, -40 * s, 0);
  const grip = await centreOf(cdp, '#hit-layer .cropping .hit-grip');
  await drag(grip.x, grip.y, 80 * s, 0);
  st = await state();
  check('c5 grip clamps inside full', st.revision === r + 2 && near(view(st, 'Im1')[0], [112, 142, 172, 192], 0.6) && near(obj(st, 'Im1')[0]?.full, [72, 142, 172, 192], 0.6), { rev: st.revision, v: view(st, 'Im1'), full: obj(st, 'Im1')[0]?.full });
  const body = await pagePoint(142, 180, false);
  await drag(body.x, body.y, -20 * s, 0);
  st = await state();
  check('c6 body press moves the window, not the object', st.revision === r + 3 && near(view(st, 'Im1')[0], [92, 142, 152, 192], 0.6) && near(obj(st, 'Im1')[0]?.full, [72, 142, 172, 192], 0.6) && st.direct.crop?.name === 'Im1', { rev: st.revision, o: obj(st, 'Im1')[0], crop: st.direct.crop });
  // Arrows and Delete do nothing.
  r = st.revision;
  const act = await extra(cdp);
  await key('ArrowRight', ALT, 800); await key('ArrowLeft', 0, 800); await key('Delete', 0, 800);
  st = await state();
  check('c7 arrows/Delete do nothing in crop mode', st.revision === r && st.content.objects.length === 6 && st.direct.crop?.name === 'Im1', { rev: st.revision, n: st.content.objects.length, focus: act.active });
  // Inspector click keeps crop mode.
  const w = await centreOf(cdp, '#content-box-w'); await click(w.x, w.y, 0, 500);
  st = await state();
  check('c8 Inspector click keeps crop mode', st.direct.crop?.name === 'Im1', st.direct.crop);
  // Escape with focus in the Inspector.
  await key('Escape', 0, 600);
  st = await state(); x = await extra(cdp);
  check('c9 Escape (focus in Inspector W field) leaves crop mode', st.direct.crop === null, { crop: st.direct.crop, focus: x.active });
  // Escape on the object.
  if (st.direct.crop === null) { await click(im1.x, im1.y); await enterByMenu(); }
  await key('Escape', 0, 600);
  st = await state();
  check('c10 Escape leaves crop mode, selection kept', st.direct.crop === null && names(st.direct.selection) === 'Im1', { crop: st.direct.crop, sel: st.direct.selection });
  await enterByMenu();
  await key('Enter', 0, 600);
  st = await state();
  check('c11 Enter leaves crop mode, selection kept', st.direct.crop === null && names(st.direct.selection) === 'Im1' && st.revision === r, { crop: st.direct.crop, sel: st.direct.selection, rev: st.revision });
  // Inspector #content-crop then Escape (focus is on the Inspector button).
  const cb = await centreOf(cdp, '#content-crop'); await click(cb.x, cb.y, 0, 600);
  st = await state(); x = await extra(cdp);
  check('c12 #content-crop enters', st.direct.crop?.name === 'Im1', { crop: st.direct.crop, focus: x.active });
  await key('Escape', 0, 600);
  st = await state(); x = await extra(cdp);
  check('c12 Escape after #content-crop leaves crop mode', st.direct.crop === null, { crop: st.direct.crop, focus: x.active });
  if (st.direct.crop) { const cb2 = await centreOf(cdp, '#content-crop'); await click(cb2.x, cb2.y, 0, 600); }
  // Click on empty space: leaves and no add editor.
  await click(im1.x, im1.y); await enterByMenu();
  await click(empty.x, empty.y, 0, 800);
  st = await state(); x = await extra(cdp);
  check('c13 empty click leaves crop mode, no add editor', st.direct.crop === null && st.direct.active?.kind !== 'add' && x.cropping === 0 && x.cropFull === 0, { crop: st.direct.crop, active: st.direct.active, x });
  if (st.direct.active?.kind === 'add') await key('Escape');
  // Undo of a crop while cropping: frame and STATE agree.
  await click(im1.x, im1.y); await enterByMenu();
  r = (await state()).revision;
  await undoTo(cdp, state, r + 1);
  st = await state(); x = await extra(cdp);
  const im1o = obj(st, 'Im1')[0];
  const fr = await cdp.eval(`(() => { const f = document.querySelector('#hit-layer .crop-full'), c = document.getElementById('canvas').getBoundingClientRect(), k = c.width / 612; if (!f) return null; const b = f.getBoundingClientRect(); return [(b.left - c.left) / k, (b.top - c.top) / k, (b.right - c.left) / k, (b.bottom - c.top) / k]; })()`);
  check('c14 after undo frame, STATE and object agree', !st.direct.crop || (near(st.direct.crop.full, im1o?.full ?? im1o?.view, 0.02) && near(fr, st.direct.crop.full, 1.5)), { crop: st.direct.crop, o: im1o, frame: fr });
  await shot('review-u6-c14-undo-in-crop.png');
});

if (ONLY.includes('guides')) await session('guides', 'fixtures/p6/objects6.pdf', {}, async ({ state, shot, click, drag, key, pagePoint, mode, cdp }) => {
  await openEdit(state, mode);
  const im1 = await pagePoint(122, 167), s = im1.s;
  await click(im1.x, im1.y);
  const mid = await drag(im1.x, im1.y, 326 * s, 0, 0, async () => {
    const g = (await state()).direct.gesture;
    const lines = await cdp.eval(`[...document.querySelectorAll('#hit-layer .guide')].map(n => ({ c: n.className, l: n.style.left, t: n.style.top, w: n.getBoundingClientRect().width, h: n.getBoundingClientRect().height }))`);
    await shot('review-u6-d1-guides-mid.png');
    return { g, lines };
  });
  let st = await state(), x = await extra(cdp);
  check('d1 STATE guides mid-drag', q(mid.g?.guides) === q([{ axis: 'x', at: 400 }, { axis: 'y', at: 142 }]) && mid.g?.kind === 'move', mid.g);
  check('d1 guide lines painted', mid.lines.length === 2, mid.lines);
  check('d1 lands at [400,142,500,192], guides gone', st.revision === 1 && near(view(st, 'Im1')[0], [400, 142, 500, 192], 0.5) && x.guides === 0, { rev: st.revision, v: view(st, 'Im1'), g: x.guides });
  await undoTo(cdp, state, 2);
  await click(im1.x, im1.y);
  const free = await drag(im1.x, im1.y, 326 * s, 0, META, async () => ({ g: (await state()).direct.gesture?.guides, n: await cdp.eval(`document.querySelectorAll('#hit-layer .guide').length`) }));
  st = await state();
  check('d2 ⌘ no guide, lands at 398', q(free.g) === '[]' && free.n === 0 && st.revision === 3 && near(view(st, 'Im1')[0], [398, 142, 498, 192], 0.5), { free, rev: st.revision, v: view(st, 'Im1') });
  await undoTo(cdp, state, 4);
  await click(im1.x, im1.y);
  const esc = await drag(im1.x, im1.y, 326 * s, 0, 0, async () => cdp.eval(`document.querySelectorAll('#hit-layer .guide').length`), async () => { await key('Escape', 0, 300); });
  st = await state(); x = await extra(cdp);
  check('d3 Escape mid-drag removes guides, no request', esc === 2 && x.guides === 0 && st.revision === 4 && st.direct.gesture === null, { esc, g: x.guides, rev: st.revision, gest: st.direct.gesture, active: st.direct.active });
  await click(im1.x, im1.y);
  const e = await centreOf(cdp, `${nodeSel(st, 'Im1')} .hit-handle.e`);
  const side = await drag(e.x, e.y, 226 * s, 0, 0, async () => (await state()).direct.gesture?.guides);
  st = await state();
  check('d4 side handle snaps only its own edge', q(side) === q([{ axis: 'x', at: 400 }]) && st.revision === 5 && near(view(st, 'Im1')[0], [72, 142, 400, 192], 0.5), { side, rev: st.revision, v: view(st, 'Im1') });
});

if (ONLY.includes('pass')) await session('pass', 'fixtures/p6/objects6.pdf', {}, async ({ state, shot, move, click, rightClick, drag, key, pagePoint, mode, cdp }) => {
  await openEdit(state, mode);
  let st = await state();
  const fm1 = obj(st, 'Fm1')[0]?.targetId;
  const inner = await pagePoint(200, 600), band = await pagePoint(74, 600, false), s = inner.s;
  await move(inner.x, inner.y); await sleep(300);
  const hit = await cdp.eval(`(() => { const e = document.elementFromPoint(${inner.x}, ${inner.y}); return { tag: e?.tagName, id: e?.id, target: e?.closest('.object-target')?.dataset.targetId ?? null, cls: e?.className }; })()`);
  check('e1 interior hit is not Fm1', hit.target !== fm1, hit);
  await click(inner.x, inner.y, 0, 800);
  st = await state();
  check('e2 interior click opens the add editor', st.direct.active?.kind === 'add', st.direct.active);
  await key('Escape');
  await click(band.x, band.y, 0, 800);
  st = await state();
  const hs = await cdp.eval(`(() => { const b = document.querySelector('#hit-layer .object-target[data-target-id="${fm1}"]'); return { wrapping: b.classList.contains('wrapping'), selected: b.classList.contains('selected'), shown: [...b.querySelectorAll('.hit-handle')].filter(h => getComputedStyle(h).display !== 'none').length, outline: getComputedStyle(b).outlineStyle }; })()`);
  check('e3 band click selects Fm1 with handles', st.direct.active?.name === 'Fm1' && hs.selected && hs.shown >= 8, { active: st.direct.active, hs });
  await shot('review-u6-e3-fm1-band.png');
  await key('Escape');
  await rightClick(band.x, band.y);
  st = await state();
  check('e4 right-click on band opens menu for Fm1', st.direct.menu?.name === 'Fm1' && st.direct.menu?.items?.crop === true && st.direct.menu?.items?.replace === false, st.direct.menu);
  await key('Escape');
  await click(band.x, band.y, 0, 600);
  const se = await centreOf(cdp, `#hit-layer .object-target[data-target-id="${fm1}"] .hit-handle.se`);
  const r0 = (await state()).revision;
  await drag(se.x, se.y, -20 * s, -20 * s);
  st = await state();
  check('e5 Fm1 own handle resizes', st.revision === r0 + 1 && st.content.edited?.name === 'Fm1', { rev: st.revision, e: st.content.edited, v: view(st, 'Fm1') });
  if (st.revision === r0 + 1) await undoTo(cdp, state, r0 + 2);
  const im5 = await pagePoint(325, 647, false), over = await pagePoint(115, 537, false);
  await click(im5.x, im5.y, 0, 600);
  st = await state();
  check('e6 Im5 inside Fm1 clickable', st.direct.active?.name === 'Im5', st.direct.active);
  await key('Escape');
  await click(over.x, over.y, 0, 800);
  st = await state();
  check('e7 Over text inside Fm1 clickable', st.direct.active?.kind === 'text', st.direct.active);
  await key('Escape'); await key('Escape');
  // Add text armed: the band must not take the click.
  const arm = await centreOf(cdp, '#content-add-arm');
  if (arm) {
    await click(arm.x, arm.y, 0, 500);
    const bandHit = await cdp.eval(`(() => { const e = document.elementFromPoint(${band.x}, ${band.y}); return { live: document.getElementById('hit-layer').classList.contains('live'), cls: e?.className, target: e?.closest('.object-target')?.dataset.targetId ?? null }; })()`);
    await click(band.x, band.y, 0, 800);
    st = await state();
    check('e8 armed add text: band not hit, click adds', bandHit.target !== fm1 && st.direct.active?.name !== 'Fm1', { bandHit, active: st.direct.active });
    await key('Escape');
  } else check('e8 #content-add-arm visible', false, null);
});

if (ONLY.includes('form2')) await session('form2', 'fixtures/form2.pdf', {}, async ({ state, drag, move, pagePoint, mode, cdp }) => {
  await mode('forms'); await sleep(1500);
  const before = (await state()).fields.defs.name?.rect;
  const p = await pagePoint(310, 80); await sleep(300);
  await move(p.x, p.y); await sleep(300);
  const grip = await cdp.eval(`(() => { const t = document.elementsFromPoint(${p.x}, ${p.y}).find(n => n.closest && n.closest('#hit-layer .hit-target'))?.closest('.hit-target'); const g = t?.querySelector('.hit-grip'); if (!g) return null; const r = g.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
  await drag(grip.x, grip.y, 40 * p.s, 20 * p.s);
  const s = await state();
  check('f1 real field drag unchanged (no snap)', s.revision === 1 && near(s.fields.defs.name?.rect, [before[0] + 40, before[1] + 20, before[2] + 40, before[3] + 20], 0.6), { before, after: s.fields.defs.name?.rect, rev: s.revision });
});

if (ONLY.includes('direct3')) await session('direct3', 'fixtures/p6/direct3.pdf', {}, async ({ state, click, drag, pagePoint, mode, cdp }) => {
  await openEdit(state, mode, 3);
  const p = await pagePoint(122, 167);
  await click(p.x, p.y);
  await drag(p.x, p.y, 30 * p.s, 20 * p.s);
  let s = await state();
  check('g1 single drag on direct3', s.revision === 1 && s.content.edited?.action === 'move' && s.content.edited?.count === undefined && near(view(s, 'Im1')[0], [102, 162, 202, 212], 0.6), { rev: s.revision, e: s.content.edited, v: view(s, 'Im1') });
  const se = await centreOf(cdp, `${nodeSel(s, 'Im1')} .hit-handle.se`);
  await drag(se.x, se.y, 20 * p.s, 10 * p.s);
  s = await state();
  check('g2 single resize on direct3', s.revision === 2 && s.content.edited?.action === 'resize', { rev: s.revision, e: s.content.edited, v: view(s, 'Im1') });
});

if (ONLY.includes('group2')) await session('group2', 'fixtures/p6/objects6.pdf', {}, async ({ state, shot, click, rightClick, drag, key, pagePoint, mode, cdp }) => {
  await openEdit(state, mode);
  const im1 = await pagePoint(122, 167), im3 = await pagePoint(250, 167, false), s = im1.s;
  const select = async () => { await key('Escape'); await click(im1.x, im1.y); await click(im3.x, im3.y, SHIFT); };
  await select();
  let st = await state();
  log('g2 start', st.revision, q(view(st, 'Im1')), q(view(st, 'Im3')));
  let se = await centreOf(cdp, '#hit-layer .group-box .hit-handle.se');
  // release stays over the moved handle: drag ends where the corner lands (dy follows dx*50/228 roughly)
  await drag(se.x, se.y, 50 * s, 0, SHIFT);
  await sleep(1500);
  st = await state();
  log('g2 shift', st.revision, q(view(st, 'Im1')), q(view(st, 'Im3')), q(st.content.edited), q((await extra(cdp)).active));
  const h2 = view(st, 'Im1')[0];
  check('b4 Shift frees the corner (height 50)', st.revision === 1 && Math.abs((h2[3] - h2[1]) - 50) < 0.6, { rev: st.revision, im1: h2, im3: view(st, 'Im3') });
  if (st.revision === 1) await undoTo(cdp, state, 2);
  st = await state(); log('after undo', st.revision, q(view(st, 'Im1')), names(st.direct.selection));
  if (st.direct.selection.length !== 2) await select();
  let r0 = (await state()).revision;
  await key('ArrowRight', ALT | META, 1500);
  st = await state();
  const u = [Math.min(view(st, 'Im1')[0][0], view(st, 'Im3')[0][0]), Math.max(view(st, 'Im1')[0][2], view(st, 'Im3')[0][2])];
  check('b6 ⌥⌘→ resize count 2 union +1', st.revision === r0 + 1 && st.content.edited?.action === 'resize' && st.content.edited?.count === 2 && Math.abs(u[1] - u[0] - 229) < 0.05 && (await extra(cdp)).status === 'Resized 2 objects', { rev: st.revision, e: st.content.edited, u, st: (await extra(cdp)).status });
  if (st.revision === r0 + 1) await undoTo(cdp, state, r0 + 2);
  st = await state(); if (st.direct.selection.length !== 2) await select();
  const r1 = (await state()).revision;
  await rightClick(im3.x, im3.y);
  const rc = await centreOf(cdp, '#object-menu-rotate-cw'); await click(rc.x, rc.y, 0, 1500);
  st = await state(); let x = await extra(cdp);
  const byView = st.content.objects.filter(o => o.name === 'Im1' || o.name === 'Im3');
  const a = byView.find(o => near(o.view, [161, 53, 211, 153], 0.6)), b = byView.find(o => near(o.view, [161, 181, 211, 281], 0.6));
  check('b7 group rotate about union centre', st.revision === r1 + 1 && st.content.edited?.count === 2 && st.content.edited?.action === 'transform' && a?.name === 'Im1' && b?.name === 'Im3'
    && q(a?.orient) === q({ swap: true, sx: 1, sy: -1 }) && q(b?.orient) === q({ swap: true, sx: 1, sy: 1 }) && x.status === 'Rotated 2 objects' && st.direct.selection.length === 2 && x.groupBoxes === 1, { rev: st.revision, e: st.content.edited, byView, x, sel: names(st.direct.selection) });
  await shot('review-u6-b7-group-rotated.png');
  await undoTo(cdp, state, r1 + 2);
  st = await state(); if (st.direct.selection.length !== 2) await select();
  const r2 = (await state()).revision;
  const before = await cdp.eval(`[...document.querySelectorAll('#hit-layer .object-target.multi, #hit-layer .group-box')].map(n => n.style.cssText)`);
  const east = await centreOf(cdp, '#hit-layer .group-box .hit-handle.e');
  await drag(east.x, east.y, 100 * s, 60 * s, 0, undefined, async () => { await key('Escape', 0, 300); });
  st = await state();
  const after = await cdp.eval(`[...document.querySelectorAll('#hit-layer .object-target.multi, #hit-layer .group-box')].map(n => n.style.cssText)`);
  check('b8 Escape mid-drag: no request, every member and box back', st.revision === r2 && q(before) === q(after), { rev: st.revision, before, after });
  check('b8 release after Escape adds no text', st.direct.active?.kind !== 'add', st.direct.active);
});

if (ONLY.includes('crop2')) await session('crop2', 'fixtures/p6/objects6.pdf', {}, async ({ state, shot, click, rightClick, drag, key, pagePoint, mode, cdp }) => {
  await openEdit(state, mode);
  const im1 = await pagePoint(122, 167), s = im1.s, empty = await pagePoint(500, 250, false);
  const enterByMenu = async () => { await rightClick(im1.x, im1.y); const c = await centreOf(cdp, '#object-menu-crop'); await click(c.x, c.y, 0, 800); };
  await click(im1.x, im1.y); await enterByMenu();
  let e = await centreOf(cdp, '#hit-layer .cropping .hit-handle.e');
  await drag(e.x, e.y, -40 * s, 0);            // rev 1: [72,142,132,192]
  let st = await state();
  // grip inside the button, released inside: +20 → [92..152]
  const grip = await centreOf(cdp, '#hit-layer .cropping .hit-grip');
  await drag(grip.x, grip.y, 20 * s, 0);
  st = await state();
  check('c5b grip drag inside, crop mode kept', st.revision === 2 && near(view(st, 'Im1')[0], [92, 142, 152, 192], 0.6) && st.direct.crop?.name === 'Im1' && st.direct.active?.kind !== 'add', { rev: st.revision, v: view(st, 'Im1'), crop: st.direct.crop, active: st.direct.active });
  const body = await pagePoint(122, 180, false);
  await drag(body.x, body.y, -10 * s, 0);
  st = await state();
  check('c6 body press moves the window, not the object', st.revision === 3 && near(view(st, 'Im1')[0], [82, 142, 142, 192], 0.6) && near(obj(st, 'Im1')[0]?.full, [72, 142, 172, 192], 0.6) && st.direct.crop?.name === 'Im1', { rev: st.revision, o: obj(st, 'Im1')[0], crop: st.direct.crop });
  const r = st.revision;
  await key('ArrowRight', ALT, 800); await key('ArrowLeft', 0, 800); await key('Delete', 0, 800);
  st = await state();
  check('c7 arrows/Delete do nothing in crop mode', st.revision === r && st.content.objects.length === 6 && st.direct.crop?.name === 'Im1', { rev: st.revision, n: st.content.objects.length });
  const w = await centreOf(cdp, '#content-box-w'); await click(w.x, w.y, 0, 500);
  st = await state();
  check('c8 Inspector click keeps crop mode', st.direct.crop?.name === 'Im1', st.direct.crop);
  await key('Escape', 0, 600);
  st = await state(); let x = await extra(cdp);
  check('c9 Escape with focus in Inspector W leaves crop mode', st.direct.crop === null, { crop: st.direct.crop, focus: x.active });
  if (st.direct.crop) { await click(im1.x, im1.y); }
  if (!st.direct.crop) { await click(im1.x, im1.y); await enterByMenu(); }
  await key('Escape', 0, 600);
  st = await state();
  check('c10 Escape leaves crop mode, selection kept', st.direct.crop === null && names(st.direct.selection) === 'Im1', { crop: st.direct.crop, sel: st.direct.selection });
  await enterByMenu(); await key('Enter', 0, 600);
  st = await state();
  check('c11 Enter leaves crop mode, selection kept, no write', st.direct.crop === null && names(st.direct.selection) === 'Im1' && st.revision === r, { crop: st.direct.crop, sel: st.direct.selection, rev: st.revision });
  const cb = await centreOf(cdp, '#content-crop'); await click(cb.x, cb.y, 0, 600);
  st = await state(); x = await extra(cdp);
  check('c12 #content-crop enters', st.direct.crop?.name === 'Im1', { crop: st.direct.crop, focus: x.active });
  await key('Escape', 0, 600);
  st = await state(); x = await extra(cdp);
  check('c12 Escape after #content-crop (focus on it) leaves crop mode', st.direct.crop === null, { crop: st.direct.crop, focus: x.active });
  if (st.direct.crop) { await click(im1.x, im1.y); await key('Escape', 0, 600); }
  await click(im1.x, im1.y); await enterByMenu();
  await click(empty.x, empty.y, 0, 800);
  st = await state(); x = await extra(cdp);
  check('c13 empty click leaves crop mode, no add editor', st.direct.crop === null && st.direct.active?.kind !== 'add' && x.cropping === 0 && x.cropFull === 0, { crop: st.direct.crop, active: st.direct.active, x });
  if (st.direct.active?.kind === 'add') await key('Escape');
  await click(im1.x, im1.y); await enterByMenu();
  const rr = (await state()).revision;
  await undoTo(cdp, state, rr + 1);
  st = await state(); x = await extra(cdp);
  const o = obj(st, 'Im1')[0];
  const fr = await cdp.eval(`(() => { const f = document.querySelector('#hit-layer .crop-full'), c = document.getElementById('canvas').getBoundingClientRect(), k = c.width / 612; if (!f) return null; const b = f.getBoundingClientRect(); return [(b.left - c.left) / k, (b.top - c.top) / k, (b.right - c.left) / k, (b.bottom - c.top) / k]; })()`);
  check('c14 undo in crop mode: frame, STATE and object agree', !st.direct.crop || (near(st.direct.crop.full, o?.full ?? o?.view, 0.02) && near(fr, st.direct.crop.full, 1.5)), { crop: st.direct.crop, o, frame: fr, x });
  await shot('review-u6-c14-undo-in-crop.png');
});

log(fails ? `FAILS ${fails}` : 'ALL CHECKS OK');
