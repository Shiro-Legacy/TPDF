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

/** A real window over the DevTools protocol (as tests/p1-ux-gui.test.mjs): drive lines on stdin, real key presses. */
async function guiSession() {
  const { createRequire } = await import('node:module');
  const { createServer } = await import('node:net');
  const electron = createRequire(import.meta.url)('electron');
  const port = await new Promise(resolve => { const server = createServer(); server.listen(0, '127.0.0.1', () => { const { port } = server.address(); server.close(() => resolve(port)); }); });
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-integration-gui-'));
  const child = spawn(electron, [`--remote-debugging-port=${port}`, '--remote-allow-origins=*', `--user-data-dir=${join(dir, 'udata')}`, '.', '--drive=-', `--drive-out=${dir}`], { cwd: root, stdio: ['pipe', 'pipe', 'pipe'] });
  const waiting = [];
  let buffer = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => {
    buffer += chunk;
    for (let at = buffer.indexOf('\n'); at >= 0; at = buffer.indexOf('\n')) {
      const line = buffer.slice(0, at); buffer = buffer.slice(at + 1);
      if (line.startsWith('STEP ')) waiting.shift()?.(JSON.parse(line.slice(5)));
    }
  });
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  let target;
  for (let i = 0; i < 150 && !target; i++) {
    try { target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(item => item.url.startsWith('app://pdf4all/index.html')); } catch { /* not up yet */ }
    if (!target) await sleep(200);
  }
  assert.ok(target, 'no DevTools target');
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let n = 0;
  const pending = new Map();
  ws.onmessage = event => { const msg = JSON.parse(String(event.data)); const p = pending.get(msg.id); if (p) { pending.delete(msg.id); if (msg.error) p.reject(new Error(msg.error.message)); else p.resolve(msg.result); } };
  const send = (method, params = {}) => new Promise((resolve, reject) => { const id = ++n; pending.set(id, { resolve, reject }); ws.send(JSON.stringify({ id, method, params })); });
  const step = async line => { const got = new Promise(resolve => waiting.push(resolve)); child.stdin.write(`${JSON.stringify(line)}\n`); return (await got).state; };
  const js = async expression => {
    const reply = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (reply.exceptionDetails) throw new Error(JSON.stringify(reply.exceptionDetails).slice(0, 300));
    return reply.result.value;
  };
  const key = async (name, code, extra = {}) => { for (const type of ['keyDown', 'keyUp']) await send('Input.dispatchKeyEvent', { type, key: name, code: name, windowsVirtualKeyCode: code, ...extra }); await sleep(150); };
  for (let i = 0; i < 100 && !(await js('document.title.startsWith("hello.pdf")').catch(() => false)); i++) await sleep(100);
  const close = async () => { ws.close(); child.stdin.end(); child.kill('SIGTERM'); await new Promise(resolve => child.once('exit', resolve)); await rm(dir, { recursive: true, force: true, maxRetries: 5 }); };
  return { step, js, key, close, sleep };
}

test('GUI: a real Escape deselects a box, then stops the redaction tool; the redacted copy joins the recent files', { skip, timeout: 180_000 }, async () => {
  const s = await guiSession();
  try {
    await s.step({ cmd: 'open', path: 'fixtures/redact.pdf' });
    await s.step({ cmd: 'mode', contents: { mode: 'redact' } });
    await s.step({ cmd: 'drag', contents: { page: 0, x: 60, y: 30, dx: 300, dy: 40 } });
    let state = await s.step({ cmd: 'click', contents: { page: 0, x: 200, y: 50 } });
    assert.equal(state.direct.active?.kind, 'redact', 'the box is selected (and focused)');
    await s.key('Escape', 27);
    state = await s.step({ cmd: 'state' });
    assert.equal(state.direct.active, null, 'the first real Escape deselected the box');
    assert.equal(state.redact.tool, true, 'and left the tool on');
    assert.equal(await s.js('document.activeElement?.classList.contains("redact-target")'), true, 'focus is still on the box');
    await s.key('Escape', 27);
    state = await s.step({ cmd: 'state' });
    assert.equal(state.redact.tool, false, 'the second real Escape stopped the tool');
    assert.equal(state.mode, 'read');
    // An open dialog takes Escape first: the tool stays on while the redaction dialog closes.
    await s.step({ cmd: 'mode', contents: { mode: 'redact' } });
    await s.step({ cmd: 'redact-apply-open' });
    await s.key('Escape', 27);
    state = await s.step({ cmd: 'state' });
    assert.equal(state.redact.dialog, false);
    assert.equal(state.redact.tool, true, 'Escape closed the dialog, not the tool');
    state = await s.step({ cmd: 'redact-confirm', contents: {}, to: '${OUT}/redact redacted.pdf' });
    assert.equal(state.doc.name, 'redact redacted.pdf');
    const recent = await s.js('window.pdf4all.recentFiles()');
    assert.ok(recent.some(entry => entry.name === 'redact redacted.pdf'), JSON.stringify(recent));
  } finally { await s.close(); }
});

test('main asks about boxes before the signed copy opens only when they are a signed file\'s boxes in memory', async () => {
  const { asksBoxesOnOpen } = await import('../build/renderer/redact-ui.js');
  assert.equal(asksBoxesOnOpen({}, undefined), false, 'no boxes, no question');
  assert.equal(asksBoxesOnOpen({}, { count: 0, memory: false }), false);
  assert.equal(asksBoxesOnOpen({}, { count: 2, memory: false }), true, 'opening another file asks');
  assert.equal(asksBoxesOnOpen({ skipDiscard: true }, { count: 2, memory: false }), false, 'the signed copy keeps marks');
  assert.equal(asksBoxesOnOpen({ skipDiscard: true }, { count: 2, memory: true }), true, 'the signed copy does not have memory boxes');
  const main = await source('src/main.ts');
  assert.match(main, /asksBoxesOnOpen\(options, redactPending\.get\(id\)\) \? askPendingBoxes\(id, 'open'\) : 'none'/, 'deliverOpen uses it');
});

test('"Sign anyway" says what happens to the boxes', async () => {
  const { pendingWords } = await import('../build/renderer/redact-ui.js');
  assert.equal(pendingWords('sign', true).detail, 'These boxes are kept only while the file is open. The signed copy will not have them, and it opens in place of this file.');
  assert.equal(pendingWords('sign', true).go, 'Sign anyway');
  assert.match(pendingWords('sign', false).detail, /keeps the boxes as marks/);
});

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

test('GUI: a file with only Adobe Reader usage rights gets its own question and banner words', { skip, timeout: 180_000 }, async () => {
  const s = await guiSession();
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-integration-ur3-'));
  try {
    // engine/tests/app_redact_boxes.rs p4_signature_corners "ur3": /Perms << /UR3 >> and its signature dictionary only.
    const { writeFile } = await import('node:fs/promises');
    const file = join(dir, 'reader-extended.pdf');
    await writeFile(file, classicPdf([
      '<< /Type /Catalog /Pages 2 0 R /Perms << /UR3 6 0 R >> >>',
      '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
      '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
      pdfStream('', 'BT /F1 20 Tf 100 300 Td (ACCT-4417-0093) Tj ET'),
      courier(),
      '<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 0 0 0] /Contents <00> /Reference [<< /TransformMethod /UR3 >>] >>',
    ]));
    let state = await s.step({ cmd: 'open', path: file });
    assert.equal(state.doc.name, 'reader-extended.pdf');
    await s.step({ cmd: 'mode', contents: { mode: 'redact' } });
    await s.step({ cmd: 'redact', contents: { page: 0, rect: [90, 470, 400, 500] } });
    await s.step({ cmd: 'redact-apply-open' });
    // Unanswered, the question is declined: STATE names which question it was.
    state = await s.step({ cmd: 'redact-confirm', contents: {}, to: join(dir, 'declined.pdf') });
    assert.deepEqual({ reason: state.redact.last.reason, question: state.redact.last.question }, { reason: 'cancelled', question: 'usage-rights' });
    await s.step({ cmd: 'redact-apply-open' });
    state = await s.step({ cmd: 'redact-confirm', contents: { unsign: true }, to: join(dir, 'reader-extended redacted.pdf') });
    assert.equal(state.redact.last.ok, true, JSON.stringify(state.redact.last));
    assert.equal(state.redact.last.unsigned, true);
    const banner = await s.js('document.getElementById("redact-banner-text").textContent');
    assert.match(banner, /The copy no longer has Adobe Reader's special permissions\./);
    assert.doesNotMatch(banner, /no signatures/);
  } finally { await s.close(); await rm(dir, { recursive: true, force: true, maxRetries: 5 }); }
});
