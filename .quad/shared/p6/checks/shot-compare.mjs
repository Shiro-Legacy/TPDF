#!/usr/bin/env node
/**
 * Compare GUI evidence (contract-compare.md §3.3 as amended by §9.10): File ▸ Compare with… by the REAL ⌥⌘K chord.
 * Launch/session()/Cdp/click()/shot() machinery copied from shot-direct9.mjs. Mechanism for the file pick: the spawned
 * Electron runs under a probe `--drive` with `--insert-from=<ABSOLUTE fixtures/nav.pdf>`, so `insert-source` answers from
 * the pre-answer and no native dialog opens. hello.pdf is A; osascript `key code 40 using {option down, command down}`
 * (addressed to the spawned Electron by unix id, sent only if it is frontmost in the same call) → gates compare.open,
 * name nav.pdf, revision 0, #reader hidden, #compare-view shown, A left of B, pixels.ratio > 0 → real CDP click at
 * #compare-show-diff's centre → showDiff true, overlay displayed → shot review-out/compare-a-side-by-side.png → focus
 * #compare-page-b, Input.insertText "2", Tab → pageB 1 (1-based field, §9.12; "3" would clamp to 2 and hide a 0-based slip) → Escape in #compare-view → open false,
 * #reader shown (§9.14) → ⌥⌘K again → CDP click #compare-close → open false, #reader shown. Every click hit-tests with
 * elementFromPoint first and logs FALLBACK + element.click() when occluded; waits gate on #status/STATE, never sleeps.
 * The fixture hash check runs in finally (exit 1 unless 9 OK). Exit 2 + OSA-UNAVAILABLE when osascript cannot drive System Events
 * (-1712 / 1002); exit 1 + "blocked: frontmost <name>" when another app is frontmost (nothing is typed).
 * Not part of npm test. Run after `npm run build`, under .quad/run.lock: node .quad/shared/p6/checks/shot-compare.mjs
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
const hashes = () => execFileSync('shasum', ['-a', '256', '-c', join(checksDir, 'fixtures.sha256')], { cwd: root, encoding: 'utf8' });
const deq = (got, want, label) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`${label}: ${q(got)} vs ${q(want)}`);
  log('ok', label, q(got));
};

/** One Electron on one fixture: probe (open + hold), fresh udata, SIGTERM + hash check in finally. */
async function session(tag, fixture, extra, body) {
  const udata = join(out, `compare-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `compare-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `compare-out-${tag}`)}`, `--report=${join(out, `compare-report-${tag}.json`)}`, ...extra],
  { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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
    const mouse = (type, x, y, extra2 = {}) => cdp.send('Input.dispatchMouseEvent', { type, x, y, ...extra2 });
    const click = async (x, y) => {
      await mouse('mouseMoved', x, y); await sleep(100);
      await mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount: 1 });
      await mouse('mouseReleased', x, y, { button: 'left', buttons: 0, clickCount: 1 });
    };
    /** The element's box centre in client px after scrolling it into view, hit-tested with elementFromPoint (shot-bates fitHit). */
    const centre = async id => {
      const c = await cdp.eval(`(() => { const b = document.getElementById(${q(id)}); b.scrollIntoView({ block: 'center' });
        const r = b.getBoundingClientRect(); const e = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
        return { id: ${q(id)}, x: r.left + r.width / 2, y: r.top + r.height / 2, hit: e === b || b.contains(e), over: e ? (e.id || e.tagName) : null }; })()`);
      log('hit', q(c));
      return c;
    };
    /** Real CDP click when the element is what the pointer hits; otherwise FALLBACK element.click(), logged. */
    const press = async c => {
      if (c.hit) return click(c.x, c.y);
      log(`FALLBACK ${c.id}: occluded by ${c.over} at (${Math.round(c.x)},${Math.round(c.y)}) → element.click()`);
      await cdp.eval(`document.getElementById(${q(c.id)}).click()`);
    };
    const key = async (key, code, keyCode) => {
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode });
    };
    const box = async id => cdp.eval(`(() => { const e = document.getElementById(${q(id)}); const r = e.getBoundingClientRect();
      return { left: r.left, right: r.right, width: r.width, height: r.height, shown: !e.hidden && getComputedStyle(e).display !== 'none' }; })()`);
    await wait(() => cdp.eval(`document.title.includes(${q(fixture.split('/').pop().replace('.pdf', ''))})`), 60_000);
    await wait(async () => /rendered in \d+ ms/.test(await cdp.eval(`document.getElementById('status').textContent`)), 20_000);
    await body({ state, shot, press, centre, box, key, cdp, child });
    log('PASS', tag);
  } finally {
    cdp?.close();
    child.kill('SIGTERM');
    log('electron closed', tag, await closed);
  }
}

class OsaUnavailable extends Error {}
class Blocked extends Error {}
/** One osascript call: bring the spawned Electron (by unix id) to the front, then ⌥⌘K (k = key code 40) only if it is frontmost. */
function optionCommandK(pid) {
  const proc = `(first process whose unix id is ${pid})`;
  try {
    const script = [
      'tell application "System Events"',
      `tell ${proc} to set frontmost to true`,
      'delay 0.4',
      `if frontmost of ${proc} then`,
      'key code 40 using {option down, command down}',
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
    if (front !== undefined) throw new Blocked(`blocked: frontmost ${front}`);
    throw new Error(`osascript: ${text.trim()}`);
  }
}

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
const CHORD = '⌥⌘K (key code 40 using {option down, command down})';
/** Sends ⌥⌘K and waits for compare.open with pixels; names the chord when it does not open. */
async function chordOpens(state, pid) {
  optionCommandK(pid);
  return wait(async () => { const v = await state(); return v.compare.open && v.compare.pixels ? v : null; }, 20_000)
    .catch(() => { throw new Error(`${CHORD} did not open compare: compare.open false`); });
}
try {
  await session('a', 'fixtures/hello.pdf', [`--insert-from=${join(root, 'fixtures/nav.pdf')}`], async ({ state, shot, press, centre, box, key, cdp, child }) => {
    deq((await state()).compare.open, false, 'compare starts closed');
    const opened = await chordOpens(state, child.pid);
    deq(opened.compare.name, 'nav.pdf', 'compare name');
    deq(opened.revision, 0, 'revision untouched');
    deq(opened.compare.showDiff, false, 'overlay starts hidden');
    deq((await box('reader')).shown, false, '#reader hidden');
    deq((await box('compare-view')).shown, true, '#compare-view shown');
    const a = await box('compare-canvas-a'), b = await box('compare-canvas-b');
    if (!(a.width > 0 && b.width > 0 && a.right <= b.left)) throw new Error(`canvas geometry: A ${q(a)} B ${q(b)}`);
    log('ok side by side', q({ a, b }));
    if (!(opened.compare.pixels.ratio > 0)) throw new Error(`pixels.ratio ${q(opened.compare.pixels)}`);
    log('ok pixels', q(opened.compare.pixels));
    await press(await centre('compare-show-diff'));
    const diff = await wait(async () => { const v = await state(); return v.compare.showDiff ? v : null; }, 5_000);
    deq(diff.compare.showDiff, true, 'CDP click shows the differences');
    deq((await box('compare-overlay')).shown, true, '#compare-overlay displayed');
    await shot('compare-a-side-by-side.png');
    // §9.12: #compare-page-b is 1-based — typing 2 and tabbing away sets pageB 1 (not 3: nav has 3 pages, so a
    // 0-based slip would clamp 3 to 2 and still pass).
    await cdp.eval(`(() => { const e = document.getElementById('compare-page-b'); e.focus(); e.select(); })()`);
    await cdp.send('Input.insertText', { text: '2' });
    await key('Tab', 'Tab', 9);
    // pageB moves at once; the text diff follows after the refresh, so gate on both.
    const paged = await wait(async () => { const v = await state(); return v.compare.pageB === 1 && q(v.compare.text?.addedLines) === q(['two']) ? v : null; }, 5_000)
      .catch(async () => { const c = (await state()).compare; throw new Error(`#compare-page-b "2" + Tab: pageB ${q(c.pageB)} addedLines ${q(c.text?.addedLines)}, want 1 ["two"]`); });
    deq([paged.compare.pageA, paged.compare.pageB, paged.compare.text.addedLines], [0, 1, ['two']], '#compare-page-b 2 → pageB 1');
    // §9.14: Escape in #compare-view leaves compare.
    await cdp.eval(`document.getElementById('compare-view').focus()`);
    await key('Escape', 'Escape', 27);
    const escaped = await wait(async () => { const v = await state(); return v.compare.open === false ? v : null; }, 5_000)
      .catch(() => { throw new Error('Escape in #compare-view did not close compare'); });
    deq(escaped.compare.last, { action: 'close', ok: true }, 'Escape closes');
    deq((await box('reader')).shown, true, '#reader shown after Escape');
    await chordOpens(state, child.pid);
    await press(await centre('compare-close'));
    const closed = await wait(async () => { const v = await state(); return v.compare.open === false ? v : null; }, 5_000);
    deq(closed.compare.last, { action: 'close', ok: true }, 'close recorded');
    deq((await box('reader')).shown, true, '#reader shown again');
    deq(closed.revision, 0, 'revision still 0');
  });
  log('PASS all');
} catch (err) {
  if (err instanceof OsaUnavailable) { console.error('OSA-UNAVAILABLE', err.message); process.exitCode = 2; }
  else if (err instanceof Blocked) { console.error(err.message); process.exitCode = 1; }
  else { console.error('FAIL', err?.stack ?? err); process.exitCode = 1; }
} finally {
  let check;
  try { check = hashes(); } catch (err) { check = String(err.stdout ?? err.message); }
  log('fixtures after\n' + check);
  const okCount = (check.match(/: OK$/gm) ?? []).length;
  if (okCount !== 9) { console.error(`FAIL fixtures.sha256: ${okCount} OK, want 9`); process.exitCode = 1; }
}
