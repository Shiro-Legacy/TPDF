/**
 * Coded simple fonts drive (contract-coded.md D5): tests/drive/coded-edit.jsonl holds the
 * exact-value expects; this test pins the step/click counts and reads the saved file back.
 * drive()/pick()/near()/red() are copies of tests/p6-drive8.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdtemp as makeTempDir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import mupdf from 'mupdf';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const NO_APP = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;
const SCRIPT = 'tests/drive/coded-edit.jsonl';
const FIXTURE = 'fixtures/p0b/edit-coded.pdf';
const FIXTURE_SHA256 = '8f6b3bfaf4eedf39003e107dbbabbea48f834c3be4b8ab489aa0e29be48191bf';

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
const brief = step => ({ cmd: step.cmd, contents: step.contents, edited: step.state?.content?.edited, refused: step.state?.content?.refused, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error });

test('drive: coded simple font edit, z/x/overflow refusals, undo/redo/save-to', { skip: NO_APP, timeout: 300_000 }, async () => {
  assert.equal(await sha(FIXTURE), FIXTURE_SHA256, 'fixture before');
  const text = await readFile(join(root, SCRIPT), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.deepEqual([Number(header[1]), Number(header[2])], [8, 0]);
  assert.equal(lines.length, 8);
  assert.equal(lines.filter(line => JSON.parse(line).cmd === 'click').length, 0);

  const { code, report, dir } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 8, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  assert.deepEqual(steps[2].state.content.refused, [{ text: 'ipsum Lorem', detail: 'unsupported-encoding' }]);
  assert.deepEqual(steps[4].state.content.refused, [{ text: 'Dolor', detail: 'unsupported-encoding' }]);

  assert.ok(dir.startsWith(tmpdir()));
  const doc = mupdf.PDFDocument.openDocument(await readFile(join(dir, 'coded.pdf')), 'application/pdf');
  try {
    const page = doc.loadPage(0);
    try {
      const structured = page.toStructuredText();
      const extracted = structured.asText().split('\n').filter(Boolean);
      structured.destroy();
      assert.deepEqual(extracted, ['PDF4All', 'ipsum Lorem', 'Dolor', 'xx']);
      const buffer = page.getObject().get('Contents').readStream();
      const content = Buffer.from(buffer.asUint8Array()).toString('latin1');
      buffer.destroy();
      // Two hex digits per code (i7 p8 s9 u10 m5 ' '6 L1 o2 r3 e4 m5); the fixture has no
      // ActualText and none is invented: the 1-byte /ToUnicode extracts the new text.
      assert.ok(content.includes('<0708090a05060102030405> Tj'), content);
      assert.equal(content.includes('ActualText'), false, content);
      assert.ok(content.includes('(PDF4All) Tj'), 'the control run is untouched');
    } finally {
      page.destroy();
    }
  } finally {
    doc.destroy();
  }
  assert.equal(await sha(FIXTURE), FIXTURE_SHA256, 'fixture after');
});
