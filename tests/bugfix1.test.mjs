/**
 * Bugfix 1 (contract-bugfix1.md D7): B1 font after undo, B2 app-written growth, B6 inherited
 * /Rotate, B4 MCP relative save folders, B3 Save on Untitled = Save As, B5 undo/redo clear the error.
 * Engine harness copied from tests/p6-text8.test.mjs:1-37; drive()/red()/brief() and the MCP helper
 * copied from tests/drive-guard.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp as makeTempDir, readFile, readdir, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import { appBinary } from './app-bin.mjs';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';
import { blankPdf } from './blank-pdf.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const engine = createEngine(mupdf);
const DIRECT3 = 'fixtures/p6/direct3.pdf';
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);

/** Every temp dir this file makes: removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));

function openBytes(bytes, name = 'b.pdf') {
  const s = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: buffer(bytes) }, s).reply;
  assert.equal(reply.ok, true, reply.error);
  return s;
}
const open = async (name = DIRECT3) => openBytes(await readFile(join(root, name)), name);
const request = (s, req) => engine.dispatch({ id: 2, ...req }, s).reply;
const scan = (s, page = 0) => request(s, { op: 'edit-targets', page });
const add = (s, point, text) => request(s, { op: 'text-add', page: 0, point, text, font: 'Helvetica', size: 12, color: [0, 0, 0] });
function edit(s, from, text) {
  const sc = scan(s);
  const target = sc.items.find(item => item.kind === 'text' && item.text === from);
  assert.ok(target, `target ${from}`);
  return request(s, { op: 'edit-text', page: 0, targetId: target.targetId, generation: sc.generation, revision: sc.revision, text });
}
/** Saves the session, reopens the bytes with mupdf, and returns page 0's `/Resources /Font` entries named PDF4AllF*. */
function savedFonts(s) {
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const doc = mupdf.Document.openDocument(new Uint8Array(saved.bytes), 'application/pdf');
  const fonts = {};
  doc.asPDF().findPage(0).get('Resources').get('Font').forEach((value, key) => {
    if (!String(key).startsWith('PDF4AllF')) return;
    const font = value.resolve();
    fonts[key] = font && font.isDictionary() && !font.get('Type').isNull() ? font.get('Type').asName() : null;
  });
  return fonts;
}

test('(a) B1: text-add after an undone text-add gets a live /Type /Font; stamp after an undone stamp too', async () => {
  const s = await open();
  assert.equal(add(s, [300, 409], 'Xray').ok, true);
  assert.equal(request(s, { op: 'undo' }).ok, true);
  const again = add(s, [300, 209], 'Yankee');
  assert.equal(again.ok, true, again.error);
  assert.notEqual(again.targetId, '', 'the added run is found by the scan');
  const item = scan(s).items.find(entry => entry.text === 'Yankee');
  assert.equal(item?.support, 'candidate', JSON.stringify(item));
  assert.deepEqual(savedFonts(s), { PDF4AllF1: 'Font' });

  const t = await open();
  assert.equal(request(t, { op: 'stamp', kind: 'watermark', text: 'DRAFT' }).ok, true);
  assert.equal(request(t, { op: 'undo' }).ok, true);
  assert.equal(request(t, { op: 'stamp', kind: 'watermark', text: 'DRAFT' }).ok, true);
  assert.deepEqual(savedFonts(t), { PDF4AllF: 'Font' });
});

test('(b) B2: an app-written run grows to the nearest obstacle, else the page edge; other runs keep the text margin', async () => {
  const lone = openBytes(blankPdf(612, 792));
  assert.equal(add(lone, [72, 700], 'Twin').ok, true);
  const grown = edit(lone, 'Twin', 'Twin 2');
  assert.equal(grown.ok, true, JSON.stringify(grown));
  assert.equal(scan(lone).items.filter(item => item.text === 'Twin 2').length, 1);

  const walled = openBytes(blankPdf(612, 792));
  assert.equal(add(walled, [72, 700], 'Twin').ok, true);
  assert.equal(add(walled, [100, 700], 'Wall').ok, true);
  const blocked = edit(walled, 'Twin', 'Twin 2');
  assert.equal(blocked.detail, 'overflow', JSON.stringify(blocked));
  assert.equal(blocked.limit, 99);

  const edge = openBytes(blankPdf(612, 792));
  assert.equal(add(edge, [72, 700], 'Twin').ok, true);
  const wide = edit(edge, 'Twin', 'Twin ' + 'W'.repeat(60));
  assert.equal(wide.detail, 'overflow', JSON.stringify(wide));
  assert.equal(wide.limit, 612, 'the page box right edge');

  const d = await open();
  const other = edit(d, 'oscar papa quebec', 'oscar papa quebec romeo sierra tango uniform victor whiskey xray');
  assert.equal(other.detail, 'overflow', JSON.stringify(other));
  assert.equal(other.limit, 219.96, 'a run the app did not write keeps the text-margin limit');
});

test('(f) B2 review: an app-written run on a page rotated after text-add cannot grow off the page', async () => {
  const s = openBytes(blankPdf(612, 792));
  assert.equal(add(s, [72, 700], 'Twin').ok, true);
  assert.equal(request(s, { op: 'page-rotate', page: 0, by: 90 }).ok, true);
  // The run now runs down the view from y 72; 48 Ws take it past the 612 pt view bottom.
  const off = edit(s, 'Twin', 'Twin ' + 'W'.repeat(48));
  assert.equal(off.ok, false, JSON.stringify(off));
  assert.equal(off.detail, 'overflow', JSON.stringify(off));
});

test('(c) B6: a /Rotate inherited from /Pages blocks object placements like a page-own /Rotate', async () => {
  const doc = mupdf.Document.openDocument(await readFile(join(root, DIRECT3)), 'application/pdf');
  const pdf = doc.asPDF();
  const page = pdf.findPage(0);
  assert.ok(page.get('Rotate').isNull(), 'direct3 p0 has no own /Rotate');
  page.get('Parent').put('Rotate', 90);
  const s = openBytes(pdf.saveToBuffer('').asUint8Array().slice(), 'rotated.pdf');
  const placements = scan(s).items.filter(item => item.kind === 'image' || item.kind === 'form');
  assert.equal(placements.length, 3);
  for (const item of placements) {
    assert.equal(item.support, 'unsupported', JSON.stringify(item));
    assert.equal(item.reason, 'unsupported-transform');
    assert.equal(item.placement, undefined);
  }
});

test('(d) B4: tandem_save makes missing folders under --out and refuses a path that escapes it', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async t => {
  const parent = await mkdtemp(join(tmpdir(), 'pdf4all-bugfix1-mcp-'));
  const out = join(parent, 'out');
  const child = spawn(appBinary, ['--mcp', `--out=${out}`], { cwd: root, stdio: ['pipe', 'pipe', 'inherit'] });
  const exited = new Promise(resolve => child.on('close', resolve));
  // A failed assertion must not leave the server (and its session) running: EOF closes both.
  t.after(async () => { if (child.exitCode === null) { child.stdin.end(); await exited; } });
  const replies = new Map();
  const waiting = new Map();
  createInterface({ input: child.stdout }).on('line', line => {
    const message = JSON.parse(line);
    const key = JSON.stringify(message.id);
    if (waiting.has(key)) { waiting.get(key)(message); waiting.delete(key); } else replies.set(key, message);
  });
  const reply = id => new Promise(resolve => {
    const key = JSON.stringify(id);
    if (replies.has(key)) { resolve(replies.get(key)); replies.delete(key); } else waiting.set(key, resolve);
  });
  let n = 0;
  const rpc = (method, params) => { const id = ++n; child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, ...(params ? { params } : {}) }) + '\n'); return reply(id); };
  const call = async (name, args = {}) => {
    const message = await rpc('tools/call', { name, arguments: args });
    assert.equal(message.error, undefined, JSON.stringify(message.error));
    const text = message.result.content[0].text;
    let step = null;
    try { step = JSON.parse(text); } catch { /* a plain-text refusal */ }
    return { isError: message.result.isError, text, step };
  };

  const init = await rpc('initialize', { protocolVersion: '2025-03-26', capabilities: {}, clientInfo: { name: 'test', version: '0' } });
  assert.equal(init.result.serverInfo.name, 'tandem-pdf');
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
  assert.equal((await call('tandem_new')).isError, false);
  const added = await call('tandem_do', { cmd: 'text-add', contents: { page: 0, point: [72, 700], text: 'Deep save', font: 'Helvetica', size: 12, color: [0, 0, 0] } });
  assert.equal(added.isError, false, added.text);
  const saved = await call('tandem_save', { path: 'sub/deep/x.pdf' });
  assert.equal(saved.isError, false, saved.text);
  assert.equal(saved.step.state.save.ok, true, saved.text);
  const written = join(out, 'sub/deep/x.pdf');
  const doc = mupdf.Document.openDocument(await readFile(written), 'application/pdf');
  assert.match(doc.loadPage(0).toStructuredText().asText(), /Deep save/);
  const escape = await call('tandem_save', { path: '../escape.pdf' });
  assert.equal(escape.isError, true, escape.text);
  assert.equal(escape.text, 'tandem_save: path must stay inside the output folder');
  assert.equal(existsSync(join(parent, 'escape.pdf')), false, 'nothing is written outside --out');
  child.stdin.end();
  assert.equal(await exited, 0);
});

async function drive(script, timeoutMs = 90_000) {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-drive-test-'));
  const report = join(dir, 'report.json');
  const child = spawn(process.execPath, [join(root, 'scripts/drive.mjs'), script, `--out=${dir}`, `--report=${report}`], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => { stdout += chunk; });
  const timer = setTimeout(() => child.kill(), timeoutMs);
  const code = await new Promise(resolve => child.on('close', resolve));
  clearTimeout(timer);
  return { code, report: JSON.parse(stdout), dir, stdout };
}
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, save: step.state?.save, name: step.state?.doc?.name, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error ?? step.error });
const newFolders = async () => new Set((await readdir(tmpdir())).filter(name => name.startsWith('pdf4all-new-')));

test('(e) B3+B5 drive: Save on Untitled asks for a path and rebinds; undo/redo clear the error bar', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const SCRIPT = 'tests/drive/bugfix1.jsonl';
  const text = await readFile(join(root, SCRIPT), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  assert.deepEqual([Number(header[1]), Number(header[2])], [11, 0]);
  const before = await newFolders();
  const { code, report, dir } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 11, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);

  const made = mupdf.Document.openDocument(await readFile(join(dir, 'made.pdf')), 'application/pdf');
  const words = made.loadPage(0).toStructuredText().asText();
  assert.match(words, /First words/);
  assert.match(words, /Second words/);
  // Save never wrote into the temp folder: its Untitled.pdf is still a blank letter page.
  const created = [...await newFolders()].filter(name => !before.has(name));
  assert.ok(created.length >= 1, 'the drive made a pdf4all-new-* folder');
  const blank = new Set([blankPdf(612, 792).byteLength, blankPdf(595.276, 841.89).byteLength]);
  for (const name of created) {
    const untitled = join(tmpdir(), name, 'Untitled.pdf');
    if (!existsSync(untitled)) continue;
    assert.ok(blank.has((await stat(untitled)).size), `${untitled} is still a blank document`);
  }
});
