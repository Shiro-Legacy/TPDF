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
