/**
 * Overnight integration of Phase 0 (redaction), Phase 1 (UX quick wins) and Phase 2 (signing worker):
 * the places where the three met. The drive is tests/drive/integration.jsonl. The window checks and the
 * catalogue words are Rust tests: crates/tandem-app/tests/gui.rs and words.rs.
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = fileURLToPath(new URL('..', import.meta.url));
const skip = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;


test('drive: Extract pages asks about boxes; Save As from a redacted copy drops its banner; "Sign anyway" with a signed file\'s boxes', { skip, timeout: 180_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-integration2-'));
  try {
    const child = spawn(process.execPath, [join(root, 'scripts/drive.mjs'), 'tests/drive/integration2.jsonl', `--out=${dir}`, `--report=${join(dir, 'report.json')}`], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => { stdout += chunk; });
    const code = await new Promise(resolve => child.on('close', resolve));
    const report = JSON.parse(stdout);
    assert.equal(code, 0, JSON.stringify(report.steps.filter(step => step.ok === false).map(step => ({ cmd: step.cmd, redact: step.state?.redact, error: step.state?.error }))).slice(0, 2000));
    const { readdir } = await import('node:fs/promises');
    const written = await readdir(dir);
    assert.ok(!written.includes('extracted.pdf'), 'a cancelled question extracts nothing');
    assert.ok(!written.includes('signed-cancelled.pdf'), 'a cancelled question signs nothing');
    assert.ok(written.includes('signed-twice.pdf'), '"Sign anyway" signed');
  } finally { await rm(dir, { recursive: true, force: true, maxRetries: 5 }); }
});

test('drive: Save As and signing ask about pending boxes; Escape peels the box, then the tool', { skip, timeout: 180_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-integration-'));
  try {
    const child = spawn(process.execPath, [join(root, 'scripts/drive.mjs'), 'tests/drive/integration.jsonl', `--out=${dir}`, `--report=${join(dir, 'report.json')}`], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => { stdout += chunk; });
    const code = await new Promise(resolve => child.on('close', resolve));
    const report = JSON.parse(stdout);
    assert.equal(code, 0, JSON.stringify(report.steps.filter(step => step.ok === false).map(step => ({ cmd: step.cmd, state: step.state }))).slice(0, 2000));
    assert.equal(report.ok, true);
  } finally { await rm(dir, { recursive: true, force: true, maxRetries: 5 }); }
});

/** The engine tests' `classic` PDF (engine/tests/app_redact_boxes.rs), objects numbered from 1. */
function classicPdf(objects) {
  let text = '%PDF-1.7\n';
  const offsets = [];
  objects.forEach((body, index) => { offsets.push(Buffer.byteLength(text, 'latin1')); text += `${index + 1} 0 obj\n${body}\nendobj\n`; });
  const xref = Buffer.byteLength(text, 'latin1');
  text += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('')}`;
  text += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(text, 'latin1');
}
const pdfStream = (dict, content) => `<< ${dict} /Length ${content.length} >>\nstream\n${content}\nendstream`;
const courier = () => `<< /Type /Font /Subtype /Type1 /BaseFont /Courier /FirstChar 32 /LastChar 126 /Widths [${Array(95).fill('600').join(' ')}] /Encoding /WinAnsiEncoding >>`;

test('drive: a box over a Form shared by two pages redacts it on both; the banner and STATE say more was removed', { skip, timeout: 180_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-integration-over-'));
  try {
    // engine/tests/app_redact_boxes.rs e3_partial_boxes_where: the header Form on both pages through inherited
    // resources; the account number boxed on both pages, the name on page 1 only.
    const { writeFile } = await import('node:fs/promises');
    await writeFile(join(dir, 'shared.pdf'), classicPdf([
      '<< /Type /Catalog /Pages 2 0 R >>',
      '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources << /Font << /F1 5 0 R >> /XObject << /Hdr 7 0 R >> >> >>',
      '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 8 0 R >>',
      '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 8 0 R >>',
      courier(),
      'null',
      pdfStream('/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>',
        'BT /F1 20 Tf 100 400 Td (NAMEVISIBLE) Tj ET BT /F1 20 Tf 100 300 Td (ACCT-4417-0093) Tj ET'),
      pdfStream('', '/Hdr Do'),
    ]));
    const lines = [
      { cmd: 'open', path: '${OUT}/shared.pdf', expect: { doc: { name: 'shared.pdf' } } },
      { cmd: 'mode', contents: { mode: 'redact' } },
      { cmd: 'redact', contents: { page: 0, rect: [90, 370, 400, 400] } },
      { cmd: 'redact', contents: { page: 0, rect: [90, 470, 400, 500] } },
      { cmd: 'redact', contents: { page: 1, rect: [90, 470, 400, 500] }, expect: { redact: { marks: 3 } } },
      { cmd: 'redact-apply-open', expect: { redact: { dialog: true } } },
      { cmd: 'redact-confirm', contents: {}, to: '${OUT}/shared redacted.pdf', expect: { doc: { name: 'shared redacted.pdf' }, error: '', redact: { banner: { name: 'shared redacted.pdf', overRedacted: true }, last: { op: 'redact-apply', ok: true, overRedacted: true } } } },
    ];
    await writeFile(join(dir, 'over.jsonl'), lines.map(line => JSON.stringify(line)).join('\n'));
    const child = spawn(process.execPath, [join(root, 'scripts/drive.mjs'), join(dir, 'over.jsonl'), `--out=${dir}`, `--report=${join(dir, 'report.json')}`], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => { stdout += chunk; });
    const code = await new Promise(resolve => child.on('close', resolve));
    const report = JSON.parse(stdout);
    assert.equal(code, 0, JSON.stringify(report.steps.filter(step => step.ok === false).map(step => ({ cmd: step.cmd, redact: step.state?.redact, error: step.state?.error }))).slice(0, 2000));
    const copy = await readFile(join(dir, 'shared redacted.pdf'), 'latin1');
    assert.doesNotMatch(copy, /ACCT-4417/, 'the boxed number is gone from the copy');
  } finally { await rm(dir, { recursive: true, force: true, maxRetries: 5 }); }
});
