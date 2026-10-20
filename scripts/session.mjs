// A live drive session (contract-agent-session.md D2): one app on `--drive=-`, one line in, one
// STEP out. `createSession()` → `{ step(line), close() }`; see docs/agent.md. The app is the Rust
// binary: TANDEM_APP_BIN if set, else crates/target/release/tandem-pdf (`appBinary()`).
// No dependency beyond node builtins.
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

/** The release build's path in this repository (`cd crates && cargo build --release -p tandem-desktop`). */
export const defaultAppBinary = join(root, 'crates/target/release', process.platform === 'win32' ? 'tandem-pdf.exe' : 'tandem-pdf');

/** The app binary the scripts and tests drive: TANDEM_APP_BIN (resolved from the cwd) if set, else the release build. */
export function appBinary() {
  return process.env.TANDEM_APP_BIN ? resolve(process.env.TANDEM_APP_BIN) : defaultAppBinary;
}

/** Fold contract click/type/key/drag/handle fields into `contents` when absent (the app does the same; idempotent). */
export function packDrive(text) {
  return text.split('\n').map(line => {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) return line;
    let row;
    try { row = JSON.parse(trimmed); } catch { return line; }
    if (!row || typeof row !== 'object' || row.contents !== undefined) return line;
    if (!['click', 'type', 'key', 'drag', 'handle'].includes(row.cmd)) return line;
    const contents = {};
    // P6 U2 §3.10: `alt` on click, `dx`/`dy` on drag and handle, `handle` names the grip or side; P6 U3 §9.8: `shift` frees the aspect ratio.
    for (const key of ['page', 'x', 'y', 'text', 'key', 'alt', 'dx', 'dy', 'handle', 'escape', 'shift', 'meta']) if (row[key] !== undefined) contents[key] = row[key];
    return Object.keys(contents).length ? JSON.stringify({ ...row, contents }) : line;
  }).join('\n');
}

/**
 * Spawns the app on `--drive=-` (fresh userData under a mkdtemp; `out` is `${OUT}`, default
 * `<tmp>/out`). `step(line)` sends one drive line (object or one-line JSON string) and resolves
 * with its STEP record `{cmd, …, state, expect?, ok?}` or `{error}`; calls queue, one in flight.
 * `close()` sends `{"cmd":"close"}`, waits for the exit and resolves with the `--report` JSON.
 * Once the app exits (however the session ends) the temp dir is removed, `out` with it unless given.
 */
export async function createSession({ out, gui = false, report } = {}) {
  const app = appBinary();
  if (!existsSync(app)) throw new Error(`no app at ${app}: build it (cd crates && cargo build --release -p tandem-desktop) or set TANDEM_APP_BIN`);
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-session-'));
  const outDir = resolve(out ?? join(dir, 'out'));
  const reportPath = resolve(report ?? join(dir, 'report.json'));
  await mkdir(outDir, { recursive: true });
  const child = spawn(app, [
    `--user-data-dir=${join(dir, 'udata')}`, '--drive=-', `--drive-out=${outDir}`, `--report=${reportPath}`,
    ...(gui ? ['--gui'] : []),
  ], { cwd: root, stdio: ['pipe', 'pipe', 'inherit'] });
  child.stdin.on('error', () => { /* the app exited; the pending step rejects on 'close' */ });
  let ended = false;
  let waiter;
  // Resolves with the report text (undefined if none), read before the temp dir goes.
  const exited = new Promise(done => child.on('close', async code => {
    ended = true;
    waiter?.reject(new Error(`session ended (exit ${code})`));
    waiter = undefined;
    const text = await readFile(reportPath, 'utf8').catch(() => undefined);
    await rm(dir, { recursive: true, force: true, maxRetries: 5 }).catch(() => {});
    done(text);
  }));
  createInterface({ input: child.stdout, crlfDelay: Infinity }).on('line', line => {
    if (!line.startsWith('STEP ')) return;
    const current = waiter;
    waiter = undefined;
    try { current?.resolve(JSON.parse(line.slice(5))); } catch (err) { current?.reject(err); }
  });
  const send = text => new Promise((resolveStep, reject) => {
    if (ended) return reject(new Error('session ended'));
    waiter = { resolve: resolveStep, reject };
    child.stdin.write(`${text}\n`);
  });
  let chain = Promise.resolve();
  return {
    out: outDir,
    dir,
    child,
    get ended() { return ended; },
    step(line) {
      const text = typeof line === 'string' ? line.trim() : JSON.stringify(line);
      if (!text || text.startsWith('#') || text.includes('\n')) return Promise.reject(new Error('step: one non-blank, non-comment line'));
      const run = chain.then(() => send(packDrive(text)));
      chain = run.catch(() => undefined);
      return run;
    },
    async close() {
      await chain;
      if (!ended) child.stdin.end('{"cmd":"close"}\n');
      const text = await exited;
      if (text === undefined) throw new Error(`no report at ${reportPath}`);
      return JSON.parse(text);
    },
  };
}
