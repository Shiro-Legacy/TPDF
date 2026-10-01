#!/usr/bin/env node
/**
 * Batch GUI evidence (contract-batch.md §4.3 as amended by §9.9, §9.14, §9.18): the batch dialog opened with a REAL macOS ⌥⌘B
 * runs a right-aligned footer "B-{bates}" (4 digits) over a mkdtemp copy of encrypted/hello/nav/rotated.pdf.
 * Launch/session()/Cdp/click()/shot() machinery copied from shot-bates.mjs (itself from shot-direct9.mjs); session() gains
 * `extraArgs` for `--batch-dir=<tmp>/batch-in` (#batch-pick is a native sheet CDP cannot answer; the --drive hold makes it smoke).
 * hello p0: osascript ⌥⌘B (addressed to the spawned Electron by unix id; refused unless it is frontmost) → #batch-dialog open;
 * CDP click #batch-pick → batch.total 4, all queued; CDP sets #batch-kind footer (+change), #batch-text, #batch-align right,
 * #batch-bates-pad 4; CDP click #batch-start; poll pdf4allState() until batch.running false and done 4; gates ok 3, failed 1,
 * names in listing order, #batch-summary text; shot review-out/batch-a-done.png; a second Start (bates field untouched) → all
 * failed (encrypted/exists), batch.bates {6,6,4}, report.txt byte-identical. finally: SIGTERM, fixtures.sha256 9 OK, the four
 * input shas by value (fixtures and the tmp copies), no pdf4all-out under fixtures/.
 * Exit 2 + OSA-UNAVAILABLE when osascript cannot drive System Events (-1712 / 1002); exit 1 + "blocked: frontmost <name>" when
 * another app is frontmost (nothing is typed), or naming the chord when the dialog does not open. Not part of npm test.
 * Run after `npm run build`, under .quad/run.lock: node .quad/shared/p6/checks/shot-batch.mjs
 */
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'node:net';
import { copyFile, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
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
const INPUTS = {
  'encrypted.pdf': '38d533d7aaaf1b3d56a25a98de22f7e6bafff41df301d85ed3ca611f244dcab6',
  'hello.pdf': '229ca9488a139d3f32b864ba24055152fe7f0faf670c9db6491144f23d275b94',
  'nav.pdf': '8e7c37500e46120b9bd774bd8aab8d093d9e21aacc522a66216406373229ed82',
  'rotated.pdf': 'dfa923b6a82d51b670c52b9b8442a2e03fe273dd09958e9f7c30a427191d1554',
};
const ANNOUNCE = 'Batch done: 3 ok, 1 failed, report in pdf4all-out';
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
const sha = async path => createHash('sha256').update(await readFile(path)).digest('hex');
const deq = (got, want, label) => {
  if (JSON.stringify(got) !== JSON.stringify(want)) throw new Error(`${label}: ${q(got)} vs ${q(want)}`);
  log('ok', label, q(got));
};

/** One Electron on one fixture: probe (open + hold), fresh udata, extra argv, SIGTERM + hash checks in finally. */
async function session(tag, fixture, extraArgs, body) {
  const udata = join(out, `batch-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `batch-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `batch-out-${tag}`)}`, `--report=${join(out, `batch-report-${tag}.json`)}`, ...extraArgs],
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
    for (const [name, want] of Object.entries(INPUTS)) deq(await sha(join(root, 'fixtures', name)), want, `fixtures/${name} sha after`);
    deq((await readdir(join(root, 'fixtures'), { recursive: true })).filter(entry => entry.split('/').pop() === 'pdf4all-out'), [], 'no pdf4all-out under fixtures/');
  }
}

class OsaUnavailable extends Error {}
/** One osascript call: bring the spawned Electron (by unix id) to the front, then ⌥⌘B only if it is frontmost. */
function optionCommandB(pid) {
  const proc = `(first process whose unix id is ${pid})`;
  try {
    const script = [
      'tell application "System Events"',
      `tell ${proc} to set frontmost to true`,
      'delay 0.4',
      `if frontmost of ${proc} then`,
      'keystroke "b" using {option down, command down}',
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
// The batch folder is a mkdtemp copy; nothing is ever written next to fixtures/.
const folder = join(await mkdtemp(join(tmpdir(), 'pdf4all-shot-batch-')), 'batch-in');
await mkdir(folder);
for (const name of Object.keys(INPUTS)) await copyFile(join(root, 'fixtures', name), join(folder, name));
try {
  await session('a', 'fixtures/hello.pdf', [`--batch-dir=${folder}`], async ({ state, shot, click, centre, cdp, child }) => {
    optionCommandB(child.pid);
    const opened = await wait(() => cdp.eval(`document.getElementById('batch-dialog').open`), 5_000).catch(() => false);
    if (!opened) throw new Error('⌥⌘B (keystroke "b" using {option down, command down}) did not open #batch-dialog');
    log('ok ⌥⌘B opens #batch-dialog');
    deq(await cdp.eval(`document.getElementById('batch-start').disabled`), true, '#batch-start disabled before a folder');
    const pick = await centre('batch-pick');
    await click(pick.x, pick.y);
    const listed = await wait(async () => { const s = await state(); return s.batch?.total === 4 ? s : null; }, 10_000);
    deq(listed.batch.files.map(row => row.status), ['queued', 'queued', 'queued', 'queued'], 'queued rows');
    deq(await cdp.eval(`document.getElementById('batch-folder').value`), folder, '#batch-folder');
    const set = (id, value) => cdp.eval(`(() => { const e = document.getElementById(${q(id)}); e.value = ${q(value)}; e.dispatchEvent(new Event('change')); return e.value; })()`);
    await set('batch-kind', 'footer');
    await set('batch-text', 'B-{bates}');
    await set('batch-align', 'right');
    await set('batch-bates-start', '1');
    await set('batch-bates-pad', '4');
    const start = await centre('batch-start');
    await click(start.x, start.y);
    const v = await wait(async () => { const s = await state(); return s.batch && !s.batch.running && s.batch.done === 4 ? s : null; }, 60_000);
    deq(v.batch.ok, 3, 'batch.ok');
    deq(v.batch.failed, 1, 'batch.failed');
    deq(v.batch.files.map(row => row.name), ['encrypted.pdf', 'hello.pdf', 'nav.pdf', 'rotated.pdf'], 'listing order');
    deq(v.batch.files.map(row => row.status), ['failed', 'ok', 'ok', 'ok'], 'row statuses');
    deq(v.batch.bates, { start: 1, next: 6, pad: 4 }, 'batch.bates');
    deq(v.batch.report, 'ok', 'batch.report');
    deq(v.revision, 0, 'the open document is untouched (revision)');
    deq(v.dirty, false, 'the open document is untouched (dirty)');
    deq(await cdp.eval(`document.getElementById('batch-summary').textContent`), ANNOUNCE, '#batch-summary');
    deq((await readdir(join(folder, 'pdf4all-out'))).sort(), ['hello.pdf', 'nav.pdf', 'report.txt', 'rotated.pdf'], 'pdf4all-out');
    await sleep(500);
    await shot('batch-a-done.png');
    // §9.13 second Start into the same folder, #batch-bates-start untouched (runBatch's finally refreshes it from batesNext).
    const report1 = await readFile(join(folder, 'pdf4all-out', 'report.txt'));
    deq(await cdp.eval(`document.getElementById('batch-bates-start').value`), '6', '#batch-bates-start after the first run');
    await click(start.x, start.y);
    const w = await wait(async () => { const s = await state(); return s.batch && !s.batch.running && s.batch.report !== 'ok' && s.batch.report !== null ? s : null; }, 60_000);
    deq(w.batch.files.map(row => [row.status, row.detail]), [['failed', 'encrypted'], ['failed', 'exists'], ['failed', 'exists'], ['failed', 'exists']], 'second run rows');
    deq([w.batch.ok, w.batch.failed, w.batch.report], [0, 4, 'exists'], 'second run ok/failed/report');
    deq(w.batch.bates, { start: 6, next: 6, pad: 4 }, 'second run batch.bates');
    deq(await cdp.eval(`document.getElementById('batch-bates-start').value`), '6', '#batch-bates-start after the second run');
    deq(await cdp.eval(`document.getElementById('batch-summary').textContent`), 'Batch done: 0 ok, 4 failed; report not written: report.txt exists', 'second run #batch-summary');
    deq((await readFile(join(folder, 'pdf4all-out', 'report.txt'))).equals(report1), true, 'report.txt byte-identical to the first run');
  });
  for (const [name, want] of Object.entries(INPUTS)) deq(await sha(join(folder, name)), want, `tmp copy ${name} sha after`);
  log('PASS all');
} catch (err) {
  if (err instanceof OsaUnavailable) { console.error('OSA-UNAVAILABLE', err.message); process.exitCode = 2; }
  else { console.error('FAIL', err?.stack ?? err); process.exitCode = 1; }
}
