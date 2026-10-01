#!/usr/bin/env node
/** Review probe (renderer lens): real CDP pointer + insertText over the stamp dialog, Bates across two documents. Not a test. */
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

const root = fileURLToPath(new URL('../../../../', import.meta.url));
const checksDir = fileURLToPath(new URL('./', import.meta.url));
const out = join(checksDir, 'review-out');
const electron = createRequire(join(root, 'package.json'))('electron');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const q = s => JSON.stringify(s);
const results = [];
const check = (cond, label, got) => { results.push({ ok: !!cond, label, got }); console.log(cond ? 'ok  ' : 'FAIL', label, q(got)); };
function freePort() { return new Promise((res, rej) => { const s = createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(e => e ? rej(e) : res(p)); }); s.on('error', rej); }); }
async function wait(fn, ms = 25_000, step = 200) {
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
  send(method, params = {}) { const id = ++this.n; return new Promise((res, rej) => { const t = setTimeout(() => { this.pending.delete(id); rej(new Error('cdp timeout ' + method)); }, 15_000); this.pending.set(id, { res: v => { clearTimeout(t); res(v); }, rej: e => { clearTimeout(t); rej(e); } }); this.ws.send(JSON.stringify({ id, method, params })); }); }
  async eval(expression) { const r = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }); if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || expression); return r.result?.value; }
  close() { try { this.ws.close(); } catch {} }
}
const hold = (n, ms) => Array.from({ length: n }, () => ({ cmd: 'wait', ms }));
const udata = join(out, 'bates-review-udata-shot');
await rm(udata, { recursive: true, force: true });
const probe = join(root, '.quad/scratch/bates/review-renderer/probe-shot.jsonl');
await writeFile(probe, [{ cmd: 'open', path: join(root, 'fixtures/nav.pdf') }, { cmd: 'header-footer', contents: { kind: 'footer', text: 'ACME-{bates}', batesStart: 500, align: 'right' } }, { cmd: 'header-footer', contents: { kind: 'header', text: 'Page {page} of {pages}' } }, ...hold(12, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
const port = await freePort();
const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
  `--drive=${probe}`, '--gui', `--drive-out=${join(root, '.quad/scratch/bates/review-renderer/out')}`, `--report=${join(root, '.quad/scratch/bates/review-renderer/report.json')}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
const closed = new Promise(done => child.on('close', done));
let cdp;
try {
  const page = await wait(async () => (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.webSocketDebuggerUrl && (t.type === 'page')));
  cdp = new Cdp(page.webSocketDebuggerUrl); await cdp.connect(); await cdp.send('Runtime.enable'); await cdp.send('Page.enable');
  try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch {}
  const state = () => cdp.eval('window.pdf4allState()');
  const shot = async name => { const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }); await writeFile(join(out, name), Buffer.from(data, 'base64')); console.log('shot', name); };
  const mouse = (type, x, y, extra = {}) => cdp.send('Input.dispatchMouseEvent', { type, x, y, ...extra });
  const click = async (x, y, clickCount = 1) => { await mouse('mouseMoved', x, y); await sleep(80); for (let c = 1; c <= clickCount; c++) { await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: c }); await mouse('mouseReleased', x, y, { button: 'left', buttons: 0, clickCount: c }); } await sleep(500); };
  const centre = sel => cdp.eval(`(() => { const e = document.querySelector(${q(sel)}); if (!e) return null; const r = e.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2, w: r.width }; })()`);
  const clickEl = async sel => { const c = await centre(sel); if (!c || !c.w) throw new Error('no box ' + sel); await click(c.x, c.y); };
  const key = async (k, code, vk, modifiers = 0) => { await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: k, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers }); await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers }); await sleep(250); };
  /** real click into the field, select-all via real ⌘A (select via JS fallback), then insertText. */
  const typeInto = async (sel, text) => { await clickEl(sel); await key('a', 'KeyA', 65, 4); await cdp.eval(`document.querySelector(${q(sel)}).select()`); if (text) await cdp.send('Input.insertText', { text }); else await key('Backspace', 'Backspace', 8); await sleep(150); };
  const val = sel => cdp.eval(`document.querySelector(${q(sel)}).value`);
  const setSelect = async (sel, v) => { await cdp.eval(`document.querySelector(${q(sel)}).focus()`); await cdp.eval(`(() => { const s = document.querySelector(${q(sel)}); s.value = ${q(v)}; s.dispatchEvent(new Event('input', { bubbles: true })); s.dispatchEvent(new Event('change', { bubbles: true })); })()`); await sleep(150); };
  const dlgOpen = () => cdp.eval(`document.getElementById('stamp-dialog').open`);

  await wait(async () => (await state()).revision === 2, 60_000); await sleep(1000);
  await clickEl('#fit-page'); await sleep(1500);
  const info = await cdp.eval(`(() => { const v = document.getElementById('viewport'), c = document.getElementById('canvas'); const r = c.getBoundingClientRect(), vr = v.getBoundingClientRect(); return { c: [r.left, r.top, r.width, r.height], v: [vr.left, vr.top, vr.width, vr.height], fit: document.getElementById('fit-page').getAttribute('aria-pressed'), cw: c.width, ch: c.height }; })()`);
  console.log('geom', q(info));
  const [l, t, w, h] = info.c;
  const { data } = await cdp.send('Page.captureScreenshot', { format: 'png', clip: { x: l, y: t, width: w, height: h, scale: 1 }, captureBeyondViewport: true });
  await writeFile(join(out, 'review-bates-e-nav-p1-canvas.png'), Buffer.from(data, 'base64'));
  // pixel scan of the canvas backing store: rightmost/leftmost dark pixel in the bottom 60 pt and top 60 pt bands
  const scan = await cdp.eval(`(() => { const c = document.getElementById('canvas'); const x = c.getContext('2d', { willReadFrequently: true }); const W = c.width, H = c.height, s = W / 792; const band = (y0, y1) => { const d = x.getImageData(0, y0, W, y1 - y0).data; let lo = W, hi = -1; for (let i = 0; i < d.length; i += 4) if (d[i] < 128 && d[i+3] > 0) { const px = (i / 4) % W; lo = Math.min(lo, px); hi = Math.max(hi, px); } return [lo / s, hi / s]; }; return { W, H, s, top: band(0, Math.round(60 * s)), bottom: band(H - Math.round(60 * s), H) }; })()`).catch(e => String(e));
  console.log('scan', q(scan));
} catch (err) { console.log('ERROR', err.stack); }
finally {
  cdp?.close(); child.kill('SIGTERM'); console.log('electron closed', await closed);
  console.log(execFileSync('shasum', ['-a', '256', '-c', join(checksDir, 'fixtures.sha256')], { cwd: root, encoding: 'utf8' }));
}
