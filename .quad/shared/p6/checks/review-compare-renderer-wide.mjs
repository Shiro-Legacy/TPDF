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


// ---------------- Run D: wide window + proper Enter typing in #compare-page-b
await session('d', [
  { cmd: 'open', path: abs('fixtures/nav.pdf') }, ...hold(2, 2000), { cmd: 'compare' }, ...hold(16, 5000),
], [`--insert-from=${abs('fixtures/hello.pdf')}`], async E => {
  let s = await wait(async () => { const v = await E.state(); return v.compare.open && v.compare.pixels ? v : null; }, 60_000);
  const enter = async () => { await E.cdp.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, text: '\r' }); await E.cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 }); await sleep(600); };
  const inp = await E.box('#compare-page-b'); info('#compare-page-b box', inp);
  const hit = await E.cdp.eval(`(() => { const r = document.getElementById('compare-page-b').getBoundingClientRect(); return document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2)?.id; })()`);
  info('centre hit', hit);
  // centre click on the field once: does it change the page (spinner)?
  await E.cdp.eval(`document.getElementById('compare-view').focus()`);
  const before = (await E.state()).compare.pageB; await E.clickEl('#compare-page-b'); await sleep(500);
  info('centre click on field: pageB before/after/value', { before, after: (await E.state()).compare.pageB, value: await E.cdp.eval(`document.getElementById('compare-page-b').value`) });
  const typeLeft = async text => { const b = await E.box('#compare-page-b'); await E.click(b.left + 6, b.top + b.height / 2); await E.cdp.eval(`document.getElementById('compare-page-b').select()`); await E.cdp.send('Input.insertText', { text }); await enter(); };
  // A = nav (3 pages), B = hello (1 page): pageB always clamps to 0. Use goto page 2 on A first.
  await E.cdp.eval(`document.getElementById('compare-view').focus()`);
  await E.key('PageDown', 'PageDown', 34); await sleep(800);
  for (const [typed, want] of [['9', 0], ['0', 0], ['-1', 0], ['1', 0], ['', 0]]) {
    await typeLeft(typed); const st = await E.state();
    info(`typed ${q(typed)}`, { pageA: st.compare.pageA, pageB: st.compare.pageB, field: await E.cdp.eval(`document.getElementById('compare-page-b').value`), summary: await E.text('#compare-summary') });
    check(st.compare.pageB === want, `typed ${q(typed)} -> pageB ${want}`, st.compare.pageB);
  }
  // wide viewport: overlay vs B registration for same-size pages
  await E.cdp.send('Emulation.setDeviceMetricsOverride', { width: 2200, height: 1100, deviceScaleFactor: 1, mobile: false }); await sleep(1200);
  await E.clickEl('#compare-show-diff'); await sleep(400);
  const b = await E.box('#compare-canvas-b'), o = await E.box('#compare-overlay'), w = await E.box('.compare-b');
  info('wide: B / overlay / wrapper', { b, o, w });
  check(Math.abs(o.width - b.width) < 1, 'wide window: overlay width == B width', { overlay: o.width, b: b.width });
  await E.shot('review-compare-i-wide-overlay.png');
});
