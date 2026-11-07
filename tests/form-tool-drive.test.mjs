/**
 * Design form, phase 1: tests/drive/form-tool.jsonl places every kind of field with the Form tool (click and drag),
 * builds a group of choice buttons a click at a time, undoes and redoes, saves and reopens. The script holds the
 * exact-value expects; this runs it and reports the red steps.
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = fileURLToPath(new URL('..', import.meta.url));
const NO_APP = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;

test('drive: the Form tool places, names and saves fields', { skip: NO_APP, timeout: 120_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'tandem-form-tool-'));
  try {
    const report = join(dir, 'report.json');
    const child = spawn(process.execPath, [join(root, 'scripts/drive.mjs'), 'tests/drive/form-tool.jsonl', `--out=${dir}`, `--report=${report}`], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => { stdout += chunk; });
    const code = await new Promise(resolve => child.on('close', resolve));
    const result = JSON.parse(stdout);
    const red = result.steps.filter(step => step.expect !== undefined && !step.ok)
      .map(step => ({ cmd: step.cmd, line: step.line, diff: step.diff, error: step.state?.error }));
    assert.deepEqual(red, [], JSON.stringify(red, null, 1));
    assert.equal(code, 0);
  } finally {
    await rm(dir, { recursive: true, force: true, maxRetries: 5 });
  }
});
