#!/usr/bin/env node
/**
 * Bates GUI evidence (contract-bates.md §3.4 as amended by §9.9, §9.10, §9.14): the stamp dialog opened with a REAL macOS ⌥⌘F
 * applies a right-aligned footer "ACME-{bates}" from 500 over nav.pdf's three pages.
 * Launch/session()/Cdp/pagePoint()/click()/shot() machinery copied from shot-direct9.mjs (no --inspect, no CDP chord).
 * nav p0: osascript ⌥⌘F (addressed to the spawned Electron by unix id; refused unless it is frontmost) → #stamp-dialog open;
 * CDP sets #stamp-kind footer (+change) → heading "Add footer"; #stamp-text, #stamp-align right, #stamp-bates-start 500 (+change);
 * Apply = CDP mouse press+release at #stamp-save's centre; gates decorate.last.ok true, decorate.bates.next 503,
 * #page-status "Added footer to 3 pages (Bates 000500–000502)"; CDP-click #fit-page; shot review-out/bates-a-footer.png.
 * Exit 2 + OSA-UNAVAILABLE when osascript cannot drive System Events (-1712 / 1002); exit 1 + "blocked: frontmost <name>" when
 * another app is frontmost (nothing is typed), or naming the chord when the dialog does not open. Not part of npm test.
 * Run after `npm run build`, under .quad/run.lock: node .quad/shared/p6/checks/shot-bates.mjs
 */
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'node:net';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
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
const NAV_SHA = '8e7c37500e46120b9bd774bd8aab8d093d9e21aacc522a66216406373229ed82';
const ANNOUNCE = 'Added footer to 3 pages (Bates 000500–000502)';
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
const nineOk = text => { const n = (text.match(/: OK$/gm) ?? []).length; if (n !== 9) throw new Error(`fixtures.sha256: ${n} OK, want 9`); };
const deq = (got, want, label) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`${label}: ${q(got)} vs ${q(want)}`);
  log('ok', label, q(got));
};

/** One Electron on one fixture: probe (open + hold), fresh udata, SIGTERM + hash check in finally. */
async function session(tag, fixture, body) {
  const udata = join(out, `bates-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `bates-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `bates-out-${tag}`)}`, `--report=${join(out, `bates-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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
    const mouse = (type, x, y, extra = {}) => cdp.send('Input.dispatchMouseEvent', { type, x, y, ...extra });
    const click = async (x, y) => {
      await mouse('mouseMoved', x, y); await sleep(100);
      await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1 });
      await mouse('mouseReleased', x, y, { button: 'left', buttons: 0, clickCount: 1 });
      await sleep(600);
    };
    /** The centre of an element's client rect. */
    const centre = id => cdp.eval(`(() => { const r = document.getElementById(${q(id)}).getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
    await wait(() => cdp.eval(`document.title.includes(${q(fixture.split('/').pop().replace('.pdf', ''))})`), 60_000);
    await sleep(800);
    await body({ state, shot, click, centre, cdp, child });
    log('PASS', tag);
  } finally {
    cdp?.close();
    child.kill('SIGTERM');
    log('electron closed', tag, await closed);
    const after = hashes();
    log('fixtures after', tag, '\n' + after);
    nineOk(after);
    const nav = createHash('sha256').update(await readFile(join(root, 'fixtures/nav.pdf'))).digest('hex');
    deq(nav, NAV_SHA, 'fixtures/nav.pdf sha after');
  }
}

class OsaUnavailable extends Error {}
/** One osascript call: bring the spawned Electron (by unix id) to the front, then ⌥⌘F only if it is frontmost. */
function optionCommandF(pid) {
  const proc = `(first process whose unix id is ${pid})`;
  try {
    const script = [
      'tell application "System Events"',
      `tell ${proc} to set frontmost to true`,
      'delay 0.4',
      `if frontmost of ${proc} then`,
      'keystroke "f" using {option down, command down}',
      'else',
      'error "not frontmost: " & (name of first process whose frontmost is true)',
      'end if',
      'end tell',
    ];
    execFileSync('osascript', script.flatMap(line => ['-e', line]), { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  } catch (err) {
    const text = String(err.stderr ?? err.message);
    if (/-1712|1002|not allowed assistive|not authori[sz]ed/i.test(text)) throw new OsaUnavailable(text.trim());
    const front = text.match(/not frontmost: ([^\n"]*)/)?.[1];
    throw new Error(front !== undefined ? `blocked: frontmost ${front}` : `osascript: ${text.trim()}`);
  }
}

const before = hashes();
log('fixtures before\n' + before);
nineOk(before);
await mkdir(out, { recursive: true });
try {
  await session('a', 'fixtures/nav.pdf', async ({ state, shot, click, centre, cdp, child }) => {
    optionCommandF(child.pid);
    const opened = await wait(() => cdp.eval(`document.getElementById('stamp-dialog').open`), 5_000).catch(() => false);
    if (!opened) throw new Error('⌥⌘F (keystroke "f" using {option down, command down}) did not open #stamp-dialog');
    log('ok ⌥⌘F opens #stamp-dialog');
    const set = (id, value) => cdp.eval(`(() => { const e = document.getElementById(${q(id)}); e.value = ${q(value)}; e.dispatchEvent(new Event('change')); return e.value; })()`);
    await set('stamp-kind', 'footer');
    deq(await cdp.eval(`document.getElementById('stamp-dialog-heading').textContent`), 'Add footer', 'heading after kind change');
    await set('stamp-text', 'ACME-{bates}');
    await set('stamp-align', 'right');
    await set('stamp-bates-start', '500');
    const statusText = () => cdp.eval(`document.getElementById('status').textContent`);
    const status0 = await statusText();
    const apply = await centre('stamp-save');
    await click(apply.x, apply.y);
    // T-R1: applyEdit sets revision before its awaits and addStamp records decorate after them, so gate on decorate.last.
    const v = await wait(async () => { const s = await state(); return s.decorate?.last ? s : null; }, 10_000);
    deq(v.revision, 1, 'revision');
    deq(v.decorate?.last?.ok, true, 'decorate.last.ok');
    deq(v.decorate?.bates?.next, 503, 'decorate.bates.next');
    deq(await cdp.eval(`document.getElementById('page-status').textContent`), ANNOUNCE, '#page-status');
    // The page canvas repaints after the edit and again after fit-page; the shot must follow both repaints,
    // else it shows the pre-stamp bitmap. #status reads "Page 1 of 3 — rendered in N ms at Z%" after each paint.
    const status1 = await wait(async () => { const t = await statusText(); return t !== status0 && /rendered in \d+ ms/.test(t) ? t : null; }, 10_000);
    log(`status after stamp ${JSON.stringify(status1)}`);
    // #fit-page sits in the toolbar's zoom bar, which the Inspector occludes at this window width (the reviewer saw
    // the same overflow on #stamp-header). Hit-test first: a real click only when the button is what the pointer hits;
    // otherwise fall back to element.click() (view setup, not the proof gesture) and say so in the log.
    const fitHit = await cdp.eval(`(() => { const b = document.getElementById('fit-page'); const r = b.getBoundingClientRect(); const e = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2); return { x: r.left + r.width / 2, y: r.top + r.height / 2, hit: e === b || b.contains(e), over: e ? (e.id || e.tagName) : null }; })()`);
    if (fitHit.hit) { await click(fitHit.x, fitHit.y); log('fit-page: real click'); }
    else { await cdp.eval(`document.getElementById('fit-page').click()`); log(`fit-page: occluded by ${fitHit.over} at (${Math.round(fitHit.x)},${Math.round(fitHit.y)}) → element.click()`); }
    const status2 = await wait(async () => { const t = await statusText(); return t !== status1 && /rendered in \d+ ms/.test(t) ? t : null; }, 10_000).catch(() => null);
    log(`status after fit-page ${JSON.stringify(status2 ?? '(unchanged — zoom already fit-page)')}`);
    deq(await cdp.eval(`document.getElementById('fit-page').getAttribute('aria-pressed')`), 'true', 'fit-page pressed');
    const canvasBottom = await cdp.eval(`(() => { const r = document.getElementById('canvas').getBoundingClientRect(); return { bottom: r.bottom, inner: window.innerHeight }; })()`);
    if (!(canvasBottom.bottom <= canvasBottom.inner)) throw new Error(`page bottom off-screen: canvas bottom ${canvasBottom.bottom} > viewport ${canvasBottom.inner}`);
    await sleep(500);
    await shot('bates-a-footer.png');
  });
  log('PASS all');
} catch (err) {
  if (err instanceof OsaUnavailable) { console.error('OSA-UNAVAILABLE', err.message); process.exitCode = 2; }
  else { console.error('FAIL', err?.stack ?? err); process.exitCode = 1; }
}
