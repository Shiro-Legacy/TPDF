#!/usr/bin/env node
/**
 * P6 U4 GUI evidence (contract-p6-unit4.md §6.3 as amended by §9.19): real CDP input, one Electron per fixture.
 * links.pdf: hover L1 in Annotate → grip + handles visible; real body drag of L1; real click, Enter, insertText, Enter → new URI.
 * tabs.pdf: real Tab presses a1 → a2 → a3 → b1 in Forms mode. direct3.pdf: plain → on Im2 in Edit (1 pt), the outlines
 * screenshot, then a real mouse drag on empty space in Redact mode → one mark. Screenshots → review-out/u4-*.png.
 * Not part of npm test. Run after `npm run build`, under .quad/run.lock: node .quad/shared/p6/checks/shot-direct4.mjs
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
const KEYS = { Enter: 13, Tab: 9, ArrowRight: 39, ArrowDown: 40 };

/** One Electron on one fixture: probe (open + hold), fresh udata, SIGTERM + hash check in finally. */
async function session(tag, fixture, body) {
  const udata = join(out, `u4-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `u4-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `u4-out-${tag}`)}`, `--report=${join(out, `u4-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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
    const click = async (x, y) => {
      await move(x, y); await sleep(100);
      await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
      await sleep(600);
    };
    const drag = async (x, y, dx, dy) => {
      await move(x, y); await sleep(150);
      await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
      for (let i = 1; i <= 8; i++) await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: x + dx * i / 8, y: y + dy * i / 8, button: 'left', buttons: 1 });
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: x + dx, y: y + dy, button: 'left', buttons: 0, clickCount: 1 });
      await sleep(1500);
    };
    const key = async (name, modifiers = 0) => {
      const code = KEYS[name];
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await sleep(500);
    };
    /** Scroll the page point into view; scale by the shown page's real CSS width over its width in pt (§9.19). */
    const pagePoint = (x, y, widthPt = 612) => cdp.eval(`(() => {
      const v = document.getElementById('viewport'), c = document.getElementById('canvas');
      let r = c.getBoundingClientRect(); const s = r.width / ${widthPt};
      v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
      r = c.getBoundingClientRect();
      return { x: r.left + ${x} * s, y: r.top + ${y} * s, s };
    })()`);
    const rectOf = sel => cdp.eval(`(() => { const e = ${sel}; if (!e) return null; e.scrollIntoView({ block: 'center' }); const r = e.getBoundingClientRect(); const cs = getComputedStyle(e); return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2, display: cs.display, visibility: cs.visibility }; })()`);
    const mode = async name => cdp.eval(`document.getElementById('mode-${name}').click()`);
    await wait(() => cdp.eval(`document.title.includes(${q(fixture.split('/').pop().replace('.pdf', ''))})`), 60_000);
    await sleep(800);
    await body({ state, shot, move, click, drag, key, pagePoint, rectOf, mode, cdp });
    log('PASS', tag);
  } finally {
    cdp?.close();
    child.kill('SIGTERM');
    log('electron closed', tag, await closed);
    log('fixtures after', tag, '\n' + hashes());
  }
}

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
try {
  // A. links.pdf in Annotate: hover handles, body drag, Enter + insertText + Enter.
  await session('links', 'fixtures/p6/links.pdf', async ({ state, shot, move, click, drag, key, pagePoint, rectOf, mode, cdp }) => {
    await mode('annotate');
    await wait(async () => (await state()).direct.links === 3, 20_000);
    const L1 = `document.querySelector('#hit-layer .link-target[data-annot-id="10"]')`;
    const box = await wait(() => rectOf(L1));
    await move(box.cx, box.cy); await sleep(300);
    for (const handle of ['grip', 'nw', 'se']) {
      const h = await rectOf(`(${L1})?.querySelector('[data-handle="${handle}"]')`);
      if (!h || h.w < 1 || h.display === 'none' || h.visibility === 'hidden') throw new Error(`A handle ${handle} not visible on hover: ${q(h)}`);
    }
    log('ok A handles visible on hover');
    await shot('u4-a-link-handles.png');
    const scale = (await pagePoint(136, 82)).s;
    const before = (await state()).links[0].rect;
    near(before, [72, 72, 200, 92], 0.05, 'A L1 before');
    const body = await rectOf(L1);
    await drag(body.cx, body.cy, 30 * scale, 40 * scale);
    const moved = await wait(async () => { const r = (await state()).links[0]?.rect; return r && Math.abs(r[0] - before[0]) > 1 ? r : null; });
    near(moved, [102, 112, 230, 132], 0.5, 'A L1 moved by a real body drag');
    await shot('u4-b-link-moved.png');
    const after = await wait(() => rectOf(L1));
    await click(after.cx, after.cy);
    await wait(async () => (await state()).direct.active?.kind === 'link');
    await key('Enter');
    await wait(async () => (await state()).direct.active?.editing === true);
    await cdp.send('Input.insertText', { text: 'https://example.org/gui' });
    await sleep(300);
    await shot('u4-c-link-editor.png');
    await key('Enter');
    const s = await wait(async () => { const v = await state(); return v.links[0]?.uri === 'https://example.org/gui' ? v : null; });
    if (!q(s.nav.links).includes('https://example.org/gui')) throw new Error(`C nav.links not updated: ${q(s.nav.links)}`);
    log('ok C retargeted by Enter + insertText + Enter', q(s.nav.links));
    await shot('u4-c-link-retargeted.png');
  });

  // B. tabs.pdf page 0 (/Tabs C) in Forms: real Tab a1 → a2 → a3 → b1.
  await session('tabs', 'fixtures/p6/tabs.pdf', async ({ state, shot, click, key, pagePoint, mode }) => {
    await mode('forms');
    await wait(async () => (await state()).fields?.order?.length === 6, 20_000);
    const a1 = await pagePoint(132, 82);
    await click(a1.x, a1.y);
    await wait(async () => (await state()).direct.active?.name === 'a1');
    const seen = ['a1'];
    for (const want of ['a2', 'a3', 'b1']) {
      await key('Tab');
      await wait(async () => (await state()).direct.active?.name === want);
      seen.push(want);
    }
    log('ok D real Tab order', seen.join(' → '));
    await shot('u4-d-tab-b1.png');
  });

  // C. direct3.pdf: outlines in Edit, plain → on Im2, then a Redact-mode drag on empty space.
  await session('direct3', 'fixtures/p6/direct3.pdf', async ({ state, shot, click, drag, key, pagePoint, rectOf, mode }) => {
    await mode('edit');
    await wait(async () => { const v = await state(); return v.direct.objects === 3 && v.direct.outlines === true; }, 20_000);
    await sleep(500);
    await shot('u4-e-edit-outlines.png');
    const sel = `(() => { const o = window.pdf4allState().content.objects.find(i => i.name === 'Im2'); return o && document.querySelector('#hit-layer .object-target[data-target-id="' + CSS.escape(String(o.targetId)) + '"]'); })()`;
    const im2 = await wait(() => rectOf(sel));
    await click(im2.cx, im2.cy);
    await wait(async () => (await state()).direct.active?.name === 'Im2');
    const before = (await state()).content.objects.find(i => i.name === 'Im2').box;
    await key('ArrowRight');
    const box = await wait(async () => { const b = (await state()).content.objects.find(i => i.name === 'Im2')?.box; return b && b[0] !== before[0] ? b : null; });
    near(box, [before[0] + 1, before[1], before[2] + 1, before[3]], 0.05, 'F Im2 plain → 1 pt');
    await shot('u4-f-im2-nudged.png');
    await mode('redact');
    await wait(async () => (await state()).mode === 'redact');
    const marks = (await state()).redact.marks;
    const start = await pagePoint(300, 500);
    await drag(start.x, start.y, 100 * start.s, 30 * start.s);
    const s = await wait(async () => { const v = await state(); return v.redact.marks === marks + 1 ? v : null; });
    near(s.redact.rects.at(-1).rect, [300, 500, 400, 530], 1, 'G unarmed Redact drag marks the area');
    await shot('u4-g-redact-drag.png');
  });
  log('PASS all');
} catch (err) {
  console.error('FAIL', err?.stack ?? err);
  process.exitCode = 1;
}
