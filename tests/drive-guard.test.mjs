/**
 * Drive guard (contract-drive-guard.md D5): an unknown drive cmd and a `content` object with
 * none of text/find/action are refused in STATE `error`, from a drive file and over MCP.
 * drive()/red()/brief() are copies of tests/p3-coded-drive.test.mjs; the MCP helper is a copy of
 * tests/agent-session.test.mjs test (d) (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync, spawn } from 'node:child_process';
import { mkdtemp as makeTempDir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import { appBinary } from './app-bin.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
/** Child processes of `ppid` (pids): the Rust server runs its session in-process, so none. */
const childrenOf = ppid => { try { return (process.platform === 'win32' ? execFileSync('powershell', ['-NoProfile', '-Command', `Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq ${Number(ppid)} } | ForEach-Object { $_.ProcessId }`], { encoding: 'utf8' }) : execFileSync('pgrep', ['-P', String(ppid)], { encoding: 'utf8' })).trim().split(/\s+/).filter(Boolean).map(Number); } catch { return []; } };
const SCRIPT = 'tests/drive/drive-guard.jsonl';
const FIXTURE = 'fixtures/p0b/edit-transformed.pdf';
const UNKNOWN = cmd => `Unknown command "${cmd}"`;
const EMPTY_CONTENT = 'content: give text, find or action';

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

const sha = async path => createHash('sha256').update(await readFile(join(root, path))).digest('hex');
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, contents: step.contents, edited: step.state?.content?.edited, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error ?? step.error });

test('drive guard (a): drive-guard.jsonl is green; unknown cmds and an empty content object are refused', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const before = await sha(FIXTURE);
  const text = await readFile(join(root, SCRIPT), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.deepEqual([Number(header[1]), Number(header[2])], [11, 0]);
  assert.equal(lines.length, 11);

  const { code, report } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 11, report.error ?? '');
  assert.equal(steps[10].state.error, 'content: give text, find or action', 'a plain string is refused (§4.3)');
  assert.equal(steps[10].state.revision, 1, 'a plain string edits nothing');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  assert.equal(steps[1].state.error, UNKNOWN('nosuch'));
  assert.equal(steps[3].state.error, UNKNOWN('feilds'));
  for (const i of [4, 5, 8]) {
    assert.equal(steps[i].state.error, EMPTY_CONTENT, JSON.stringify(brief(steps[i])));
    assert.equal(steps[i].state.revision, 0);
    assert.equal(steps[i].state.content.edited, null);
  }
  assert.equal(steps[5].cmd, 'content');
  assert.equal(steps[5].contents, undefined, 'step 6 is a content line with no contents');
  assert.match(steps[6].state.error, /^content: /, 'a content string that is not JSON is refused');
  assert.notEqual(steps[6].state.error, EMPTY_CONTENT);
  assert.equal(steps[6].state.revision, 0);
  assert.equal(steps[7].state.search.count, 1, 'the self-test did not run: ALPHA is intact');
  assert.equal(steps[9].state.error, '', 'a well-formed content step clears the stale refusal');
  assert.equal(await sha(FIXTURE), before, 'fixture unchanged');
});

test('drive guard (b): over MCP an unknown cmd and an empty content object come back isError', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
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
    return { isError: message.result.isError, text: message.result.content[0].text, step: JSON.parse(message.result.content[0].text) };
  };

  try {
    const init = await rpc('initialize', { protocolVersion: '2025-03-26', capabilities: {}, clientInfo: { name: 'test', version: '0' } });
    assert.equal(init.result.serverInfo.name, 'tandem-pdf');
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
    const opened = await call('tandem_open', { path: FIXTURE });
    assert.equal(opened.isError, false, JSON.stringify(brief(opened.step)));
    assert.deepEqual(childrenOf(child.pid), [], 'the session runs inside the server: no child app');
    const unknown = await call('tandem_do', { cmd: 'nosuch' });
    assert.equal(unknown.isError, true, JSON.stringify(brief(unknown.step)));
    assert.match(unknown.text, /Unknown command/);
    assert.equal(unknown.step.state.revision, 0);
    const empty = await call('tandem_do', { cmd: 'content', contents: {} });
    assert.equal(empty.isError, true, JSON.stringify(brief(empty.step)));
    assert.equal(empty.step.state.error, EMPTY_CONTENT);
    assert.equal(empty.step.state.revision, 0);
    assert.equal(empty.step.state.dirty, false);
    const bare = await call('tandem_do', { cmd: 'content' });
    assert.equal(bare.isError, true, JSON.stringify(brief(bare.step)));
    assert.equal(bare.step.state.error, EMPTY_CONTENT);
    assert.equal(bare.step.state.revision, 0);
    child.stdin.end();
    assert.equal(await exited, 0);
  } finally {
    // A failed assertion must not leave the server (and its session) running: EOF closes both.
    if (child.exitCode === null) { child.stdin.end(); await exited; }
  }
});
