/**
 * Phase 1 UX quick wins (docs/rust-and-ux-plan.md §3, §4 row 1), driven through the real app:
 * tests/drive/p1-ux.jsonl. drive()/red()/brief() are copies of tests/drive-guard.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp as makeTempDir, readFile, rm } from 'node:fs/promises';
import mupdf from 'mupdf';
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

const notesIn = async path => mupdf.Document.openDocument(await readFile(path), 'application/pdf').loadPage(0).getAnnotations().filter(annot => annot.getType() === 'Text').map(annot => annot.getContents()).sort();
const fieldIn = async (path, name) => mupdf.Document.openDocument(await readFile(path), 'application/pdf').loadPage(0).getWidgets().find(widget => widget.getName() === name)?.getValue();

test('p1-ux drive: armed tools, Escape, stray clicks, the catalogue, markup from a selection, notes, Save As and the filled-form question', { skip: CI_NO_ELECTRON, timeout: 240_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/p1-ux.jsonl', 200_000);
  assert.equal(red(report.steps).length, 0, JSON.stringify(red(report.steps).map(brief)));
  assert.equal(code, 0);
  // Item 5: the selection's quad covers the same word as the search match's.
  const bySearch = report.steps.find(step => step.cmd === 'highlight').state.markup.last.quads[0];
  const bySelection = report.steps.find(step => step.cmd === 'strikeout').state.markup.last.quads[0];
  for (const i of [0, 2, 4, 6]) assert.ok(Math.abs(bySearch[i] - bySelection[i]) < 1, `x ${i}: ${bySearch} vs ${bySelection}`);
  for (const i of [1, 3, 5, 7]) assert.ok(Math.abs(bySearch[i] - bySelection[i]) < 8, `y ${i}: ${bySearch} vs ${bySelection}`);
  // Item 6: the Comments list names a markup by the words it marks, and a note by its text.
  const typed = report.steps.findIndex(step => step.cmd === 'type' && step.text === 'Check this figure');
  const labels = report.steps[typed + 1].state.commentLabels;
  for (const label of ['Strikethrough: ‘PDF4All’', 'Highlight: ‘PDF4All’', 'Note: Check this figure']) assert.ok(labels.includes(label), JSON.stringify(labels));
  // Item 7: the original kept nothing of the edits after Save As; the copy holds the first; the renamed file both.
  assert.deepEqual(await notesIn(join(dir, 'orig.pdf')), []);
  assert.deepEqual(await notesIn(join(dir, 'copy.pdf')), ['edit one']);
  assert.deepEqual(await notesIn(join(dir, 'renamed.pdf')), ['edit one', 'edit two']);
  // Item 8: "Save Over" wrote the filled value into the form itself, and no "(filled)" copy was made.
  assert.equal(await fieldIn(join(dir, 'myform.pdf'), 'name'), 'Ada Lovelace');
  await assert.rejects(readFile(join(dir, 'myform (filled).pdf')));
});
