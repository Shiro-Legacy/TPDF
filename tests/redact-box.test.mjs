/**
 * Redaction redesign (docs/rust-and-ux-plan.md §2.3): the redaction tool's words and helpers
 * (src/renderer/redact-ui.ts), and the drive of the new flow (tests/drive/redact-box.jsonl):
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
import {
  areasText, bannerText, pagesName, confirmHeading, countText, flattenQuestion, groupByPage, panelHeading, pendingText,
  redactButtonText, refusalText, REDACT_CHIP, SANITIZE_KEYS, SANITIZE_LABELS, sanitizeAll, sanitizeFrom,
  pendingQuestion, pendingWords, sizeLimitText, tintShapes, wholePageRect,
} from '../build/renderer/redact-ui.js';
import { MAX_REDACT_SOURCE_BYTES } from '../build/renderer/limits.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const skip = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const madeDirs = [];
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));

test('redact-ui words follow §2.3 and §3.2 (plain, counted, MB not MiB)', () => {
  assert.equal(REDACT_CHIP, 'Drag over anything you want removed. Esc to stop.');
  assert.equal(confirmHeading(3), 'Permanently remove everything inside 3 boxes?');
  assert.equal(confirmHeading(1), 'Permanently remove everything inside 1 box?');
  assert.equal(redactButtonText(3), 'Redact 3 areas…');
  assert.equal(redactButtonText(1), 'Redact 1 area…');
  assert.equal(redactButtonText(0), 'Redact areas…');
  assert.equal(panelHeading(3), 'Redactions (3)');
  assert.equal(countText(3, 2), '3 boxes on 2 pages');
  assert.equal(countText(0, 0), 'No boxes yet');
  assert.equal(areasText(1), '1 area');
  assert.equal(bannerText('contract redacted.pdf', 3), "You are viewing the redacted copy 'contract redacted.pdf'. 3 areas removed and checked.");
  assert.equal(pendingText(3), '3 areas are marked but not redacted yet.');
  assert.equal(pendingText(1), '1 area is marked but not redacted yet.');
  // 12 MiB = 12,582,912 bytes: stated in decimal MB, rounded down so the stated size always works.
  assert.equal(sizeLimitText(MAX_REDACT_SOURCE_BYTES), 'Redaction works on files up to 12.5 MB in this version.');
  assert.ok(!sizeLimitText().includes('MiB'));
  assert.equal(refusalText('too-large'), 'Redaction works on files up to 12.5 MB in this version. This file is larger.');
  assert.equal(refusalText('something-else'), undefined);
  const flat = flattenQuestion([3]);
  assert.equal(`${flat.heading} ${flat.text}`, "Page 4 contains artwork that can't be cut precisely. Redact page 4 by turning it into an image? Its text will no longer be selectable.");
  assert.equal(flat.yes, 'Flatten page 4');
  assert.equal(flattenQuestion([1, 4, 6]).yes, 'Flatten pages 2, 5 and 7');
  assert.equal(pagesName([0, 2], true), 'Pages 1 and 3');
  assert.equal(bannerText('a redacted.pdf', 2, { flattened: [3], unsigned: true }),
    "You are viewing the redacted copy 'a redacted.pdf'. 2 areas removed and checked. Page 4 was turned into an image. The copy has no signatures.");
  // R5 decision: a raw-check failure with hidden information kept says what to do.
  assert.match(refusalText('verify-failed', [0], 'raw', true), /^Some of the text inside your boxes is still in the document's comments, form fields or properties.*Turn on "Also remove hidden information, other comments and form entries" and try again/);
  assert.match(refusalText('verify-failed', [0], 'raw', false), /final check on page 1, so nothing was saved/);
  assert.equal(SANITIZE_LABELS.xmp, 'Extra document details saved by other apps');
  assert.equal(SANITIZE_LABELS.struct, 'Descriptions written for screen readers');
  assert.equal(refusalText('verify-failed', [1, 3]), 'The redacted copy did not pass its final check on pages 2 and 4, so nothing was saved. Try a larger box, or redact the whole page.');
  assert.match(refusalText('verify-failed', [0]), /final check on page 1, so nothing was saved/);
  // No internals in any switch name (X7): no XMP, no "tagged", no "annotations".
  assert.deepEqual(SANITIZE_KEYS, ['info', 'xmp', 'attachments', 'outlines', 'annots', 'links', 'fields', 'layers', 'struct']);
  for (const key of SANITIZE_KEYS) assert.doesNotMatch(SANITIZE_LABELS[key], /XMP|[Tt]agged|annotation|MiB/, key);
});

test('redact-ui helpers: boxes by page, whole page, sanitize switches', () => {
  const boxes = [
    { page: 2, id: 9, rect: [10, 300, 50, 320], search: false },
    { page: 0, id: 4, rect: [10, 500, 50, 520], search: true },
    { page: 0, id: 3, rect: [200, 100, 250, 120], search: false },
    { page: 0, id: 5, rect: [20, 100, 60, 120], search: false },
  ];
  assert.deepEqual(groupByPage(boxes).map(group => [group.page, group.boxes.map(box => box.id)]), [[0, [5, 3, 4]], [2, [9]]]);
  assert.deepEqual(groupByPage([]), []);
  assert.deepEqual(wholePageRect({ width: 595.2756, height: 841.8898 }), [0, 0, 595.28, 841.89]);
  assert.deepEqual(sanitizeAll(false), Object.fromEntries(SANITIZE_KEYS.map(key => [key, false])));
  assert.deepEqual(sanitizeFrom(undefined, undefined), sanitizeAll(true), 'all on by default (§2.3 step 5)');
  assert.deepEqual(sanitizeFrom(false, { xmp: true, bogus: true }), { ...sanitizeAll(false), xmp: true });
  assert.deepEqual(sanitizeFrom(true, { struct: false, info: 'no' }), { ...sanitizeAll(true), struct: false });
});

test('redact-inspect replies become tint shapes; a truncated box is tinted whole', () => {
  const region = {
    page: 0, rect: [10, 10, 100, 40], text: 'abc', needsFlatten: false, truncated: false,
    glyphs: [[10, 10, 50, 10, 50, 20, 10, 20], [1, 2, 3]],
    images: [{ rect: [20, 20, 30, 30], whole: true }],
    shapes: [{ rect: [0, 0, 5, 5], kind: 'path', whole: false }],
    annots: [{ kind: 'widget', rect: [60, 10, 90, 30], label: 'name' }],
  };
  assert.deepEqual(tintShapes([region]).map(shape => shape.kind), ['glyph', 'image', 'shape', 'annot'], 'a malformed quad is skipped');
  assert.deepEqual(tintShapes([{ ...region, truncated: true }]), [{ kind: 'shape', points: [10, 10, 100, 40] }]);
});

test('the redaction dialog and Help say MB, keep the nine switches and drop the stale alternate-text warning', async () => {
  const html = await readFile(join(root, 'src/renderer/index.html'), 'utf8');
  const dialog = /<dialog id="redact-dialog"[\s\S]*?<\/dialog>/.exec(html)?.[0] ?? '';
  assert.ok(dialog.includes('Also remove hidden information, other comments and form entries (author, document properties, attachments, bookmarks…)'));
  assert.ok(dialog.includes('<em>Recommended</em>'));
  assert.match(dialog, /<details id="redact-details">\s*<summary>Details<\/summary>/);
  assert.equal((dialog.match(/id="redact-sanitize-[a-z]+" type="checkbox" checked/g) ?? []).length, 9, 'all nine on by default');
  assert.ok(dialog.includes('>Redact and save as…</button>'));
  assert.ok(!html.includes('not removed by a rect mark'), 'the stale warning is gone (§2.4)');
  assert.ok(!html.includes('redact-inline-alt'));
  assert.ok(!/Redaction[^<]*MiB/.test(html), 'no MiB in redaction text');
});

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

test('the pending-boxes question names each way out', () => {
  assert.deepEqual(pendingWords('export', false).go, 'Export anyway');
  assert.deepEqual(pendingWords('print', false).go, 'Print anyway');
  assert.deepEqual(pendingWords('sign', false).go, 'Sign anyway');
  assert.deepEqual(pendingWords('save', false).go, 'Save marks only');
  assert.deepEqual(pendingWords('save', true).go, 'Save without the boxes');
});

test('main asks before unsaved boxes are lost; a signed file\'s boxes say they live only while it is open', () => {
  const kept = pendingQuestion(3, false, 'close');
  assert.equal(kept.message, '3 areas are marked but not redacted yet.');
  assert.deepEqual(kept.buttons, ['Redact Now…', 'Close Without Redacting', 'Cancel']);
  assert.match(kept.detail, /^Closing now discards the boxes and any other unsaved changes/);
  const memory = pendingQuestion(1, true, 'open');
  assert.equal(memory.message, '1 area is marked but not redacted yet.');
  assert.deepEqual(memory.buttons, ['Redact Now…', 'Discard the Boxes', 'Cancel']);
  assert.match(memory.detail, /^These boxes are kept only while the file is open\. Opening another file now discards them\./);
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
