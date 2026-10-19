/**
 * Phase 1 UX quick wins (docs/rust-and-ux-plan.md §3, §4 row 1), driven through the real app:
 * tests/drive/p1-ux.jsonl. drive()/red()/brief() are copies of tests/drive-guard.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp as makeTempDir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';

const root = fileURLToPath(new URL('..', import.meta.url));
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;

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
const brief = step => ({ cmd: step.cmd, contents: step.contents, tool: step.state?.tool, active: step.state?.direct?.active, revision: step.state?.revision, error: step.state?.error ?? step.error, announcement: step.state?.announcement });

test('p1-ux drive: armed tools show a chip, Escape stops them, stray clicks add nothing', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const { code, report } = await drive('tests/drive/p1-ux.jsonl', 150_000);
  assert.equal(red(report.steps).length, 0, JSON.stringify(red(report.steps).map(brief)));
  assert.equal(code, 0);
  // Item 5: the selection's quad covers the same word as the search match's.
  const bySearch = report.steps.find(step => step.cmd === 'highlight').state.markup.last.quads[0];
  const bySelection = report.steps.find(step => step.cmd === 'strikeout').state.markup.last.quads[0];
  for (const i of [0, 2, 4, 6]) assert.ok(Math.abs(bySearch[i] - bySelection[i]) < 1, `x ${i}: ${bySearch} vs ${bySelection}`);
  for (const i of [1, 3, 5, 7]) assert.ok(Math.abs(bySearch[i] - bySelection[i]) < 8, `y ${i}: ${bySearch} vs ${bySelection}`);
  // Item 6: the Comments list names a markup by the words it marks, and a note by its text.
  const comments = report.steps.at(-1).state.comments;
  assert.ok(comments.some(item => item.type === 'StrikeOut' && item.label === 'Strikethrough: ‘PDF4All’'), JSON.stringify(comments));
  assert.ok(comments.some(item => item.type === 'Highlight' && item.label === 'Highlight: ‘PDF4All’'), JSON.stringify(comments));
  assert.ok(comments.some(item => item.type === 'Text' && item.contents === 'Check this figure' && item.label === 'Note: Check this figure'), JSON.stringify(comments));
});
