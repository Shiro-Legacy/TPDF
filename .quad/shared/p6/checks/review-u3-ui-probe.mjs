#!/usr/bin/env node
/** P6 U3 review (renderer + human UX lens): real CDP mouse/keys on direct3.pdf, then one field drag on form2.pdf.
 * Run under .quad/run.lock after npm run build. Writes review-out/review-u3-*.png and review-out/review-u3-ui.json. */
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
const results = [];
const check = (id, ok, detail) => { results.push({ id, ok: !!ok, detail }); log(ok ? 'OK  ' : 'BAD ', id, q(detail)); };
const hashes = () => execFileSync('shasum', ['-a', '256', '-c', join(checksDir, 'fixtures.sha256')], { cwd: root, encoding: 'utf8' });
const near = (a, b, tol = 0.5) => Array.isArray(a) && a.length === b.length && a.every((v, i) => Math.abs(v - b[i]) <= tol);

async function session(name, lines, body) {
  const udata = join(out, `u3-review-udata-${name}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `review-u3-ui-${name}.jsonl`);
  await writeFile(probe, [...lines, ...hold(60, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `review-u3-${name}-out`)}`, `--report=${join(out, `review-u3-${name}-report.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  const closed = new Promise(done => child.on('close', done));
  let cdp;
  try {
    cdp = await attach(port);
    try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch {}
    await body(tools(cdp));
  } catch (err) { check(`${name}:crash`, false, String(err?.stack ?? err)); }
  finally { cdp?.close(); child.kill('SIGTERM'); log('electron closed', await closed); }
}
function tools(cdp) {
  const t = {};
  t.cdp = cdp;
  t.state = () => cdp.eval('window.pdf4allState()');
  t.eval = s => cdp.eval(s);
  t.shot = async name => { const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }); await writeFile(join(out, name), Buffer.from(data, 'base64')); log('shot', name); };
  t.move = (x, y, modifiers = 0, buttons = 0) => cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, modifiers, buttons, button: buttons ? 'left' : 'none' });
  t.press = (x, y, modifiers = 0, clickCount = 1) => cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount, modifiers });
  t.release = (x, y, modifiers = 0, clickCount = 1) => cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount, modifiers });
  t.click = async (x, y, wait = 700) => { await t.move(x, y); await sleep(100); await t.press(x, y); await t.release(x, y); await sleep(wait); };
  t.dblclick = async (x, y) => { await t.move(x, y); await sleep(100); await t.press(x, y, 0, 1); await t.release(x, y, 0, 1); await t.press(x, y, 0, 2); await t.release(x, y, 0, 2); await sleep(1500); };
  t.drag = async (x, y, dx, dy, { modifiers = 0, escape = false, wait = 1500 } = {}) => {
    await t.move(x, y); await sleep(150); await t.press(x, y, modifiers);
    for (let i = 1; i <= 8; i++) await t.move(x + dx * i / 8, y + dy * i / 8, modifiers, 1);
    if (escape) { await t.key('Escape', 'Escape', 27); await sleep(150); }
    await t.release(x + dx, y + dy, modifiers); await sleep(wait);
  };
  t.key = async (key, code, vk, modifiers = 0, text) => {
    await cdp.send('Input.dispatchKeyEvent', { type: text ? 'keyDown' : 'rawKeyDown', key, code, windowsVirtualKeyCode: vk, modifiers, ...(text ? { text, unmodifiedText: text } : {}) });
    await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: vk, modifiers });
  };
  t.type = text => cdp.send('Input.insertText', { text });
  t.rect = sel => cdp.eval(`(() => { const e = ${sel}; if (!e) return null; if (e.closest('#hit-layer')) e.scrollIntoView({ block: 'center', inline: 'center' }); const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2 }; })()`);
  t.ops = () => cdp.eval('window.__ops.splice(0)');
  t.ui = () => cdp.eval(`({ error: document.getElementById('error').hidden ? '' : document.getElementById('error').textContent, active: (() => { const a = document.activeElement; return a ? a.tagName + '.' + a.className + (a.dataset.targetId ? '#' + a.dataset.targetId : '') + (a.dataset.annotId ? '@' + a.dataset.annotId : '') : null; })(), undo: !document.getElementById('undo').disabled })`);
  return t;
}
async function at(t, x, y, pw = 612) {
  return t.eval(`(() => {
    const v = document.getElementById('viewport'), c = document.getElementById('canvas');
    let r = c.getBoundingClientRect(); const s = r.width / ${pw};
    v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
    r = c.getBoundingClientRect();
    return { x: r.left + ${x} * s, y: r.top + ${y} * s, s };
  })()`);
}
const HOOK = `(() => { if (window.__ops) return true; window.__ops = []; window.__mangle = null; const orig = Worker.prototype.postMessage;
  Worker.prototype.postMessage = function (msg, tr) { try { if (msg && msg.op && !['state','render','fields','notes','annots','edit-targets','redact-preview','text','search','outline','links','thumb','thumbnail','page-ids','page-text'].includes(msg.op)) window.__ops.push(msg.op + (msg.action ? ':' + msg.action : '')); if (window.__mangle && msg && msg.op === window.__mangle) msg = { ...msg, generation: -7, revision: -7 }; } catch {} return orig.call(this, msg, tr); }; return true; })()`;
const objBox = async (t, name) => (await t.state()).content.objects.find(o => o.name === name)?.box;
const objSel = name => `(() => { const o = window.pdf4allState().content.objects.find(i => i.name === ${q(name)}); return o && document.querySelector('#hit-layer .object-target[data-target-id="' + CSS.escape(String(o.targetId)) + '"]'); })()`;
async function undoClick(t) { const u = await t.rect(`document.getElementById('undo')`); await t.click(u.cx, u.cy, 1500); }
async function handles(t, sel) {
  const box = await t.rect(sel); await t.move(box.cx, box.cy); await sleep(300);
  return t.eval(`(() => { const b = ${sel}; return [...b.querySelectorAll('[data-handle]')].map(h => { const r = h.getBoundingClientRect(); const cx = r.x + r.width / 2, cy = r.y + r.height / 2; const hit = document.elementFromPoint(cx, cy); return { name: h.dataset.handle, visible: getComputedStyle(h).display !== 'none' && r.width > 0, hit: hit === h, cx, cy }; }); })()`);
}

await mkdir(out, { recursive: true });
log('fixtures before\n' + hashes());
const direct3 = join(root, 'fixtures/p6/direct3.pdf');
await session('main', [
  { cmd: 'open', path: direct3, images: [join(root, 'fixtures/img-rgb.png')] },
  { cmd: 'redact', contents: { page: 0, rect: [300, 600, 400, 650] } },
  { cmd: 'redact', contents: { search: 'Move me' } },
], async t => {
  await wait(() => t.eval(`document.title.includes('direct3')`), 60_000);
  await wait(async () => (await t.state()).redact.marks === 2, 30_000);
  await sleep(800);
  await t.eval(HOOK);
  await t.eval(`document.getElementById('mode-edit').click()`);
  await wait(async () => (await t.state()).direct.objects === 3, 20_000);
  let s = await t.state(); const rev0 = s.revision;
  check('a0 objects/targets', s.direct.objects === 3, { objects: s.direct.objects, targets: s.direct.targets, marks: s.direct.marks });
  const im2c = await at(t, 430, 72); const S = im2c.s;
  // (a) hover handles
  const hs = await handles(t, objSel('Im2'));
  check('a1 Im2 hover: grip + 8 handles visible and hit-testable', hs.length === 9 && hs.every(h => h.visible && h.hit), hs.map(h => `${h.name}:${h.visible ? 'v' : '-'}${h.hit ? 'h' : '-'}`));
  await t.shot('review-u3-a1-hover-handles.png');
  // body drag
  await t.ops();
  const b2 = await t.rect(objSel('Im2'));
  await t.drag(b2.cx, b2.cy, 50 * S, 30 * S);
  s = await t.state(); let ops = await t.ops(); let ui = await t.ui();
  check('a2 body drag Im2 → one edit-object move', ops.length === 1 && ops[0] === 'edit-object:move' && near(await objBox(t, 'Im2'), [450, 670, 510, 710]) && s.dirty && s.canUndo && s.revision === rev0 + 1,
    { ops, box: await objBox(t, 'Im2'), dirty: s.dirty, canUndo: s.canUndo, rev: s.revision, edited: s.content.edited, active: s.direct.active, focus: ui.active });
  check('a2b after drag STATE direct.active is object Im2 with §9.11 shape', s.direct.active?.kind === 'object' && s.direct.active?.name === 'Im2' && Array.isArray(s.direct.active?.placement) && typeof s.direct.active?.box?.left === 'number' && Array.isArray(s.direct.active?.ops), s.direct.active);
  await t.shot('review-u3-a2-body-drag.png');
  await undoClick(t); s = await t.state(); await t.ops();
  check('a3 undo restores Im2', near(await objBox(t, 'Im2'), [400, 700, 460, 740], 0.01), { box: await objBox(t, 'Im2'), rev: s.revision });
  // se corner keeps 2:1
  let hsi = await handles(t, objSel('Im1')); let se = hsi.find(h => h.name === 'se');
  await t.drag(se.cx, se.cy, 20 * S, 20 * S);
  ops = await t.ops(); let b = await objBox(t, 'Im1');
  check('a4 Im1 se keeps 2:1', ops.length === 1 && ops[0] === 'edit-object:resize' && near(b, [72, 580, 212, 650]), { ops, box: b });
  await t.shot('review-u3-a4-corner-ratio.png');
  await undoClick(t); await t.ops();
  hsi = await handles(t, objSel('Im1')); se = hsi.find(h => h.name === 'se');
  await t.drag(se.cx, se.cy, 20 * S, 0, { modifiers: 8 });
  ops = await t.ops(); b = await objBox(t, 'Im1');
  check('a5 Im1 Shift+se frees ratio (120x50)', ops.length === 1 && near(b, [72, 600, 192, 650]), { ops, box: b });
  await undoClick(t); await t.ops();
  // Esc mid-drag
  s = await t.state(); const revE = s.revision;
  const b3 = await t.rect(objSel('Im2'));
  await t.drag(b3.cx, b3.cy, 40 * S, 40 * S, { escape: true });
  s = await t.state(); ops = await t.ops(); const b3after = await t.rect(objSel('Im2'));
  check('a6 Esc mid-drag: no request, box snaps back', ops.length === 0 && s.revision === revE && Math.abs(b3after.x - b3.x) < 1 && Math.abs(b3after.y - b3.y) < 1 && s.direct.gesture === null,
    { ops, rev: s.revision, before: [b3.x, b3.y], after: [b3after.x, b3after.y], gesture: s.direct.gesture, error: s.error, mode: s.mode });
  // drag off the page → clamps (no refusal possible from the grip)
  const b4 = await t.rect(objSel('Im2'));
  await t.drag(b4.cx, b4.cy, 400 * S, -200 * S);
  ops = await t.ops(); b = await objBox(t, 'Im2'); s = await t.state();
  check('a7 drag off page clamps to the page edge (one op)', ops.length === 1 && near(b, [552, 752, 612, 792]), { ops, box: b, error: s.error });
  await undoClick(t); await t.ops();
  // forced refusal: mangle generation/revision on edit-object → stale-target
  await t.eval(`window.__mangle = 'edit-object'`);
  s = await t.state(); const revR = s.revision, refusedN = s.content.refused.length;
  const b5 = await t.eval(`(() => { const e = ${objSel('Im2')}; return { x: e.style.left, y: e.style.top }; })()`);
  { const r5 = await t.rect(objSel('Im2')); await t.drag(r5.cx, r5.cy, 30 * S, 30 * S, { wait: 2000 }); }
  await t.eval(`window.__mangle = null`);
  s = await t.state(); ops = await t.ops(); ui = await t.ui(); const b5after = await t.eval(`(() => { const e = ${objSel('Im2')}; return e ? { x: e.style.left, y: e.style.top } : null; })()`);
  check('a8 refused gesture snaps back + #error + content.refused', ops.length === 1 && s.revision === revR && ui.error !== '' && s.content.refused.length === refusedN + 1 && b5after?.x === b5.x && b5after?.y === b5.y,
    { ops, rev: s.revision, error: ui.error, refused: s.content.refused.slice(-1), before: [b5.x, b5.y], after: [b5after.x, b5after.y] });
  await t.shot('review-u3-a8-refusal.png');
  // (b) click → focus stays; Inspector row selected without focus
  const b6 = await t.rect(objSel('Im2'));
  await t.click(b6.cx, b6.cy);
  s = await t.state(); ui = await t.ui();
  const row = await t.eval(`(() => { const r = document.querySelector('#content-targets li[aria-selected="true"]'); return r ? { text: r.textContent, focused: r === document.activeElement || r.contains(document.activeElement) } : null; })()`);
  check('b1 click Im2: focus on the object button, Inspector row selected without focus', /object-target/.test(ui.active) && row && !row.focused && s.direct.active?.kind === 'object', { errorStill: ui.error, stateError: s.error, active: ui.active, row, directActive: s.direct.active, ops: await t.ops() });
  await t.shot('review-u3-b1-click-select.png');
  await t.key('Delete', 'Delete', 46);
  await sleep(1500);
  s = await t.state(); ops = await t.ops();
  check('b2 real Delete deletes Im2 (one op)', ops.length === 1 && ops[0] === 'edit-object:delete' && s.direct.objects === 2, { ops, objects: s.direct.objects, edited: s.content.edited, ann: s.announcement });
  await undoClick(t); s = await t.state(); await t.ops();
  check('b3 undo restores Im2', s.direct.objects === 3, { objects: s.direct.objects });
  let b7 = await t.rect(objSel('Im2')); await t.click(b7.cx, b7.cy); await t.ops();
  await t.key('ArrowRight', 'ArrowRight', 39, 1); await sleep(1500);
  ui = await t.ui();
  await t.key('ArrowRight', 'ArrowRight', 39, 1); await sleep(1500);
  ops = await t.ops(); b = await objBox(t, 'Im2'); const ui2 = await t.ui();
  check('b4 ⌥→ twice moves Im2 2 pt (two ops)', ops.length === 2 && near(b, [402, 700, 462, 740], 0.05), { ops, box: b, focusAfter1: ui.active, focusAfter2: ui2.active });
  // ⌥⌘→ width
  await t.key('ArrowRight', 'ArrowRight', 39, 5); await sleep(1500);
  ops = await t.ops(); b = await objBox(t, 'Im2');
  check('b5 ⌥⌘→ widens 1 pt', ops.length === 1 && near(b, [402, 700, 463, 740], 0.05), { ops, box: b });
  // Enter → replace (images arg answers pick-images)
  await t.key('Enter', 'Enter', 13, 0, '\r'); await sleep(2500);
  s = await t.state(); ops = await t.ops();
  check('b6 Enter on focused image → replace', ops.length === 1 && ops[0] === 'edit-object:replace', { ops, edited: s.content.edited, focus: (await t.ui()).active });
  const afterRep = (await t.state()); log('objects after replace', q(afterRep.content.objects.map(o => [o.name, o.box])), 'active', q(afterRep.direct.active));
  const repName = afterRep.content.objects.filter(o => o.kind === 'image').sort((a, c) => Math.hypot(a.box[0] - 402, a.box[1] - 700) - Math.hypot(c.box[0] - 402, c.box[1] - 700))[0].name;
  check('b6b after Enter replace the replaced image keeps on-page focus / direct.active', afterRep.direct.active?.kind === 'object', { repName, active: afterRep.direct.active });
  // double-click → replace
  b7 = await t.rect(objSel(repName)); await t.dblclick(b7.cx, b7.cy);
  s = await t.state(); ops = await t.ops();
  check('b7 double-click image → replace', ops.filter(o => o === 'edit-object:replace').length === 1 && ops.length === 1, { ops, edited: s.content.edited });
  // Undo back to baseline for the objects (4 ops)
  for (let i = 0; i < 5; i++) await undoClick(t);
  await t.ops(); b = await objBox(t, 'Im2');
  check('b8 five undos restore Im2', near(b, [400, 700, 460, 740], 0.01), { box: b });
  // (c) add text
  s = await t.state(); const revC = s.revision;
  let p = await at(t, 300, 500);
  await t.click(p.x, p.y);
  s = await t.state(); ui = await t.ui();
  check('c1 click empty → add editor at the click, focused', s.direct.active?.kind === 'add' && near(s.direct.active.point, [300, 500], 1) && /TEXTAREA/.test(ui.active), { active: s.direct.active, focus: ui.active });
  await t.type('Line one'); await sleep(150);
  await t.key('Enter', 'Enter', 13, 0, '\r'); await sleep(150);
  await t.type('Line two'); await sleep(300);
  s = await t.state();
  check('c2 Enter inserts a newline', s.direct.active?.draft === 'Line one\nLine two', { draft: s.direct.active?.draft });
  await t.shot('review-u3-c2-add-editor.png');
  log('error before commit', q((await t.ui()).error));
  await t.key('Enter', 'Enter', 13, 4); await sleep(2500);
  s = await t.state(); ops = await t.ops(); ui = await t.ui();
  const lastId = s.content.lastAdd?.targetId;
  check('c3 ⌘Enter commits ONE text-add; new run focused', ops.length === 1 && ops[0] === 'text-add' && s.direct.active === null && s.revision === revC + 1 && !!lastId && ui.active?.includes('#' + lastId), { ops, rev: s.revision, lastAdd: s.content.lastAdd, focus: ui.active, error: s.error });
  await t.shot('review-u3-c3-text-added.png');
  await undoClick(t); s = await t.state(); await t.ops();
  check('c4 undo removes the added run', !s.content.bounds.some(i => i.targetId === lastId) && s.revision === revC + 2, { rev: s.revision, lastAdd: s.content.lastAdd });
  // click away from an EMPTY add editor: closes, no new editor
  p = await at(t, 300, 560); await t.click(p.x, p.y);
  s = await t.state();
  const opened = s.direct.active?.kind === 'add';
  p = await at(t, 300, 520); await t.click(p.x, p.y, 1200);
  s = await t.state(); ops = await t.ops();
  check('c5 click away from an empty add editor closes it and opens none', opened && s.direct.active === null && ops.length === 0, { opened, active: s.direct.active, ops });
  // click away from a non-empty one commits (and opens none)
  p = await at(t, 300, 560); await t.click(p.x, p.y);
  await t.type('Commit by click'); await sleep(200);
  p = await at(t, 300, 470); await t.click(p.x, p.y, 2500);
  s = await t.state(); ops = await t.ops();
  check('c6 click away from a non-empty add editor commits once and opens none', ops.length === 1 && ops[0] === 'text-add' && s.direct.active === null, { ops, active: s.direct.active, error: s.error });
  await undoClick(t); await t.ops();
  // click on an object while an add editor with a draft is open
  p = await at(t, 300, 560); await t.click(p.x, p.y); await t.type('Then object'); await sleep(200);
  const b8 = await t.rect(objSel('Im1')); await t.click(b8.cx, b8.cy, 2500);
  s = await t.state(); ops = await t.ops(); ui = await t.ui();
  check('c7 add draft + click on an object: commits once, object focused', ops.length === 1 && ops[0] === 'text-add', { ops, active: s.direct.active, focus: ui.active });
  await undoClick(t); await t.ops();
  // page 1 refusal
  const nx = await t.rect(`document.getElementById('next')`); await t.click(nx.cx, nx.cy, 2000);
  s = await t.state();
  p = await at(t, 300, 300, 792); await t.click(p.x, p.y, 1000);
  s = await t.state(); ui = await t.ui();
  check('c8 page 1 click → exact #error + content.refused, no editor', s.page === 1 && ui.error === 'Text cannot be added on a rotated page (unsupported-transform)' && s.direct.active === null && q(s.content.refused.at(-1)) === q({ text: '', detail: 'unsupported-transform' }), { page: s.page, error: ui.error, refused: s.content.refused.at(-1), active: s.direct.active });
  await t.shot('review-u3-c8-rotated-refusal.png');
  await t.click(nx.cx, nx.cy, 2000);
  s = await t.state();
  check('c9 page 2: objects 0 (Bg), targets 1', s.page === 2 && s.direct.objects === 0 && s.direct.targets === 1, { page: s.page, objects: s.direct.objects, targets: s.direct.targets, error: s.error });
  p = await at(t, 90, 87.3); await t.click(p.x, p.y);
  s = await t.state();
  check('c10 "On top" clickable on page 2', s.direct.active?.kind === 'text' && s.direct.active?.value === 'On top', s.direct.active);
  await t.key('Escape', 'Escape', 27); await sleep(300);
  p = await at(t, 300, 500); await t.click(p.x, p.y);
  s = await t.state();
  check('c11 page 2 empty-space click (Bg under it) → add editor', s.direct.active?.kind === 'add', { active: s.direct.active });
  await t.key('Escape', 'Escape', 27); await sleep(300);
  const pv = await t.rect(`document.getElementById('prev')`); await t.click(pv.cx, pv.cy, 1500); await t.click(pv.cx, pv.cy, 2000);
  await t.ops();
  // (d) Redact mode
  await t.eval(`document.getElementById('mode-redact').click()`);
  await wait(async () => (await t.state()).direct.marks === 2, 20_000);
  s = await t.state();
  check('d0 redact mode: targets 0, objects 0, marks 2, rects 2', s.direct.targets === 0 && s.direct.objects === 0 && s.direct.marks === 2 && s.redact.rects.length === 2, { direct: { t: s.direct.targets, o: s.direct.objects, m: s.direct.marks }, rects: s.redact.rects });
  const area = s.redact.rects.find(r => !r.search), srch = s.redact.rects.find(r => r.search);
  const markSel = id => `document.querySelector('#hit-layer .redact-target[data-annot-id="${id}"]')`;
  const mh = await handles(t, markSel(area.id));
  check('d1 mark hover: grip + 8 handles visible and hit-testable', mh.length === 9 && mh.every(h => h.visible && h.hit), mh.map(h => `${h.name}:${h.visible ? 'v' : '-'}${h.hit ? 'h' : '-'}`));
  const countText = () => t.eval(`document.getElementById('redact-count')?.textContent`);
  const count0 = await countText();
  let m = await t.rect(markSel(area.id));
  await t.drag(m.cx, m.cy, 40 * S, 0, { wait: 2000 });
  s = await t.state(); ops = await t.ops();
  const areaAfter = s.redact.rects.find(r => r.id === area.id);
  check('d2 real body drag moves the area mark (one redact-set)', ops.length === 1 && ops[0] === 'redact-set' && near(areaAfter?.rect, [340, 600, 440, 650]), { ops, rect: areaAfter, count0, count1: await countText(), active: s.direct.active });
  check('d2b direct.active redact shape', s.direct.active?.kind === 'redact' && Array.isArray(s.direct.active?.rect) && typeof s.direct.active?.box?.left === 'number', s.direct.active);
  await t.shot('review-u3-d2-mark-moved.png');
  m = await t.rect(markSel(srch.id));
  await t.drag(m.cx, m.cy, 0, 100 * S, { wait: 2000 });
  s = await t.state(); ops = await t.ops();
  check('d3 search mark moved → search:false', ops.length === 1 && s.redact.rects.every(r => r.search === false), { ops, rects: s.redact.rects });
  // Esc mid-drag on a mark
  m = await t.rect(markSel(area.id)); const revM = s.revision;
  await t.drag(m.cx, m.cy, 30 * S, 30 * S, { escape: true });
  s = await t.state(); ops = await t.ops(); const m2 = await t.rect(markSel(area.id));
  check('d4 Esc mid-drag on a mark: no request, snaps back', ops.length === 0 && s.revision === revM && m2 && Math.abs(m2.x - m.x) < 1, { ops, rev: s.revision, before: [m.x, m.y], after: m2 && [m2.x, m2.y] });
  // click → focus stays on mark, aria-current row
  m = await t.rect(markSel(area.id)); await t.click(m.cx, m.cy);
  ui = await t.ui(); s = await t.state();
  const cur = await t.eval(`[...document.querySelectorAll('#notes li[aria-current="true"], [aria-current="true"]')].map(e => e.textContent.slice(0, 60))`);
  check('d5 click mark: focus on mark, Annotations row aria-current', /redact-target/.test(ui.active) && cur.length >= 1 && s.direct.active?.kind === 'redact', { focus: ui.active, current: cur, active: s.direct.active });
  await t.key('Delete', 'Delete', 46); await sleep(2000);
  s = await t.state(); ops = await t.ops();
  check('d6 real Delete removes the mark (one op)', ops.length === 1 && s.annots.Redact === 1 && s.direct.marks === 1, { ops, redact: s.annots.Redact, marks: s.direct.marks, ann: s.announcement });
  await undoClick(t); s = await t.state(); await t.ops();
  check('d7 undo restores the mark', s.annots.Redact === 2 && s.direct.marks === 2, { redact: s.annots.Redact, marks: s.direct.marks, preview: s.redact.marks, count: await countText() });
  // corner resize on mark (free)
  const mh2 = await handles(t, markSel(area.id)); const mse = mh2.find(h => h.name === 'se');
  await t.drag(mse.cx, mse.cy, 20 * S, 10 * S, { wait: 2000 });
  s = await t.state(); ops = await t.ops();
  check('d8 mark se resize (free)', ops.length === 1 && near(s.redact.rects.find(r => r.id === area.id)?.rect, [340, 600, 460, 660]), { ops, rect: s.redact.rects.find(r => r.id === area.id) });
  await t.shot('review-u3-d8-mark-resized.png');
  // (e) help
  const help = await t.eval(`document.getElementById('help-dialog')?.textContent ?? document.body.textContent`);
  check('e1 Help mentions images drag/resize/Delete/Enter, add text, marks, 90 %, search mark, ⌥⌘↑/↓', ['Shift frees', 'click empty space', 'redaction marks can be dragged', '90 %', 'a moved search mark becomes an area mark', 'Apply refuses until it is marked again', '⌥⌘↑'].every(k => help.includes(k)), null);
  s = await t.state();
  check('e2 edit mode object key off in redact: objects 0', s.direct.objects === 0, { objects: s.direct.objects });
});
// (f) field grip on form2.pdf
await session('form', [{ cmd: 'open', path: join(root, 'fixtures/form2.pdf') }], async t => {
  await wait(() => t.eval(`document.title.includes('form2')`), 60_000);
  await sleep(1000); await t.eval(HOOK);
  await t.eval(`document.getElementById('mode-forms')?.click()`); await sleep(800);
  const sel = `document.querySelector('#hit-layer .hit-target[data-field-id]')`;
  await wait(() => t.rect(sel), 20_000);
  await t.eval(`${sel}.scrollIntoView({ block: 'center' })`); await sleep(300);
  const hs = await handles(t, sel);
  const s0 = await t.state(); const g = hs.find(h => h.name === 'grip'); const S = (await at(t, 0, 0)).s;
  await t.drag(g.cx, g.cy, 20 * S, 20 * S, { wait: 2000 });
  const s1 = await t.state(); const ops = await t.ops();
  check('f1 field grip drag → one field-props', hs.filter(h => h.visible).length === 9 && ops.length === 1 && ops[0] === 'field-props' && s1.revision === s0.revision + 1, { handles: hs.map(h => `${h.name}:${h.visible ? 'v' : '-'}${h.hit ? 'h' : '-'}`), ops, rev: [s0.revision, s1.revision], mode: s1.mode });
  await t.shot('review-u3-f1-field-drag.png');
});
await writeFile(join(out, 'review-u3-ui.json'), JSON.stringify(results, null, 1));
log('fixtures after\n' + hashes());
log('SUMMARY', results.filter(r => r.ok).length, '/', results.length);
