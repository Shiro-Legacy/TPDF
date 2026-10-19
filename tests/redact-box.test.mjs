/**
 * Redaction redesign (docs/rust-and-ux-plan.md §2.3): the redaction tool's words and helpers
 * (src/renderer/redact-ui.ts), and the drive of the new flow (tests/drive/redact-box.jsonl):
 * draw a box, confirm, the copy opens, the original closes and stays unchanged on disk.
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import mupdf from 'mupdf';
import {
  areasText, bannerText, confirmHeading, countText, flattenQuestion, groupByPage, panelHeading, pendingText,
  redactButtonText, refusalText, REDACT_CHIP, SANITIZE_KEYS, SANITIZE_LABELS, sanitizeAll, sanitizeFrom,
  sharedText, sizeLimitText, tintShapes, wholePageRect,
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
  assert.match(sharedText({ page: 0, kind: 'image', otherPages: [1] }), /^Page 1: this picture also appears on page 2 without a box/);
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
  assert.ok(dialog.includes('Also remove hidden information (author, document properties, attachments, bookmarks…)'));
  assert.ok(dialog.includes('<em>Recommended</em>'));
  assert.match(dialog, /<details id="redact-details">\s*<summary>Details<\/summary>/);
  assert.equal((dialog.match(/id="redact-sanitize-[a-z]+" type="checkbox" checked/g) ?? []).length, 9, 'all nine on by default');
  assert.ok(dialog.includes('>Redact and save as…</button>'));
  assert.ok(!html.includes('not removed by a rect mark'), 'the stale warning is gone (§2.4)');
  assert.ok(!html.includes('redact-inline-alt'));
  assert.ok(!/Redaction[^<]*MiB/.test(html), 'no MiB in redaction text');
});

async function drive(script) {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-redact-box-'));
  madeDirs.push(dir);
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
