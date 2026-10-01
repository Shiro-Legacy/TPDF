#!/usr/bin/env node
/**
 * Coded simple fonts GUI evidence (contract-coded.md D8): edit-coded.pdf, Edit text mode, CDP click on the `Lorem ipsum`
 * row of the Existing content list (#content-targets) -> the replace form opens on it with #content-repertoire visible
 * ("... Lorem ipsuDl"); REAL osascript keystrokes (addressed to the spawned Electron by unix id; refused unless it is
 * frontmost, checked in the same call) select all, type `ipsum Lorem` and press Return (Apply) -> STATE
 * content.edited.to === "ipsum Lorem", revision 1; shot review-out/coded-edited.png.
 * Launch/session()/click()/shot() machinery copied from shot-direct9.mjs.
 * Exit 2 + OSA-UNAVAILABLE when osascript cannot drive System Events; exit 1 + "blocked: frontmost <name>" when another
 * app is frontmost (nothing is typed). Not part of npm test. Run after `npm run build`, under .quad/run.lock:
 * node .quad/shared/p6/checks/shot-coded.mjs
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
const KEYS = { Enter: 13, Tab: 9, Escape: 27, ArrowRight: 39, ArrowDown: 40, c: 67, v: 86, x: 88, a: 65 };
const SHIFT = 8, ALT = 1, META = 4;

/** One Electron on one fixture: probe (open + hold), fresh udata, SIGTERM + hash check in finally. */
async function session(tag, fixture, body) {
  const udata = join(out, `coded-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `coded-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `coded-out-${tag}`)}`, `--report=${join(out, `coded-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  const closed = new Promise(done => child.on('close', done));
  let cdp;
  try {
    cdp = await attach(port);
    try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch { /* headless window manager */ }
    const state = () => cdp.eval('window.pdf4allState()');
    /** §9.30 (P6 U6): the first pagePoint (Im1 row) scrolls and saves scrollTop; shot() only asserts it (a scroll would close the menu). */
    let saved = null;
    const shot = async name => {
      const top = await cdp.eval(`document.getElementById('viewport').scrollTop`);
      if (saved !== null && top !== saved) throw new Error(`${name}: viewport.scrollTop ${top} !== saved ${saved}`);
      const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
      await writeFile(join(out, name), Buffer.from(data, 'base64'));
      log('shot', name, Date.now() - T0);
    };
    const mouse = (type, x, y, extra = {}) => cdp.send('Input.dispatchMouseEvent', { type, x, y, ...extra });
    const move = (x, y, modifiers = 0) => mouse('mouseMoved', x, y, { modifiers });
    /** modifiers (Shift 8, Alt 1, Meta 4) ride on moved, pressed and released. */
    const click = async (x, y, modifiers = 0) => {
      await move(x, y, modifiers); await sleep(100);
      await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1, modifiers });
      await mouse('mouseReleased', x, y, { button: 'left', buttons: 0, clickCount: 1, modifiers });
      await sleep(600);
    };
    const rightClick = async (x, y) => {
      await move(x, y); await sleep(100);
      await mouse('mousePressed', x, y, { button: 'right', buttons: 2, clickCount: 1 });
      await mouse('mouseReleased', x, y, { button: 'right', buttons: 0, clickCount: 1 });
      await sleep(600);
    };
    /** `during` runs after the last move, before the release (mid-gesture STATE). */
    const drag = async (x, y, dx, dy, modifiers = 0, during) => {
      await move(x, y, modifiers); await sleep(150);
      await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1, modifiers });
      for (let i = 1; i <= 8; i++) await mouse('mouseMoved', x + dx * i / 8, y + dy * i / 8, { button: 'left', buttons: 1, modifiers });
      const seen = during ? await during() : undefined;
      await mouse('mouseReleased', x + dx, y + dy, { button: 'left', buttons: 0, clickCount: 1, modifiers });
      await sleep(1500);
      return seen;
    };
    const key = async (name, modifiers = 0) => {
      const code = KEYS[name];
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await sleep(500);
    };
    /** Page pt (y-down) → client px; scale by the shown page's real CSS width over its width in pt; `scroll` centres the point first. */
    const pagePoint = async (x, y, scroll = true, widthPt = 612) => {
      const p = await cdp.eval(`(() => {
        const v = document.getElementById('viewport'), c = document.getElementById('canvas');
        let r = c.getBoundingClientRect(); const s = r.width / ${widthPt};
        if (${scroll}) v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
        r = c.getBoundingClientRect();
        return { x: r.left + ${x} * s, y: r.top + ${y} * s, s, top: v.scrollTop };
      })()`);
      if (scroll) { await sleep(300); saved = await cdp.eval(`document.getElementById('viewport').scrollTop`); }
      return p;
    };
    const mode = async name => cdp.eval(`document.getElementById('mode-${name}').click()`);
    await wait(() => cdp.eval(`document.title.includes(${q(fixture.split('/').pop().replace('.pdf', ''))})`), 60_000);
    await sleep(800);
    await body({ state, shot, move, click, rightClick, drag, key, pagePoint, mode, cdp, child });
    log('PASS', tag);
  } finally {
    cdp?.close();
    child.kill('SIGTERM');
    log('electron closed', tag, await closed);
    log('fixtures after', tag, '\n' + hashes());
  }
}

const deq = (got, want, label) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`${label}: ${q(got)} vs ${q(want)}`);
  log('ok', label, q(got));
};
class OsaUnavailable extends Error {}
/** One osascript call: front the spawned Electron (by unix id); only if it is frontmost: ⌘A, type `text`, Return. */
function typeAndApply(pid, text) {
  const proc = `(first process whose unix id is ${pid})`;
  try {
    const script = [
      'tell application "System Events"',
      `tell ${proc} to set frontmost to true`,
      'delay 0.4',
      `if frontmost of ${proc} then`,
      'keystroke "a" using command down',
      `keystroke ${JSON.stringify(text)}`,
      'delay 0.2',
      'keystroke return',
      'else',
      'error "not frontmost: " & (name of first process whose frontmost is true)',
      'end if',
      'end tell',
    ];
    execFileSync('osascript', script.flatMap(line => ['-e', line]), { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  } catch (err) {
    const message = String(err.stderr ?? err.message);
    if (/-1712|1002|not allowed assistive|not authori[sz]ed/i.test(message)) throw new OsaUnavailable(message.trim());
    const front = message.match(/not frontmost: ([^\n"]*)/)?.[1];
    throw new Error(front !== undefined ? `blocked: frontmost ${front}` : `osascript: ${message.trim()}`);
  }
}

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
try {
  await session('a', 'fixtures/p0b/edit-coded.pdf', async ({ state, shot, click, mode, cdp, child }) => {
    await mode('edit');
    await wait(async () => (await state()).content?.candidates === 4);
    const row = await wait(() => cdp.eval(`(() => {
      const r = [...document.querySelectorAll('#content-targets li[role="option"]')].find(li => li.textContent.includes('Lorem ipsum'));
      if (!r) return null;
      r.scrollIntoView({ block: 'center' });
      const b = r.getBoundingClientRect();
      return { x: b.left + b.width / 2, y: b.top + b.height / 2 };
    })()`));
    await click(row.x, row.y);
    const form = await wait(() => cdp.eval(`(() => {
      const f = document.getElementById('content-edit-form'), rep = document.getElementById('content-repertoire');
      const sel = document.querySelector('#content-targets li[aria-selected="true"]');
      return !f.hidden && !rep.hidden && sel ? { selected: sel.textContent, repertoire: rep.textContent, value: document.getElementById('content-new-text').value, focused: document.activeElement?.id } : null;
    })()`), 10_000);
    if (!form.selected.includes('Lorem ipsum')) throw new Error(`selected row ${q(form.selected)}`);
    deq(form.repertoire.endsWith(': Lorem ipsuDl'), true, `#content-repertoire visible: ${form.repertoire}`);
    deq([form.value, form.focused], ['Lorem ipsum', 'content-new-text'], 'form opens on the run, input focused');
    typeAndApply(child.pid, 'ipsum Lorem');
    const done = await wait(async () => { const v = await state(); return v.revision === 1 ? v : null; }, 10_000);
    deq(done.content.edited, { from: 'Lorem ipsum', to: 'ipsum Lorem' }, 'content.edited');
    deq([done.content.refused, done.dirty], [[], true], 'no refusal, dirty');
    await shot('coded-edited.png');
  });
  log('PASS all');
} catch (err) {
  if (err instanceof OsaUnavailable) { console.error('OSA-UNAVAILABLE', err.message); process.exitCode = 2; }
  else { console.error('FAIL', err?.stack ?? err); process.exitCode = 1; }
}
