/**
 * Agent session (contract-agent-session.md D7): the stdin session (`--drive=-`, STEP lines,
 * `close`), scripts/session.mjs, the blank document (`new`, src/blank-pdf.ts), the batch
 * script tests/drive/agent-session.jsonl in both modes, and the zero-dependency MCP server.
 * drive()/red()/brief() are copies of tests/p3-coded-drive.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp as makeTempDir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test, { after } from 'node:test';
import { ALT_APP, appBinary } from './app-bin.mjs';
import mupdf from 'mupdf';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
/** The MCP server is the Rust app's `--mcp` mode (TANDEM_APP_BIN names the binary). */
const MCP_SKIP = CI_NO_ELECTRON || (ALT_APP ? false : 'needs TANDEM_APP_BIN (the MCP server is the Rust app\'s --mcp mode)');
/** Child processes of `ppid` (pids): the Rust server runs its session in-process, so none. */
const childrenOf = ppid => { try { return (process.platform === 'win32' ? execFileSync('powershell', ['-NoProfile', '-Command', `Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq ${Number(ppid)} } | ForEach-Object { $_.ProcessId }`], { encoding: 'utf8' }) : execFileSync('pgrep', ['-P', String(ppid)], { encoding: 'utf8' })).trim().split(/\s+/).filter(Boolean).map(Number); } catch { return []; } };
const SCRIPT = 'tests/drive/agent-session.jsonl';
const A4 = [0, 0, 595.276, 841.89];

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
const brief = step => ({ cmd: step.cmd, contents: step.contents, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error ?? step.error });
const near = (got, want, label) => assert.ok(Array.isArray(got) && got.length === want.length && got.every((v, i) => Math.abs(v - want[i]) <= 0.01), `${label}: ${JSON.stringify(got)} vs ${JSON.stringify(want)}`);
const sessionModule = () => import(pathToFileURL(join(root, 'scripts/session.mjs')).href);
/** A failed assertion must not leave the session's Electron running: EOF finishes it. */
const ending = session => { if (!session.ended) session.child.stdin.end(); };
/** Windows has no pgrep: the same match over the Win32_Process command lines (pids, one per line; never the query's own shell). */
const WIN = process.platform === 'win32';
const winPids = where => execFileSync('powershell', ['-NoProfile', '-Command', `Get-CimInstance Win32_Process | Where-Object { $_.ProcessId -ne $PID -and ${where} } | ForEach-Object { $_.ProcessId }`], { encoding: 'utf8' }).trim();
/** This session's own `--drive=-` Electron (matched by its unique user-data-dir), never a parallel test file's. */
const sessionDrives = session => { try { return WIN ? winPids(`$_.CommandLine -like '*${join(session.dir, 'udata')} *--drive=-*'`) : execFileSync('pgrep', ['-f', '--', `${session.dir}/udata .*--drive=-`], { encoding: 'utf8' }).trim(); } catch { return ''; } };

/** One saved PDF read back: page count, first page bounds, widgets {name: value}, page text. */
function readPdf(bytes) {
  const doc = mupdf.PDFDocument.openDocument(bytes, 'application/pdf');
  try {
      const page = doc.loadPage(0);
      try {
        const widgets = Object.fromEntries((page.getWidgets() ?? []).map(widget => [widget.getName(), widget.getValue()]));
        const structured = page.toStructuredText();
        const text = structured.asText();
        structured.destroy();
        return { pages: doc.countPages(), bounds: page.getBounds(), widgets, text, repaired: typeof doc.wasRepaired === 'function' ? doc.wasRepaired() : false };
      } finally {
        page.destroy();
      }
  } finally {
    doc.destroy();
  }
}

test('agent session (a): a stdin session interleaves: the second line is built from the first STATE', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async t => {
  const { createSession } = await sessionModule();
  const session = await createSession();
  t.after(() => ending(session));
  const times = [];
  const sent = async line => { const at = Date.now(); const result = await session.step(line); times.push([at, Date.now()]); return result; };
  const opened = await sent({ cmd: 'open', path: 'fixtures/form.pdf' });
  assert.equal(opened.state?.doc?.name, 'form.pdf', JSON.stringify(brief(opened)));
  const name = Object.entries(opened.state.fields.defs).find(([, def]) => def.kind === 'text')?.[0];
  assert.ok(name, 'form.pdf has a text field in STATE fields.defs');
  const filled = await sent({ cmd: 'fields', contents: { action: 'fill', name, value: 'Ada' } });
  assert.equal(filled.state.fields.values[name], 'Ada');
  assert.equal(filled.state.dirty, true);
  assert.ok(times[0][1] <= times[1][0], 'the first result arrived before the second line was written');
  const report = await session.close();
  assert.equal(report.ok, true, report.error ?? '');
  assert.deepEqual(report.steps.map(step => step.cmd), ['open', 'fields']);
  assert.equal(report.steps[1].state.fields.values[name], 'Ada');
  assert.equal(sessionDrives(session), '', 'no --drive=- Electron outlives close()');
});

test('agent session (b): a bad stdin line is reported and the session keeps answering', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async t => {
  const { createSession } = await sessionModule();
  const session = await createSession();
  t.after(() => ending(session));
  const bad = await session.step('{nope');
  assert.match(String(bad.error), /^bad line: /);
  const state = await session.step({ cmd: 'state' });
  assert.equal(typeof state.state?.revision, 'number', JSON.stringify(brief(state)));
  assert.notEqual(sessionDrives(session), '', 'the straggler check finds this session while it runs');
  const report = await session.close();
  assert.equal(report.ok, true);
  assert.equal(sessionDrives(session), '');
});

test('agent session (c): agent-session.jsonl is green as a file script and over a session; created.pdf reads back', { skip: CI_NO_ELECTRON, timeout: 300_000 }, async t => {
  const text = await readFile(join(root, SCRIPT), 'utf8');
  const header = text.match(/(\d+) lines, (\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the line, step and click counts');
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.deepEqual([Number(header[1]), Number(header[2]), Number(header[3])], [8, 7, 0]);
  assert.equal(lines.length, 8);
  assert.equal(JSON.parse(lines.at(-1)).cmd, 'close');

  const { code, report, dir } = await drive(SCRIPT, 150_000);
  assert.equal(report.steps.length, 7, report.error ?? '');
  assert.equal(red(report.steps).length, 0, JSON.stringify(red(report.steps).map(brief)));
  assert.equal(code, 0);
  assert.ok(dir.startsWith(tmpdir()));
  const saved = readPdf(await readFile(join(dir, 'created.pdf')));
  assert.equal(saved.pages, 1);
  near(saved.bounds, A4, 'A4 page bounds');
  assert.equal(saved.widgets.customer, 'Ada');
  assert.ok(saved.text.includes('Invoice 42'), saved.text);

  const { createSession } = await sessionModule();
  const out = await mkdtemp(join(tmpdir(), 'pdf4all-session-test-'));
  const session = await createSession({ out });
  t.after(() => ending(session));
  for (const line of lines.slice(0, -1)) {
    const step = await session.step(line);
    assert.equal(step.ok, true, JSON.stringify(brief(step)));
  }
  const sessionReport = await session.close();
  assert.equal(sessionReport.ok, true);
  assert.equal(sessionReport.steps.length, 7);
  assert.equal(readPdf(await readFile(join(out, 'created.pdf'))).widgets.customer, 'Ada');
  assert.equal(sessionDrives(session), '');
});

test('agent session (d): the MCP server speaks newline JSON-RPC and drives a session', { skip: MCP_SKIP, timeout: 180_000 }, async () => {
  const out = await mkdtemp(join(tmpdir(), 'tandem-mcp-test-'));
  const child = spawn(appBinary, ['--mcp', `--out=${out}`], { cwd: root, stdio: ['pipe', 'pipe', 'inherit'] });
  const exited = new Promise(resolve => child.on('close', resolve));
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
    return { isError: message.result.isError, step: JSON.parse(message.result.content[0].text) };
  };

  try {
  const init = await rpc('initialize', { protocolVersion: '2025-03-26', capabilities: {}, clientInfo: { name: 'test', version: '0' } });
  assert.equal(init.result.serverInfo.name, 'tandem-pdf');
  assert.equal(init.result.protocolVersion, '2025-03-26');
  assert.deepEqual(init.result.capabilities, { tools: {} });
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
  const list = await rpc('tools/list');
  assert.deepEqual(list.result.tools.map(tool => tool.name).sort(), ['tandem_do', 'tandem_help', 'tandem_new', 'tandem_open', 'tandem_save', 'tandem_state']);
  for (const tool of list.result.tools) assert.equal(tool.inputSchema.type, 'object', tool.name);

  const created = await call('tandem_new', { size: 'letter' });
  assert.equal(created.isError, false, JSON.stringify(brief(created.step)));
  assert.equal(created.step.state.doc.name, 'Untitled.pdf');
  assert.deepEqual(childrenOf(child.pid), [], 'the session runs inside the server: no child app');
  const added = await call('tandem_do', { cmd: 'text-add', contents: { page: 0, point: [72, 700], text: 'MCP 7', font: 'Helvetica', size: 12, color: [0, 0, 0] } });
  assert.equal(added.isError, false, JSON.stringify(brief(added.step)));
  assert.equal(added.step.state.dirty, true);
  const saved = await call('tandem_save', { path: 'mcp.pdf' });
  assert.equal(saved.isError, false, JSON.stringify(brief(saved.step)));
  assert.ok(existsSync(join(out, 'mcp.pdf')), 'tandem_save with a relative path lands under --out');
  const back = readPdf(await readFile(join(out, 'mcp.pdf')));
  near(back.bounds, [0, 0, 612, 792], 'letter page bounds');
  assert.ok(back.text.includes('MCP 7'), back.text);
  const refused = await rpc('tools/call', { name: 'tandem_do', arguments: { cmd: 'close' } });
  assert.equal(refused.result.isError, true);
  const help = await rpc('tools/call', { name: 'tandem_help', arguments: { topic: 'fill' } });
  assert.equal(help.result.isError, false);
  assert.match(help.result.content[0].text, /fields/);
  const unknown = await rpc('tools/call', { name: 'nosuch', arguments: {} });
  assert.equal(unknown.error.code, -32602);
  const method = await rpc('nosuch/method');
  assert.equal(method.error.code, -32601);
  child.stdin.write('{garbage\n');
  const garbage = await reply(null);
  assert.equal(garbage.error.code, -32700);
  assert.equal(garbage.id, null);
  assert.deepEqual((await rpc('ping')).result, {});
  // A long step does not hold up the answers that need no session.
  const order = [];
  const waited = rpc('tools/call', { name: 'tandem_do', arguments: { cmd: 'wait', ms: 1500 } }).then(m => { order.push('wait'); return m; });
  const pinged = rpc('ping').then(m => { order.push('ping'); return m; });
  await Promise.all([waited, pinged]);
  assert.deepEqual(order, ['ping', 'wait']);
  child.stdin.end();
  assert.equal(await exited, 0);
  } finally {
    // A failed assertion must not leave the server (and its session) running: EOF closes both.
    if (child.exitCode === null) { child.stdin.end(); await exited; }
  }
});

test('agent session (f): steps sent at once queue in order; secrets read [redacted] in STEP; a leaked secret fails the run', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async t => {
  // The fixture password is read from the existing script at run time, never copied into this file.
  const unlock = (await readFile(join(root, 'tests/drive/password-pages.jsonl'), 'utf8')).split('\n').map(line => line.trim()).filter(line => line.startsWith('{')).map(line => JSON.parse(line)).find(line => line.cmd === 'password');
  const { createSession } = await sessionModule();
  const session = await createSession();
  t.after(() => ending(session));
  const [opened, unlocked, state] = await Promise.all([
    session.step({ cmd: 'open', path: 'fixtures/encrypted.pdf' }),
    session.step(unlock),
    session.step({ cmd: 'state' }),
  ]);
  assert.deepEqual([opened.cmd, unlocked.cmd, state.cmd], ['open', 'password', 'state']);
  assert.equal(opened.state.doc.needsPassword, true);
  assert.equal(unlocked.contents, '[redacted]');
  assert.equal(unlocked.state.doc.needsPassword, false, JSON.stringify(brief(unlocked)));
  assert.ok(!JSON.stringify([opened, unlocked, state]).includes(unlock.contents), 'no STEP carries the password');
  await assert.rejects(session.step({ cmd: 'find', contents: { query: unlock.contents } }), /session ended/);
  const report = await session.close();
  assert.equal(report.ok, false);
  assert.equal(report.error, 'password leaked in STATE');
  assert.ok(!JSON.stringify(report).includes(unlock.contents), 'the report never carries the password');
  assert.equal(sessionDrives(session), '');
});

test('agent session (e): blankPdf writes one page with the asked MediaBox and a valid xref', async () => {
  const { blankPdf } = await import(pathToFileURL(join(root, 'build/blank-pdf.js')).href);
  for (const [w, h] of [[612, 792], [595.276, 841.89], [72, 14400]]) {
    const bytes = blankPdf(w, h);
    assert.ok(bytes instanceof Uint8Array);
    assert.equal(Buffer.from(bytes.subarray(0, 8)).toString('latin1'), '%PDF-1.7');
    const read = readPdf(bytes);
    assert.equal(read.pages, 1);
    near(read.bounds, [0, 0, w, h], `MediaBox ${w}x${h}`);
    assert.equal(read.repaired, false, 'opens without repair');
    assert.deepEqual(read.widgets, {});
    // Every xref offset points at its `n 0 obj`.
    const text = Buffer.from(bytes).toString('latin1');
    const start = Number(text.match(/startxref\s+(\d+)/)[1]);
    assert.ok(text.startsWith('xref', start));
    const offsets = [...text.slice(start).matchAll(/^(\d{10}) 00000 n/gm)].map(m => Number(m[1]));
    assert.ok(offsets.length >= 3);
    offsets.forEach((offset, i) => assert.ok(text.startsWith(`${i + 1} 0 obj`, offset), `object ${i + 1} at ${offset}`));
  }
});
