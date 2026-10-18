/**
 * Batch (contract-batch.md §1.3, §4, §9): one stamp action over a folder of PDFs.
 * Engine half (a)-(f): the `batch-file` op on a transient document, never the open one (§9.1-9.7).
 * batchOutName (§9.12) unit tests, main.ts source pins, and the drive half (tests/drive/batch.jsonl
 * run against staged tmp folders, §9.16-9.18). drive()/pick()/red() are copies of
 * tests/p6-drive8.test.mjs (not shared on purpose); drive() stages the folders before spawning.
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { copyFile, mkdir, mkdtemp as makeTempDir, readdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;

const toArrayBuffer = buffer => buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
const bytesOf = async path => toArrayBuffer(await readFile(join(root, path)));

/** mupdf with `Document.openDocument` counted and every returned document's destroy counted (§9.7 (d)). */
const counts = { opened: 0, destroyed: 0 };
let failSave = null; // 'throw' | 'huge': every transient PDF's saveToBuffer throws or reports an over-ceiling buffer
const Document = new Proxy(mupdf.Document, {
  get(target, key) {
    if (key !== 'openDocument') return Reflect.get(target, key);
    return (...args) => {
      const doc = target.openDocument(...args);
      counts.opened++;
      const destroy = doc.destroy.bind(doc);
      doc.destroy = () => { counts.destroyed++; destroy(); };
      const asPDF = doc.asPDF.bind(doc);
      doc.asPDF = () => {
        const pdf = asPDF();
        if (pdf && failSave === 'throw') pdf.saveToBuffer = () => { throw new Error('boom save'); };
        if (pdf && failSave === 'huge') {
          pdf.saveToBuffer = () => ({ asUint8Array: () => ({ byteLength: 50 * 1024 * 1024 + 1 }), destroy: () => { counts.buffers = (counts.buffers ?? 0) + 1; } });
        }
        return pdf;
      };
      return doc;
    };
  },
});
const engine = createEngine({ ...mupdf, Document });

function op(session, request) {
  return engine.dispatch({ id: 900, ...request }, session);
}

async function openFixture(path, session = engine.createSession()) {
  const reply = engine.dispatch({ id: 1, op: 'open', name: path.split('/').pop(), pdf: await bytesOf(path) }, session).reply;
  return { session, reply };
}

async function batch(session, path, action) {
  const bytes = typeof path === 'string' ? await bytesOf(path) : path;
  return op(session, { op: 'batch-file', name: 'x.pdf', bytes, action });
}

function textsOf(bytes) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  try {
    const texts = [];
    for (let index = 0; index < doc.countPages(); index++) {
      const page = doc.loadPage(index);
      try {
        texts.push(page.toStructuredText().asText());
      } finally {
        page.destroy();
      }
    }
    return texts;
  } finally {
    doc.destroy();
  }
}

const FOOTER = { kind: 'footer', text: 'N-{bates}', batesStart: 7, batesPad: 3 };

test('batch (a): batch-file stamps a transient copy; the open document is untouched', async () => {
  const { session } = await openFixture('fixtures/hello.pdf');
  assert.equal(op(session, { op: 'stamp', kind: 'header', text: 'Open doc' }).reply.ok, true);
  const before = session.doc;
  const saved = engine.serializeDocument(session).bytes;
  const request = { id: 5, op: 'batch-file', name: 'nav.pdf', bytes: await bytesOf('fixtures/nav.pdf'), action: FOOTER };
  const { reply, transfer } = engine.dispatch(request, session);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.pages, 3);
  assert.equal(reply.pageCount, 3);
  assert.deepEqual(reply.bates, { from: 7, to: 9 });
  assert.ok(reply.bytes instanceof ArrayBuffer);
  assert.deepEqual(transfer, [reply.bytes], 'the bytes are transferred, not cloned');
  assert.equal(request.bytes, null, 'the request drops its copy (§9.1)');
  const texts = textsOf(reply.bytes);
  assert.equal(texts.length, 3);
  for (const [index, want] of ['N-007', 'N-008', 'N-009'].entries()) assert.match(texts[index], new RegExp(want));
  assert.equal(session.doc, before, 'the same open document');
  assert.equal(session.doc.countPages(), 1);
  assert.deepEqual(engine.journalState(session), { revision: 1, dirty: true, canUndo: true, canRedo: false });
  assert.deepEqual(Buffer.from(engine.serializeDocument(session).bytes), Buffer.from(saved), 'byte-identical serialization');
  // No {bates}: no bates key.
  const plain = (await batch(session, 'fixtures/hello.pdf', { kind: 'watermark', text: 'DRAFT' })).reply;
  assert.equal(plain.ok, true, plain.error);
  assert.equal('bates' in plain, false);
  assert.equal(plain.pages, 1);
  // §9.3: the open document's pending security never reaches a batch output (SAVE_OPTIONS, not saveOptions(session)).
  const secured = op(session, { op: 'security-set', mode: 'aes-256', userPassword: 'u-secret-1', ownerPassword: 'o-secret-1', permissions: { print: true } }).reply;
  assert.equal(secured.ok, true, secured.error);
  const guarded = (await batch(session, 'fixtures/nav.pdf', FOOTER)).reply;
  assert.equal(guarded.ok, true, guarded.error);
  const out = mupdf.Document.openDocument(guarded.bytes, 'application/pdf');
  try {
    assert.equal(out.needsPassword(), false, 'the batch output carries no password');
  } finally {
    out.destroy();
  }
  engine.closeDocument(session.doc);
});

test('batch (b): refusals encrypted, read-only, not-pdf, bad-pages, too-large', async () => {
  const { session } = await openFixture('fixtures/hello.pdf');
  const png = (await readdir(join(root, 'fixtures'))).find(name => name.endsWith('img-rgb.png'));
  assert.ok(png, 'the PNG fixture');
  for (const [source, action, reason] of [
    ['fixtures/encrypted.pdf', FOOTER, 'encrypted'],
    ['fixtures/encrypted-noedit.pdf', FOOTER, 'encrypted'],
    ['fixtures/p5/signed-ec.pdf', FOOTER, 'read-only'],
    ['fixtures/p5/certified-p1.pdf', FOOTER, 'read-only'],
    [`fixtures/${png}`, FOOTER, 'not-pdf'],
    [toArrayBuffer(Buffer.alloc(100, 0x41)), FOOTER, 'not-pdf'],
    [toArrayBuffer(Buffer.from('%PDF')), FOOTER, 'not-pdf'],
    ['fixtures/nav.pdf', { ...FOOTER, pages: '9' }, 'bad-pages'],
    ['fixtures/nav.pdf', { ...FOOTER, pages: 'x' }, 'bad-pages'],
    [new ArrayBuffer(engine.maxPdfBytes + 1), FOOTER, 'too-large'],
    // A page tree whose /Kids points at a missing object: mupdf throws after the open (E-R1).
    [toArrayBuffer(Buffer.from('%PDF-1.4\n1 0 obj <</Type/Catalog/Pages 2 0 R>> endobj\n2 0 obj <</Type/Pages/Kids[5 0 R]/Count 3>> endobj\ntrailer <</Root 1 0 R>>\n%%EOF\n')), FOOTER, 'not-pdf'],
  ]) {
    const { reply } = await batch(session, source, action);
    assert.equal(reply.ok, false, `${source}: ${JSON.stringify(reply)}`);
    assert.equal(reply.reason, reason, `${source}: ${JSON.stringify(reply)}`);
    assert.equal(typeof reply.error, 'string');
  }
  // A range string stamps only those pages.
  const { reply } = await batch(session, 'fixtures/nav.pdf', { ...FOOTER, pages: '3, 1' });
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.pages, 2);
  assert.deepEqual(reply.bates, { from: 7, to: 8 });
  const texts = textsOf(reply.bytes);
  assert.match(texts[0], /N-007/);
  assert.doesNotMatch(texts[1], /N-0/);
  assert.match(texts[2], /N-008/);
  assert.deepEqual(engine.journalState(session), { revision: 0, dirty: false, canUndo: false, canRedo: false });
  engine.closeDocument(session.doc);
});

test('batch (c): a stamp refusal surfaces as stamp-error with the token as detail', async () => {
  const session = engine.createSession();
  const opened = counts.opened;
  for (const [action, detail] of [
    [{ kind: 'footer', text: '{bates}' }, 'bates-start-required'],
    [{ kind: 'footer', text: '{bates}', batesStart: -1 }, 'invalid-bates'],
    [{ kind: 'footer', text: 'x', batesPad: 16 }, 'invalid-bates'],
    [{ kind: 'footer', text: 'x', align: 'centre' }, 'invalid-align'],
    [{ kind: 'footer', text: '' }, 'text'],
    [{ kind: 'side', text: 'x' }, 'text'],
  ]) {
    const { reply } = await batch(session, 'fixtures/nav.pdf', action);
    assert.equal(reply.ok, false, JSON.stringify(action));
    assert.equal(reply.reason, 'stamp-error', JSON.stringify(reply));
    assert.equal(reply.detail, detail, JSON.stringify(reply));
    assert.match(reply.error, /^stamp: /);
    assert.equal(counts.opened, opened, `${detail}: checkStampAction runs before the transient open (§9.5)`);
  }
  // §9.1: too-large is checked first, before the action and before any open.
  for (const action of [FOOTER, { ...FOOTER, align: 'centre' }]) {
    const { reply } = await batch(session, new ArrayBuffer(engine.maxPdfBytes + 1), action);
    assert.equal(reply.reason, 'too-large', JSON.stringify(reply));
    assert.equal(counts.opened, opened, 'too-large never opens');
  }
  // The range bound needs the page count: start 10^15-2 over nav's 3 pages.
  const { reply } = await batch(session, 'fixtures/nav.pdf', { kind: 'footer', text: '{bates}', batesStart: 1e15 - 2, batesPad: 15 });
  assert.equal(reply.reason, 'stamp-error');
  assert.equal(reply.detail, 'invalid-bates');
});

test('batch (e): no open document is needed', async () => {
  const session = engine.createSession();
  const { reply } = await batch(session, 'fixtures/nav.pdf', FOOTER);
  assert.equal(reply.ok, true, reply.error);
  assert.equal(reply.pages, 3);
  assert.equal(session.doc, null);
});

test('batch (f): a locked open document does not block batch-file', async () => {
  const { session } = await openFixture('fixtures/encrypted-noedit.pdf');
  assert.equal(session.locked, true);
  const { reply } = await batch(session, 'fixtures/nav.pdf', FOOTER);
  assert.equal(reply.ok, true, reply.error);
  engine.closeDocument(session.doc);
});

test('batchOutName: only listed names and report.txt, never a path', async () => {
  const { batchOutName } = await import('../build/policy.js');
  assert.equal(typeof batchOutName, 'function', 'policy.ts exports batchOutName');
  const listed = ['a.pdf', 'nav.pdf'];
  assert.equal(batchOutName('../x.pdf', listed), null);
  assert.equal(batchOutName('a/b.pdf', listed), null);
  assert.equal(batchOutName('a\\b.pdf', [...listed, 'a\\b.pdf']), null);
  assert.equal(batchOutName('..', ['..']), null);
  assert.equal(batchOutName('.', ['.']), null);
  assert.equal(batchOutName('', ['']), null);
  assert.equal(batchOutName('x.pdf', listed), null, 'unlisted');
  assert.equal(batchOutName('nav.pdf', listed), 'nav.pdf');
  assert.equal(batchOutName('report.txt', listed), 'report.txt');
  assert.equal(batchOutName(42, listed), null);
});

test('main.ts: pick-folder and batch-io check the sender first; ⌥⌘B once', async () => {
  const src = await readFile(join(root, 'src/main.ts'), 'utf8');
  for (const channel of ['pick-folder', 'batch-io']) {
    const lines = src.split('\n');
    const at = lines.findIndex(line => line.includes(`ipcMain.handle('${channel}'`));
    assert.ok(at >= 0, `${channel} handler`);
    assert.ok(lines.slice(at + 1, at + 3).some(line => line.includes('if (!isTrustedOpen(event))')), `${channel}: isTrustedOpen within 2 lines`);
  }
  assert.equal((src.match(/accelerator: 'Alt\+CmdOrCtrl\+B'/g) ?? []).length, 1);
  assert.match(src, /let batchDirArg = /);
  assert.match(src, /if \(args\.folder !== undefined\) batchDirArg = pathOf\(args\.folder\);/);
  assert.match(src, /listing\.names\.includes\(name\)\) return \{ ok: false, reason: 'not-listed' \}/);
});

// ------------------------------------------------------------------ drive half (§9.16-9.18)

const SCRIPT = 'tests/drive/batch.jsonl';
const INPUTS = {
  'encrypted.pdf': '38d533d7aaaf1b3d56a25a98de22f7e6bafff41df301d85ed3ca611f244dcab6',
  'hello.pdf': '229ca9488a139d3f32b864ba24055152fe7f0faf670c9db6491144f23d275b94',
  'nav.pdf': '8e7c37500e46120b9bd774bd8aab8d093d9e21aacc522a66216406373229ed82',
  'rotated.pdf': 'dfa923b6a82d51b670c52b9b8442a2e03fe273dd09958e9f7c30a427191d1554',
};
const sha = async path => createHash('sha256').update(await readFile(path)).digest('hex');
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, batch: step.state?.batch, announcement: step.state?.announcement, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error });

/** A tmp OUT with batch-in/ and batch-c/ holding copies of the four fixtures (never fixtures/ itself). */
async function stage() {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-batch-'));
  for (const sub of ['batch-in', 'batch-c']) {
    await mkdir(join(dir, sub));
    for (const name of Object.keys(INPUTS)) await copyFile(join(root, 'fixtures', name), join(dir, sub, name));
  }
  return dir;
}

/** The p6-drive8 drive() with the staged dir as --out. */
async function drive(script, dir, timeoutMs = 150_000) {
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

async function pdfWords(path) {
  const doc = mupdf.Document.openDocument(await readFile(path), 'application/pdf');
  try {
    const pages = [];
    for (let index = 0; index < doc.countPages(); index++) {
      const page = doc.loadPage(index);
      try {
        pages.push(page.toStructuredText().asText());
      } finally {
        page.destroy();
      }
    }
    return pages;
  } finally {
    doc.destroy();
  }
}

/** Per page: pageWidth − 36 − the right edge of the first line matching `re` (NaN when no line matches). §4.1 align:right. */
async function rightGaps(path, re) {
  const doc = mupdf.Document.openDocument(await readFile(path), 'application/pdf');
  try {
    const gaps = [];
    for (let index = 0; index < doc.countPages(); index++) {
      const page = doc.loadPage(index);
      try {
        const [x0, , x1] = page.getBounds();
        const lines = JSON.parse(page.toStructuredText().asJSON()).blocks.flatMap(block => block.lines ?? []);
        const line = lines.find(candidate => re.test(candidate.text));
        gaps.push(line ? x1 - x0 - 36 - (line.bbox.x + line.bbox.w) : NaN);
      } finally {
        page.destroy();
      }
    }
    return gaps;
  } finally {
    doc.destroy();
  }
}

test('drive batch: a footer with {bates} over a folder, the open document untouched, rerun refuses, cancelAfter', { skip: CI_NO_ELECTRON, timeout: 300_000 }, async () => {
  for (const [name, want] of Object.entries(INPUTS)) assert.equal(await sha(join(root, 'fixtures', name)), want, `${name} before`);
  const text = await readFile(join(root, SCRIPT), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.deepEqual([Number(header[1]), Number(header[2])], [7, 0]);
  assert.equal(lines.length, 7);
  const dir = await stage();
  const { code, report } = await drive(SCRIPT, dir);
  const steps = report.steps;
  assert.equal(steps.length, 7, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const S = n => steps[n - 1];
  assert.deepEqual(S(3).state.batch.files.map(row => row.name), ['encrypted.pdf', 'hello.pdf', 'nav.pdf', 'rotated.pdf']);
  assert.deepEqual(S(7).state.batch.files.map(row => row.status), ['failed', 'ok', 'queued', 'queued']);

  // The outputs: footer text and Bates continuity in listing order (§9.8), page counts unchanged.
  const out = join(dir, 'batch-in', 'pdf4all-out');
  assert.deepEqual((await readdir(out)).sort(), ['hello.pdf', 'nav.pdf', 'report.txt', 'rotated.pdf']);
  const hello = await pdfWords(join(out, 'hello.pdf'));
  assert.equal(hello.length, 1);
  assert.match(hello[0], /B-0001/);
  const nav = await pdfWords(join(out, 'nav.pdf'));
  assert.equal(nav.length, 3);
  for (const [index, want] of ['B-0002', 'B-0003', 'B-0004'].entries()) {
    assert.match(nav[index], new RegExp(want));
    assert.doesNotMatch(nav[index], /Open doc/, 'the open document header never reaches the batch output');
  }
  const rotated = await pdfWords(join(out, 'rotated.pdf'));
  assert.equal(rotated.length, 1);
  assert.match(rotated[0], /B-0005/);
  // align:right reaches the output bytes: each B-000n line ends within 10 pt of pageWidth − 36 (rotated.pdf included).
  for (const name of ['hello.pdf', 'nav.pdf', 'rotated.pdf']) {
    for (const [index, gap] of (await rightGaps(join(out, name), /B-\d{4}/)).entries()) assert.ok(Math.abs(gap) <= 10, `${name} p${index} right gap ${gap}`);
  }
  // §9.13: read after the rerun (step 6), so this exact first-run literal is what proves the rerun never overwrote report.txt.
  const report1 = await readFile(join(out, 'report.txt'), 'utf8');
  assert.equal(report1, 'encrypted.pdf\tfailed\t0\tencrypted\t\nhello.pdf\tok\t1\t\t0001-0001\nnav.pdf\tok\t3\t\t0002-0004\nrotated.pdf\tok\t1\t\t0005-0005\n');
  // cancelAfter 2: two report lines, one output.
  const outC = join(dir, 'batch-c', 'pdf4all-out');
  assert.deepEqual((await readdir(outC)).sort(), ['hello.pdf', 'report.txt']);
  assert.equal(await readFile(join(outC, 'report.txt'), 'utf8'), 'encrypted.pdf\tfailed\t0\tencrypted\t\nhello.pdf\tok\t1\t\t0009-0009\n');
  assert.match((await pdfWords(join(outC, 'hello.pdf')))[0], /B-0009/);
  // The inputs are untouched, and nothing lands under fixtures/.
  for (const sub of ['batch-in', 'batch-c']) {
    for (const [name, want] of Object.entries(INPUTS)) assert.equal(await sha(join(dir, sub, name)), want, `${sub}/${name} after`);
  }
  for (const [name, want] of Object.entries(INPUTS)) assert.equal(await sha(join(root, 'fixtures', name)), want, `${name} after`);
  assert.deepEqual((await readdir(join(root, 'fixtures'), { recursive: true })).filter(entry => entry.split(/[\\/]/).pop() === 'pdf4all-out'), []);
});

test('drive batch: a symlinked pdf4all-out is never written through', { skip: CI_NO_ELECTRON, timeout: 300_000 }, async () => {
  const dir = await stage();
  const elsewhere = await mkdtemp(join(tmpdir(), 'pdf4all-batch-elsewhere-'));
  // 'junction' is Windows-only (ignored elsewhere): a directory link that needs no privilege and still reads as a symbolic link.
  await symlink(elsewhere, join(dir, 'batch-in', 'pdf4all-out'), 'junction');
  const script = join(dir, 'symlink.jsonl');
  await writeFile(script, '{"cmd":"batch","folder":"${OUT}/batch-in","contents":{"kind":"footer","text":"B-{bates}","batesPad":4}}\n{"cmd":"batch"}\n');
  const { report } = await drive(script, dir);
  const batch = report.steps[0].state.batch;
  assert.deepEqual(batch.files.map(row => [row.name, row.status, row.detail]), [
    ['encrypted.pdf', 'failed', 'encrypted'],
    ['hello.pdf', 'failed', 'unwritable'],
    ['nav.pdf', 'failed', 'unwritable'],
    ['rotated.pdf', 'failed', 'unwritable'],
  ]);
  assert.equal(batch.report, 'unwritable');
  assert.equal(batch.bates.next, batch.bates.start, 'no write, no counter move');
  assert.deepEqual(await readdir(elsewhere), []);
  // §9.9: a drive batch line without contents is refused, never the menu's dialog.
  assert.equal(report.steps[1].state.error, 'batch: contents required');
  assert.equal(report.steps[1].state.batch.running, false);
});
