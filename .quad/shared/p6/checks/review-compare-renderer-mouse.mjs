#!/usr/bin/env node
/** Review probe (renderer lens, Compare): real CDP pointer/keys over #compare-view. Not a test. */
import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { rm, writeFile, mkdir } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../../../../', import.meta.url));
const checksDir = fileURLToPath(new URL('./', import.meta.url));
const out = join(checksDir, 'review-out');
const scratch = join(root, '.quad/scratch/compare/review-renderer');
const electron = createRequire(join(root, 'package.json'))('electron');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const q = s => JSON.stringify(s);
const check = (cond, label, got) => console.log(cond ? 'ok  ' : 'FAIL', label, q(got));
const info = (label, got) => console.log('info', label, q(got));
function freePort() { return new Promise((res, rej) => { const s = createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(e => e ? rej(e) : res(p)); }); s.on('error', rej); }); }
async function wait(fn, ms = 25_000, step = 100) {
  const t0 = Date.now(); let last;
  while (Date.now() - t0 < ms) { try { last = await fn(); } catch (e) { last = e; } if (last && !(last instanceof Error)) return last; await sleep(step); }
  throw new Error(`timeout: ${last instanceof Error ? last.message : q(last)}`);
}
class Cdp {
  constructor(url) { this.url = url; this.n = 0; this.pending = new Map(); }
  async connect() {
    this.ws = new WebSocket(this.url);
    await new Promise((res, rej) => { this.ws.addEventListener('open', res, { once: true }); this.ws.addEventListener('error', rej, { once: true }); });
    this.ws.addEventListener('message', ev => { const m = JSON.parse(String(ev.data)); if (m.id && this.pending.has(m.id)) { const { res, rej } = this.pending.get(m.id); this.pending.delete(m.id); m.error ? rej(new Error(m.error.message)) : res(m.result); } });
  }
  send(method, params = {}) { const id = ++this.n; return new Promise((res, rej) => { const t = setTimeout(() => { this.pending.delete(id); rej(new Error('cdp timeout ' + method)); }, 30_000); this.pending.set(id, { res: v => { clearTimeout(t); res(v); }, rej: e => { clearTimeout(t); rej(e); } }); this.ws.send(JSON.stringify({ id, method, params })); }); }
  async eval(expression) { const r = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || expression); return r.result?.value; }
  close() { try { this.ws.close(); } catch {} }
}
const hold = (n, ms) => Array.from({ length: n }, () => ({ cmd: 'wait', ms }));

async function session(tag, lines, extra, body) {
  const udata = join(out, `compare-review-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(scratch, `probe-${tag}.jsonl`);
  await writeFile(probe, lines.map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(scratch, `out-${tag}`)}`, `--report=${join(scratch, `report-${tag}.json`)}`, ...extra], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  const closed = new Promise(done => child.on('close', done));
  let cdp;
  try {
    const page = await wait(async () => (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.webSocketDebuggerUrl && t.type === 'page'), 60_000, 300);
    cdp = new Cdp(page.webSocketDebuggerUrl); await cdp.connect(); await cdp.send('Runtime.enable'); await cdp.send('Page.enable');
    try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch {}
    const E = {
      cdp, child,
      state: () => cdp.eval('window.pdf4allState()'),
      shot: async name => { const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }); await writeFile(join(out, name), Buffer.from(data, 'base64')); console.log('shot', name); },
      mouse: (type, x, y, e2 = {}) => cdp.send('Input.dispatchMouseEvent', { type, x, y, ...e2 }),
    };
    E.click = async (x, y) => { await E.mouse('mouseMoved', x, y); await sleep(60); await E.mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1 }); await E.mouse('mouseReleased', x, y, { button: 'left', buttons: 0, clickCount: 1 }); await sleep(300); };
    E.box = sel => cdp.eval(`(() => { const e = document.querySelector(${q(sel)}); if (!e) return null; const r = e.getBoundingClientRect(); return { left: r.left, top: r.top, right: r.right, bottom: r.bottom, width: r.width, height: r.height, shown: !e.hidden && getComputedStyle(e).display !== 'none', iw: e.width, ih: e.height }; })()`);
    E.clickEl = async sel => { const b = await E.box(sel); if (!b || !b.width) throw new Error('no box ' + sel); await E.click(b.left + b.width / 2, b.top + b.height / 2); };
    E.key = async (k, code, vk, modifiers = 0, pause = 200) => { await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: k, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers }); await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers }); await sleep(pause); };
    E.typeInto = async (sel, text) => { await E.clickEl(sel); await E.key('a', 'KeyA', 65, 4); await cdp.eval(`document.querySelector(${q(sel)}).select()`); await cdp.send('Input.insertText', { text }); await E.key('Enter', 'Enter', 13); await sleep(300); };
    E.settle = async () => { await sleep(200); await cdp.eval(`new Promise(r => setTimeout(r, 50))`); };
    E.hashCanvas = sel => cdp.eval(`(async () => { const c = document.querySelector(${q(sel)}); if (!c.width) return 'empty'; const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; const h = await crypto.subtle.digest('SHA-256', d); return c.width + 'x' + c.height + ':' + [...new Uint8Array(h)].slice(0, 8).map(b => b.toString(16).padStart(2, '0')).join(''); })()`);
    E.redCount = sel => cdp.eval(`(() => { const c = document.querySelector(${q(sel)}); if (!c.width) return -1; const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0; for (let i = 0; i < d.length; i += 4) if (d[i] > 200 && d[i+1] < 80 && d[i+2] < 80) n++; return n; })()`);
    E.text = sel => cdp.eval(`document.querySelector(${q(sel)})?.textContent ?? null`);
    await body(E);
  } catch (err) { console.log('FAIL exception', tag, err?.stack ?? err); }
  finally { cdp?.close(); child.kill('SIGTERM'); console.log('electron closed', tag, await closed); }
}
const abs = p => join(root, p);
await mkdir(out, { recursive: true });
const only = process.argv[2];

// ---------------- Run A: hello vs nav, then nav as A twice
if (!only || only === 'a') await session('a', [
  { cmd: 'open', path: abs('fixtures/hello.pdf') }, ...hold(4, 2500), { cmd: 'compare' }, ...hold(20, 5000),
  { cmd: 'open', path: abs('fixtures/nav.pdf') }, ...hold(20, 5000), { cmd: 'open', path: abs('fixtures/nav.pdf') }, ...hold(30, 5000),
], [`--insert-from=${abs('fixtures/nav.pdf')}`], async E => {
  await wait(async () => (await E.state()).doc?.name === 'hello.pdf', 60_000);
  await sleep(1500);
  const zoomBefore = await E.text('#zoom-level'); const statusBefore = await E.text('#status');
  info('before compare zoom/status', { zoomBefore, statusBefore, canvas: await E.box('#canvas') });
  let s = await wait(async () => { const v = await E.state(); return v.compare.open && v.compare.pixels ? v : null; }, 30_000);
  info('compare STATE', s.compare); info('announcement', s.announcement); info('error', s.error);
  check((await E.box('#reader')).shown === false, '#reader hidden', await E.box('#reader'));
  check((await E.box('#compare-view')).shown === true, '#compare-view shown', null);
  const a = await E.box('#compare-canvas-a'), b = await E.box('#compare-canvas-b'), o = await E.box('#compare-overlay'), wrap = await E.box('.compare-b');
  check(a.width > 0 && b.width > 0 && a.right <= b.left, 'side by side', { a, b });
  check(o.shown === false && s.compare.showDiff === false, 'overlay hidden before click', o);
  const summary = await E.text('#compare-summary');
  check(summary === s.announcement, 'summary === announcement', { summary, ann: s.announcement });
  check(/^Page 1 vs 1: 0 lines same, 1 removed, 1 added; \d+\.\d % of pixels differ$/.test(summary), 'summary pattern', summary);
  const rows = await E.cdp.eval(`[...document.querySelectorAll('#compare-list tbody tr')].map(r => ({ title: r.title, cells: [...r.cells].map(c => c.textContent) }))`);
  check(rows.length === 2 && rows.every(r => /^Page \d+, -?\d+ -?\d+ -?\d+ -?\d+ pt$/.test(r.title)), 'list rows titled', rows);
  const hashA0 = await E.hashCanvas('#compare-canvas-a');
  await E.clickEl('#compare-show-diff');
  s = await E.state();
  const o2 = await E.box('#compare-overlay');
  check(s.compare.showDiff === true && o2.shown, 'real click shows overlay', { showDiff: s.compare.showDiff, o2 });
  info('geometry wrap/B/overlay (css px, intrinsic)', { wrap, b: await E.box('#compare-canvas-b'), o: o2 });
  const bx = await E.box('#compare-canvas-b');
  info('overlay scale vs B scale', { overlayX: o2.width / o2.iw, overlayY: o2.height / o2.ih, bX: bx.width / bx.iw, bY: bx.height / bx.ih });
  info('overlay red px', await E.redCount('#compare-overlay'));
  await E.shot('review-compare-a-hello-nav-overlay.png');
  await E.clickEl('#compare-show-diff');
  check(await E.hashCanvas('#compare-canvas-a') === hashA0, 'A canvas identical after showDiff on/off', hashA0);
  // page-b typing, 1-based, clamps
  for (const [typed, want] of [['9', 2], ['0', 0], ['-1', 0], ['2', 1], ['', null], ['1.5', null]]) {
    const before = (await E.state()).compare.pageB;
    await E.typeInto('#compare-page-b', typed);
    await sleep(500);
    const st = await E.state();
    info(`typed ${q(typed)} -> pageB / field / summary`, { pageB: st.compare.pageB, before, field: await E.cdp.eval(`document.getElementById('compare-page-b').value`), summary: await E.text('#compare-summary'), error: st.error });
    if (want !== null) check(st.compare.pageB === want, `typed ${q(typed)} pageB`, st.compare.pageB);
  }
  // focus + Escape from inside the number input? (ignored by design)
  // latency: a pageB refresh (letter pages): MutationObserver on summary
  const lat = await E.cdp.eval(`(async () => { const el = document.getElementById('compare-summary'); const inp = document.getElementById('compare-page-b'); const out = []; for (const v of ['2','3','1']) { const t = performance.now(); const done = new Promise(r => new MutationObserver((m, ob) => { ob.disconnect(); r(); }).observe(el, { childList: true, characterData: true, subtree: true })); inp.value = v; inp.dispatchEvent(new Event('change')); await done; out.push(Math.round(performance.now() - t)); } return out; })()`);
  info('letter refresh latency ms (pageB 2,3,1; B uncached)', lat);
  // wait for nav as A
  s = await wait(async () => { const v = await E.state(); return v.doc?.name === 'nav.pdf' && v.compare.pixels ? v : null; }, 150_000, 200);
  await sleep(800); s = await E.state();
  check(s.compare.open && s.compare.pixels.differing === 0 && s.compare.pixels.ratio === 0, 'nav as A: slot survives, differing 0', s.compare);
  info('announcement after open (risk 3)', s.announcement); info('summary after open', await E.text('#compare-summary'));
  check((await E.box('#reader')).shown === false, 'reader still hidden after open', null);
  await E.cdp.eval(`document.getElementById('compare-view').focus()`);
  // PageDown x5 fast (real key events), then settle
  for (let i = 0; i < 5; i++) await E.key('PageDown', 'PageDown', 34, 0, 10);
  await sleep(2500); s = await E.state();
  const sum5 = await E.text('#compare-summary');
  check(s.compare.pageA === 2 && s.compare.pageB === 2 && s.compare.pixels.differing === 0 && /^Page 3 vs 3:/.test(sum5), 'PageDown x5: final page 3 vs 3', { c: s.compare, sum5, ann: s.announcement, page: s.page });
  // hand-set pageB holds until A changes
  await E.typeInto('#compare-page-b', '1'); await sleep(600);
  s = await E.state(); check(s.compare.pageA === 2 && s.compare.pageB === 0, 'hand pageB 0 while A 2', s.compare);
  await E.clickEl('#compare-show-diff'); await sleep(300);
  const hA = await E.hashCanvas('#compare-canvas-a'); await E.shot('review-compare-b-nav3-vs-nav1-overlay.png');
  await E.clickEl('#compare-show-diff'); await E.clickEl('#compare-show-diff');
  check(await E.hashCanvas('#compare-canvas-a') === hA, 'A canvas identical after toggling twice', hA);
  s = await E.state(); check(s.compare.pageB === 0, 'pageB still hand-set after toggles', s.compare.pageB);
  await E.cdp.eval(`document.getElementById('compare-view').focus()`);
  await E.key('PageUp', 'PageUp', 33); await sleep(1500);
  s = await E.state(); check(s.compare.pageA === 1 && s.compare.pageB === 1, 'PageUp: A 1, B follows 1', s.compare);
  info('announcement after PageUp (risk 3)', s.announcement);
  // second open of nav
  const oldSummary = await E.text('#compare-summary');
  s = await wait(async () => { const v = await E.state(); return v.revision === 0 && v.compare.pageA === 0 && v.doc?.name === 'nav.pdf' && (await E.text('#compare-summary')) !== oldSummary ? v : null; }, 150_000, 200);
  await sleep(1000); s = await E.state();
  const canv = await E.cdp.eval(`[...document.querySelectorAll('#compare-view canvas')].map(c => c.id + ':' + c.width + 'x' + c.height)`);
  check(canv.length === 3, 'second open: still three compare canvases', canv);
  info('second open compare', s.compare);
  // risk 1: page turn + rotate in flight
  await E.cdp.eval(`document.getElementById('compare-view').focus()`);
  await E.key('PageDown', 'PageDown', 34, 0, 0);
  const rot = await E.box('#page-rotate-right'); info('rotate button', rot);
  if (rot && rot.width) await E.click(rot.left + rot.width / 2, rot.top + rot.height / 2); else await E.cdp.eval(`document.getElementById('page-rotate-right').click()`);
  await sleep(2500); s = await E.state();
  const settled = { rev: s.revision, c: s.compare, summary: await E.text('#compare-summary'), ann: s.announcement, rot: s.rotations ?? s.doc?.rotations };
  info('after PageDown+rotate', settled);
  await E.typeInto('#compare-page-b', String(s.compare.pageB + 1)); await sleep(1500);
  const s2 = await E.state();
  check(q(s2.compare.pixels) === q(settled.c.pixels) && q(s2.compare.text) === q(settled.c.text), 'rotate in flight: settled == forced refresh', { forced: s2.compare, summary: await E.text('#compare-summary') });
  await E.shot('review-compare-c-rotated-a.png');
  // Escape closes
  await E.cdp.eval(`document.getElementById('compare-view').focus()`);
  await E.key('Escape', 'Escape', 27); await sleep(1500);
  s = await E.state();
  check(!s.compare.open && (await E.box('#reader')).shown && !(await E.box('#compare-view')).shown, 'Escape closes, reader back', s.compare);
  info('after close zoom/status/canvas', { zoom: await E.text('#zoom-level'), status: await E.text('#status'), canvas: await E.box('#canvas'), ann: s.announcement, zoomState: s.zoom });
  info('red px in main #canvas after close', await E.redCount('#canvas'));
  await E.shot('review-compare-d-after-escape.png');
});

// ---------------- Run B: box-a (portrait) vs land.pdf (landscape); encrypted main; guard; unlock
if (!only || only === 'b') await session('b', [
  { cmd: 'open', path: join(scratch, 'box-a.pdf') }, ...hold(2, 2000), { cmd: 'compare' }, ...hold(8, 5000),
  { cmd: 'open', path: abs('fixtures/encrypted.pdf') }, ...hold(4, 5000), { cmd: 'compare' }, ...hold(4, 5000),
  { cmd: 'password', contents: 'pdf4all-user' }, ...hold(12, 5000),
], [`--insert-from=${join(scratch, 'land.pdf')}`], async E => {
  let s = await wait(async () => { const v = await E.state(); return v.compare.open && v.compare.pixels ? v : null; }, 60_000);
  info('box vs land compare', s.compare);
  await E.clickEl('#compare-show-diff'); await sleep(300);
  const b = await E.box('#compare-canvas-b'), o = await E.box('#compare-overlay'), a = await E.box('#compare-canvas-a');
  info('A/B/overlay boxes', { a, b, o });
  // Where does the overlay put the box edge? A box is at x100..300,y100..200 (pt, top-down) in both; land B also has it -> no red there.
  const probe = await E.cdp.eval(`(() => { const o = document.getElementById('compare-overlay'); const d = o.getContext('2d').getImageData(0, 0, o.width, o.height).data; let minY = 1e9, maxY = -1, minX = 1e9, maxX = -1; for (let y = 0; y < o.height; y++) for (let x = 0; x < o.width; x++) if (d[(y * o.width + x) * 4 + 3]) { if (y < minY) minY = y; if (y > maxY) maxY = y; if (x < minX) minX = x; if (x > maxX) maxX = x; } return { minX, maxX, minY, maxY, w: o.width, h: o.height }; })()`);
  info('overlay red extent (overlay px)', probe);
  const sy = o.height / o.ih, bsy = b.height / b.ih;
  check(Math.abs(sy - bsy) < 0.01 && Math.abs(o.width / o.iw - b.width / b.iw) < 0.01, 'overlay px scale equals B px scale (registration)', { overlayScale: [o.width / o.iw, sy], bScale: [b.width / b.iw, bsy] });
  await E.shot('review-compare-e-portrait-vs-landscape.png');
  // encrypted main while compare open
  s = await wait(async () => { const v = await E.state(); return v.doc?.name === 'encrypted.pdf' ? v : null; }, 90_000, 200);
  await sleep(1500); s = await E.state();
  info('encrypted main: compare STATE', s.compare);
  info('encrypted main: view', { reader: await E.box('#reader'), view: await E.box('#compare-view'), summary: await E.text('#compare-summary'), a: await E.hashCanvas('#compare-canvas-a'), pw: await E.box('#password-form') });
  await E.shot('review-compare-f-encrypted-main.png');
  s = await wait(async () => { const v = await E.state(); return v.error ? v : null; }, 40_000, 200);
  info('guard error', s.error);
  s = await wait(async () => { const v = await E.state(); return v.doc?.needsPassword === false ? v : null; }, 60_000, 200);
  await sleep(2500); s = await E.state();
  info('after unlock compare STATE', s.compare);
  info('after unlock view', { summary: await E.text('#compare-summary'), reader: (await E.box('#reader')).shown, a: await E.hashCanvas('#compare-canvas-a'), error: s.error });
  await E.shot('review-compare-g-after-unlock.png');
});

// ---------------- Run C: MAX_PIXEL_EDGE pages
if (!only || only === 'c') await session('c', [
  { cmd: 'open', path: join(scratch, 'big-a.pdf') }, ...hold(2, 2000), { cmd: 'compare' }, ...hold(12, 5000),
], [`--insert-from=${join(scratch, 'big-b.pdf')}`], async E => {
  const t0 = Date.now();
  let s = await wait(async () => { const v = await E.state(); return v.compare.open && v.compare.pixels ? v : null; }, 60_000, 100);
  info('big compare', { c: s.compare, sinceAttach: Date.now() - t0 });
  const lat = await E.cdp.eval(`(async () => { const el = document.getElementById('compare-summary'); const inp = document.getElementById('compare-page-b'); const out = []; for (let i = 0; i < 3; i++) { const t = performance.now(); const done = new Promise(r => new MutationObserver((m, ob) => { ob.disconnect(); r(); }).observe(el, { childList: true, characterData: true, subtree: true })); inp.value = '1'; inp.dispatchEvent(new Event('change')); await done; out.push(Math.round(performance.now() - t)); } return out; })()`);
  info('4096pt refresh latency ms (A,B cached: text + pixel loop + paint)', lat);
  await E.shot('review-compare-h-big.png');
});
