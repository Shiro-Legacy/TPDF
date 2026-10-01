#!/usr/bin/env node
/** P6 U4 review (renderer + human UX lens): real CDP mouse/keys. Run under .quad/run.lock after npm run build.
 * Writes review-out/review-u4-*.png and review-out/review-u4-ui.json. Saves only to .quad/scratch/review-u4-ui/. */
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdir, rm, writeFile, stat } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
const root = fileURLToPath(new URL('../../../../', import.meta.url));
const checksDir = fileURLToPath(new URL('./', import.meta.url));
const out = join(checksDir, 'review-out');
const scratch = join(root, '.quad/scratch/review-u4-ui');
const electron = createRequire(join(root, 'package.json'))('electron');
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
async function wait(fn, ms = 20_000, step = 200) {
  const t0 = Date.now(); let last;
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
  close() { try { this.ws.close(); } catch {} }
}
async function attach(port) {
  const page = await wait(async () => { const res = await fetch(`http://127.0.0.1:${port}/json/list`); const list = await res.json(); return list.find(t => t.webSocketDebuggerUrl && (t.type === 'page' || /index\.html/.test(t.url || ''))); });
  const cdp = new Cdp(page.webSocketDebuggerUrl); await cdp.connect(); await cdp.send('Runtime.enable'); await cdp.send('Page.enable'); return cdp;
}
const hold = (n, ms) => Array.from({ length: n }, () => ({ cmd: 'wait', ms }));
const results = [];
const check = (id, ok, detail) => { results.push({ id, ok: !!ok, detail }); log(ok ? 'OK  ' : 'BAD ', id, q(detail)); };
const hashes = () => execFileSync('shasum', ['-a', '256', '-c', join(checksDir, 'fixtures.sha256')], { cwd: root, encoding: 'utf8' });
const near = (a, b, tol = 0.5) => Array.isArray(a) && a.length === b.length && a.every((v, i) => Math.abs(v - b[i]) <= tol);
const KEYS = { Enter: 13, Tab: 9, ArrowRight: 39, ArrowDown: 40, ArrowLeft: 37, ArrowUp: 38, Escape: 27, Delete: 46 };
const ALT = 1, META = 4, SHIFT = 8;

const ONLY = process.env.ONLY ? process.env.ONLY.split(',') : null;
async function session(name, fixture, extraArgs, body, lines = []) {
  if (ONLY && !ONLY.includes(name)) return;
  const udata = join(out, `u4-review-udata-${name}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `review-u4-ui-${name}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: fixture }, ...lines, ...hold(60, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.', ...extraArgs,
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `review-u4-${name}-out`)}`, `--report=${join(out, `review-u4-${name}-report.json`)}`], { cwd: root, stdio: ['ignore', 'pipe', 'inherit'] });
  const stdout = []; child.stdout.on('data', d => stdout.push(String(d)));
  const closed = new Promise(done => child.on('close', done));
  let cdp;
  try {
    cdp = await attach(port);
    try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch {}
    const base = fixture.split('/').pop().replace('.pdf', '');
    await wait(() => cdp.eval(`document.title.includes(${q(base)})`), 60_000);
    await sleep(800);
    await cdp.eval(HOOK);
    await body(tools(cdp, stdout));
  } catch (err) { check(`${name}:crash`, false, String(err?.stack ?? err)); }
  finally { cdp?.close(); child.kill('SIGTERM'); log('electron closed', name, await closed); }
}
function tools(cdp, stdout) {
  const t = { cdp, stdout };
  t.state = () => cdp.eval('window.pdf4allState()');
  t.eval = s => cdp.eval(s);
  t.shot = async name => { const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }); await writeFile(join(out, name), Buffer.from(data, 'base64')); log('shot', name); shots.push(name); };
  t.move = (x, y, modifiers = 0, buttons = 0) => cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, modifiers, buttons, button: buttons ? 'left' : 'none' });
  t.press = (x, y, modifiers = 0, clickCount = 1) => cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount, modifiers });
  t.release = (x, y, modifiers = 0, clickCount = 1) => cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount, modifiers });
  t.click = async (x, y, w = 800) => { await t.move(x, y); await sleep(100); await t.press(x, y); await t.release(x, y); await sleep(w); };
  t.drag = async (x, y, dx, dy, { modifiers = 0, escape = false, w = 1500 } = {}) => {
    await t.move(x, y); await sleep(150); await t.press(x, y, modifiers);
    for (let i = 1; i <= 8; i++) await t.move(x + dx * i / 8, y + dy * i / 8, modifiers, 1);
    if (escape) { await t.key('Escape'); await sleep(150); }
    await t.release(x + dx, y + dy, modifiers); await sleep(w);
  };
  t.key = async (key, modifiers = 0, w = 0) => {
    const vk = KEYS[key]; const text = key === 'Enter' ? '\r' : undefined;
    await cdp.send('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', key, code: key, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers, ...(text ? { text, unmodifiedText: text } : {}) });
    await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code: key, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers });
    if (w) await sleep(w);
  };
  t.type = text => cdp.send('Input.insertText', { text });
  t.rect = sel => cdp.eval(`(() => { const e = ${sel}; if (!e) return null; const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2 }; })()`);
  t.ops = () => cdp.eval('window.__ops.splice(0)');
  t.ui = () => cdp.eval(`({ error: document.getElementById('error').hidden ? '' : document.getElementById('error').textContent, nav: document.getElementById('nav-status').textContent, active: (() => { const a = document.activeElement; return a ? a.tagName + '.' + a.className + (a.dataset.annotId ? '@' + a.dataset.annotId : '') + (a.dataset.fieldId ? '#' + a.dataset.fieldId : '') : null; })(), undo: !document.getElementById('undo').disabled, scroll: document.getElementById('viewport').scrollTop })`);
  t.btn = async id => { const r = await t.rect(`document.getElementById(${q(id)})`); await t.click(r.cx, r.cy, 1500); };
  /** Page point → client point, scaled by the canvas CSS width / page width in pt, scrolled to centre. */
  t.at = (x, y, pw = 612) => cdp.eval(`(() => {
    const v = document.getElementById('viewport'), c = document.getElementById('canvas');
    let r = c.getBoundingClientRect(); const s = r.width / ${pw};
    v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
    r = c.getBoundingClientRect();
    return { x: r.left + ${x} * s, y: r.top + ${y} * s, s };
  })()`);
  t.mode = async m => { await t.eval(`document.getElementById('mode-${m}').click()`); await sleep(1000); };
  return t;
}
const shots = [];
const HOOK = `(() => { if (window.__ops) return true; window.__ops = []; window.__mangle = null; const orig = Worker.prototype.postMessage;
  Worker.prototype.postMessage = function (msg, tr) { try { if (msg && msg.op && !['state','render','fields','notes','annots','edit-targets','redact-preview','text','search','outline','links','thumb','thumbnail','page-ids','page-text'].includes(msg.op)) window.__ops.push(msg.op); if (window.__mangle && msg && msg.op === window.__mangle.op) msg = { ...msg, ...window.__mangle.patch }; } catch {} return orig.call(this, msg, tr); }; return true; })()`;
const L = id => `document.querySelector('#hit-layer .link-target[data-annot-id="${id}"]')`;
async function handles(t, sel) {
  const box = await t.rect(sel); await t.move(box.cx, box.cy); await sleep(300);
  return t.eval(`(() => { const b = ${sel}; return [...b.querySelectorAll('[data-handle]')].map(h => { const r = h.getBoundingClientRect(); const cx = r.x + r.width / 2, cy = r.y + r.height / 2; const hit = document.elementFromPoint(cx, cy); return { name: h.dataset.handle, visible: getComputedStyle(h).display !== 'none' && r.width > 0, hit: hit === h, cx, cy }; }); })()`);
}
const linkRect = async (t, i) => (await t.state()).links[i]?.rect;
const undo = async t => { await t.btn('undo'); await t.ops(); };

await mkdir(out, { recursive: true }); await mkdir(scratch, { recursive: true });
log('fixtures before\n' + hashes());
const links = join(root, 'fixtures/p6/links.pdf');
const saved = join(scratch, 'links-saved.pdf');
await rm(saved, { force: true });
let preSave = null;

// ---------- (a)(b)(c)(d) links.pdf ----------
await session('links', links, [`--save-to=${saved}`], async t => {
  await wait(async () => (await t.state()).direct.links === 3, 20_000);
  let s = await t.state(); let ui;
  check('a0 read mode, 3 link targets, links[] 3', s.mode === 'read' && s.links.length === 3 && s.links.map(l => l.annotId).join() === '10,11,13', { mode: s.mode, links: s.links });
  const S = (await t.at(136, 82)).s;
  let p = await t.at(136, 82); await t.move(p.x, p.y); await sleep(300);
  const cur = await t.eval(`(() => { const e = document.elementFromPoint(${p.x}, ${p.y}); return { cls: e.className, cursor: getComputedStyle(e).cursor, handles: e.querySelectorAll('[data-handle]').length }; })()`);
  check('a1 Read hover L1: pointer cursor, no handles', cur.cursor === 'pointer' && /link-target/.test(cur.cls) && cur.handles === 0, cur);
  await t.shot('review-u4-a1-read-hover.png');
  p = await t.at(136, 132); await t.click(p.x, p.y, 1500);
  s = await t.state();
  check('a2 real click L2 → page 2, nav.last link-follow page', s.page === 2 && q(s.nav.last) === q({ action: 'link-follow', kind: 'page', page: 2 }), { page: s.page, last: s.nav.last });
  await t.btn('prev'); await t.btn('prev');
  await wait(async () => { const v = await t.state(); return v.page === 0 && v.direct.links === 3; });
  const outBefore = t.stdout.join('').split('P2_LINK').length;
  p = await t.at(136, 82); await t.click(p.x, p.y, 1500);
  s = await t.state();
  const p2 = t.stdout.join('').split('\n').filter(l => l.includes('P2_LINK'));
  check('a3 click L1 → nav.last uri, P2_LINK logged in main, page stays 0', s.page === 0 && q(s.nav.last) === q({ action: 'link-follow', kind: 'uri', uri: 'https://example.org/one' }) && p2.length === outBefore && p2.at(-1)?.includes('"allowed":true'), { page: s.page, last: s.nav.last, p2 });
  p = await t.at(136, 182); await t.click(p.x, p.y, 1000);
  s = await t.state(); ui = await t.ui();
  const navVisible = await t.eval(`(() => { const e = document.getElementById('nav-status'); const r = e.getBoundingClientRect(); return r.width > 0 && r.height > 0 && !e.closest('[hidden]'); })()`);
  check('a4 click L3 (ftp) → blocked, message shown', s.nav.last?.kind === 'blocked' && /ftp:/.test(ui.nav), { last: s.nav.last, nav: ui.nav, navStatusVisible: navVisible, error: ui.error });
  await t.shot('review-u4-a4-blocked.png');

  // ---- Annotate ----
  await t.mode('annotate'); await t.ops();
  await wait(async () => (await t.state()).direct.links === 3);
  const hs = await handles(t, L(10));
  check('b1 Annotate hover L1: grip + 8 handles visible and hit-testable', hs.length === 9 && hs.every(h => h.visible && h.hit), hs.map(h => `${h.name}:${h.visible ? 'v' : '-'}${h.hit ? 'h' : '-'}`));
  await t.shot('review-u4-b1-link-handles.png');
  s = await t.state(); const rev0 = s.revision;
  let b = await t.rect(L(10));
  await t.drag(b.cx, b.cy, 30 * S, 40 * S);
  s = await t.state(); let ops = await t.ops();
  check('b2 body drag L1 → ONE link-set, rect moved, dirty, canUndo, nav.last', q(ops) === q(['link-set']) && near(s.links[0].rect, [102, 112, 230, 132]) && s.dirty && s.canUndo && s.revision === rev0 + 1 && q(s.nav.last) === q({ action: 'link-set', index: 0 }), { ops, rect: s.links[0].rect, dirty: s.dirty, canUndo: s.canUndo, rev: s.revision, last: s.nav.last, active: s.direct.active });
  check('b2b direct.active after drag is link §1.8 shape', s.direct.active?.kind === 'link' && s.direct.active.annotId === 10 && s.direct.active.id === 0 && s.direct.active.editing === false && typeof s.direct.active.box?.left === 'number' && s.direct.active.target?.kind === 'uri', s.direct.active);
  await t.shot('review-u4-b2-link-moved.png');
  await undo(t); s = await t.state();
  check('b3 undo restores L1', near(s.links[0].rect, [72, 72, 200, 92], 0.01) && s.dirty === false, { rect: s.links[0].rect, dirty: s.dirty });
  let hh = await handles(t, L(10)); let se = hh.find(h => h.name === 'se');
  await t.drag(se.cx, se.cy, 20 * S, 10 * S);
  s = await t.state(); ops = await t.ops();
  check('b4 se corner drag resizes L1 (one link-set)', q(ops) === q(['link-set']) && near(s.links[0].rect, [72, 72, 220, 102]), { ops, rect: s.links[0].rect });
  await undo(t);
  s = await t.state(); const revE = s.revision;
  b = await t.rect(L(10));
  await t.drag(b.cx, b.cy, 40 * S, 40 * S, { escape: true });
  s = await t.state(); ops = await t.ops(); const bE = await t.rect(L(10));
  check('b5 Esc mid-drag: no request, box snaps back', ops.length === 0 && s.revision === revE && Math.abs(bE.x - b.x) < 1 && Math.abs(bE.y - b.y) < 1, { ops, rev: s.revision, before: [b.x, b.y], after: [bE.x, bE.y] });
  b = await t.rect(L(10));
  await t.drag(b.cx, b.cy, -200 * S, -200 * S);
  s = await t.state(); ops = await t.ops(); ui = await t.ui();
  check('b6 drag off the page → clamped to the page edge by gestureRect (one link-set, no error)', q(ops) === q(['link-set']) && near(s.links[0].rect, [0, 0, 128, 20]), { ops, rect: s.links[0].rect, error: ui.error });
  await undo(t);
  // forced engine refusal: mangle annotId → no-such-annot; box must snap back and #error show engine text
  await t.eval(`window.__mangle = { op: 'link-set', patch: { annotId: 9999 } }`);
  s = await t.state(); const revR = s.revision;
  const st = () => t.eval(`(() => { const e = ${L(10)}; return e ? [e.style.left, e.style.top] : null; })()`); const stB = await st();
  b = await t.rect(L(10));
  await t.drag(b.cx, b.cy, 30 * S, 30 * S, { w: 2000 });
  await t.eval(`window.__mangle = null`);
  s = await t.state(); ops = await t.ops(); ui = await t.ui(); const bR = await t.rect(L(10)); const stA = await st();
  check('b7 engine refusal on a drag: snaps back, #error = engine error, nav.last detail, not dirty', q(ops) === q(['link-set']) && s.revision === revR && /has no link 9999/.test(ui.error) && s.error === ui.error && s.nav.last?.detail === 'no-such-annot' && q(stA) === q(stB) && near(s.links[0].rect, [72, 72, 200, 92], 0.01) && !s.dirty, { ops, rev: s.revision, style: [stB, stA], rect: s.links[0].rect, error: ui.error, stateError: s.error, last: s.nav.last, before: [b.x, b.y], after: [bR.x, bR.y] });
  await t.shot('review-u4-b7-refusal.png');
  // click → focus + aria-current row
  b = await t.rect(L(10)); await t.click(b.cx, b.cy);
  s = await t.state(); ui = await t.ui();
  const row = await t.eval(`[...document.querySelectorAll('#link-list li[aria-current="true"]')].map(r => r.textContent)`);
  check('b8 click L1 → focus, direct.active link, Links row aria-current', /link-target/.test(ui.active) && s.direct.active?.kind === 'link' && s.direct.active.annotId === 10 && s.direct.active.editing === false && row.length === 1, { focus: ui.active, active: s.direct.active, row });
  const scroll0 = ui.scroll;
  await t.key('ArrowRight', 0, 1500); s = await t.state(); ops = await t.ops(); ui = await t.ui();
  check('b9 plain → moves L1 1 pt (one op), no scroll, focus kept', q(ops) === q(['link-set']) && near(s.links[0].rect, [73, 72, 201, 92], 0.05) && ui.scroll === scroll0 && /link-target/.test(ui.active), { ops, rect: s.links[0].rect, scroll: [scroll0, ui.scroll], focus: ui.active });
  await t.key('ArrowDown', SHIFT, 1500); s = await t.state(); ops = await t.ops();
  check('b10 Shift+↓ moves 10 pt', q(ops) === q(['link-set']) && near(s.links[0].rect, [73, 82, 201, 102], 0.05), { ops, rect: s.links[0].rect, focus: (await t.ui()).active });
  await t.key('ArrowRight', ALT, 1500); s = await t.state(); ops = await t.ops();
  check('b11 ⌥→ still 1 pt', q(ops) === q(['link-set']) && near(s.links[0].rect, [74, 82, 202, 102], 0.05), { ops, rect: s.links[0].rect });
  ui = await t.ui();
  await t.key('ArrowRight', META, 1000); s = await t.state(); ops = await t.ops();
  check('b12 ⌘→ not consumed (no op)', ops.length === 0 && near(s.links[0].rect, [74, 82, 202, 102], 0.05), { ops, rect: s.links[0].rect });
  for (let i = 0; i < 3; i++) await undo(t);
  s = await t.state();
  check('b13 three undos restore L1', near(s.links[0].rect, [72, 72, 200, 92], 0.01) && !s.dirty, { rect: s.links[0].rect, dirty: s.dirty });
  // Delete + undo
  b = await t.rect(L(10)); await t.click(b.cx, b.cy);
  await t.key('Delete', 0, 1500); s = await t.state(); ops = await t.ops();
  check('b14 real Delete deletes L1 (one op), focus drops', q(ops) === q(['link-delete']) && s.direct.links === 2 && s.links.length === 2 && s.direct.active === null, { ops, links: s.links.length, direct: s.direct.links, active: s.direct.active, nav: (await t.ui()).nav });
  await undo(t); s = await t.state();
  check('b15 undo restores L1 with its target', s.links.length === 3 && s.links[0].uri === 'https://example.org/one' && s.links[0].annotId === 10, { links: s.links });
  // Enter → editor, URI selected, insertText, Enter
  b = await t.rect(L(10)); await t.click(b.cx, b.cy);
  await t.key('Enter', 0, 500); s = await t.state();
  const sel = await t.eval(`(() => { const e = document.activeElement; return { cls: e.className, v: e.value, a: e.selectionStart, b: e.selectionEnd }; })()`);
  check('b16 Enter opens the inline editor with the URI selected', s.direct.active?.editing === true && s.direct.active.draft === 'https://example.org/one' && /link-editor/.test(sel.cls) && sel.a === 0 && sel.b === sel.v.length, { active: s.direct.active, sel });
  await t.shot('review-u4-b16-link-editor.png');
  await t.type('https://example.org/changed'); await sleep(200);
  await t.key('Enter', 0, 1500); s = await t.state(); ops = await t.ops();
  const rowText = await t.eval(`[...document.querySelectorAll('#link-list li')].map(r => r.textContent).join(' | ')`);
  check('b17 Enter commits → links[0].uri changed, Links row updated, editor closed', q(ops) === q(['link-set']) && s.links[0].uri === 'https://example.org/changed' && rowText.includes('example.org/changed') && s.direct.active?.editing !== true, { ops, uri: s.links[0].uri, rowText, active: s.direct.active });
  // bad scheme
  s = await t.state(); const revS = s.revision;
  b = await t.rect(L(10)); await t.click(b.cx, b.cy);
  await t.key('Enter', 0, 400); await t.type('ftp://nope'); await t.key('Enter', 0, 1500);
  s = await t.state(); ops = await t.ops(); ui = await t.ui();
  check('b18 bad scheme: editor stays open with draft, exact #error, detail scheme, revision unchanged', q(ops) === q(['link-set']) && s.direct.active?.editing === true && s.direct.active.draft === 'ftp://nope' && ui.error === 'link-set: ftp: is not a link this app opens' && s.error === ui.error && s.nav.last?.detail === 'scheme' && s.revision === revS && s.links[0].uri === 'https://example.org/changed', { ops, active: s.direct.active, error: ui.error, stateErr: s.error, last: s.nav.last, rev: [revS, s.revision] });
  await t.shot('review-u4-b18-scheme-refusal.png');
  await t.key('Escape', 0, 800); s = await t.state(); ops = await t.ops();
  check('b19 Escape closes the editor, no request', ops.length === 0 && s.direct.active?.editing !== true, { ops, active: s.direct.active });
  // KNOWN DEVIATION §9.6: dropDirect link branch shows a refusal on a mode change
  b = await t.rect(L(10)); await t.click(b.cx, b.cy);
  await t.key('Enter', 0, 400); await t.type('ftp://nope'); await sleep(200);
  await t.eval(`document.getElementById('error').hidden = true`);
  await t.mode('read'); await sleep(1500);
  s = await t.state(); ops = await t.ops(); ui = await t.ui();
  check('b20 §9.6 dropDirect on mode change commits SILENTLY (expect no #error)', ui.error === '' && !s.error, { ops, error: ui.error, stateError: s.error, last: s.nav.last, mode: s.mode });
  await t.shot('review-u4-b20-dropdirect-error.png');
  await t.mode('annotate'); await t.eval(`document.getElementById('error').hidden = true`); await t.ops();
  // page-link retarget with digits
  b = await t.rect(L(11)); await t.click(b.cx, b.cy);
  await t.key('Enter', 0, 400); s = await t.state();
  const draft2 = s.direct.active?.draft;
  await t.type('1'); await t.key('Enter', 0, 1500); s = await t.state(); ops = await t.ops();
  check('b21 L2 editor draft "3"; digits 1 → page 0', draft2 === '3' && q(ops) === q(['link-set']) && s.links[1].kind === 'page' && s.links[1].page === 0, { draft2, ops, l2: s.links[1] });
  // out-of-range page number
  b = await t.rect(L(11)); await t.click(b.cx, b.cy);
  await t.key('Enter', 0, 400); await t.type('9'); await t.key('Enter', 0, 800); s = await t.state(); ops = await t.ops(); ui = await t.ui();
  check('b22 page 9 → "Enter a page number between 1 and 3", editor open, no request', ops.length === 0 && ui.error === 'Enter a page number between 1 and 3' && s.direct.active?.editing === true, { ops, error: ui.error, active: s.direct.active });
  await t.key('Escape', 0, 500); await t.ops();
  // blur commits
  b = await t.rect(L(10)); await t.click(b.cx, b.cy);
  await t.key('Enter', 0, 400); await t.type('https://example.org/blur'); await sleep(200);
  p = await t.at(400, 600); await t.click(p.x, p.y, 1800);
  s = await t.state(); ops = await t.ops();
  check('b23 blur (click empty page) commits once', q(ops) === q(['link-set']) && s.links[0].uri === 'https://example.org/blur' && s.direct.active?.editing !== true, { ops, uri: s.links[0].uri, active: s.direct.active });
  // undo closes the editor (unchanged value → no request; the undo itself runs)
  b = await t.rect(L(10)); await t.click(b.cx, b.cy);
  await t.key('Enter', 0, 400); s = await t.state(); const revU = s.revision;
  await t.btn('undo'); s = await t.state(); ops = await t.ops();
  check('b24 Undo with an unchanged editor open: editor closed, only the undo runs', !ops.includes('link-set') && s.direct.active?.editing !== true && s.links[0].uri === 'https://example.org/changed', { ops, active: s.direct.active, uri: s.links[0].uri, rev: [revU, s.revision] });
  // double-click opens the editor
  await t.eval(`(() => { window.__ev = []; for (const k of ['pointerdown','click','dblclick']) document.addEventListener(k, e => window.__ev.push(k + ':' + (e.target.className || e.target.tagName) + ':' + e.detail), true); })()`);
  await t.at(136, 82); b = await t.rect(L(10)); await t.move(b.cx, b.cy); await t.press(b.cx, b.cy, 0, 1); await t.release(b.cx, b.cy, 0, 1); await t.press(b.cx, b.cy, 0, 2); await t.release(b.cx, b.cy, 0, 2); await sleep(800);
  s = await t.state(); ops = await t.ops();
  check('b25 double-click opens the link editor, no request', s.direct.active?.editing === true && ops.length === 0, { active: s.direct.active, ops, ev: await t.eval('window.__ev') });
  await t.key('Escape', 0, 400);

  // ---- (c) Draw link ----
  await t.eval(`document.getElementById('tab-links').click()`); await sleep(300);
  s = await t.state(); const n0 = s.links.length;
  await t.btn('link-draw');
  const pressed = await t.eval(`document.getElementById('link-draw').getAttribute('aria-pressed')`);
  p = await t.at(300, 400);
  await t.drag(p.x, p.y, 120 * p.s, 20 * p.s);
  s = await t.state(); ops = await t.ops();
  const dlg = await t.eval(`({ open: document.getElementById('link-dialog').open, kind: document.getElementById('link-kind')?.value, pressed: document.getElementById('link-draw').getAttribute('aria-pressed') })`);
  check('c1 armed real drag → dialog with select#link-kind, nav.linkDraft, nav.last link-draw, disarmed', pressed === 'true' && dlg.open && dlg.kind && dlg.pressed === 'false' && s.nav.linkDraft?.page === 0 && near(s.nav.linkDraft.rect, [300, 400, 420, 420]) && s.nav.last?.action === 'link-draw' && ops.length === 0, { pressed, dlg, draft: s.nav.linkDraft, last: s.nav.last, ops });
  await t.shot('review-u4-c1-draw-dialog.png');
  // choose Web with a real select change, type URI, Save
  await t.eval(`(() => { const k = document.getElementById('link-kind'); k.value = 'uri'; k.dispatchEvent(new Event('change', { bubbles: true })); const i = document.getElementById('link-target'); i.focus(); i.select(); })()`);
  await t.type('https://example.org/drawn'); await sleep(200);
  await t.btn('link-save'); s = await t.state(); ops = await t.ops();
  const nl = s.links.find(l => l.uri === 'https://example.org/drawn');
  check('c2 Save → one link-add, new link in links[] and painted, draft cleared', q(ops) === q(['link-add']) && s.links.length === n0 + 1 && nl && near(nl.rect, [300, 400, 420, 420]) && s.direct.links === n0 + 1 && s.nav.linkDraft === null, { ops, n: s.links.length, nl, direct: s.direct.links, draft: s.nav.linkDraft });
  await t.shot('review-u4-c2-drawn.png');
  await t.btn('link-draw');
  p = await t.at(300, 470); await t.drag(p.x, p.y, 2, 0);
  s = await t.state(); ops = await t.ops(); ui = await t.ui();
  const dlg2 = await t.eval(`({ open: document.getElementById('link-dialog').open, pressed: document.getElementById('link-draw').getAttribute('aria-pressed') })`);
  check('c3 2 px drag: disarmed, no draft, no dialog, no error', !dlg2.open && dlg2.pressed === 'false' && s.nav.linkDraft === null && ui.error === '' && ops.length === 0, { dlg2, draft: s.nav.linkDraft, error: ui.error, ops });
  await t.btn('link-draw');
  p = await t.at(300, 470); await t.drag(p.x, p.y, 3 * p.s, 2 * p.s);
  s = await t.state(); ui = await t.ui();
  check('c4 3×2 pt drag → "Draw a bigger link", disarmed, no draft', ui.error === 'Draw a bigger link' && s.nav.linkDraft === null && (await t.eval(`document.getElementById('link-draw').getAttribute('aria-pressed')`)) === 'false', { error: ui.error, draft: s.nav.linkDraft });
  // Escape on the draw dialog clears the draft
  await t.btn('link-draw'); p = await t.at(300, 520); await t.drag(p.x, p.y, 100 * p.s, 20 * p.s);
  await t.key('Escape', 0, 600); s = await t.state();
  check('c5 Escape on a drawn dialog → closed, nav.linkDraft null, nothing added', !(await t.eval(`document.getElementById('link-dialog').open`)) && s.nav.linkDraft === null && s.links.length === n0 + 1, { draft: s.nav.linkDraft, n: s.links.length });
  await t.ops();

  // ---- (d) rotated page 1 ----
  await t.btn('next');
  await wait(async () => { const v = await t.state(); return v.page === 1 && v.direct.links === 1; });
  s = await t.state();
  const l4 = s.links[0].rect; const S1 = (await t.at(710, 136, 792)).s;
  b = await t.rect(L(14));
  await t.drag(b.cx, b.cy, 40 * S1, 30 * S1);
  s = await t.state(); ops = await t.ops();
  check('d1 rotated page: real drag right+down moves L4 right+down (not mirrored)', q(ops) === q(['link-set']) && near(s.links[0].rect, [l4[0] + 40, l4[1] + 30, l4[2] + 40, l4[3] + 30], 0.6), { ops, before: l4, after: s.links[0].rect });
  await t.shot('review-u4-d1-rotated-move.png');
  preSave = { p0: null, p1: s.links };
  await t.btn('prev');
  await wait(async () => (await t.state()).page === 0);
  preSave.p0 = (await t.state()).links;
  await t.btn('save-copy');
  await wait(async () => { try { return (await stat(saved)).size > 0; } catch { return false; } }, 20_000);
  await sleep(1000);
  // Help
  const help = await t.eval(`(() => { const d = document.getElementById('help-dialog'); return { rows: d.querySelectorAll('table tbody tr').length, text: d.textContent }; })()`);
  const want = ['followed by clicking them in Read mode', 'Draw link', '/Tabs order', 'plain arrows nudge', 'Outline objects', 'dragging on empty page space in Redact mode', 'Forms, Edit text and Redact modes paint no link targets', 'top index (/TI) is not changed', '←/→ move the selection'];
  check('h1 Help: U4 text present, 11 rows, "not yet editable" gone', help.rows === 11 && want.every(w => help.text.includes(w)) && !help.text.includes('not yet editable'), { rows: help.rows, missing: want.filter(w => !help.text.includes(w)) });
});

// Undo toolbar button while a CHANGED link editor is open (real mouse: the button's mousedown blurs the editor first)
await session('undo2', links, [], async t => {
  await t.mode('annotate');
  await wait(async () => (await t.state()).direct.links === 3, 20_000); await t.ops();
  await t.at(136, 82); const b = await t.rect(L(10)); await t.click(b.cx, b.cy);
  await t.key('Enter', 0, 400); await t.type('https://example.org/undo-click'); await sleep(200);
  await t.btn('undo'); await sleep(1000);
  const s = await t.state(); const ops = await t.ops();
  check('b26 Undo button with a changed link draft: no link-set lands (§1.5 undo closes without a request)', !ops.includes('link-set') && s.links[0].uri === 'https://example.org/one', { ops, uri: s.links[0].uri, dirty: s.dirty, rev: s.revision, canUndo: s.canUndo, active: s.direct.active });
  await t.shot('review-u4-b26-undo-button-commits.png');
});

// reopen the saved copy
await session('reopen', saved, [], async t => {
  await wait(async () => (await t.state()).direct.links >= 3, 20_000);
  let s = await t.state(); const p0 = s.links;
  await t.btn('next'); await wait(async () => (await t.state()).page === 1);
  await sleep(800); s = await t.state(); const p1 = s.links;
  const same = (a, b) => a && b && a.length === b.length && a.every((l, i) => near(l.rect, b[i].rect, 0.01) && l.uri === b[i].uri && l.page === b[i].page && l.kind === b[i].kind);
  check('d2 save → reopen: page 0 and rotated page 1 links persisted', same(p0, preSave?.p0) && same(p1, preSave?.p1), { p0, pre0: preSave?.p0, p1, pre1: preSave?.p1 });
});

// field over a link (nav.pdf, Read mode)
await session('nav', join(root, 'fixtures/nav.pdf'), [], async t => {
  await wait(async () => (await t.state()).links?.length >= 1, 20_000); await sleep(800);
  await wait(async () => (await t.state()).direct.targets >= 1, 20_000); await sleep(500);
  log('nav mode after fields add', (await t.state()).mode);
  await t.mode('read'); await sleep(800);
  const s = await t.state(); log('nav mode', s.mode, 'links', s.direct.links);
  const fieldsRects = await t.eval(`[...document.querySelectorAll('#hit-layer .hit-target[data-field-id]')].map(e => ({ id: e.dataset.fieldId, r: e.getBoundingClientRect().toJSON() }))`);
  const linkRects = await t.eval(`[...document.querySelectorAll('#hit-layer .link-target')].map(e => ({ id: e.dataset.annotId, r: e.getBoundingClientRect().toJSON() }))`);
  const inside = (a, b) => a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
  const over = fieldsRects.find(f => linkRects.some(l => inside(f.r, l.r)));
  const under = linkRects.find(l => fieldsRects.some(f => inside(l.r, f.r)));
  log('nav fields', q(fieldsRects.length), 'links', q(linkRects.length), 'field-in-link', q(over), 'link-under-field', q(under));
  if (over) {
    await t.click(over.r.x + over.r.width / 2, over.r.y + over.r.height / 2, 1200);
    const s2 = await t.state();
    check('e0 Read mode: a field over a link still fills (field wins, no link-follow)', s2.direct.active?.kind === 'field' && s2.nav.last?.action !== 'link-follow' && s2.page === s.page, { active: s2.direct.active, last: s2.nav.last, page: s2.page });
    await t.shot('review-u4-e0-field-over-link.png');
    await t.key('Escape', 0, 400);
    const pl = await t.at(136, 106); await t.click(pl.x, pl.y, 1200); const s3 = await t.state();
    check('e0b Read mode: the link part outside the field still follows', s3.nav.last?.action === 'link-follow', { last: s3.nav.last, page: s3.page });
  } else check('e0 nav.pdf has a field inside a link', false, { fieldsRects, linkRects });
}, [{ cmd: 'fields', contents: { action: 'add', kind: 'text', name: 'edge', rect: [72, 72, 252, 96] } }]);

// ---------- (e) tabs.pdf ----------
await session('tabs', join(root, 'fixtures/p6/tabs.pdf'), [], async t => {
  await t.mode('forms');
  const want = [
    { tabs: 'C', used: 'C', order: ['a1', 'a2', 'a3', 'b1', 'b2', 'b3'], start: [132, 82] },
    { tabs: 'A', used: 'A', order: ['t3', 't1', 't2'], start: [428, 82] },
    { tabs: 'S', used: 'S', order: ['s2', 's3', 's1'], start: [280, 82] },
    { tabs: 'S', used: 'fallback', order: ['u1', 'u2'], start: [132, 82] },
  ];
  for (let pg = 0; pg < 4; pg++) {
    if (pg) { await t.btn('next'); }
    await wait(async () => { const v = await t.state(); return v.page === pg && v.fields?.order?.length === want[pg].order.length; }, 20_000);
    let s = await t.state();
    check(`e${pg}a page ${pg} fields.tabs/tabsOrder/order`, s.fields.tabs === want[pg].tabs && s.fields.tabsOrder === want[pg].used && q(s.fields.order) === q(want[pg].order), { tabs: s.fields.tabs, used: s.fields.tabsOrder, order: s.fields.order });
    const p = await t.at(...want[pg].start); await t.click(p.x, p.y, 800);
    const seen = [(await t.state()).direct.active?.name];
    for (let i = 1; i < want[pg].order.length; i++) { await t.key('Tab', 0, 700); seen.push((await t.state()).direct.active?.name); }
    check(`e${pg}b page ${pg} real Tab walks ${want[pg].order.join('→')}`, q(seen) === q(want[pg].order), { seen });
    await t.key('Tab', SHIFT, 700); const back = (await t.state()).direct.active?.name;
    check(`e${pg}c page ${pg} Shift+Tab goes back`, back === want[pg].order.at(-2), { back });
    if (pg === 0) {
      await t.shot('review-u4-e0-tab.png');
      await t.key('Escape', 0, 400); await t.key('Escape', 0, 400);
      const p0 = await t.at(132, 82); await t.click(p0.x, p0.y, 800);
      const viaBtn = [(await t.state()).direct.active?.name];
      for (let i = 0; i < 3; i++) { await t.btn('field-next'); viaBtn.push((await t.state()).direct.active?.name); }
      check('e0d field-bar Next follows the /Tabs C order', q(viaBtn) === q(['a1', 'a2', 'a3', 'b1']), { viaBtn, page: (await t.state()).page });
      if ((await t.state()).page !== 0) { await t.btn('prev'); }
    }
    await t.key('Escape', 0, 400);
  }
  const s = await t.state();
  check('e5 no field was dirtied by Tab/Next', !s.dirty, { dirty: s.dirty, rev: s.revision });
});

// ---------- (f)(g) direct3.pdf ----------
await session('direct3', join(root, 'fixtures/p6/direct3.pdf'), [], async t => {
  await t.mode('edit');
  await wait(async () => (await t.state()).direct.objects === 3, 20_000);
  await t.move(5, 5); await sleep(400);
  let s = await t.state();
  const outl = await t.eval(`[...document.querySelectorAll('#hit-layer .object-target')].map(e => getComputedStyle(e).outlineStyle + ' ' + getComputedStyle(e).outlineColor)`);
  const txt = await t.eval(`[...document.querySelectorAll('#hit-layer .text-target:not(.unsupported)')].slice(0, 3).map(e => getComputedStyle(e).outlineStyle)`);
  check('f1 Edit: outlines on without hover (default), direct.outlines true', s.direct.outlines === true && outl.length === 3 && outl.every(o => o.startsWith('dashed')) && txt.every(o => o === 'dashed'), { outlines: s.direct.outlines, outl, txt });
  await t.shot('review-u4-f1-outlines.png');
  const cb = await t.rect(`document.getElementById('edit-outlines')`);
  await t.click(cb.cx, cb.cy, 600); s = await t.state();
  const off = { state: s.direct.outlines, ls: await t.eval(`localStorage.getItem('edit-outlines')`), cls: await t.eval(`document.getElementById('hit-layer').classList.contains('outlined')`), style: await t.eval(`getComputedStyle(document.querySelector('#hit-layer .object-target')).outlineStyle`) };
  check('f2 checkbox off → no outlines, localStorage "false", STATE false', off.state === false && off.ls === 'false' && !off.cls && off.style !== 'dashed', off);
  await t.shot('review-u4-f2-outlines-off.png');
  await t.click(cb.cx, cb.cy, 600); s = await t.state();
  check('f3 checkbox on again → persisted "true"', s.direct.outlines === true && (await t.eval(`localStorage.getItem('edit-outlines')`)) === 'true', { state: s.direct.outlines });
  await t.ops();
  const sel = `(() => { const o = window.pdf4allState().content.objects.find(i => i.name === 'Im2'); return o && document.querySelector('#hit-layer .object-target[data-target-id="' + CSS.escape(String(o.targetId)) + '"]'); })()`;
  const im2 = await t.rect(sel); await t.click(im2.cx, im2.cy);
  const before = (await t.state()).content.objects.find(o => o.name === 'Im2').box; const sc = (await t.ui()).scroll;
  await t.key('ArrowRight', 0, 1800); s = await t.state(); let ops = await t.ops();
  const after = s.content.objects.find(o => o.name === 'Im2').box;
  check('f4 plain → on Im2 moves 1 pt (one op), no scroll', ops.length === 1 && near(after, [before[0] + 1, before[1], before[2] + 1, before[3]], 0.05) && (await t.ui()).scroll === sc, { ops, before, after });
  await t.btn('undo'); await t.ops();
  // ---- (g) Redact ----
  await t.mode('redact');
  await wait(async () => (await t.state()).mode === 'redact');
  s = await t.state(); const m0 = s.redact.marks;
  let p = await t.at(300, 500);
  await t.drag(p.x, p.y, 100 * p.s, 30 * p.s, { w: 2000 });
  s = await t.state(); ops = await t.ops();
  check('g1 unarmed real drag on empty space → one mark with the dragged rect', ops.length === 1 && s.redact.marks === m0 + 1 && near(s.redact.rects.at(-1)?.rect, [300, 500, 400, 530], 0.6) && (await t.ui()).error === '', { ops, marks: s.redact.marks, rects: s.redact.rects });
  const selText = await t.eval(`String(window.getSelection())`);
  await t.shot('review-u4-g1-redact-drag.png');
  p = await t.at(300, 300); await t.drag(p.x, p.y, 2, 0, { w: 1000 });
  s = await t.state(); ops = await t.ops(); let ui = await t.ui();
  check('g2 2 px drag → no mark, no error', ops.length === 0 && s.redact.marks === m0 + 1 && ui.error === '', { ops, marks: s.redact.marks, error: ui.error });
  await t.click(p.x, p.y, 1000); s = await t.state(); ops = await t.ops(); ui = await t.ui();
  check('g3 click → no mark, no error', ops.length === 0 && s.redact.marks === m0 + 1 && ui.error === '', { ops, marks: s.redact.marks, error: ui.error, selText });
  await t.btn('redact-area');
  const armed = await t.eval(`document.getElementById('redact-area').getAttribute('aria-pressed')`);
  p = await t.at(100, 300); await t.drag(p.x, p.y, 80 * p.s, 40 * p.s, { w: 2000 });
  s = await t.state(); ops = await t.ops();
  check('g4 Mark area still works and disarms', armed === 'true' && ops.length === 1 && s.redact.marks === m0 + 2 && near(s.redact.rects.at(-1)?.rect, [100, 300, 180, 340], 0.6) && (await t.eval(`document.getElementById('redact-area').getAttribute('aria-pressed')`)) === 'false', { armed, ops, marks: s.redact.marks, rect: s.redact.rects.at(-1) });
  await t.btn('redact-area'); p = await t.at(100, 600); await t.click(p.x, p.y, 1000);
  s = await t.state(); ops = await t.ops(); ui = await t.ui();
  check('g5 armed Mark area + click: no degenerate request, disarmed, no error', ops.length === 0 && ui.error === '' && (await t.eval(`document.getElementById('redact-area').getAttribute('aria-pressed')`)) === 'false', { ops, error: ui.error });
});

// field grip on form2.pdf
await session('form2', join(root, 'fixtures/form2.pdf'), [], async t => {
  await t.mode('forms');
  const sel = `document.querySelector('#hit-layer .hit-target[data-field-id]')`;
  await wait(() => t.rect(sel), 20_000);
  const hs = await handles(t, sel);
  const s0 = await t.state(); const g = hs.find(h => h.name === 'grip'); const S = (await t.at(0, 0)).s;
  await t.drag(g.cx, g.cy, 20 * S, 20 * S, { w: 2000 });
  const s1 = await t.state(); const ops = await t.ops();
  check('i1 field grip drag → one field-props', hs.filter(h => h.visible).length === 9 && q(ops) === q(['field-props']) && s1.revision === s0.revision + 1, { handles: hs.map(h => `${h.name}:${h.visible ? 'v' : '-'}${h.hit ? 'h' : '-'}`), ops });
  await t.shot('review-u4-i1-field-drag.png');
});

await writeFile(join(out, 'review-u4-ui.json'), JSON.stringify({ results, shots }, null, 1));
log('fixtures after\n' + hashes());
log('SUMMARY', results.filter(r => r.ok).length, '/', results.length, 'shots', shots.join(' '));
