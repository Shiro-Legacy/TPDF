#!/usr/bin/env node
/**
 * P5 U3 GUI evidence (contract-p5-unit3.md §4.3 as amended by §9.33, §9.34, §9.36): real CDP input, one Electron per document.
 * First runs scripts/drive.mjs tests/drive/sign-field.jsonl --out=review-out/u3-drive-out (must exit 0) for the signed file.
 * empty-sigfield.pdf in Forms: real click on "Draw signature field" + real drag → a new .sig-target (shot); real click on
 * Signature1 → #cert-sign-dialog with the #cert-sign-field line (shot, then Escape). field-signed2.pdf from the drive run:
 * "Signed" badges (shot); real click on Verify → "✓ Valid, untrusted" badges (shot). Screenshots → review-out/u3-*.png.
 * Trust store: no trusted-certs.pem under any u3-udata-*, the user's store mtime (or absence) unchanged.
 * Not part of npm test. Run after `npm run build`, under .quad/run.lock: node .quad/shared/p5/checks/shot-sign-u3.mjs
 */
import { spawn, execFileSync } from 'node:child_process';
import { existsSync, readdirSync, statSync } from 'node:fs';
import { createServer } from 'node:net';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { isAbsolute, join } from 'node:path';

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
const hashes = () => execFileSync('shasum', ['-a', '256', '-c', join(checksDir, '../../p6/checks/fixtures.sha256')], { cwd: root, encoding: 'utf8' });
const KEYS = { Enter: 13, Tab: 9, Escape: 27, ArrowRight: 39, ArrowDown: 40 };

/** One Electron on one fixture: probe (open + hold), fresh udata, SIGTERM + hash check in finally. */
async function session(tag, fixture, body) {
  const udata = join(out, `u3-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(out, `u3-probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: isAbsolute(fixture) ? fixture : join(root, fixture) }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(out, `u3-out-${tag}`)}`, `--report=${join(out, `u3-report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
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
    // Debt1 row 16c: a real CDP click on the mode button's rect, not a synthetic .click().
    const mode = async name => {
      const r = await wait(() => rectOf(`document.querySelector('label[for="mode-${name}"]')`));
      await click(r.cx, r.cy);
      await wait(async () => (await state()).mode === name);
    };
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

const trustStore = join(process.env.HOME ?? '', 'Library/Application Support/pdf4all-p0/trusted-certs.pem');
const mtime = path => existsSync(path) ? statSync(path).mtimeMs : null;
/** Debt1 row 16c: how many of `sel` intersect the viewport (the shots only show those). */
const inView = sel => `(() => { const v = document.getElementById('viewport').getBoundingClientRect(); return [...document.querySelectorAll(${q(sel)})].filter(e => { const r = e.getBoundingClientRect(); return r.bottom > v.top && r.top < v.bottom && r.right > v.left && r.left < v.right; }).length; })()`;
const caption = sel => `(() => [...document.querySelectorAll(${q(sel)})].map(e => getComputedStyle(e, '::after').content))()`;

log('fixtures before\n' + hashes());
await mkdir(out, { recursive: true });
const trustBefore = mtime(trustStore);
try {
  // §9.33 the signed file comes from this script's own drive run.
  const driveOut = join(out, 'u3-drive-out');
  await rm(driveOut, { recursive: true, force: true });
  execFileSync(process.execPath, ['scripts/drive.mjs', 'tests/drive/sign-field.jsonl', `--out=${driveOut}`, `--report=${join(out, 'u3-drive-report.json')}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  log('ok drive sign-field.jsonl exit 0', Date.now() - T0);

  // A. empty-sigfield.pdf in Forms: draw a field with a real drag, then a real click on Signature1.
  await session('draw', 'fixtures/p5/empty-sigfield.pdf', async ({ state, shot, click, drag, key, pagePoint, rectOf, mode, cdp }) => {
    await mode('forms');
    await wait(async () => (await state()).signatures?.length === 2, 20_000);
    const arm = await wait(() => rectOf(`document.getElementById('sigfield-draw')`));
    await click(arm.cx, arm.cy);
    await wait(() => cdp.eval(`document.getElementById('sigfield-draw').getAttribute('aria-pressed') === 'true'`));
    const start = await pagePoint(72, 300);
    await drag(start.x, start.y, 220 * start.s, 60 * start.s);
    const s = await wait(async () => { const v = await state(); return v.signatures?.length === 3 ? v : null; });
    if (s.signatures[2].field !== 'Signature4' || s.signatures[2].status !== 'empty') throw new Error(`A drawn field: ${q(s.signatures[2])}`);
    near(s.signatures[2].rect, [72, 300, 292, 360], 1, 'A drawn Signature4');
    const targets = await cdp.eval(`document.querySelectorAll('#hit-layer .sig-target').length`);
    if (targets !== 3) throw new Error(`A .sig-target count ${targets}`);
    log('ok A captions', q(await cdp.eval(caption('#hit-layer .sig-target'))));
    await shot('u3-a-drawn-field.png');
    const s1 = await pagePoint(206, 650);
    await click(s1.x, s1.y);
    const dialog = await wait(async () => { const v = await state(); return v.sign?.dialog ? v.sign.dialog : null; });
    if (dialog.field !== 'Signature1' || dialog.page !== 0) throw new Error(`B dialog: ${q(dialog)}`);
    const line = await cdp.eval(`(() => { const e = document.getElementById('cert-sign-field'); return e && !e.hidden && document.getElementById('cert-sign-dialog').open ? e.textContent : null; })()`);
    if (!line || !line.includes('Signature1')) throw new Error(`B #cert-sign-field: ${q(line)}`);
    log('ok B field line', q(line));
    await shot('u3-b-sign-dialog.png');
    await key('Escape');
    await wait(async () => (await state()).sign?.dialog === null && !(await cdp.eval(`document.getElementById('cert-sign-dialog').open`)));
    log('ok B Escape closed the dialog');
  });

  // B. field-signed2.pdf: "Signed" badges, then a real click on Verify → "✓ Valid, untrusted".
  await session('signed', join(driveOut, 'field-signed2.pdf'), async ({ state, shot, click, key, rectOf, cdp }) => {
    await wait(async () => (await state()).signatures?.length === 3, 20_000);
    const statuses = v => v.signatures.map(item => `${item.field}:${item.status}`).join(' ');
    let v = await state();
    if (statuses(v) !== 'Signature1:signed Signature2:empty Signature4:signed') throw new Error(`C statuses: ${statuses(v)}`);
    const signedCaps = await cdp.eval(caption('#hit-layer .sig-target[data-signed="true"]'));
    if (signedCaps.length !== 2 || !signedCaps.every(c => c.includes('Signed'))) throw new Error(`C captions: ${q(signedCaps)}`);
    log('ok C captions', q(signedCaps));
    log('C badges in view', await cdp.eval(inView('#hit-layer .sig-target[data-signed="true"]')), 'of', signedCaps.length);
    await shot('u3-c-signed-badges.png');
    const verify = await wait(() => rectOf(`document.getElementById('verify-open')`));
    await click(verify.cx, verify.cy);
    v = await wait(async () => { const s = await state(); return s.signatures?.[0]?.status === 'valid' && s.signatures[2]?.status === 'valid' ? s : null; }, 30_000);
    await sleep(500);
    await shot('u3-d-verify-dialog.png');
    if (await cdp.eval(`document.getElementById('verify-dialog')?.open === true`)) await key('Escape');
    await sleep(500);
    const validCaps = await cdp.eval(caption('#hit-layer .sig-target.valid.untrusted'));
    if (validCaps.length !== 2 || !validCaps.every(c => c.includes('Valid, untrusted'))) throw new Error(`D captions: ${q(validCaps)}`);
    log('ok D captions', q(validCaps), statuses(v));
    log('D badges in view', await cdp.eval(inView('#hit-layer .sig-target.valid')), 'of', validCaps.length);
    await shot('u3-d-valid-badges.png');
  });

  // §9.36 trust store untouched.
  const stray = readdirSync(out).filter(name => name.startsWith('u3-udata-')).filter(name => existsSync(join(out, name, 'trusted-certs.pem')));
  if (stray.length) throw new Error(`trust store written under ${stray.join(', ')}`);
  if (mtime(trustStore) !== trustBefore) throw new Error(`user trust store changed: ${trustBefore} → ${mtime(trustStore)}`);
  log('ok trust store', trustBefore === null ? 'absent' : 'unchanged');
  log('PASS all');
} catch (err) {
  console.error('FAIL', err?.stack ?? err);
  process.exitCode = 1;
}
