#!/usr/bin/env node
// Replay a drive script against the built app (design-agent-drive.md).
// Usage: node scripts/drive.mjs <script.jsonl> [--gui] [--out=<dir>] [--report=<path>] [--app=<binary>]
//    or: node scripts/drive.mjs --session [--gui] [--out=<dir>] [--report=<path>]  (drive lines on stdin,
//        one result JSON line out per line in; the report at stdin EOF; contract-agent-session.md D2)
// Prints the report (one JSON object: steps with the STATE after each command) and exits 0
// iff every `expect` matched. Needs `npm run build` first; no other Electron on this userData.
// `--app=<binary>` drives a packaged app instead (its executable, e.g. `out/Tandem PDF-darwin-arm64/Tandem PDF.app/
// Contents/MacOS/Tandem PDF`), still from the repository folder, so a script's relative fixture paths resolve the same.
// `TPDF_DRIVE_APP` sets the same default, so a test that runs this script drives the packaged app (M10 package check).
// The per-run temp dir (userData, and out/report unless --out/--report) is removed once Electron
// has exited, on success, failure and SIGINT/SIGTERM/SIGHUP: pass --out to keep ${OUT} files.
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline';
import { createSession, packDrive } from './session.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const electron = createRequire(import.meta.url)('electron');
const SIGNALS = ['SIGINT', 'SIGTERM', 'SIGHUP'];
const args = process.argv.slice(2);
const script = args.find(arg => !arg.startsWith('--'));
if (args.includes('--session') && !script) {
  if (!existsSync(join(root, 'build/main.js'))) { console.error('build/ missing: run `npm run build` first'); process.exit(2); }
  const option = name => args.find(arg => arg.startsWith(`${name}=`))?.slice(name.length + 1);
  const session = await createSession({ out: option('--out'), report: option('--report'), gui: args.includes('--gui') });
  // session.close() removes the session's temp dir after Electron exits; a signal ends the session first.
  for (const signal of SIGNALS) process.on(signal, () => { session.child.kill(signal); session.close().catch(() => {}).finally(() => process.exit(1)); });
  for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
    const text = line.trim();
    if (!text || text.startsWith('#')) continue;
    let result;
    try { result = await session.step(text); } catch (err) { console.log(JSON.stringify({ error: err.message })); break; }
    console.log(JSON.stringify(result));
  }
  let code = 1;
  try {
    const report = await session.close();
    code = report.ok ? 0 : 1;
    await new Promise(done => process.stdout.write(JSON.stringify(report, null, 2) + '\n', done));
  } catch { console.error('no report written'); }
  process.exit(code);
}
if (!script) { console.error('usage: node scripts/drive.mjs <script.jsonl> [--gui] [--out=<dir>] [--report=<path>] [--app=<binary>]'); process.exit(2); }
const value = name => args.find(arg => arg.startsWith(`${name}=`))?.slice(name.length + 1);
// TANDEM_APP_BIN runs the scripts against another app binary (the Rust Mac app, crates/tandem-desktop).
const packaged = value('--app') ?? (process.env.TANDEM_APP_BIN || process.env.TPDF_DRIVE_APP || undefined);
if (packaged !== undefined && !existsSync(packaged)) { console.error(`--app: no such file: ${packaged}`); process.exit(2); }
if (packaged === undefined && !existsSync(join(root, 'build/main.js'))) { console.error('build/ missing: run `npm run build` first'); process.exit(2); }
const dir = await mkdtemp(join(tmpdir(), 'pdf4all-drive-'));
try {
  const out = resolve(value('--out') ?? join(dir, 'out'));
  const report = resolve(value('--report') ?? join(dir, 'report.json'));
  await mkdir(out, { recursive: true });
  // main forwards only `contents`. Contract click/type/key keep page/x/y/text/key beside cmd.
  const packed = join(dir, 'script.jsonl');
  await writeFile(packed, packDrive(await readFile(resolve(script), 'utf8')));
  const child = spawn(packaged === undefined ? electron : resolve(packaged), [
    `--user-data-dir=${join(dir, 'udata')}`, ...(packaged === undefined ? ['.'] : []), `--drive=${packed}`, `--drive-out=${out}`, `--report=${report}`,
    ...(args.includes('--gui') ? ['--gui'] : []),
  ], { cwd: root, stdio: ['ignore', 'ignore', 'inherit'] });
  // A signal stops Electron; its 'close' then runs the cleanup below instead of orphaning it.
  // Electron quits 0 on SIGTERM, so a signalled run exits 1 itself (never green).
  let signalled = false;
  for (const signal of SIGNALS) process.on(signal, () => { signalled = true; child.kill(signal); });
  const code = await new Promise(done => child.on('close', done));
  try { process.stdout.write(await readFile(report, 'utf8')); } catch { console.error('no report written'); }
  // `exitCode`, not `exit()`: a report over one pipe buffer (64 KiB) would otherwise be
  // truncated before the write drains, and the driver reads this report from stdout.
  process.exitCode = signalled ? 1 : code ?? 1;
} finally {
  // A leftover dir must not turn a green run red (session.mjs swallows it the same way).
  await rm(dir, { recursive: true, force: true, maxRetries: 5 }).catch(err => console.error(`temp dir not removed: ${err.message}`));
}
