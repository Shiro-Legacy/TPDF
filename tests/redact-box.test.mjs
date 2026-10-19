/**
 * Redaction redesign (docs/rust-and-ux-plan.md §2.3): the drive of the new flow (tests/drive/redact-box.jsonl;
 * the tool's words and helpers are Rust tests: crates/tandem-app/tests/words.rs and helpers.rs):
 * draw a box, confirm, the copy opens, the original closes and stays unchanged on disk.
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { deflateSync } from 'node:zlib';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import mupdf from 'mupdf';

const root = fileURLToPath(new URL('..', import.meta.url));
const skip = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const madeDirs = [];
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));

async function drive(script, prepare = async () => {}) {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-redact-box-'));
  madeDirs.push(dir);
  await prepare(dir);
  const report = join(dir, 'report.json');
  const child = spawn(process.execPath, [join(root, 'scripts/drive.mjs'), script, `--out=${dir}`, `--report=${report}`], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => { stdout += chunk; });
  const timer = setTimeout(() => child.kill(), 150_000);
  const code = await new Promise(resolve => child.on('close', resolve));
  clearTimeout(timer);
  return { code, report: JSON.parse(stdout), dir };
}

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const redactAnnots = doc => {
  let count = 0;
  for (let page = 0; page < doc.countPages(); page++) count += doc.loadPage(page).getAnnotations().filter(item => item.getType() === 'Redact').length;
  return count;
};

test('drive: Redact tool → box → confirm → the copy opens, the original closes unchanged', { skip, timeout: 180_000 }, async () => {
  const before = sha256(await readFile(join(root, 'fixtures/redact.pdf')));
  const { code, report, dir } = await drive('tests/drive/redact-box.jsonl');
  assert.equal(code, 0, JSON.stringify(report.steps.filter(step => step.ok !== true).map(step => ({ cmd: step.cmd, state: step.state }))));
  assert.equal(report.ok, true);
  const steps = report.steps.filter(step => step.cmd);
  const confirmed = steps.find(step => step.cmd === 'redact-confirm').state;
  assert.equal(confirmed.doc.name, 'redact redacted.pdf', 'the copy is what the window shows');
  assert.equal(confirmed.canUndo, false, 'the copy opens with no undo history');
  assert.ok(confirmed.redact.last.verified.text >= 1, 'apply checked the boxed text is gone');
  assert.equal(sha256(await readFile(join(root, 'fixtures/redact.pdf'))), before, 'the original file is never written');

  // The copy: no marks, and nothing of the secret is left inside the box (the box itself is drawn black).
  const copy = mupdf.Document.openDocument(await readFile(join(dir, 'redact redacted.pdf')), 'application/pdf');
  try {
    assert.equal(redactAnnots(copy), 0);
    const words = [];
    copy.loadPage(0).toStructuredText().walk({ onChar: (c, origin) => words.push({ c, x: origin[0], y: origin[1] }) });
    const inside = words.filter(item => item.x >= 60 && item.x <= 360 && item.y >= 30 && item.y <= 70).map(item => item.c).join('');
    assert.ok(!inside.includes('P0B'), `nothing of the secret inside the box: ${inside}`);
    // R1: the words at the box's edges ("visible", "here") go whole too.
    const line = words.filter(item => item.y >= 30 && item.y <= 70).map(item => item.c).join('');
    assert.ok(!line.includes('visible') && !line.includes('here'), `edge glyphs gone: ${line}`);
  } finally { copy.destroy(); }
  // R6: the legacy op with `regions` redacted the same line with no mark ever written.
  const regions = mupdf.Document.openDocument(await readFile(join(dir, 'regions.pdf')), 'application/pdf');
  try {
    const words = [];
    regions.loadPage(0).toStructuredText().walk({ onChar: (c, origin) => words.push({ c, x: origin[0], y: origin[1] }) });
    assert.ok(!words.filter(item => item.y >= 30 && item.y <= 70).map(item => item.c).join('').includes('P0B'));
  } finally { regions.destroy(); }
  const underneath = steps.find(step => step.cmd === 'redact-underneath' && step.contents?.on === true).state.redact;
  assert.ok(underneath.tint >= 1, 'redact-inspect named what the box takes');
  // "Save marks only" kept the box as a mark in the saved copy of the original.
  const marks = mupdf.Document.openDocument(await readFile(join(dir, 'marks.pdf')), 'application/pdf');
  try { assert.equal(redactAnnots(marks), 1); } finally { marks.destroy(); }
});

test('drive: the copy never replaces the original; other changes can be saved first; memory boxes count as unsaved', { skip, timeout: 180_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/redact-safety.jsonl');
  assert.equal(code, 0, JSON.stringify(report.steps.filter(step => step.ok !== true).map(step => ({ cmd: step.cmd, state: step.state }))));
  // H1: orig2.pdf is still the unredacted file (its top line and no applied redaction).
  const orig2 = mupdf.Document.openDocument(await readFile(join(dir, 'orig2.pdf')), 'application/pdf');
  try { assert.ok(orig2.loadPage(0).toStructuredText().asText().includes('visible P0B-SECRET-7731 here')); } finally { orig2.destroy(); }
  // M2: "Save them to the original first" kept the note and the box in orig.pdf; the copy has the note's page text redacted.
  const orig = mupdf.Document.openDocument(await readFile(join(dir, 'orig.pdf')), 'application/pdf');
  try {
    const types = orig.loadPage(0).getAnnotations().map(item => item.getType());
    assert.ok(types.includes('Text') && types.includes('Redact'), types.join());
  } finally { orig.destroy(); }
  await assert.rejects(readFile(join(dir, 'never.pdf')), 'a cancelled apply writes nothing');
  // Export and Print ask while boxes are pending: Cancel writes nothing; an unanswered drive goes on.
  await assert.rejects(readFile(join(dir, 'export-cancelled.txt')), 'a cancelled export writes nothing');
  assert.ok((await readFile(join(dir, 'export.txt'), 'utf8')).length > 0);
  await assert.rejects(readFile(join(dir, 'print-cancelled.pdf')), 'a cancelled print writes nothing');
  await assert.rejects(readFile(join(dir, 'print-redact.pdf')), '"Redact now…" opens the dialog instead of printing');
});

/** The engine's R4 fixture shape: a Gouraud mesh over page 1's top half with MESHSECRET on it; page 2 says PAGETWO. */
function meshPdf() {
  const objects = [];
  const stream = (dict, data) => { const bytes = deflateSync(Buffer.isBuffer(data) ? data : Buffer.from(data, 'latin1')); return Buffer.concat([Buffer.from(`<< ${dict} /Filter /FlateDecode /Length ${bytes.length} >>\nstream\n`, 'latin1'), bytes, Buffer.from('\nendstream', 'latin1')]); };
  const page = contents => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /Shading << /Sh 6 0 R >> >> /Contents ${contents} 0 R >>`;
  const tri = Buffer.from([0, 0, 128, 0, 0, 255, 128, 128, 0, 0, 255, 255, 0, 255, 128, 64, 0, 255, 255, 200, 0, 0, 255, 32]);
  objects.push('<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>', page(7), page(8),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>',
    stream('/ShadingType 4 /ColorSpace /DeviceGray /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 612 0 792 0 1]', tri),
    stream('', 'q 0 396 612 396 re W n /Sh sh Q BT /F1 20 Tf 100 600 Td (MESHSECRET) Tj ET BT /F1 20 Tf 100 200 Td (KEEPTEXT) Tj ET'),
    stream('', 'BT /F1 20 Tf 100 600 Td (PAGETWO) Tj ET'));
  const parts = [Buffer.from('%PDF-1.7\n', 'latin1')];
  let length = parts[0].length;
  const offsets = objects.map((body, index) => {
    const at = length;
    const chunk = Buffer.concat([Buffer.from(`${index + 1} 0 obj\n`, 'latin1'), Buffer.isBuffer(body) ? body : Buffer.from(body, 'latin1'), Buffer.from('\nendobj\n', 'latin1')]);
    parts.push(chunk); length += chunk.length;
    return at;
  });
  const size = objects.length + 1;
  parts.push(Buffer.from(`xref\n0 ${size}\n0000000000 65535 f \n${offsets.map(o => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}trailer\n<< /Size ${size} /Root 1 0 R >>\nstartxref\n${length}\n%%EOF\n`, 'latin1'));
  return Buffer.concat(parts);
}

test('drive: a signed file makes an unsigned redacted copy with consent; artwork that cannot be cut flattens its page with consent', { skip, timeout: 240_000 }, async () => {
  const signedBefore = sha256(await readFile(join(root, 'fixtures/p0b/signed-intact.pdf')));
  const { code, report, dir } = await drive('tests/drive/redact-signed-flatten.jsonl', dir => writeFile(join(dir, 'mesh.pdf'), meshPdf()));
  assert.equal(code, 0, JSON.stringify(report.steps.filter(step => step.ok !== true).map(step => ({ cmd: step.cmd, state: step.state }))));
  assert.equal(sha256(await readFile(join(root, 'fixtures/p0b/signed-intact.pdf'))), signedBefore, 'the signed original is never written');
  await assert.rejects(readFile(join(dir, 'signed-no.pdf')), 'no consent, no file');
  await assert.rejects(readFile(join(dir, 'mesh-no.pdf')), 'no consent, no file');
  const unsigned = await readFile(join(dir, 'signed redacted.pdf'));
  assert.ok(!unsigned.includes('/ByteRange'), 'the copy carries no signature');
  const signedCopy = mupdf.Document.openDocument(unsigned, 'application/pdf');
  try { assert.ok(!signedCopy.loadPage(0).toStructuredText().asText().includes('PDF4All signed'), 'the boxed text is gone'); } finally { signedCopy.destroy(); }
  const flat = mupdf.Document.openDocument(await readFile(join(dir, 'mesh redacted.pdf')), 'application/pdf');
  try {
    assert.ok(!flat.loadPage(0).toStructuredText().asText().includes('MESHSECRET'), 'page 1 became a picture without the secret');
    assert.ok(flat.loadPage(1).toStructuredText().asText().includes('PAGETWO'), 'page 2 keeps its text');
  } finally { flat.destroy(); }
  assert.ok(!(await readFile(join(dir, 'mesh redacted.pdf'))).includes('MESHSECRET'));
});
