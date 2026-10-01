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
const udata = join(out, 'bates-review-udata');
await rm(udata, { recursive: true, force: true });
const probe = join(root, '.quad/scratch/bates/review-renderer/probe.jsonl');
await writeFile(probe, [{ cmd: 'open', path: join(root, 'fixtures/nav.pdf') }, ...hold(30, 5000), { cmd: 'open', path: join(root, 'fixtures/hello.pdf') }, ...hold(30, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
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
  await wait(() => cdp.eval(`document.title.includes('nav')`), 60_000); await sleep(1000);

  // organize mode (real click on its label), then the Header/footer button with the real pointer
  const lab = await centre('label[for="mode-organize"]');
  if (lab && lab.w) await click(lab.x, lab.y); else await cdp.eval(`document.getElementById('mode-organize').click()`);
  await sleep(500);
  const s0 = await state();
  check(!('decorate' in s0), 'fresh nav: no decorate key', Object.keys(s0));
  console.log('diag', q(await cdp.eval(`({ mode: document.querySelector('input[name=mode]:checked')?.id, dis: document.getElementById('stamp-header').disabled, box: (() => { const r = document.getElementById('stamp-header').getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; })(), hit: (() => { const r = document.getElementById('stamp-header').getBoundingClientRect(); return document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2)?.id; })() })`)));
  await shot('review-bates-0-organize.png');
  await cdp.eval(`document.getElementById('stamp-header').click()`); await sleep(400);
  check(await dlgOpen(), 'stamp dialog open by real click on #stamp-header', true);
  check(await val('#stamp-bates-start') === '1' && await val('#stamp-bates-pad') === '6', 'prefill start 1 pad 6', [await val('#stamp-bates-start'), await val('#stamp-bates-pad')]);
  await setSelect('#stamp-kind', 'footer');
  check(await cdp.eval(`document.getElementById('stamp-dialog-heading').textContent`) === 'Add footer', 'heading Add footer', await cdp.eval(`document.getElementById('stamp-dialog-heading').textContent`));
  await typeInto('#stamp-text', 'ACME-{bates}');
  await setSelect('#stamp-align', 'right');
  await typeInto('#stamp-bates-start', '500');
  await shot('review-bates-a-dialog-filled.png');
  await clickEl('#stamp-save'); await sleep(1500);
  let s = await state();
  check(!(await dlgOpen()), 'dialog closed after Apply', false);
  check(q(s.decorate?.last) === q({ action: 'stamp', kind: 'footer', pages: [0, 1, 2], ok: true, bates: { from: 500, to: 502 } }), 'decorate.last', s.decorate?.last);
  check(q(s.decorate?.bates) === q({ next: 503, pad: 6, last: { from: 500, to: 502, pages: [0, 1, 2] } }), 'decorate.bates', s.decorate?.bates);
  const ps = await cdp.eval(`document.getElementById('page-status').textContent`);
  check(ps === 'Added footer to 3 pages (Bates 000500–000502)', '#page-status', ps);
  check(s.revision === 1 && s.dirty === true && s.error === '', 'revision 1 dirty error empty', { revision: s.revision, dirty: s.dirty, error: s.error });
  const rev1 = s.revision;

  // reopen: prefill 503 / 6; watermark disables align and changes heading
  await cdp.eval(`document.getElementById('stamp-header').click()`); await sleep(400);
  check(await val('#stamp-bates-start') === '503' && await val('#stamp-bates-pad') === '6', 'reopen prefill 503/6', [await val('#stamp-bates-start'), await val('#stamp-bates-pad')]);
  check(await val('#stamp-align') === 'left', 'reopen align reset to left', await val('#stamp-align'));
  await setSelect('#stamp-kind', 'watermark');
  const wm = await cdp.eval(`({ d: document.getElementById('stamp-align').disabled, h: document.getElementById('stamp-dialog-heading').textContent, sd: document.getElementById('stamp-bates-start').disabled })`);
  check(wm.d === true && wm.h === 'Watermark', 'watermark: align disabled, heading Watermark', wm);
  await setSelect('#stamp-kind', 'header');
  check(await cdp.eval(`document.getElementById('stamp-align').disabled`) === false, 'header: align enabled again', false);
  // refusal: start abc with {bates}
  await typeInto('#stamp-text', 'X-{bates}');
  await typeInto('#stamp-bates-start', 'abc');
  await clickEl('#stamp-save'); await sleep(800);
  s = await state();
  check(await dlgOpen(), 'abc: dialog stays open', true);
  check(s.error === 'Bates start must be a whole number and digits 1–15', 'abc: error text', s.error);
  check(s.revision === rev1 && s.dirty === true, 'abc: revision unchanged', s.revision);
  check(q(s.decorate?.last?.bates) === q({ from: 500, to: 502 }), 'abc: decorate.last after GUI refusal (stale?)', s.decorate?.last);
  await shot('review-bates-b-refusal-abc.png');
  // empty start
  await typeInto('#stamp-bates-start', '');
  check(await val('#stamp-bates-start') === '', 'start emptied', await val('#stamp-bates-start'));
  await clickEl('#stamp-save'); await sleep(800);
  s = await state();
  check(await dlgOpen() && s.revision === rev1 && s.error.startsWith('Bates start'), 'empty start: refused, open, revision unchanged', { open: await dlgOpen(), rev: s.revision, error: s.error });
  // pad 16 / pad 0 / start 16 digits / ' 7 '
  for (const [st, pd, want] of [['7', '16', false], ['7', '0', false], ['1234567890123456', '6', false], ['0x10', '6', false], ['1e3', '6', false]]) {
    await typeInto('#stamp-bates-start', st); await typeInto('#stamp-bates-pad', pd);
    await clickEl('#stamp-save'); await sleep(600);
    s = await state();
    check(await dlgOpen() && s.revision === rev1, `start ${q(st)} pad ${q(pd)} refused`, { open: await dlgOpen(), rev: s.revision });
  }
  // text without {bates}, start abc: must not refuse
  await typeInto('#stamp-bates-pad', '6');
  await typeInto('#stamp-text', 'Page {page} of {pages}');
  await typeInto('#stamp-bates-start', 'abc');
  await clickEl('#stamp-save'); await sleep(1500);
  s = await state();
  check(!(await dlgOpen()) && s.revision === rev1 + 1, 'no {bates} + start abc: applied', { open: await dlgOpen(), rev: s.revision });
  check(s.error === '', 'stale Bates error cleared after a later success', s.error);
  check(q(s.decorate?.last) === q({ action: 'stamp', kind: 'header', pages: [0, 1, 2], ok: true }) && s.decorate?.bates?.next === 503, 'plain header: last has no bates, next still 503', s.decorate);
  const ps2 = await cdp.eval(`document.getElementById('page-status').textContent`);
  check(ps2 === 'Added header to 3 pages', '#page-status plain header', ps2);
  await clickEl('#fit-page'); await sleep(1500);
  await shot('review-bates-c-nav-fitpage.png');
  // pixel-ish check: the canvas box to map right margin; report canvas rect
  console.log('canvas', q(await cdp.eval(`(() => { const r = document.getElementById('canvas').getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; })()`)));

  // second document
  await wait(() => cdp.eval(`document.title.includes('hello')`), 200_000, 500); await sleep(1500);
  s = await state();
  check(!('decorate' in s), 'hello: decorate key absent after open', Object.keys(s).filter(k => k === 'decorate'));
  const lab2 = await centre('label[for="mode-organize"]');
  if (!(await cdp.eval(`document.getElementById('mode-organize').checked`))) { if (lab2 && lab2.w) await click(lab2.x, lab2.y); else await cdp.eval(`document.getElementById('mode-organize').click()`); }
  await cdp.eval(`document.getElementById('stamp-header').click()`); await sleep(400);
  check(await val('#stamp-bates-start') === '503', 'hello: prefill 503 carried across open', await val('#stamp-bates-start'));
  await typeInto('#stamp-text', 'DOC {bates}');
  await clickEl('#stamp-save'); await sleep(1500);
  s = await state();
  check(q(s.decorate?.last) === q({ action: 'stamp', kind: 'header', pages: [0], ok: true, bates: { from: 503, to: 503 } }) && s.decorate?.bates?.next === 504, 'hello header continues from 503', s.decorate);
  const ps3 = await cdp.eval(`document.getElementById('page-status').textContent`);
  console.log('page-status single page:', q(ps3));
  // undo with the real pointer on #undo
  await clickEl('#undo'); await sleep(1500);
  s = await state();
  check(s.revision === 0 && s.decorate?.bates?.next === 504, 'undo: revision 0, next stays 504 (D2 ceiling)', { revision: s.revision, bates: s.decorate?.bates, last: s.decorate?.last });
  await cdp.eval(`document.getElementById('stamp-header').click()`); await sleep(400);
  check(await val('#stamp-bates-start') === '504', 'after undo prefill 504', await val('#stamp-bates-start'));
  await clickEl('#stamp-cancel');
  // Help
  const help = await cdp.eval(`(() => { const u = document.getElementById('help-ceilings'); const n1 = u.nextElementSibling, n2 = n1?.nextElementSibling; let between = ''; for (let n = u.nextSibling; n && n !== n1; n = n.nextSibling) between += n.nodeType === 3 ? n.textContent.trim() : '<' + n.nodeName + '>'; return { n1: n1?.outerHTML.slice(0, 60), n2id: n2?.id, between, li: n2?.textContent }; })()`);
  check(help.n1?.startsWith('<h3>Headers, footers and Bates numbers') && help.n2id === 'help-bates' && help.between === '', 'Help placement', help);
  const names = await cdp.eval(`['stamp-align','stamp-bates-start','stamp-bates-pad','stamp-kind','stamp-text'].map(id => { const e = document.getElementById(id); return [id, e.labels ? [...e.labels].map(l => l.textContent) : null, e.getAttribute('aria-label')]; })`);
  check(names.every(([, l]) => l && l.length === 1 && l[0]), 'new controls labelled', names);
  await shot('review-bates-d-hello-after-undo.png');
} catch (err) { console.log('ERROR', err.stack); results.push({ ok: false, label: 'exception', got: String(err) }); }
finally {
  cdp?.close(); child.kill('SIGTERM'); console.log('electron closed', await closed);
  console.log(execFileSync('shasum', ['-a', '256', '-c', join(checksDir, 'fixtures.sha256')], { cwd: root, encoding: 'utf8' }));
  console.log('SUMMARY', results.filter(r => r.ok).length, 'ok', results.filter(r => !r.ok).length, 'fail');
}
