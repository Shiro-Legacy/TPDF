#!/usr/bin/env node
/** Review probe (renderer lens, Batch): real CDP pointer/keys over #batch-dialog, menu item clicked in main via --inspect. Not a test. */
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { copyFile, mkdir, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import mupdf from 'mupdf';

const root = fileURLToPath(new URL('../../../../', import.meta.url));
const checksDir = fileURLToPath(new URL('./', import.meta.url));
const out = join(checksDir, 'review-out');
const scratch = join(root, '.quad/scratch/batch/review-renderer');
const electron = createRequire(join(root, 'package.json'))('electron');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const q = s => JSON.stringify(s);
const results = [];
const check = (cond, label, got) => { results.push({ ok: !!cond, label, got }); console.log(cond ? 'ok  ' : 'FAIL', label, q(got)); };
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
  send(method, params = {}) { const id = ++this.n; return new Promise((res, rej) => { const t = setTimeout(() => { this.pending.delete(id); rej(new Error('cdp timeout ' + method)); }, 15_000); this.pending.set(id, { res: v => { clearTimeout(t); res(v); }, rej: e => { clearTimeout(t); rej(e); } }); this.ws.send(JSON.stringify({ id, method, params })); }); }
  async eval(expression, extra = {}) { const r = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, ...extra }); if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text || expression); return r.result?.value; }
  close() { try { this.ws.close(); } catch {} }
}
const hold = (n, ms) => Array.from({ length: n }, () => ({ cmd: 'wait', ms }));
const FOUR = ['encrypted.pdf', 'hello.pdf', 'nav.pdf', 'rotated.pdf'];
async function stage(name, files = FOUR) {
  const dir = join(scratch, name); await rm(dir, { recursive: true, force: true }); await mkdir(dir, { recursive: true });
  for (const f of files) await copyFile(join(root, 'fixtures', typeof f === 'string' ? f : f[0]), join(dir, typeof f === 'string' ? f : f[1]));
  return dir;
}
const pageTexts = bytes => { const doc = mupdf.Document.openDocument(bytes, 'application/pdf'); const t = []; for (let i = 0; i < doc.countPages(); i++) t.push(doc.loadPage(i).toStructuredText().asText()); return t; };

async function session(tag, dir, body) {
  const udata = join(out, `batch-review-udata-${tag}`);
  await rm(udata, { recursive: true, force: true });
  const probe = join(scratch, `probe-${tag}.jsonl`);
  await writeFile(probe, [{ cmd: 'open', path: join(root, 'fixtures/nav.pdf') }, { cmd: 'header-footer', contents: { kind: 'header', text: 'Open doc' } }, ...hold(40, 5000)].map(l => JSON.stringify(l)).join('\n') + '\n');
  const port = await freePort(); const iport = await freePort();
  const child = spawn(electron, [`--inspect=${iport}`, `--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${udata}`, '.',
    `--drive=${probe}`, '--gui', `--drive-out=${join(scratch, `out-${tag}`)}`, `--report=${join(scratch, `report-${tag}.json`)}`, `--batch-dir=${dir}`], { cwd: root, stdio: ['ignore', 'ignore', 'ignore'] });
  const closed = new Promise(done => child.on('close', done));
  let cdp, main;
  try {
    const page = await wait(async () => (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.webSocketDebuggerUrl && t.type === 'page'));
    cdp = new Cdp(page.webSocketDebuggerUrl); await cdp.connect(); await cdp.send('Runtime.enable'); await cdp.send('Page.enable');
    const mt = await wait(async () => (await (await fetch(`http://127.0.0.1:${iport}/json/list`)).json())[0]);
    main = new Cdp(mt.webSocketDebuggerUrl); await main.connect(); await main.send('Runtime.enable');
    try { const { windowId } = await cdp.send('Browser.getWindowForTarget'); await cdp.send('Browser.setWindowBounds', { windowId, bounds: { width: 1280, height: 900, windowState: 'normal' } }); } catch {}
    const T = {
      cdp, main,
      state: () => cdp.eval('window.pdf4allState()'),
      shot: async name => { const { data } = await cdp.send('Page.captureScreenshot', { format: 'png' }); await writeFile(join(out, name), Buffer.from(data, 'base64')); console.log('shot', name); },
      async click(x, y) { const m = (type, extra = {}) => cdp.send('Input.dispatchMouseEvent', { type, x, y, ...extra }); await m('mouseMoved'); await sleep(40); await m('mousePressed', { button: 'left', buttons: 1, clickCount: 1 }); await m('mouseReleased', { button: 'left', buttons: 0, clickCount: 1 }); await sleep(60); },
      centre: sel => cdp.eval(`(() => { const e = document.querySelector(${q(sel)}); if (!e) return null; const r = e.getBoundingClientRect(); const x = r.left + r.width / 2, y = r.top + r.height / 2; return { x, y, w: r.width, hit: document.elementFromPoint(x, y)?.id ?? null }; })()`),
      async clickEl(sel) { const c = await T.centre(sel); if (!c || !c.w) throw new Error('no box ' + sel); if (c.hit !== sel.slice(1)) console.log('warn: hit', sel, c.hit); await T.click(c.x, c.y); },
      async key(k, code, vk, modifiers = 0) { await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key: k, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers }); await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key: k, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers }); await sleep(80); },
      async typeInto(sel, text) { await T.clickEl(sel); await T.key('a', 'KeyA', 65, 4); await cdp.eval(`document.querySelector(${q(sel)}).select()`); await cdp.send('Input.insertText', { text }); await sleep(100); },
      async setSelect(sel, v) { await cdp.eval(`(() => { const s = document.querySelector(${q(sel)}); s.focus(); s.value = ${q(v)}; s.dispatchEvent(new Event('input', { bubbles: true })); s.dispatchEvent(new Event('change', { bubbles: true })); })()`); await sleep(100); },
      val: sel => cdp.eval(`document.querySelector(${q(sel)}).value`),
      dis: () => cdp.eval(`({ pick: document.getElementById('batch-pick').disabled, start: document.getElementById('batch-start').disabled, open: document.getElementById('batch-dialog').open })`),
      menu: () => main.eval(`(() => { const f = require('electron').Menu.getApplicationMenu().items.find(i => i.label === 'File'); const it = f.submenu.items.find(i => i.label === 'Batch…'); it.click(); return it.accelerator; })()`, { includeCommandLineAPI: true }),
      rows: () => cdp.eval(`[...document.querySelectorAll('#batch-list tbody tr')].map(tr => [...tr.cells].map(td => td.textContent))`),
      summary: () => cdp.eval(`document.getElementById('batch-summary').textContent`),
    };
    await wait(async () => { const s = await T.state(); return s.doc?.name === 'nav.pdf' && s.revision === 1; }, 60_000);
    await sleep(500);
    await body(T);
  } catch (err) { console.log('ERROR', tag, err.stack); results.push({ ok: false, label: `exception ${tag}`, got: String(err) }); }
  finally { cdp?.close(); main?.close(); child.kill('SIGTERM'); console.log('electron closed', tag, await closed); }
}

const pick = s => s && { total: s.total, done: s.done, ok: s.ok, failed: s.failed, running: s.running, cancelled: s.cancelled, report: s.report, bates: s.bates, files: s.files.map(f => ({ name: f.name, status: f.status, ...(f.status === 'ok' ? { pages: f.pages } : {}), ...(f.detail ? { detail: f.detail } : {}), ...(f.bates ? { bates: f.bates } : {}) })) };
const WANT = { total: 4, done: 4, ok: 3, failed: 1, running: false, cancelled: false, report: 'ok', bates: { start: 1, next: 6, pad: 4 }, files: [{ name: 'encrypted.pdf', status: 'failed', detail: 'encrypted' }, { name: 'hello.pdf', status: 'ok', pages: 1, bates: { from: 1, to: 1 } }, { name: 'nav.pdf', status: 'ok', pages: 3, bates: { from: 2, to: 4 } }, { name: 'rotated.pdf', status: 'ok', pages: 1, bates: { from: 5, to: 5 } }] };

await mkdir(scratch, { recursive: true });
// ---------------------------------------------------------------- A: the main run, rerun, close/reopen, open doc untouched
const ONLY = process.env.ONLY ?? 'abcd';
const dirA = await stage('in');
if (ONLY.includes('a')) await session('a', dirA, async T => {
  let s = await T.state();
  check(!('batch' in s), 'A0: no batch key before the dialog', Object.keys(s).includes('batch'));
  const acc = await T.menu(); await sleep(400);
  check(acc === 'Alt+CmdOrCtrl+B', 'A1: File ▸ Batch… accelerator', acc);
  let d = await T.dis();
  check(d.open && d.start && !d.pick, 'A1: dialog open by menu item, start disabled, pick enabled', d);
  s = await T.state();
  console.log('A1 batch slice after open', q(s.batch));
  await T.shot('review-batch-a0-open.png');
  await T.clickEl('#batch-pick'); await sleep(600);
  s = await T.state(); d = await T.dis();
  check(await T.val('#batch-folder') === dirA && s.batch.total === 4 && s.batch.files.every(f => f.status === 'queued') && !d.start, 'A2: pick fills folder, 4 queued, start enabled', { folder: await T.val('#batch-folder'), total: s.batch.total, start: d.start });
  check(q(s.batch.files.map(f => f.name)) === q(FOUR), 'A2: listing order', s.batch.files.map(f => f.name));
  console.log('A2 summary', q(await T.summary()));
  await T.setSelect('#batch-kind', 'footer');
  await T.typeInto('#batch-text', 'B-{bates}');
  await T.setSelect('#batch-align', 'right');
  await T.typeInto('#batch-bates-pad', '4');
  check(await T.val('#batch-bates-start') === '1', 'A3: start prefilled 1', await T.val('#batch-bates-start'));
  await T.shot('review-batch-a1-filled.png');
  const c = await T.centre('#batch-start');
  await T.click(c.x, c.y);
  const during = await T.dis(); const sd = (await T.state()).batch;
  console.log('A4 right after start click', q(during), 'running', sd.running, 'done', sd.done);
  await wait(async () => (await T.state()).batch.running === false);
  s = await T.state();
  check(q(pick(s.batch)) === q(WANT), 'A5: STATE batch vs §9.16', pick(s.batch));
  check(s.announcement === 'Batch done: 3 ok, 1 failed, report in pdf4all-out', 'A5: announcement', s.announcement);
  check(await T.summary() === 'Batch done: 3 ok, 1 failed, report in pdf4all-out', 'A5: #batch-summary', await T.summary());
  check(s.doc?.name === 'nav.pdf' && s.revision === 1 && s.dirty === true && s.canUndo === true && s.error === '', 'A5: open doc untouched', { doc: s.doc, revision: s.revision, dirty: s.dirty, canUndo: s.canUndo, error: s.error });
  check(q(Object.keys(s.batch).sort()) === q(['action', 'bates', 'cancelled', 'done', 'failed', 'files', 'folder', 'ok', 'out', 'report', 'running', 'total']), 'A5: batch keys', Object.keys(s.batch).sort());
  console.log('A5 rows', q(await T.rows()));
  await T.shot('review-batch-a2-done.png');
  const outDir = join(dirA, 'pdf4all-out');
  const listing = (await readdir(outDir)).sort();
  check(q(listing) === q(['hello.pdf', 'nav.pdf', 'report.txt', 'rotated.pdf']), 'A6: pdf4all-out listing', listing);
  const report1 = await readFile(join(outDir, 'report.txt'));
  check(report1.toString() === 'encrypted.pdf\tfailed\t0\tencrypted\t\nhello.pdf\tok\t1\t\t0001-0001\nnav.pdf\tok\t3\t\t0002-0004\nrotated.pdf\tok\t1\t\t0005-0005\n', 'A6: report.txt §9.17', report1.toString());
  const navT = pageTexts(await readFile(join(outDir, 'nav.pdf')));
  check(navT.every((t, i) => t.includes(`B-000${i + 2}`)) && navT.every(t => t.includes('Open doc') === false), 'A6: nav out has B-0002..4 and no open-doc header', navT.map(t => t.trim().split('\n').slice(-2)));
  // rerun
  const c2 = await T.centre('#batch-start'); check(!(await T.dis()).start, 'A7: start enabled after run', true);
  await T.click(c2.x, c2.y);
  await wait(async () => { const b = (await T.state()).batch; return b.running === false && b.done === 4; });
  s = await T.state();
  check(q(s.batch.files.map(f => [f.status, f.detail])) === q([['failed', 'encrypted'], ['failed', 'exists'], ['failed', 'exists'], ['failed', 'exists']]) && s.batch.report === 'exists' && q(s.batch.bates) === q({ start: 6, next: 6, pad: 4 }), 'A7: rerun all exists', pick(s.batch));
  check(s.announcement === 'Batch done: 0 ok, 4 failed; report not written: report.txt exists', 'A7: announcement §9.13', s.announcement);
  check(Buffer.compare(report1, await readFile(join(outDir, 'report.txt'))) === 0, 'A7: report.txt byte-identical', true);
  await T.shot('review-batch-a3-rerun.png');
  // close + reopen through the menu
  await T.clickEl('#batch-close'); await sleep(300);
  check(!(await T.dis()).open, 'A8: #batch-close closes', true);
  await T.menu(); await sleep(400);
  s = await T.state();
  check((await T.dis()).open, 'A8: reopened by menu', true);
  console.log('A8 after reopen: folder', q(await T.val('#batch-folder')), 'rows', (await T.rows()).length, 'summary', q(await T.summary()), 'start', q(await T.val('#batch-bates-start')), 'text', q(await T.val('#batch-text')), 'kind', await T.val('#batch-kind'));
  // Escape when idle closes
  await T.key('Escape', 'Escape', 27); await sleep(300);
  check(!(await T.dis()).open, 'A9: Escape closes idle dialog', true);
  // the open document's page has no batch footer: search
  await T.typeInto('#search-input', 'B-000'); await T.key('Enter', 'Enter', 13); await sleep(1200);
  s = await T.state();
  check(s.search?.count === 0, 'A10: open nav has no B-000 text', s.search);
  await T.typeInto('#search-input', 'Open doc'); await T.key('Enter', 'Enter', 13); await sleep(1200);
  s = await T.state();
  check(s.search?.count === 3, 'A10: control: Open doc x3', s.search);
  await T.shot('review-batch-a4-opendoc.png');
});

// ---------------------------------------------------------------- B: slow folder: disabled while running, Escape, #batch-cancel
const many = [];
for (let i = 1; i <= 40; i++) many.push(['nav.pdf', `n${String(i).padStart(2, '0')}.pdf`]);
const dirB = await stage('slow', many);
if (ONLY.includes('b')) await session('b', dirB, async T => {
  await T.menu(); await sleep(300);
  await T.clickEl('#batch-pick'); await sleep(500);
  await T.typeInto('#batch-text', 'S-{bates}');
  const pre = await T.centre('#batch-start');
  console.log('B0 #batch-start before scroll', q(pre), 'dialog', q(await T.cdp.eval(`(() => { const d = document.getElementById('batch-dialog'); const r = d.getBoundingClientRect(); return { top: r.top, h: r.height, scrollH: d.scrollHeight, clientH: d.clientHeight, vh: innerHeight }; })()`)));
  await T.cdp.eval(`document.getElementById('batch-start').scrollIntoView({ block: 'center' })`); await sleep(200);
  const c = await T.centre('#batch-start');
  await T.click(c.x, c.y);
  const d = await T.dis(); const b = (await T.state()).batch;
  check(b.running && d.pick && d.start, 'B1: while running pick+start disabled', { d, running: b.running, done: b.done });
  await T.shot('review-batch-b0-running.png');
  await T.key('Escape', 'Escape', 27);
  const d2 = await T.dis();
  check(d2.open, 'B2: Escape while running keeps dialog open', d2);
  await wait(async () => (await T.state()).batch.running === false, 60_000);
  let s = await T.state();
  const rep = (await readFile(join(dirB, 'pdf4all-out/report.txt'), 'utf8')).split('\n').filter(Boolean);
  check(s.batch.cancelled && s.batch.done < 40 && s.batch.files.slice(s.batch.done).every(f => f.status === 'queued') && rep.length === s.batch.done, 'B2: Escape cancelled after running file', { done: s.batch.done, cancelled: s.batch.cancelled, reportLines: rep.length, ann: s.announcement });
  await T.shot('review-batch-b1-cancelled.png');
  // restart then click #batch-cancel with the pointer
  await T.click(c.x, c.y); await sleep(150);
  const cc = await T.centre('#batch-cancel'); console.log('B3 cancel box during run', q(cc)); await T.click(cc.x, cc.y);
  await wait(async () => (await T.state()).batch.running === false, 60_000);
  s = await T.state();
  const counts = s.batch.files.reduce((m, f) => (m[f.status + ':' + f.detail] = (m[f.status + ':' + f.detail] ?? 0) + 1, m), {});
  check(s.batch.cancelled && s.batch.done < 40 && (await T.dis()).open, 'B3: #batch-cancel click mid-run stops, dialog stays', { done: s.batch.done, counts, report: s.batch.report, ann: s.announcement });
  await T.shot('review-batch-b2-cancel-click.png');
});

// ---------------------------------------------------------------- C: empty / non-PDF folder
const dirC = await stage('nopdf', []);
await writeFile(join(dirC, 'notes.txt'), 'hi'); await mkdir(join(dirC, 'fake.pdf'));
if (ONLY.includes('c')) await session('c', dirC, async T => {
  await T.menu(); await sleep(300);
  await T.clickEl('#batch-pick'); await sleep(500);
  const s = await T.state();
  check(s.batch.total === 0 && (await T.dis()).start && (await T.summary()) === 'No PDF files in this folder', 'C1: no PDFs → message, start disabled', { total: s.batch.total, summary: await T.summary(), folder: s.batch.folder, error: s.error });
  await T.shot('review-batch-c0-nopdf.png');
});

// ---------------------------------------------------------------- D: symlinked pdf4all-out
const dirD = await stage('sym');
const elsewhere = join(scratch, 'elsewhere'); await rm(elsewhere, { recursive: true, force: true }); await mkdir(elsewhere);
await symlink(elsewhere, join(dirD, 'pdf4all-out'));
if (ONLY.includes('d')) await session('d', dirD, async T => {
  await T.menu(); await sleep(300);
  await T.clickEl('#batch-pick'); await sleep(500);
  await T.typeInto('#batch-text', 'B-{bates}');
  await T.clickEl('#batch-start');
  await wait(async () => (await T.state()).batch.running === false);
  const s = await T.state();
  check(q(s.batch.files.map(f => f.detail)) === q(['encrypted', 'unwritable', 'unwritable', 'unwritable']) && (await readdir(elsewhere)).length === 0 && s.batch.bates.next === s.batch.bates.start, 'D1: symlink → unwritable, nothing elsewhere, counter unmoved', { details: s.batch.files.map(f => f.detail), elsewhere: await readdir(elsewhere), bates: s.batch.bates, report: s.batch.report, ann: s.announcement });
  await T.shot('review-batch-d0-symlink.png');
});

console.log(execFileSync('shasum', ['-a', '256', '-c', join(checksDir, 'fixtures.sha256')], { cwd: root, encoding: 'utf8' }));
console.log('SUMMARY', results.filter(r => r.ok).length, 'ok', results.filter(r => !r.ok).length, 'fail');
