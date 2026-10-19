/**
 * Overnight integration of Phase 0 (redaction), Phase 1 (UX quick wins) and Phase 2 (signing worker):
 * the places where the three met. The drive is tests/drive/integration.jsonl.
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
const source = name => readFile(join(root, name), 'utf8');

test('the redaction words live in the catalogue; redact-ui re-exports the same functions', async () => {
  const { MSG, userText, isCatalogueText } = await import('../build/renderer/messages.js');
  const ui = await import('../build/renderer/redact-ui.js');
  for (const name of ['areasText', 'bannerText', 'pendingQuestion', 'refusalText', 'sizeLimitText', 'flattenQuestion', 'pendingWords']) {
    assert.equal(ui[name], MSG.redact[name], name);
  }
  assert.equal(ui.REDACT_CHIP, MSG.redact.REDACT_CHIP);
  // A file named like an internal operation is never rewritten in a redaction message.
  const text = MSG.redact.savedOpened('redact: secret.pdf');
  assert.equal(text, 'Saved and opened the redacted copy redact: secret.pdf.');
  assert.ok(isCatalogueText(text));
  assert.equal(userText(text), text);
  assert.equal(userText(MSG.redact.addedPage(3)), 'Added a box over all of page 3. Nothing is removed until you redact.');
  // No error token in a failed copy's sentence.
  assert.doesNotMatch(MSG.redact.notSaved('write-failed'), /\(|write-failed/);
  assert.match(MSG.redact.notSaved('disk-full'), /not enough disk space/);
  assert.equal(MSG.redact.cancelled('sign'), 'Signing cancelled');
});

test('Phase 2 certificate sentences and verifier words go through the catalogue', async () => {
  const { MSG } = await import('../build/renderer/messages.js');
  assert.match(MSG.sign.certificate('unsupported-key'), /P-256 or P-384/);
  assert.match(MSG.sign.certificate('too-costly'), /takes too long to open/);
  assert.equal(MSG.sign.failed('too-costly'), MSG.sign.certificate('too-costly'));
  assert.equal(MSG.verify.notes('no-signing-time', 'no date given'), 'no date given', "the verifier's own words win");
  assert.equal(MSG.verify.notes('time-changed'), 'the date was edited after signing');
  assert.equal(MSG.verify.trust('untrusted'), 'not in your trusted list');
  assert.equal(MSG.verify.trust('none'), 'not in your trusted list');
  assert.equal(MSG.verify.trust('trusted', 'P0B Test CA'), 'trusted by P0B Test CA');
  assert.equal(MSG.sign.notForSigning.buttons[MSG.sign.notForSigning.defaultId], 'Cancel', 'Return never signs with it');
});

test('one layered Escape: the redaction tool is its last layer, not a listener of its own', async () => {
  const ui = await source('src/renderer/redact-ui.ts');
  assert.doesNotMatch(ui, /on\("keydown"/, 'redact-ui has no Escape listener of its own');
  const renderer = await source('src/renderer/renderer.ts');
  assert.match(renderer, /!event\.isComposing && escapeOwners\.size === 0 && !directActive && !document\.querySelector\("dialog\[open\]"\) && \(armedTool\(\) \|\| redactEscapeLayer\(\)\)/);
  assert.match(renderer, /if \(hasDirectFocus\(\)\) dropDirectFocus\(\);\n\s+else if \(armedTool\(\)\) disarmTools\(\);\n\s+else exitRedactTool\(\);/);
  // A deselected box keeps focus but no longer swallows Escape.
  assert.match(renderer, /if \(event\.key === "Escape"\) \{ if \(!hasDirectFocus\(\)\) return; event\.preventDefault\(\); dropDirectFocus\(\); return; \}/);
});

test('main: the signed copy opens without the boxes question; redacted copies are recent files', async () => {
  const main = await source('src/main.ts');
  assert.match(main, /const boxes = options\.skipDiscard \? 'none' : askPendingBoxes\(id, 'open'\);/);
  const bind = main.slice(main.indexOf('function bindOpen('), main.indexOf('function askPendingBoxes('));
  assert.match(bind, /recentFiles\.add\(pending\.record\.path\)/);
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

test('Verify shows the signing time as a plain local date; STATE keeps the ISO time', async () => {
  const { MSG, userText } = await import('../build/renderer/messages.js');
  const iso = '2026-10-19T06:36:20.000Z';
  const at = { locale: 'en-US', timeZone: 'America/New_York' };
  assert.equal(MSG.verify.signedOn(iso, false, false, at), 'Oct 19, 2026, 2:36 AM');
  // Only the signer's claim (no trusted timestamp): "Dated …", as Verify's sentence says.
  assert.equal(MSG.verify.signedOn(iso, true, false, at), 'Dated Oct 19, 2026, 2:36 AM');
  assert.match(MSG.verify.signedOn(iso, true, false, { locale: 'en-GB', timeZone: 'UTC' }).replace(/\s/g, ' '), /^Dated 19 Oct 2026, 0?6:36$/);
  for (const missing of [undefined, null, '', 'not a date']) assert.equal(MSG.verify.signedOn(missing), 'No date');
  assert.equal(MSG.verify.signedOn(iso, true, true), 'No date', 'a "no-signing-time" verdict has no date to show');
  // The system locale is the default, and the text is catalogue text.
  const local = MSG.verify.signedOn(iso);
  assert.match(local, /^Dated .*2026/);
  assert.doesNotMatch(local, /T06:36|Z$/);
  assert.equal(userText(local), local);
  const renderer = await source('src/renderer/renderer.ts');
  assert.match(renderer, /MSG\.verify\.signedOn\(row\.signingTime, row\.timestamped !== true, reason\.split\(" "\)\.includes\("no-signing-time"\)\)/);
  assert.match(renderer, /plain: row\.plain, signingTime: row\.signingTime \?\? null \}/, 'STATE keeps the raw ISO time');
  assert.match(await source('src/renderer/index.html'), /<th scope="col">Date<\/th>/);
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

test('the redaction words for shared content and Adobe Reader usage rights', async () => {
  const { MSG } = await import('../build/renderer/messages.js');
  const over = 'Some content outside your boxes was also removed, because it is shared with other parts of the document. Check the copy.';
  assert.equal(MSG.redact.OVER_REDACTED, over);
  assert.equal(MSG.redact.bannerText('a.pdf', 2, { overRedacted: true }), `You are viewing the redacted copy 'a.pdf'. 2 areas removed and checked. ${over}`);
  assert.doesNotMatch(MSG.redact.bannerText('a.pdf', 2), /shared/);
  assert.equal(MSG.redact.saved('a.pdf', true), `Saved the redacted copy a.pdf. ${over}`);
  assert.match(MSG.redact.bannerText('a.pdf', 1, { unsigned: true, usageRights: true }), /no longer has Adobe Reader's special permissions\.$/);
  assert.match(MSG.redact.bannerText('a.pdf', 1, { unsigned: true }), /The copy has no signatures\.$/);
  assert.equal(`${MSG.redact.USAGE_RIGHTS_QUESTION.heading} ${MSG.redact.USAGE_RIGHTS_QUESTION.text}`,
    'This file has special permissions from Adobe Reader. Redacting removes them. Make a redacted copy without them? The original keeps them.');
  // The engine says only `signed` today, and the open reply cannot tell usage rights from a time stamp:
  // the Reader wording is used only when the engine says so.
  const renderer = await source('src/renderer/renderer.ts');
  assert.match(renderer, /return refused\?\.detail === "usage-rights" \|\| \(doc\?\.info as \{ usageRights\?: unknown \} \| undefined\)\?\.usageRights === true;/);
});

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
