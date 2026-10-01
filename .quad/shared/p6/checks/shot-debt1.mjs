#!/usr/bin/env node
/**
 * Debt sweep 1 GUI evidence (.quad/shared/debt/scope-debt-1.md rows 3, 10, 18): real CDP mouse and keys, one Electron per document.
 * direct3.pdf in Edit mode (row 10): A handles stay visible on a `.selected` object with the pointer away; B a 2 px body drag is a
 * click (no op) and a 6 px drag moves; C a real double-click replaces Im2 (the picker answers from --images); D a click on empty
 * page opens the add editor, which grows while typing at the page's font size. tabs.pdf (rows 3, 18): #field-count names the
 * page's count on page 1 and page 2; the Save As button reads the menu's "Save As…". Screenshots → review-out/debt1-*.png.
 * Not part of npm test. Run after `npm run build`, under .quad/run.lock: node .quad/shared/p6/checks/shot-debt1.mjs
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
const KEYS = { Enter: 13, Escape: 27 };
let checks = 0;
const check = (ok, label, detail) => { if (!ok) throw new Error(`${label}: ${q(detail)}`); checks++; log('ok', label, q(detail)); };

/** One Electron on one fixture: probe (open + hold), fresh udata, SIGTERM in finally. */
async function session(tag, fixture, extra, body) {
  const udata = join(out, 'debt1-udata', tag);
  await rm(udata, { recursive: true, force: true });
  await mkdir(udata, { recursive: true });
  const probe = join(out, `debt1-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `debt1-out-${tag}`)}`, `--report=${join(out, `debt1-report-${tag}.json`)}`, ...extra], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  const closed = new Promise(done => child.on('close', done));
  let cdp;
  try {
    cdp = await attach(port);
    try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch { /* headless window manager */ }
    const state = () => cdp.eval('window.pdf4allState()');
    const shot = async name => {
      const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
      await writeFile(join(out, name), Buffer.from(data, 'base64'));
      log('shot', name, Date.now() - T0);
    };
    const move = (x, y) => cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
    const press = (x, y, clickCount) => cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount });
    const release = (x, y, clickCount) => cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount });
    const click = async (x, y) => { await move(x, y); await sleep(100); await press(x, y, 1); await release(x, y, 1); await sleep(600); };
    const dblclick = async (x, y) => { await move(x, y); await sleep(100); await press(x, y, 1); await release(x, y, 1); await press(x, y, 2); await release(x, y, 2); await sleep(1500); };
    const drag = async (x, y, dx, dy) => {
      await move(x, y); await sleep(150);
      await press(x, y, 1);
      for (let i = 1; i <= 8; i++) await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: x + dx * i / 8, y: y + dy * i / 8, button: 'left', buttons: 1 });
      await release(x + dx, y + dy, 1);
      await sleep(1500);
    };
    const key = async name => {
      const code = KEYS[name];
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code });
      await sleep(500);
    };
    /** Scroll the page point into view; return its client point and the px-per-pt scale. */
    const pagePoint = (x, y) => cdp.eval(`(() => {
      const v = document.getElementById('viewport'), c = document.getElementById('canvas');
      let r = c.getBoundingClientRect(); const s = r.width / 612;
      v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
      r = c.getBoundingClientRect();
      return { x: r.left + ${x} * s, y: r.top + ${y} * s, s };
    })()`);
    const rectOf = sel => cdp.eval(`(() => { const e = ${sel}; if (!e) return null; e.scrollIntoView({ block: 'center' }); const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2 }; })()`);
    const mode = async name => {
      const r = await wait(() => rectOf(`document.querySelector('label[for="mode-${name}"]')`));
      await click(r.cx, r.cy);
      await wait(async () => (await state()).mode === name);
    };
    await wait(() => cdp.eval(`document.title.includes(${q(fixture.split('/').pop().replace('.pdf', ''))})`), 60_000);
    await sleep(800);
    await body({ state, shot, move, click, dblclick, drag, key, pagePoint, rectOf, mode, cdp });
    log('PASS', tag);
  } finally {
    cdp?.close();
    child.kill('SIGTERM');
    log('electron closed', tag, await closed);
  }
}

const objectSel = name => `(() => { const o = window.pdf4allState().content.objects.find(i => i.name === ${q(name)}); return o && document.querySelector('#hit-layer .object-target[data-target-id="' + CSS.escape(String(o.targetId)) + '"]'); })()`;
const handleDisplay = sel => `(() => [...document.querySelectorAll(${q(sel)})].map(e => getComputedStyle(e).display))()`;

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
try {
  await session('direct3', 'fixtures/p6/direct3.pdf', [`--images=${join(root, 'fixtures/img-rgb.png')}`], async ({ state, shot, move, click, dblclick, drag, key, pagePoint, rectOf, mode, cdp }) => {
    await mode('edit');
    await wait(async () => (await state()).direct.objects === 3, 20_000);
    const s = (await pagePoint(430, 72)).s;

    // A. A real click selects Im2; with the pointer away (no :hover) its handles stay visible because it is .selected, the others' do not.
    const im2 = await wait(() => rectOf(objectSel('Im2')));
    await click(im2.cx, im2.cy);
    await wait(async () => (await state()).direct.active?.name === 'Im2');
    const away = await pagePoint(300, 500);
    await move(away.x, away.y); await sleep(400);
    const selected = await cdp.eval(handleDisplay('#hit-layer .object-target.selected .hit-handle'));
    check(selected.length > 0 && selected.every(d => d !== 'none'), 'A handles visible on .selected', selected);
    const others = await cdp.eval(handleDisplay('#hit-layer .object-target:not(.selected) .hit-handle'));
    check(others.length > 0 && others.every(d => d === 'none'), 'A handles hidden on unselected objects', others);
    await shot('debt1-a-selected-handles.png');

    // B. 2 CSS px is a click (no op); 6 CSS px on the body is a move (one op).
    const im1 = await wait(() => rectOf(objectSel('Im1')));
    const r0 = (await state()).revision, box0 = (await state()).content.objects.find(i => i.name === 'Im1').box;
    await drag(im1.cx, im1.cy, 2, 0);
    const r1 = (await state()).revision;
    check(r1 === r0 && q((await state()).content.objects.find(i => i.name === 'Im1').box) === q(box0), 'B 2 px drag writes nothing', { r0, r1 });
    const im1b = await wait(() => rectOf(objectSel('Im1')));
    await drag(im1b.cx, im1b.cy, 6, 0);
    const movedState = await wait(async () => { const v = await state(); return v.revision === r0 + 1 ? v : null; });
    const box1 = movedState.content.objects.find(i => i.name === 'Im1').box;
    check(movedState.content.edited?.action === 'move' && Math.abs((box1[0] - box0[0]) - 6 / s) < 0.6, 'B 6 px drag moves Im1', { box0, box1, s });
    await shot('debt1-b-drag-threshold.png');

    // C. A real double-click on Im2 replaces its image (one op).
    const im2b = await wait(() => rectOf(objectSel('Im2')));
    await dblclick(im2b.cx, im2b.cy);
    const replaced = await wait(async () => { const v = await state(); return v.content.edited?.action === 'replace' ? v : null; });
    check(replaced.content.edited.name === 'Im2' && replaced.revision === r0 + 2, 'C double-click replaces Im2', replaced.content.edited);
    await shot('debt1-c-dblclick-replace.png');
    if ((await state()).direct.active) { await key('Escape'); }

    // D. A click on empty page opens the add editor; it grows as text arrives and its font px is the page font size × zoom.
    const empty = await pagePoint(300, 500);
    await click(empty.x, empty.y);
    const a0 = await wait(async () => { const a = (await state()).direct.active; return a?.kind === 'add' ? a : null; });
    await cdp.send('Input.insertText', { text: 'Hi' }); await sleep(300);
    const a1 = (await state()).direct.active;
    await cdp.send('Input.insertText', { text: ' there, a much longer line of text' }); await sleep(300);
    const a2 = (await state()).direct.active;
    check(a2.draft === 'Hi there, a much longer line of text' && a2.box.width > a1.box.width && a1.box.width >= a0.box.width, 'D add editor grows while typing', [a0.box.width, a1.box.width, a2.box.width]);
    const fontPx = await cdp.eval(`parseFloat(getComputedStyle(document.activeElement).fontSize)`);
    check(a2.fontSize > 0 && Math.abs(fontPx - a2.fontSize * s) < 0.6, 'D add editor font px = fontSize × zoom', { fontPx, fontSize: a2.fontSize, s });
    await shot('debt1-d-add-editor.png');
    await key('Escape');
    await wait(async () => (await state()).direct.active === null);
  });

  await session('tabs', 'fixtures/p6/tabs.pdf', [], async ({ state, shot, click, rectOf, mode, cdp }) => {
    // Row 18: the toolbar button says what the File menu says.
    const saveAs = await cdp.eval(`document.getElementById('save-copy').textContent.trim()`);
    check(saveAs === 'Save As…', 'row 18 Save As button text', saveAs);
    // Row 3: the count is the shown page's.
    await mode('forms');
    await wait(async () => (await state()).page === 0 && (await cdp.eval(`document.getElementById('field-count').textContent`)).includes('6'));
    const page1 = await cdp.eval(`document.getElementById('field-count').textContent`);
    check(page1 === 'This page has 6 form fields.', 'row 3 #field-count page 1', page1);
    await shot('debt1-e-field-count-p1.png');
    const next = await wait(() => rectOf(`document.getElementById('next')`));
    await click(next.cx, next.cy);
    await wait(async () => (await state()).page === 1 && (await cdp.eval(`document.getElementById('field-count').textContent`)).includes('3'));
    const page2 = await cdp.eval(`document.getElementById('field-count').textContent`);
    check(page2 === 'This page has 3 form fields.', 'row 3 #field-count page 2', page2);
    await shot('debt1-f-field-count-p2.png');
  });
  log('PASS all', checks, 'checks');
} catch (err) {
  console.error('FAIL', err?.stack ?? err);
  process.exitCode = 1;
} finally {
  log('fixtures after\n' + hashes());
}
