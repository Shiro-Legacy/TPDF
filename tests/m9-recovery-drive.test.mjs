// M9 recovery (engine/design/app-m9-recovery.md) in the app: the recovery log the session keeps over opens,
// edits, saves and an unlock (tests/drive/recovery-log.jsonl), read back from STATE. The restarts and replays
// themselves (an engine that stops mid-request, a replay that fails, the budget, saves in place) are Rust tests:
// crates/tandem-app/tests/recovery.rs. The in-process engine has no request watchdog, so the Electron hang
// tests (a patched copy of build/ with short timeouts) have no counterpart.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;

const summary = result => JSON.stringify(result.steps?.map(step => ({ cmd: step.cmd, ok: step.ok, doc: step.state?.doc?.name ?? null, notes: step.state?.notes, revision: step.state?.revision, error: step.state?.error, recovery: step.state?.recovery })) ?? result);

test('drive script: the recovery log over opens, edits, saves and an unlock', { skip: CI_NO_ELECTRON, timeout: 120_000 }, async () => {
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'tpdf-m9-log-')));
  try {
    const run = spawnSync(process.execPath, ['scripts/drive.mjs', 'tests/drive/recovery-log.jsonl', `--out=${dir}`], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 });
    const report = JSON.parse(run.stdout);
    assert.equal(report.ok, true, summary(report));
    assert.equal(run.status, 0);
    assert.ok(!run.stdout.includes('pdf4all-user'), 'the password is in no STATE');
  } finally {
    await rm(dir, { recursive: true, force: true, maxRetries: 5 });
  }
});
