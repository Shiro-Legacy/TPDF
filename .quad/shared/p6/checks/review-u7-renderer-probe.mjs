#!/usr/bin/env node
// U7 review, renderer lens: real CDP mouse/keys probe (copied helpers from shot-direct8.mjs). Touches the OS clipboard: saved + restored.
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
const hashes = () => execFileSync('shasum', ['-a', '256', '-c', join(checksDir, 'fixtures.sha256')], { cwd: root, encoding: 'utf8' });
const KEYS = { Enter: 13, Tab: 9, Escape: 27, ArrowRight: 39, ArrowLeft: 37, ArrowDown: 40, ArrowUp: 38, Delete: 46, PageDown: 34, PageUp: 33, F10: 121, c: 67, v: 86, x: 88, a: 65 };
const SHIFT = 8, CTRL = 2, META = 4;
const results = [];
const chk = (label, ok, detail) => { results.push({ label, ok: !!ok, detail }); log(ok ? 'ok  ' : 'FAIL', label, q(detail)); };
const names = sel => (sel ?? []).map(m => m.name).join(',');
const views = (s, name) => s.content.objects.filter(o => o.name === name).map(o => o.view);
const OSC = () => execFileSync('pbpaste', { encoding: 'utf8' });
const setOSC = t => execFileSync('pbcopy', { input: t });

async function session(tag, fixture, body) {
  const udata = join(out, `u7-review-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `review-u7-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(120, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort(), iport = await freePort();
  const child = spawn(electron, [`--inspect=${iport}`, `--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `review-u7-out-${tag}`)}`, `--report=${join(out, `review-u7-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  const closed = new Promise(done => child.on('close', done));
  let cdp, mcdp;
  try {
    cdp = await attach(port);
    const mt = await wait(async () => (await (await fetch(`http://127.0.0.1:${iport}/json/list`)).json())[0]);
    mcdp = new Cdp(mt.webSocketDebuggerUrl); await mcdp.connect(); await mcdp.send('Runtime.enable');
    const main = async expr => { const r = await mcdp.send('Runtime.evaluate', { includeCommandLineAPI: true, awaitPromise: true, returnByValue: true, expression: `(async () => { const E = (typeof process.getBuiltinModule === 'function' && process.getBuiltinModule('electron')) || require('electron'); const wc = E.BrowserWindow.getAllWindows()[0].webContents; ${expr} })()` });
      if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text); return r.result?.value; };
    try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch { }
    const state = () => cdp.eval('window.pdf4allState()');
    const shot = async name => { const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }); await writeFile(join(out, name), Buffer.from(data, 'base64')); log('shot', name); };
    const mouse = (type, x, y, extra = {}) => cdp.send('Input.dispatchMouseEvent', { type, x, y, ...extra });
    const move = (x, y, modifiers = 0) => mouse('mouseMoved', x, y, { modifiers });
    const click = async (x, y, modifiers = 0) => {
      await move(x, y, modifiers); await sleep(100);
      await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1, modifiers });
      await mouse('mouseReleased', x, y, { button: 'left', buttons: 0, clickCount: 1, modifiers });
      await sleep(700);
    };
    const rightClick = async (x, y, modifiers = 0) => {
      await move(x, y); await sleep(100);
      await mouse('mousePressed', x, y, { button: 'right', buttons: 2, clickCount: 1, modifiers });
      await mouse('mouseReleased', x, y, { button: 'right', buttons: 0, clickCount: 1, modifiers });
      await sleep(700);
    };
    const drag = async (x, y, dx, dy, modifiers = 0) => {
      await move(x, y, modifiers); await sleep(150);
      await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1, modifiers });
      for (let i = 1; i <= 8; i++) await mouse('mouseMoved', x + dx * i / 8, y + dy * i / 8, { button: 'left', buttons: 1, modifiers });
      await mouse('mouseReleased', x + dx, y + dy, { button: 'left', buttons: 0, clickCount: 1, modifiers });
      await sleep(1500);
    };
    const key = async (name, modifiers = 0) => {
      const code = KEYS[name];
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await sleep(600);
    };
    const chord = async (letter, command) => {
      const code = KEYS[letter], k = { key: letter, code: `Key${letter.toUpperCase()}`, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers: META };
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', ...k, commands: [command] });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', ...k });
      await sleep(1000);
    };
    const pagePoint = async (x, y, scroll = true, widthPt = 612) => {
      const p = await cdp.eval(`(() => {
        const v = document.getElementById('viewport'), c = document.getElementById('canvas');
        let r = c.getBoundingClientRect(); const s = r.width / ${widthPt};
        if (${scroll}) v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
        r = c.getBoundingClientRect();
        return { x: r.left + ${x} * s, y: r.top + ${y} * s, s };
      })()`);
      if (scroll) await sleep(300);
      return p;
    };
    const mode = async name => { await cdp.eval(`document.getElementById('mode-${name}').click()`); await sleep(1000); };
    const undo = async rev => { await cdp.eval(`document.getElementById('undo').click()`); await wait(async () => (await state()).revision === rev, 10_000).catch(e => chk(`undo → rev ${rev}`, false, e.message)); await sleep(500); };
    const active = () => cdp.eval(`(() => { const a = document.activeElement; return a ? (a.id || a.className || a.tagName) : null; })()`);
    await wait(() => cdp.eval(`document.title.includes(${q(fixture.split('/').pop().replace('.pdf', ''))})`), 60_000);
    await sleep(800);
    await body({ state, shot, move, click, rightClick, drag, key, chord, pagePoint, mode, cdp, main, undo, active });
  } catch (err) { chk(`${tag} aborted`, false, String(err?.stack ?? err).slice(0, 600)); }
  finally {
    cdp?.close(); mcdp?.close(); child.kill('SIGTERM');
    log('electron closed', tag, await closed);
  }
}

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
const savedOS = OSC();
log('OS clipboard saved (restored in finally)');
const only = process.argv[2];
try {
  if (!only || only === 'objects6') await session('objects6', 'fixtures/p6/objects6.pdf', async ({ state, shot, click, rightClick, key, chord, pagePoint, mode, cdp, undo, active }) => {
    await mode('edit');
    await wait(async () => (await state()).direct.objects === 6, 20_000);
    const im1 = await pagePoint(122, 167);
    await click(im1.x, im1.y);
    let s = await state(); const r0 = s.revision;
    chk('a click Im1 selects', names(s.direct.selection) === 'Im1', s.direct.selection);
    await chord('c', 'copy'); s = await state();
    chk('a ⌘C clipboard count 1 image 1, rev same, not dirty', s.direct.clipboard?.count === 1 && s.direct.clipboard.kinds.image === 1 && s.revision === r0 && s.dirty === false, { clip: s.direct.clipboard, rev: s.revision, dirty: s.dirty, ann: s.announcement });
    await chord('v', 'paste'); s = await state();
    chk('a ⌘V objects 7, PA1 selected, one revision, box +10', s.direct.objects === 7 && names(s.direct.selection) === 'PA1' && s.revision === r0 + 1, { objects: s.direct.objects, sel: names(s.direct.selection), rev: s.revision, pa1: views(s, 'PA1'), ann: s.announcement, edited: s.content.edited, focus: await active() });
    await shot('review-u7-a1-paste1.png');
    await chord('v', 'paste'); s = await state();
    chk('a ⌘V again → PA2 cascaded', s.direct.objects === 8 && names(s.direct.selection) === 'PA2' && s.revision === r0 + 2, { objects: s.direct.objects, sel: names(s.direct.selection), rev: s.revision, pa2: views(s, 'PA2'), ann: s.announcement });
    await shot('review-u7-a2-paste2.png');
    await undo(r0 + 3); await undo(r0 + 4); s = await state();
    chk('a undo ×2 → objects 6', s.direct.objects === 6, { objects: s.direct.objects, sel: s.direct.selection });
    await key('Escape');
    const im2 = await pagePoint(425, 92);
    await click(im2.x, im2.y);
    await chord('x', 'cut'); s = await state();
    chk('a ⌘X Im2 → objects 5, clip 1, "Cut image Im2", edited delete', s.direct.objects === 5 && s.direct.clipboard?.count === 1 && s.announcement === 'Cut image Im2' && s.content.edited?.action === 'delete' && s.direct.selection.length === 0,
      { objects: s.direct.objects, clip: s.direct.clipboard, ann: s.announcement, edited: s.content.edited, sel: s.direct.selection, rev: s.revision });
    const rCut = s.revision;
    // page menu by real right-click on empty space
    const e1 = await pagePoint(500, 450);
    await rightClick(e1.x, e1.y); s = await state();
    chk('a right-click empty → page menu, only paste', s.direct.menu?.kind === 'page' && Object.entries(s.direct.menu.items).every(([k, v]) => v === (k === 'paste')) && s.direct.active?.kind !== 'add', { menu: s.direct.menu, active: s.direct.active, focus: await active() });
    await shot('review-u7-a3-page-menu.png');
    await key('ArrowDown'); const f1 = await active(); await key('ArrowUp'); const f2 = await active();
    chk('a page menu ↑/↓ keep focus on the enabled Paste', /paste/.test(String(f1)) && /paste/.test(String(f2)), { f1, f2 });
    await key('Escape'); s = await state();
    chk('a Escape closes the page menu, focus #page', s.direct.menu === null && (await active()) === 'page', { menu: s.direct.menu, focus: await active() });
    const e2 = await pagePoint(300, 450);
    await rightClick(e2.x, e2.y);
    await key('Enter'); await sleep(800); s = await state();
    const pasted = views(s, 'PA1')[0];
    chk('a page-menu Paste (Enter) → PA1 top-left at click point (300,450)', !!pasted && Math.abs(pasted[0] - 300) <= 1 && Math.abs(pasted[1] - 450) <= 1 && s.revision === rCut + 1, { pasted, rev: s.revision, ann: s.announcement, edited: s.content.edited, sel: names(s.direct.selection) });
    await shot('review-u7-a4-page-paste.png');
    await undo(rCut + 2);
    await key('Escape');
    // Control-click on empty space (macOS right-click)
    const e3 = await pagePoint(300, 520);
    await click(e3.x, e3.y, CTRL); s = await state();
    chk('a Control-click empty → page menu, no add editor', s.direct.menu?.kind === 'page' && s.direct.active?.kind !== 'add', { menu: s.direct.menu?.kind, active: s.direct.active });
    await key('Escape'); await sleep(300); s = await state();
    chk('a after Control-click + Escape no add editor', s.direct.active?.kind !== 'add' && s.direct.menu === null, { active: s.direct.active });
    // Control-click on an object → object menu, no drag, no add
    const im1c = await pagePoint(122, 167); await click(im1c.x, im1c.y, CTRL); s = await state();
    chk('a Control-click Im1 → object menu for Im1', s.direct.menu?.name === 'Im1' && s.revision === rCut + 2, { menu: s.direct.menu, rev: s.revision });
    await key('Escape');
    // Shift+F10 on #page
    await cdp.eval(`document.querySelectorAll('#hit-layer .selected').length`);
    await key('Escape');
    await cdp.eval(`document.getElementById('page').focus()`);
    await key('F10', SHIFT); s = await state();
    const pos = await cdp.eval(`(() => { const m = document.getElementById('object-menu').getBoundingClientRect(), v = document.getElementById('viewport').getBoundingClientRect(); return { m: [m.left, m.top], v: [v.left, v.top] }; })()`);
    chk('a Shift+F10 on #page → page menu at viewport top-left', s.direct.menu?.kind === 'page' && Math.abs(pos.m[0] - pos.v[0]) <= 1 && Math.abs(pos.m[1] - pos.v[1]) <= 1, { menu: s.direct.menu?.kind, pos });
    await key('Escape');
    // (b) chord guard: Inspector W field
    const im1w = await pagePoint(122, 167); await click(im1w.x, im1w.y);
    const before = (await state()).direct.clipboard;
    setOSC('sentinel-u7');
    const w = await cdp.eval(`(() => { const e = document.getElementById('content-box-w'); e.scrollIntoView({ block: 'nearest' }); e.focus(); e.select(); return { value: e.value, active: document.activeElement.id }; })()`);
    await chord('c', 'copy'); s = await state();
    chk('b ⌘C in W → OS clipboard = W text, direct.clipboard unchanged', OSC() === w.value && JSON.stringify(s.direct.clipboard) === JSON.stringify(before), { os: OSC(), w, clip: s.direct.clipboard });
    // Help dialog open
    await cdp.eval(`document.getElementById('page').focus()`); const im1h = await pagePoint(122, 167); await click(im1h.x, im1h.y);
    setOSC('sentinel-help');
    await cdp.eval(`document.getElementById('help-dialog').showModal()`); await sleep(400);
    const rH = (await state()).revision;
    await chord('c', 'copy'); await chord('v', 'paste'); s = await state();
    chk('b Help open: ⌘C/⌘V do nothing', OSC() === 'sentinel-help' && s.revision === rH && s.direct.objects === 5, { os: OSC(), rev: s.revision, objects: s.direct.objects });
    const help = await cdp.eval(`document.getElementById('help-dialog').innerText`);
    await writeFile(join(out, 'review-u7-help.txt'), help);
    await key('Escape');
  });

  if (!only || only === 'direct3') await session('direct3', 'fixtures/p6/direct3.pdf', async ({ state, shot, click, rightClick, drag, key, chord, pagePoint, mode, cdp, undo, active }) => {
    await mode('edit');
    await wait(async () => (await state()).direct.objects === 3, 20_000);
    const tap = async (x, y, mods = 0) => { const p = await pagePoint(x, y); await click(p.x, p.y, mods); return state(); };
    const snap = async label => { const t = await state(); log('STEP', label, q({ sel: t.direct.selection.map(m => m.kind + ':' + m.name), active: t.direct.active?.kind ?? null, rev: t.revision, ann: t.announcement, err: t.error, clip: t.direct.clipboard?.kinds, mode: t.mode })); return t; };
    let s = await state(); const r0 = s.revision;
    // Shift-click on empty space with a selection
    s = await tap(122, 167); s = await tap(300, 520, SHIFT);
    chk('x Shift-click on empty space keeps the selection, opens no add editor', s.direct.selection.length === 1 && s.direct.active?.kind !== 'add', { sel: s.direct.selection, active: s.direct.active?.kind });
    await key('Escape'); await key('Escape'); await cdp.eval(`document.getElementById('page').focus()`); await key('Escape');
    s = await tap(100.78, 386.57, SHIFT);
    chk('c Shift-click run → [text], no editor', names(s.direct.selection) === 'Move me' && s.direct.active === null, { sel: s.direct.selection, active: s.direct.active });
    s = await tap(122, 167, SHIFT);
    const g = await cdp.eval(`(() => { const b = [...document.querySelectorAll('#hit-layer .group-box')]; return { boxes: b.length, handles: b.flatMap(x => [...x.querySelectorAll('.hit-handle')]).length, collapsed: getSelection().isCollapsed }; })()`);
    chk('c Shift-click Im1 → [text,image], one group box, no handles', s.direct.selection.map(m => m.kind).join() === 'text,image' && g.boxes === 1 && g.handles === 0, { sel: s.direct.selection, g, focus: await active() });
    await shot('review-u7-c1-mixed.png');
    await key('ArrowRight'); s = await snap('arrow');
    chk('c real ArrowRight → selection move count 2, one rev', s.content.edited?.kind === 'selection' && s.content.edited.count === 2 && s.revision === r0 + 1 && s.direct.selection.length === 2, { edited: s.content.edited, rev: s.revision, im1: views(s, 'Im1') });
    const run2 = await pagePoint(101.78, 386.57);
    await drag(run2.x, run2.y, 30 * run2.s, 0);
    s = await snap('drag+0');
    for (let i = 1; i < 4; i++) { await sleep(700); s = await snap('drag+' + i); }
    chk('c real drag on run moves both, selection kept, no editor', s.revision === r0 + 2 && s.direct.active?.kind !== 'text' && s.direct.selection.length === 2 && Math.abs((views(s, 'Im1')[0] ?? [])[0] - 103) <= 1, { rev: s.revision, active: s.direct.active?.kind, im1: views(s, 'Im1'), sel: s.direct.selection, bounds: s.content.bounds.at(-1) });
    await shot('review-u7-c2-after-drag.png');
    const rD = s.revision;
    await key('Delete'); s = await snap('delete');
    chk('c Delete → #error text, rev unchanged', s.error === 'Text runs cannot be deleted; deselect them first or use Cut' && s.revision === rD && s.direct.objects === 3, { err: s.error, rev: s.revision, objects: s.direct.objects });
    if (s.revision !== rD) await undo(s.revision + 1);
    s = await state();
    const sel0 = s.direct.selection.length;
    s = await tap(145.98, 87.73, SHIFT);
    chk('c Shift-click paragraph → announcement, no membership', s.announcement === 'Paragraphs cannot join a selection' && s.direct.selection.length === sel0 && s.direct.active === null, { ann: s.announcement, sel: s.direct.selection, active: s.direct.active?.kind });
    await key('Escape'); s = await state();
    chk('c Escape clears', s.direct.selection.length === 0, { sel: s.direct.selection, focus: await active() });
    // drag started on Im1 instead
    s = await tap(131.57 - 20, 386.57, SHIFT); s = await snap('reselect run');
    const im = views(s, 'Im1')[0]; s = await tap((im[0] + im[2]) / 2, (im[1] + im[3]) / 2, SHIFT); s = await snap('reselect im1');
    const rI = s.revision; const pI = await pagePoint((im[0] + im[2]) / 2, (im[1] + im[3]) / 2);
    await drag(pI.x, pI.y, 0, 20 * pI.s); await sleep(1500); s = await snap('drag on Im1');
    chk('c real drag on Im1 member moves both, selection kept', s.revision === rI + 1 && s.direct.selection.length === 2, { rev: s.revision, sel: s.direct.selection, edited: s.content.edited });
    await key('Escape');
    // (d) copy the run, paste on page 2 with the real chord
    const runNow = s.content.bounds.find(b => !b.paragraphId && b.fontSize === 14)?.bounds ?? [72, 376.95, 129.57, 396.19];
    s = await tap((runNow[0] + runNow[2]) / 2, (runNow[1] + runNow[3]) / 2, SHIFT);
    const rC = s.revision;
    await chord('c', 'copy'); s = await snap('copy run');
    chk('d ⌘C run → kinds.text 1, rev same', s.direct.clipboard?.kinds?.text === 1 && s.direct.clipboard.count === 1 && s.revision === rC, { clip: s.direct.clipboard, rev: s.revision });
    // mixed copy
    const imNow = views(s, 'Im1')[0]; s = await tap((imNow[0] + imNow[2]) / 2, (imNow[1] + imNow[3]) / 2, SHIFT);
    await chord('c', 'copy'); s = await snap('copy mixed');
    chk('d mixed ⌘C → text 1 image 1', s.direct.clipboard?.kinds?.text === 1 && s.direct.clipboard.kinds.image === 1, { clip: s.direct.clipboard, sel: s.direct.selection });
    // cut the mixed selection
    await chord('x', 'cut'); s = await snap('cut mixed');
    chk('d ⌘X mixed → run stays, Im1 removed, announcement', s.direct.objects === 2 && s.revision === rC + 1 && /Cut 1 item; 1 text run stays \(it cannot be deleted\)$/.test(s.announcement) && s.direct.selection.length === 0, { objects: s.direct.objects, ann: s.announcement, rev: s.revision, edited: s.content.edited });
    await undo(rC + 2);
    await mode('forms');
    const e = await pagePoint(300, 560);
    await rightClick(e.x, e.y);
    await key('Enter'); await sleep(1200); s = await snap('forms paste');
    chk('d mixed paste in Fill forms → "Pasted 2 items; 1 image shown in Edit text; 1 text run shown in Edit text"', /Pasted 2 items/.test(s.announcement) && /image/.test(s.announcement) && /text run/.test(s.announcement), { ann: s.announcement, edited: s.content.edited, rev: s.revision });
    const rP = s.revision;
    await undo(rP + 1);
    await mode('edit'); s = await state();
    chk('d undo of mixed paste restores objects 3', s.direct.objects === 3, { objects: s.direct.objects });
    await cdp.eval(`document.getElementById('next').click()`); await sleep(800);
    await cdp.eval(`document.getElementById('next').click()`); await sleep(1500);
    s = await state(); const nb = s.content.bounds.length; const rV = s.revision;
    await chord('v', 'paste'); await sleep(800); s = await snap('paste p2');
    chk('d ⌘V mixed on page 2 → run + image pasted, both selected', s.page === 2 && s.content.bounds.length === nb + 1 && s.revision === rV + 1 && s.direct.selection.length === 2, { sel: s.direct.selection, edited: s.content.edited, ann: s.announcement });
    await shot('review-u7-d1-p2-paste.png');
  });

  if (!only || only === 'fields') await session('fields', 'fixtures/p3/field-defs.pdf', async ({ state, shot, click, key, chord, pagePoint, mode, cdp, undo, active }) => {
    await mode('forms');
    await wait(async () => (await state()).fields?.count === 5, 20_000);
    let s = await state(); const r0 = s.revision;
    log('fields', q(s.fields));
    const cb = Object.entries(s.fields.defs).find(([, d]) => d.kind === 'checkbox');
    if (cb) {
      const r = cb[1].rect, p = await pagePoint((r[0] + r[2]) / 2, (r[1] + r[3]) / 2);
      const v0 = JSON.stringify(s.fields.values ?? s.fields);
      await click(p.x, p.y, SHIFT); s = await state();
      chk('c Shift-click checkbox → not toggled, seeds selection', JSON.stringify(s.fields.values ?? s.fields) === v0 && s.revision === r0 && s.direct.selection[0]?.kind === 'field', { name: cb[0], sel: s.direct.selection, rev: s.revision, dirty: s.dirty });
      await key('Escape');
    } else chk('c checkbox present', false, Object.keys(s.fields.defs));
    const plain = await pagePoint(132, 259.5);
    // plain click opens the inline input: ⌘C/⌘V belong to it
    await click(plain.x, plain.y); s = await state();
    const inl = await cdp.eval(`(() => { const a = document.activeElement; if (a && a.tagName === 'INPUT') { a.value = 'abc'; a.select(); } return { tag: a?.tagName, cls: a?.className }; })()`);
    setOSC('sentinel-field');
    const clipF = JSON.stringify(s.direct.clipboard);
    await chord('c', 'copy'); const osF = OSC(); s = await state();
    chk('b field inline <input>: ⌘C copies its text, no engine copy', osF === 'abc' && JSON.stringify(s.direct.clipboard) === clipF, { inl, osF, clip: s.direct.clipboard });
    await key('Escape'); await sleep(600);
    await cdp.eval(`document.getElementById('page').focus()`); await key('Escape');
    s = await state(); const r1 = s.revision;
    await click(plain.x, plain.y, SHIFT);
    await chord('c', 'copy'); await chord('v', 'paste'); s = await state();
    chk('d Forms Shift-click + ⌘C/⌘V → plain copy at +10, selected', s.fields.count === 6 && !!s.fields.defs['plain copy'] && s.direct.selection.at(-1)?.name === 'plain copy' && s.revision === r1 + 1, { count: s.fields.count, pc: s.fields.defs['plain copy'], sel: s.direct.selection, ann: s.announcement, edited: s.content.edited, rev: s.revision });
    await shot('review-u7-e1-field-paste.png');
    await undo(r1 + 2);
  });

  if (!only || only === 'links') await session('links', 'fixtures/p6/links.pdf', async ({ state, shot, click, key, chord, pagePoint, mode, cdp, undo, active }) => {
    await mode('annotate');
    await wait(async () => (await state()).links?.length >= 1, 20_000);
    let s = await state(); const r0 = s.revision, n0 = s.links.length;
    const l = await pagePoint(136, 82);
    await click(l.x, l.y); s = await state();
    const focusL = await active();
    await chord('c', 'copy'); s = await state();
    chk('d plain click on a link then real ⌘C → link copied', s.direct.clipboard?.kinds?.link === 1, { active: s.direct.active?.kind, sel: s.direct.selection, focus: focusL, clip: s.direct.clipboard });
    await shot('review-u7-f1-link-click.png');
    await key('Escape'); await key('Escape');
    await click(l.x, l.y, SHIFT); s = await state();
    await chord('c', 'copy'); await chord('v', 'paste'); s = await state();
    chk('d Shift-click link + ⌘C/⌘V → links +1, selected', s.links.length === n0 + 1 && s.revision === r0 + 1 && s.direct.selection[0]?.kind === 'link', { links: s.links.length, rev: s.revision, sel: s.direct.selection, ann: s.announcement, clip: s.direct.clipboard, err: s.error });
    if (s.revision === r0 + 1) await undo(r0 + 2);
  });

  if (!only || only === 'hello') await session('hello', 'fixtures/hello.pdf', async ({ state, shot, click, drag, key, chord, pagePoint, mode, cdp, undo, active }) => {
    await mode('redact');
    const a = await pagePoint(300, 500);
    await drag(a.x, a.y, 100 * a.s, 30 * a.s);
    let s = await state(); const r0 = s.revision;
    chk('e redact drag → 1 mark', s.redact.marks === 1, { redact: s.redact.rects });
    const m = await pagePoint(350, 515);
    await click(m.x, m.y); s = await state();
    const focusM = await active();
    await chord('c', 'copy'); s = await state();
    chk('d plain click on a mark then real ⌘C → mark copied', s.direct.clipboard?.kinds?.mark === 1, { active: s.direct.active, sel: s.direct.selection, focus: focusM, clip: s.direct.clipboard });
    await shot('review-u7-g1-mark-click.png');
    await key('Escape'); await key('Escape');
    await click(m.x, m.y, SHIFT);
    await chord('c', 'copy'); await chord('v', 'paste'); s = await state();
    chk('d Shift-click mark + ⌘C/⌘V → rects +1', s.redact.rects.length === 2 && s.revision === r0 + 1, { rects: s.redact.rects, rev: s.revision, sel: s.direct.selection, ann: s.announcement, err: s.error });
  });
  if (only === 'misc') await session('misc', 'fixtures/hello.pdf', async ({ state, shot, click, rightClick, key, pagePoint, mode, cdp, active }) => {
    await mode('redact');
    const e = await pagePoint(300, 450);
    await rightClick(e.x, e.y); let s = await state();
    chk('m Redact right-click empty → page menu, no mark', s.direct.menu?.kind === 'page' && s.redact.marks === 0, { menu: s.direct.menu?.kind, marks: s.redact.marks });
    await key('Escape');
    await cdp.eval(`document.getElementById('redact-area').click()`); await sleep(300);
    const e2 = await pagePoint(300, 450);
    await rightClick(e2.x, e2.y); s = await state();
    const armed = await cdp.eval(`document.getElementById('redact-area').getAttribute('aria-pressed')`);
    chk('m Redact armed Mark area + right-click → page menu, no mark, still armed', s.direct.menu?.kind === 'page' && s.redact.marks === 0 && armed === 'true', { menu: s.direct.menu?.kind, marks: s.redact.marks, rects: s.redact.rects, rev: s.revision, dirty: s.dirty, ann: s.announcement, armed, gesture: s.direct.gesture });
    await shot('review-u7-m1-redact-armed-rightclick.png');
    await key('Escape');
    await mode('annotate');
    await cdp.eval(`document.getElementById('link-draw').click()`); await sleep(300);
    const e3 = await pagePoint(300, 450);
    await rightClick(e3.x, e3.y); s = await state();
    const armedL = await cdp.eval(`document.getElementById('link-draw').getAttribute('aria-pressed')`);
    chk('m Annotate armed Draw link + right-click → page menu, no link, still armed', s.direct.menu?.kind === 'page' && (s.links?.length ?? 0) === 0 && armedL === 'true', { menu: s.direct.menu, links: s.links?.length, rev: s.revision, ann: s.announcement, err: s.error, active: s.direct.active, dialogs: await cdp.eval(`[...document.querySelectorAll('dialog[open]')].map(d => d.id)`), focus: await active(), armedL, gesture: s.direct.gesture });
    await shot('review-u7-m2-linkdraw-rightclick.png');
    await key('Escape');
  });
  if (only === 'form2') await session('form2', 'fixtures/form2.pdf', async ({ state, shot, click, drag, move, key, pagePoint, mode, cdp, undo }) => {
    await mode('forms');
    await wait(async () => (await state()).fields?.count > 0, 20_000);
    let s = await state(); const r0 = s.revision;
    const [name, def] = Object.entries(s.fields.defs).find(([, d]) => d.kind === 'text');
    const c = await pagePoint((def.rect[0] + def.rect[2]) / 2, (def.rect[1] + def.rect[3]) / 2);
    await move(c.x, c.y); await sleep(300);
    const g = await cdp.eval(`(() => { const b = [...document.querySelectorAll('#hit-layer [data-field-id]')].find(n => n.getAttribute('aria-label')); const x = [...document.querySelectorAll('#hit-layer [data-field-id] .hit-grip')].map(n => { const r = n.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width }; }); return x; })()`);
    const near0 = g.sort((p, q) => Math.hypot(p.x - c.x, p.y - c.y) - Math.hypot(q.x - c.x, q.y - c.y))[0];
    await drag(near0.x, near0.y, 20 * c.s, 10 * c.s); s = await state();
    const moved = s.fields.defs[name]?.rect;
    chk('f form2 real field grip drag → rect +20,+10, one rev', s.revision === r0 + 1 && Math.abs(moved[0] - def.rect[0] - 20) <= 1 && Math.abs(moved[1] - def.rect[1] - 10) <= 1, { name, before: def.rect, after: moved, rev: s.revision, active: s.direct.active?.kind });
    await shot('review-u7-f2-form2-drag.png');
    await undo(r0 + 2);
  });
} finally {
  setOSC(savedOS); log('OS clipboard restored');
  await writeFile(join(checksDir, '../../../scratch/p6u7/review-renderer/probe-results.json'), JSON.stringify(results, null, 1));
  log('fixtures after\n' + hashes());
  log('SUMMARY', results.filter(r => r.ok).length, 'ok /', results.length);
}
