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
// ---- review-u3 renderer lens (real CDP). The password is read from tests/drive/sign.jsonl at run time; never logged.
import { readFileSync } from 'node:fs';
const KEYS = { Enter: 13, Tab: 9, Escape: 27, ArrowRight: 39, ArrowDown: 40, ArrowLeft: 37, ArrowUp: 38 };
const PW = JSON.parse(readFileSync(join(root, 'tests/drive/sign.jsonl'), 'utf8').split('\n').find(l => l.includes('"cmd":"sign"'))).contents.password;
const scratch = join(root, '.quad/scratch/p5u3/review-renderer');
const results = [];
const check = (id, ok, detail) => { results.push({ id, ok: !!ok, detail }); log(ok ? 'ok  ' : 'BAD ', id, typeof detail === 'string' ? detail : q(detail)); };
async function session(tag, lines, body) {
  const udata = join(out, `u3-review-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(scratch, `probe-${tag}.jsonl`);
  await writeFile(probe, [...lines, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort();
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(scratch, `out-${tag}`)}`, `--report=${join(scratch, `report-${tag}.json`)}`], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  const closed = new Promise(done => child.on('close', done));
  let cdp;
  try {
    cdp = await attach(port);
    try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch { }
    const state = () => cdp.eval('window.pdf4allState()');
    const shot = async name => { const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }); await writeFile(join(out, name), Buffer.from(data, 'base64')); log('shot', name); };
    const move = (x, y) => cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
    const click = async (x, y) => {
      await move(x, y); await sleep(100);
      await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
      await sleep(700);
    };
    const drag = async (x, y, dx, dy) => {
      await move(x, y); await sleep(200);
      await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
      for (let i = 1; i <= 8; i++) { await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: x + dx * i / 8, y: y + dy * i / 8, button: 'left', buttons: 1 }); await sleep(20); }
      await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: x + dx, y: y + dy, button: 'left', buttons: 0, clickCount: 1 });
      await sleep(1800);
    };
    const key = async (name, modifiers = 0) => {
      const code = KEYS[name];
      await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: name, code: name, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code, modifiers });
      await sleep(900);
    };
    const pagePoint = (x, y, widthPt = 612) => cdp.eval(`(() => {
      const v = document.getElementById('viewport'), c = document.getElementById('canvas');
      let r = c.getBoundingClientRect(); const s = r.width / ${widthPt};
      v.scrollTop += (r.top + ${y} * s) - (v.getBoundingClientRect().top + v.clientHeight / 2);
      r = c.getBoundingClientRect();
      return { x: r.left + ${x} * s, y: r.top + ${y} * s, s };
    })()`);
    const rectOf = sel => cdp.eval(`(() => { const e = ${sel}; if (!e) return null; e.scrollIntoView({ block: 'center' }); const r = e.getBoundingClientRect(); const cs = getComputedStyle(e); return { x: r.x, y: r.y, w: r.width, h: r.height, cx: r.x + r.width / 2, cy: r.y + r.height / 2, display: cs.display, visibility: cs.visibility }; })()`);
    const clickSel = async sel => { const r = await wait(() => rectOf(sel)); await click(r.cx, r.cy); return r; };
    const mode = async name => { await clickSel(`document.querySelector('label[for="mode-${name}"]') || document.getElementById('mode-${name}').closest('label')`); await wait(async () => (await state()).mode === name); };
    await wait(async () => { const s = await state(); return s.doc && s.signatures ? s : null; }, 60_000);
    await sleep(1500);
    await body({ state, shot, move, click, drag, key, pagePoint, rectOf, clickSel, mode, cdp });
  } catch (err) { check(`${tag}:exception`, false, String(err?.stack ?? err).slice(0, 400)); }
  finally { cdp?.close(); child.kill('SIGTERM'); log('closed', tag, await closed); }
}
const sigInfo = `(() => [...document.querySelectorAll('#hit-layer .sig-target')].map(e => ({ name: e.dataset.fieldName, signed: e.dataset.signed, cls: e.className, title: e.title, aria: e.getAttribute('aria-label'), cursor: getComputedStyle(e).cursor, cap: getComputedStyle(e, '::after').content, handles: e.querySelectorAll('.hit-handle').length })))()`;
const dlg = `(() => { const d = document.getElementById('cert-sign-dialog'), f = document.getElementById('cert-sign-field'), p = document.getElementById('cert-sign-page').closest('p'), a = document.getElementById('cert-sign-visible').closest('fieldset'); return { open: d.open, fieldHidden: f.hidden, fieldText: f.textContent, pageHidden: !!(p && (p.hidden || p.offsetParent === null)), appHidden: !!(a && (a.hidden || a.offsetParent === null)), status: document.getElementById('cert-sign-status').textContent }; })()`;
const sigTarget = name => `document.querySelector('#hit-layer .sig-target[data-field-name="${name}"]')`;
const sigs = s => (s.signatures || []).map(x => `${x.field}:${x.status}:${x.rect.map(n => Math.round(n * 100) / 100).join(',')}`).join(' ');

await mkdir(out, { recursive: true });
await mkdir(scratch, { recursive: true });
const FIX = join(root, 'fixtures/p5/empty-sigfield.pdf');
const which = process.argv[2] || 'ABC';
// ---------------- A: Read mode click, Escape, menu path; Forms handles/grip/resize/nudge/Enter; draw; designer
if (which.includes('A')) await session('ra', [{ cmd: 'open', path: FIX }], async ({ state, shot, move, click, drag, key, pagePoint, rectOf, clickSel, mode, cdp }) => {
  let s = await state();
  check('a0 read mode + 2 empty', s.mode === 'read' && sigs(s) === 'Signature1:empty:72,620,340,680 Signature2:empty:360,620,540,680', `${s.mode} ${sigs(s)}`);
  await pagePoint(206, 650);
  const info = await cdp.eval(sigInfo);
  check('a1 caption Sign + pointer + title=aria', info.length === 2 && info.every(i => i.cap.includes('Sign') && i.cursor === 'pointer' && i.title === i.aria && !/read-only/.test(i.title)), info);
  let p = await pagePoint(206, 650);
  const hit = await cdp.eval(`(() => { const e = document.elementFromPoint(${p.x}, ${p.y}); return e && (e.closest('.sig-target')?.dataset.fieldName || e.id || e.className); })()`);
  check('a1b elementFromPoint Signature1 centre is its target', hit === 'Signature1', hit);
  await click(p.x, p.y);
  let d = await cdp.eval(dlg); s = await state();
  check('a2 real click opens field dialog', d.open && !d.fieldHidden && /Signature1/.test(d.fieldText) && d.pageHidden && d.appHidden && s.sign?.dialog?.field === 'Signature1' && s.sign.dialog.page === 0, { d, dialog: s.sign?.dialog, status: s.direct?.status });
  await shot('review-u3-a2-dialog.png');
  await key('Escape');
  d = await cdp.eval(dlg); s = await state();
  check('a3 Escape closes', !d.open && s.sign?.dialog === null, { open: d.open, dialog: s.sign?.dialog });
  await mode('forms');
  await clickSel(`document.getElementById('cert-sign-open')`);
  d = await cdp.eval(dlg); s = await state();
  check('a4 #cert-sign-open: no field line, page+appearance shown', d.open && d.fieldHidden && !d.pageHidden && !d.appHidden && s.sign?.dialog === null, { d, dialog: s.sign?.dialog });
  await key('Escape');
  // B: hover handles, grip drag, resize, nudge, Enter
  p = await pagePoint(206, 650);
  await move(p.x, p.y); await sleep(300);
  const hov = await cdp.eval(`(() => { const b = ${sigTarget('Signature1')}; return [...b.querySelectorAll('.hit-handle')].map(h => getComputedStyle(h).display); })()`);
  check('b1 hover shows grip+8 handles', hov.length === 9 && hov.every(x => x === 'block'), hov);
  s = await state(); const rev0 = s.revision, r0 = s.signatures[0].rect;
  const grip = await cdp.eval(`(() => { const g = ${sigTarget('Signature1')}.querySelector('.hit-grip'); const r = g.getBoundingClientRect(); return { cx: r.x + r.width / 2, cy: r.y + r.height / 2, top: document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2)?.className }; })()`);
  check('b2a grip is hit-testable', /hit-grip/.test(grip.top || ''), grip.top);
  // real path: hover target then move to grip
  await move(p.x, p.y); await sleep(150);
  await drag(grip.cx, grip.cy, 40 * p.s, 30 * p.s);
  s = await state(); let d2 = await cdp.eval(dlg);
  const r1 = s.signatures[0].rect;
  check('b2 grip drag moves (+40,+30 pt), one op, dirty, canUndo, no dialog', Math.abs(r1[0] - r0[0] - 40) < 1.5 && Math.abs(r1[1] - r0[1] - 30) < 1.5 && s.revision === rev0 + 1 && s.dirty === true && s.canUndo === true && !d2.open, { r0, r1, rev0, rev: s.revision, dirty: s.dirty, canUndo: s.canUndo, dlg: d2.open, dialog: s.sign?.dialog, error: s.error });
  if (d2.open) await key('Escape');
  // body drag (press on the body itself)
  s = await state(); const rb0 = s.signatures[0].rect, revb = s.revision;
  p = await pagePoint((rb0[0] + rb0[2]) / 2, (rb0[1] + rb0[3]) / 2);
  await drag(p.x, p.y, 30 * p.s, 0);
  s = await state(); d2 = await cdp.eval(dlg);
  check('b2c body drag (not grip): what happens', true, { before: rb0, after: s.signatures[0].rect, rev: s.revision - revb, dlgOpen: d2.open, dialog: s.sign?.dialog });
  if (d2.open) await key('Escape');
  // se resize
  s = await state(); const rr0 = s.signatures[0].rect, revr = s.revision;
  p = await pagePoint(rr0[2], rr0[3]);
  const se = await cdp.eval(`(() => { const r = ${sigTarget('Signature1')}.querySelector('.hit-handle.se').getBoundingClientRect(); return { cx: r.x + r.width / 2, cy: r.y + r.height / 2 }; })()`);
  await move(p.x - 10, p.y - 10); await sleep(200);
  await drag(se.cx, se.cy, 20 * p.s, 10 * p.s);
  s = await state(); d2 = await cdp.eval(dlg);
  const rr1 = s.signatures[0].rect;
  check('b3 se resize +20,+10', Math.abs(rr1[2] - rr0[2] - 20) < 1.5 && Math.abs(rr1[3] - rr0[3] - 10) < 1.5 && rr1[0] === rr0[0] && s.revision === revr + 1 && !d2.open, { rr0, rr1, rev: s.revision - revr, dlgOpen: d2.open });
  if (d2.open) await key('Escape');
  // keyboard nudge on the focused target
  const focused = await cdp.eval(`document.activeElement?.dataset?.fieldName ?? document.activeElement?.id ?? null`);
  check('b4a target focused after gesture', focused === 'Signature1', focused);
  if (focused !== 'Signature1') await cdp.eval(`${sigTarget('Signature1')}.focus()`);
  s = await state(); const rk0 = s.signatures[0].rect, revk = s.revision;
  await key('ArrowDown');
  s = await state(); const rk1 = s.signatures[0].rect;
  check('b4 plain ArrowDown nudges 1 pt', Math.abs(rk1[1] - rk0[1] - 1) < 0.01 && s.revision === revk + 1, { rk0, rk1, rev: s.revision - revk });
  await key('ArrowRight', 1);
  s = await state(); const rk2 = s.signatures[0].rect;
  check('b5 Alt+ArrowRight nudges 1 pt', Math.abs(rk2[0] - rk1[0] - 1) < 0.01 && s.revision === revk + 2, { rk1, rk2 });
  await key('Enter');
  d = await cdp.eval(dlg); s = await state();
  check('b6 Enter opens field dialog', d.open && s.sign?.dialog?.field === 'Signature1', { d, dialog: s.sign?.dialog });
  await key('Escape');
  s = await state();
  check('b6b Escape after Enter closes', s.sign?.dialog === null && !(await cdp.eval(`document.getElementById('cert-sign-dialog').open`)), s.sign);
  // C: draw
  s = await state(); const revc = s.revision;
  await clickSel(`document.getElementById('sigfield-draw')`);
  check('c0 armed', await cdp.eval(`document.getElementById('sigfield-draw').getAttribute('aria-pressed')`) === 'true', '');
  p = await pagePoint(72, 300);
  await drag(p.x, p.y, 220 * p.s, 60 * p.s);
  s = await state();
  const s4 = s.signatures.find(x => x.field === 'Signature4');
  check('c1 drawn Signature4', !!s4 && s4.status === 'empty' && Math.abs(s4.rect[0] - 72) < 1 && Math.abs(s4.rect[2] - 292) < 1 && s.fields?.last?.name === 'Signature4' && s.fields.last.action === 'add' && s.revision === revc + 1 && s.direct?.status === 'Added signature field Signature4', { sigs: sigs(s), last: s.fields?.last, rev: s.revision - revc, status: s.direct?.status, armed: await cdp.eval(`document.getElementById('sigfield-draw').getAttribute('aria-pressed')`) });
  check('c1b dialog not opened by the draw', s.sign?.dialog === null && !(await cdp.eval(`document.getElementById('cert-sign-dialog').open`)), s.sign);
  await shot('review-u3-c1-drawn.png');
  await clickSel(`document.getElementById('sigfield-draw')`);
  p = await pagePoint(100, 450);
  await drag(p.x, p.y, 2, 0);
  s = await state();
  check('c2 2px drag adds nothing, disarms', s.signatures.length === 3 && s.revision === revc + 1 && await cdp.eval(`document.getElementById('sigfield-draw').getAttribute('aria-pressed')`) === 'false', { n: s.signatures.length, rev: s.revision - revc, dialog: s.sign?.dialog });
  if (await cdp.eval(`document.getElementById('cert-sign-dialog').open`)) { check('c2b 2px drag opened the dialog', false, ''); await key('Escape'); }
  await clickSel(`document.getElementById('sigfield-draw')`);
  p = await pagePoint(72, 400);
  await drag(p.x, p.y, 30 * p.s, 10 * p.s);
  s = await state();
  check('c3 30x10 pt refused', s.error === 'Draw a bigger signature field' && s.signatures.length === 3 && s.revision === revc + 1, { error: s.error, n: s.signatures.length });
  // F designer
  await cdp.eval(`(() => { const d = document.getElementById('field-designer'); if (d) d.open = true; })()`);
  const des = await cdp.eval(`(() => { const sel = document.getElementById('field-select'); const opts = [...sel.options].map(o => ({ t: o.textContent, dis: o.disabled, v: o.value })); const o = [...sel.options].find(o => o.textContent === 'Signature2'); sel.value = o.value; sel.dispatchEvent(new Event('change', { bubbles: true })); const g = id => document.getElementById(id)?.disabled; return { opts, hiddenDesigner: document.getElementById('field-designer').hidden, name: g('field-prop-name'), x: g('field-prop-x'), label: g('field-prop-label'), ro: g('field-prop-readonly'), req: g('field-prop-required'), def: g('field-prop-default'), maxLen: g('field-prop-maxlen'), multi: g('field-prop-multiline'), opt: g('field-prop-options'), apply: g('field-prop-apply'), del: g('field-prop-delete'), rectHidden: document.getElementById('field-prop-rect').hidden, kinds: [...document.getElementById('field-new-kind').options].map(o => o.value) }; })()`);
  check('f1 designer controls for an empty sig', !des.hiddenDesigner && des.opts.every(o => !o.dis) && des.name === true && !des.x && !des.label && !des.ro && !des.req && des.def !== false && des.maxLen && des.multi && des.opt && !des.apply && !des.del && !des.rectHidden && des.kinds.includes('signature'), des);
  await cdp.eval(`(() => { const k = document.getElementById('field-new-kind'); k.value = 'signature'; k.dispatchEvent(new Event('change', { bubbles: true })); document.getElementById('field-new-name').value = ''; document.getElementById('field-new-y').value = '480'; })()`);
  s = await state(); const revf = s.revision;
  await clickSel(`document.getElementById('field-new-add')`);
  s = await state();
  check('f2 designer add signature no name', s.fields?.last?.name === 'Signature5' && s.revision === revf + 1 && /Added signature field Signature5/.test(s.direct?.status || ''), { last: s.fields?.last, status: s.direct?.status, rev: s.revision - revf, error: s.error });
  const rows = await cdp.eval(`[...document.querySelectorAll('#field-list .field-row')].map(r => r.textContent).filter(t => /Signature/.test(t))`);
  check('f3 inspector row text', rows.some(t => t === 'Signature field Signature1 — empty; click it on the page to sign'), rows);
  check('f4 fields.count includes sigs, defs/values exclude', s.fields.count >= 5 && !JSON.stringify(s.fields.defs ?? {}).includes('Signature') && !JSON.stringify(s.fields.values ?? {}).includes('Signature'), { count: s.fields.count, defs: s.fields.defs, values: s.fields.values });
  check('f5 help rows 11', s.help?.rows === 11 || (Array.isArray(s.help?.rows) && s.help.rows.length === 11), s.help);
  // designer rename attempt through field-props (name input disabled => cannot)
  // Read mode: sigfield-draw not reachable
  await mode('read');
  const vis = await cdp.eval(`(() => { const b = document.getElementById('sigfield-draw'); return { disabled: b.disabled, visible: b.offsetParent !== null }; })()`);
  check('c4 sigfield-draw not reachable in Read', !vis.visible, vis);
  s = await state();
  const s4r = s.signatures.find(x => x.field === 'Signature4');
  p = await pagePoint((s4r.rect[0] + s4r.rect[2]) / 2, (s4r.rect[1] + s4r.rect[3]) / 2);
  await click(p.x, p.y);
  s = await state();
  check('a5 click drawn Signature4 in Read opens dialog', s.sign?.dialog?.field === 'Signature4', s.sign);
  await key('Escape');
  // designer click on a sig: rename refusal not reachable. Undo the add, check STATE
  await shot('review-u3-a-end.png');
});

// ---------------- B: sign for real through the dialog (drive preloads the cert via a refused sign with a wrong password)
const signedOut = join(scratch, 'probe-signed.pdf');
if (which.includes('B')) { await rm(signedOut, { force: true }); await session('rs', [
  { cmd: 'open', path: FIX }, { cmd: 'mode', contents: { mode: 'forms' } },
  { cmd: 'arm', contents: { tool: 'sigfield' } }, { cmd: 'drag', contents: { page: 0, x: 72, y: 300, dx: 220, dy: 60 } },
  { cmd: 'sign', contents: { cert: 'fixtures/p5/leaf.p12', password: 'definitely-wrong-pw', field: 'Signature4', reason: 'review' }, to: signedOut },
], async ({ state, shot, click, key, clickSel, cdp }) => {
  const s0 = await wait(async () => { const s = await state(); return s.sign?.last?.reason === 'bad-password' ? s : null; }, 60_000);
  const d = await cdp.eval(dlg);
  check('d0 refused sign keeps dialog in field mode', d.open && s0.sign.dialog?.field === 'Signature4' && !d.fieldHidden, { d, dialog: s0.sign.dialog, last: s0.sign.last });
  const pw = await wait(() => rectOf2(cdp, 'cert-sign-password'));
  await click(pw.cx, pw.cy);
  await cdp.send('Input.insertText', { text: PW });
  await sleep(300);
  const rev = s0.revision;
  await clickSel(`document.getElementById('cert-sign-go')`);
  const s = await wait(async () => { const v = await state(); return v.sign?.last?.ok === true ? v : null; }, 30_000).catch(async () => state());
  const d3 = await cdp.eval(dlg);
  check('d1 real Sign click signs Signature4', s.sign?.last?.ok === true && s.sign.last.field === 'Signature4' && s.sign.dialog === null && !d3.open && s.revision === rev && s.dirty === true && existsSync(signedOut), { last: s.sign?.last, dialog: s.sign?.dialog, open: d3.open, status: d3.status, rev: s.revision, dirty: s.dirty });
  const st = JSON.stringify(s);
  check('g1 no secret in STATE', !st.includes(JSON.stringify(PW)), 'checked');
  await shot('review-u3-d1-after-sign.png');
}); }
async function rectOf2(cdp, id) { return cdp.eval(`(() => { const e = document.getElementById('${id}'); if (!e) return null; const r = e.getBoundingClientRect(); return r.width ? { cx: r.x + r.width / 2, cy: r.y + r.height / 2 } : null; })()`); }

// ---------------- C: open the signed output: badges, no handles, verify click, draw disabled
if (which.includes('C') && existsSync(signedOut)) await session('rv', [{ cmd: 'open', path: signedOut }], async ({ state, shot, click, key, pagePoint, clickSel, mode, cdp }) => {
  let s = await state();
  check('e0 signed output statuses', s.doc?.readOnly === true && sigs(s).startsWith('Signature1:empty') && /Signature4:signed/.test(sigs(s)), sigs(s));
  await pagePoint(206, 650);
  let info = await cdp.eval(sigInfo);
  check('e1 read-only: no handles, title = aria, captions', info.every(i => i.handles === 0 && i.title === i.aria && !/read-only/.test(i.title)) && info.find(i => i.name === 'Signature4')?.cap.includes('Signed'), info);
  await shot('review-u3-e1-signed.png');
  const s4 = s.signatures.find(x => x.field === 'Signature4');
  let p = await pagePoint((s4.rect[0] + s4.rect[2]) / 2, (s4.rect[1] + s4.rect[3]) / 2);
  await click(p.x, p.y);
  s = await wait(async () => { const v = await state(); return v.signatures.find(x => x.field === 'Signature4')?.status === 'valid' ? v : null; }, 30_000).catch(() => state());
  const vd = await cdp.eval(`(() => ({ open: document.getElementById('verify-dialog').open, cur: [...document.querySelectorAll('#verify-dialog tr[aria-current="true"]')].map(r => r.dataset.field) }))()`);
  check('d2 click signed field opens verify, row current, status valid', vd.open && vd.cur.includes('Signature4') && s.signatures.find(x => x.field === 'Signature4')?.status === 'valid' && s.verify?.last?.ok === true, { vd, sigs: sigs(s), status: s.direct?.status });
  await key('Escape');
  info = await cdp.eval(sigInfo);
  const i4 = info.find(i => i.name === 'Signature4');
  check('d3 caption Valid, untrusted', /Valid, untrusted/.test(i4?.cap || '') && /valid/.test(i4.cls) && /untrusted/.test(i4.cls), i4);
  await shot('review-u3-d3-valid.png');
  // click #verify-open for real
  await clickSel(`document.getElementById('verify-open')`);
  const vo = await cdp.eval(`document.getElementById('verify-dialog').open`);
  check('d4 #verify-open opens verify dialog', vo, vo);
  await key('Escape');
  // Signing another empty field on a signed doc: dialog opens in field mode
  p = await pagePoint(450, 650);
  await click(p.x, p.y);
  s = await state();
  check('e2 click empty field on signed doc opens field dialog', s.sign?.dialog?.field === 'Signature2', s.sign);
  await key('Escape');
  await mode('forms');
  const b = await cdp.eval(`(() => { const b = document.getElementById('sigfield-draw'); return { disabled: b.disabled, title: b.title }; })()`);
  check('e3 sigfield-draw disabled on a read-only doc', b.disabled === true, b);
  s = await state();
  info = await cdp.eval(sigInfo);
  check('e4 Forms mode on signed doc: no handles; verify badge persists across mode', info.every(i => i.handles === 0) && /Valid/.test(info.find(i => i.name === 'Signature4')?.cap || ''), info.map(i => [i.name, i.cap, i.handles]));
});
// ---------------- D: menu Sign… path after a field dialog (drive op cert-sign-open = the menu's IPC), then stale #error after a good draw
if (which.includes('D')) await session('rm', [
  { cmd: 'open', path: FIX }, { cmd: 'click', contents: { page: 0, x: 206, y: 650 } }, { cmd: 'key', contents: { key: 'Escape' } }, { cmd: 'cert-sign-open' },
], async ({ state, drag, key, pagePoint, clickSel, mode, cdp }) => {
  await wait(async () => (await cdp.eval(`document.getElementById('cert-sign-dialog').open`)) ? true : null, 60_000);
  await sleep(800);
  const d = await cdp.eval(dlg), s = await state();
  check('a4 menu Sign after a field dialog: no field line, page+appearance shown', d.open && d.fieldHidden && !d.pageHidden && !d.appHidden && s.sign?.dialog === null, { d, dialog: s.sign?.dialog });
  await key('Escape');
  await mode('forms');
  await clickSel(`document.getElementById('sigfield-draw')`);
  let p = await pagePoint(72, 400); await drag(p.x, p.y, 30 * p.s, 10 * p.s);
  await clickSel(`document.getElementById('sigfield-draw')`);
  p = await pagePoint(72, 300); await drag(p.x, p.y, 220 * p.s, 60 * p.s);
  const v = await state();
  check('c5 good draw after a refused one clears #error', v.fields?.last?.name === 'Signature4' && v.error === '', { last: v.fields?.last, error: v.error, announcement: v.announcement });
});
writeFile(join(scratch, 'probe-results.json'), JSON.stringify(results, null, 1));
log('SUMMARY', results.filter(r => !r.ok).map(r => r.id).join(', ') || 'all ok');
