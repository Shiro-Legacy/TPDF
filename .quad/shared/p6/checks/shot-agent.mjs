#!/usr/bin/env node
/**
 * Agent session GUI evidence (contract-agent-session.md D8): the app launched with NO document or drive flags (a normal
 * GUI window on a scratch --user-data-dir under review-out, CDP on a free port); REAL osascript ⌘N (addressed to the
 * spawned Electron by unix id; refused unless it is frontmost, checked in the same call) -> File ▸ New… -> CDP
 * window.pdf4allState() shows doc.name "Untitled.pdf", doc.pageCount 1, dirty false; shot review-out/agent-new.png.
 * Launch/attach/Cdp machinery copied from shot-coded.mjs.
 * Exit 2 + OSA-UNAVAILABLE when osascript cannot drive System Events; exit 1 + "blocked: frontmost <name>" when another
 * app is frontmost (nothing is typed). Not part of npm test. Run after `npm run build`, under .quad/run.lock, only while
 * the human is away: node .quad/shared/p6/checks/shot-agent.mjs
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

const hashes = () => execFileSync('shasum', ['-a', '256', '-c', join(checksDir, 'fixtures.sha256')], { cwd: root, encoding: 'utf8' });
const deq = (got, want, label) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`${label}: ${q(got)} vs ${q(want)}`);
  log('ok', label, q(got));
};
class OsaUnavailable extends Error {}
/** One osascript call: front the spawned Electron (by unix id); only if it is frontmost: ⌘N. */
function pressNew(pid) {
  const proc = `(first process whose unix id is ${pid})`;
  try {
    const script = [
      'tell application "System Events"',
      `tell ${proc} to set frontmost to true`,
      'delay 0.4',
      `if frontmost of ${proc} then`,
      'keystroke "n" using command down',
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
const udata = join(out, 'agent-udata');
await rm(udata, { recursive: true, force: true });
const port = await freePort();
// No --drive, --open or --fixture: the plain app, as a human starts it (a scratch userData keeps the real one untouched).
const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.'], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
const closed = new Promise(done => child.on('close', done));
let cdp;
try {
  cdp = await attach(port);
  try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch { /* headless window manager */ }
  const state = () => cdp.eval('window.pdf4allState()');
  await wait(async () => typeof (await state())?.revision === 'number', 60_000);
  const before = await state();
  log('before', q(before.doc));
  if (before.doc?.name === 'Untitled.pdf') throw new Error('Untitled.pdf is open before ⌘N');
  pressNew(child.pid);
  const after = await wait(async () => { const v = await state(); return v.doc?.name === 'Untitled.pdf' ? v : null; }, 20_000);
  deq([after.doc.name, after.doc.pageCount, after.pages.count, after.dirty, after.error], ['Untitled.pdf', 1, 1, false, ''], 'File ▸ New… opened a blank Untitled.pdf');
  await wait(() => cdp.eval(`document.title.includes('Untitled')`), 10_000);
  await sleep(800);
  const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' });
  await writeFile(join(out, 'agent-new.png'), Buffer.from(data, 'base64'));
  log('shot', 'agent-new.png', Date.now() - T0);
  log('PASS all');
} catch (err) {
  if (err instanceof OsaUnavailable) { console.error('OSA-UNAVAILABLE', err.message); process.exitCode = 2; }
  else { console.error('FAIL', err?.stack ?? err); process.exitCode = 1; }
} finally {
  cdp?.close();
  child.kill('SIGTERM');
  log('electron closed', await closed);
  log('fixtures after\n' + hashes());
}
