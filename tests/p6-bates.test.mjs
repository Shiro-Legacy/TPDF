/**
 * Bates drive (contract-bates.md §3.1-§3.2 as amended by §9.13, §9.14, §9.18): tests/drive/bates.jsonl holds the exact-value
 * expects; this file pins the counts, deep-equals S8/S9 `decorate.last`, and reads the two saved files with mupdf
 * (page texts and the right-aligned footer's right edge within 10 pt of width − 36).
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
const SCRIPT = 'tests/drive/bates.jsonl';
const FIXTURES = {
  'fixtures/nav.pdf': '8e7c37500e46120b9bd774bd8aab8d093d9e21aacc522a66216406373229ed82',
  'fixtures/hello.pdf': '229ca9488a139d3f32b864ba24055152fe7f0faf670c9db6491144f23d275b94',
};

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

function at(object, path) {
  return path.split('.').reduce((value, key) => (value == null ? undefined : value[key]), object);
}

/** The step whose cmd and line keys match; `want` may use dotted paths into the state. */
function pick(report, cmd, want = {}) {
  const hits = report.steps.filter(step => step.cmd === cmd
    && Object.entries(want).every(([key, value]) => JSON.stringify(at(step, key)) === JSON.stringify(value)));
  assert.ok(hits.length >= 1, `no ${cmd} step matching ${JSON.stringify(want)}`);
  return hits[0];
}

/** Every number within ±tol (pt). */
function near(got, want, tol, label) {
  assert.ok(Array.isArray(got) && got.length === want.length && got.every((value, i) => Math.abs(value - want[i]) <= tol),
    `${label}: ${JSON.stringify(got)} vs ${JSON.stringify(want)} ±${tol}`);
}
const sha = async path => createHash('sha256').update(await readFile(join(root, path))).digest('hex');
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, contents: step.contents, decorate: step.state?.decorate, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error });

/** The ACME line on one page of a saved file: its text and bbox right edge, with the page's width − 36. */
function batesLine(doc, page) {
  const loaded = doc.loadPage(page);
  try {
    const json = JSON.parse(loaded.toStructuredText().asJSON());
    const line = json.blocks.flatMap(block => block.lines ?? []).find(item => /ACME-0001\d\d/.test(item.text));
    assert.ok(line, `page ${page}: an ACME Bates line`);
    return { text: line.text, edge: line.bbox.x + line.bbox.w, target: loaded.getBounds()[2] - 36 };
  } finally {
    loaded.destroy();
  }
}

test('drive Bates: a {bates} footer counts across two files, survives open and undo, refuses a bad start', { skip: NO_APP, timeout: 300_000 }, async () => {
  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} before`);
  const text = await readFile(join(root, SCRIPT), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.deepEqual([Number(header[1]), Number(header[2])], [9, 0]);
  assert.equal(lines.length, 9);
  const { code, report, dir } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 9, report.error ?? '');
  assert.equal(steps.filter(step => step.cmd === 'click').length, 0);
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const S = n => steps[n - 1];

  // §9.13: after an open the decorate key is absent; S5's from 103 proves the counter survived.
  assert.equal(S(4).state.decorate, undefined);
  assert.deepEqual(S(2).state.decorate.bates, { next: 103, pad: 6, last: { from: 100, to: 102, pages: [0, 1, 2] } });
  assert.deepEqual(S(5).state.decorate.bates, { next: 104, pad: 6, last: { from: 103, to: 103, pages: [0] } });
  assert.deepEqual(S(7).state.decorate.bates.next, 104, 'undo does not move the counter back (D2 ceiling)');
  // §9.18: S8/S9 decorate.last exactly (no stale bates on the refusal).
  assert.deepEqual(S(8).state.decorate.last, { action: 'stamp', kind: 'footer', pages: [0], ok: false });
  assert.equal(S(8).state.error, 'Bates start must be a whole number and digits 1–15');
  assert.deepEqual(S(9).state.decorate.last, { action: 'stamp', kind: 'header', pages: [0], ok: true, bates: { from: 104, to: 104 } });
  assert.equal(pick(report, 'header-footer', { 'state.revision': 3 }).state.decorate.bates.next, 105);

  // §3.2: the saved files (under the drive's tmp OUT, never fixtures/).
  assert.ok(dir.startsWith(tmpdir()));
  const open = async name => mupdf.Document.openDocument(await readFile(join(dir, name)), 'application/pdf');
  const a = await open('bates-a.pdf');
  try {
    assert.equal(a.countPages(), 3);
    for (const [page, want] of [[0, /ACME-000100/], [1, /ACME-000101/], [2, /ACME-000102/]]) {
      const line = batesLine(a, page);
      assert.match(line.text, want);
      assert.equal(line.target, 576);
      near([line.edge], [line.target], 10, `bates-a p${page} right edge`);
    }
  } finally {
    a.destroy();
  }
  const b = await open('bates-b.pdf');
  try {
    const line = batesLine(b, 0);
    assert.match(line.text, /ACME-000103/);
    assert.equal(line.target, 576);
    near([line.edge], [line.target], 10, 'bates-b p0 right edge');
  } finally {
    b.destroy();
  }
  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} after`);
});

test('drive Bates pad: a string batesPad carries to the next stamp, a bad align refuses', { skip: NO_APP, timeout: 300_000 }, async () => {
  const script = 'tests/drive/bates-pad.jsonl';
  const nav = 'fixtures/nav.pdf';
  assert.equal(await sha(nav), FIXTURES[nav], `${nav} before`);
  const text = await readFile(join(root, script), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.deepEqual([Number(header[1]), Number(header[2])], [5, 0]);
  assert.equal(lines.length, 5);
  const { code, report, dir } = await drive(script, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 5, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const S = n => steps[n - 1];
  // T-R2: pad "4" (a string, not the default 6) is kept and prefills the next {bates} stamp.
  assert.deepEqual(S(2).state.decorate.bates, { next: 4, pad: 4, last: { from: 1, to: 3, pages: [0, 1, 2] } });
  assert.deepEqual(S(3).state.decorate.bates, { next: 7, pad: 4, last: { from: 4, to: 6, pages: [0, 1, 2] } });
  assert.equal(S(4).state.error, 'Align must be left or right');
  assert.equal(S(4).state.revision, 2);
  assert.equal(S(4).state.decorate.last.ok, false);
  assert.equal(S(4).state.decorate.last.bates, undefined);
  assert.deepEqual(S(4).state.decorate.bates, S(3).state.decorate.bates);

  assert.ok(dir.startsWith(tmpdir()));
  const doc = mupdf.Document.openDocument(await readFile(join(dir, 'bates-pad.pdf')), 'application/pdf');
  try {
    for (const [page, footer, head] of [[0, 'ACME-0001', '0004'], [1, 'ACME-0002', '0005'], [2, 'ACME-0003', '0006']]) {
      const loaded = doc.loadPage(page);
      try {
        const words = loaded.toStructuredText().asText().split(/\s+/);
        assert.ok(words.includes(footer), `p${page} footer ${footer}: ${JSON.stringify(words)}`);
        assert.ok(words.includes(head), `p${page} header ${head}: ${JSON.stringify(words)}`);
      } finally {
        loaded.destroy();
      }
    }
  } finally {
    doc.destroy();
  }
  assert.equal(await sha(nav), FIXTURES[nav], `${nav} after`);
});
