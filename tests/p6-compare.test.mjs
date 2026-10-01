/**
 * Compare (contract-compare.md §1.3 and §3.2 as amended by §9.1-§9.5, §9.8, §9.16).
 * Engine half: createEngine on build/, no worker, no renderer: (a)-(g) plus the signed-intact main doc (§9.1),
 * the locked main doc, the size cap at MAX_PDF_BYTES + 1 (§9.3) and the 2000-line ceiling (§9.7).
 * Drive half: tests/drive/compare.jsonl holds the exact-value expects; this file pins the header counts, the
 * §9.16 ledger, differing > 0 and ratio = differing/total (4 dp) for hello-vs-nav, total = ceil(612S)*ceil(792S).
 * drive()/pick()/red() are copies of tests/p6-drive8.test.mjs (not shared on purpose).
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
import { createEngine, MAX_PDF_BYTES } from '../build/renderer/engine.js';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const SCRIPT = 'tests/drive/compare.jsonl';
const FIXTURES = {
  'fixtures/nav.pdf': '8e7c37500e46120b9bd774bd8aab8d093d9e21aacc522a66216406373229ed82',
  'fixtures/hello.pdf': '229ca9488a139d3f32b864ba24055152fe7f0faf670c9db6491144f23d275b94',
};

const engine = createEngine(mupdf);
const ab = buffer => buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
const fixture = async path => ab(await readFile(join(root, path)));
let nextId = 1;
const op = (session, request) => engine.dispatch({ id: nextId++, ...request }, session).reply;
const hash = pixels => createHash('sha256').update(Buffer.from(pixels)).digest('hex');

async function session(path, name = path.split('/').pop()) {
  const s = engine.createSession();
  if (path) {
    const reply = op(s, { op: 'open', name, pdf: await fixture(path) });
    assert.equal(reply.ok, true, reply.error);
  }
  return s;
}
async function compareOpen(s, path) {
  return op(s, { op: 'compare-open', name: path.split('/').pop(), pdf: await fixture(path) });
}
/** Every number within ±tol (pt). */
function near(got, want, tol, label) {
  assert.ok(Array.isArray(got) && got.length === want.length && got.every((value, i) => Math.abs(value - want[i]) <= tol),
    `${label}: ${JSON.stringify(got)} vs ${JSON.stringify(want)} ±${tol}`);
}

/** p2-decorate.test.mjs:23-35's buildPdf: one letter page, one Helvetica line per entry of `lines`. */
function buildPdf(objects) {
  const parts = ['%PDF-1.4\n'];
  const offsets = [];
  objects.forEach((body, index) => {
    offsets.push(parts.join('').length);
    parts.push(`${index + 1} 0 obj\n${body}\nendobj\n`);
  });
  const xref = parts.join('').length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const offset of offsets) parts.push(`${String(offset).padStart(10, '0')} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  return ab(Buffer.from(parts.join(''), 'latin1'));
}
function linesPdf(lines, size = 18, step = 30) {
  const content = lines.map((text, i) => `BT /F1 ${size} Tf 72 ${740 - i * step} Td (${text}) Tj ET\n`).join('');
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    `<< /Length ${content.length} >>\nstream\n${content}endstream`,
  ]);
}
/** A valid one-page PDF of exactly `total` bytes: an unreferenced zero-filled stream object carries the padding. */
function paddedPdf(total) {
  const head = '%PDF-1.4\n';
  const objs = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>'];
  const build = pad => {
    let body = head;
    const offsets = [];
    for (const [i, o] of objs.entries()) { offsets.push(body.length); body += `${i + 1} 0 obj\n${o}\nendobj\n`; }
    offsets.push(body.length);
    const padHead = `4 0 obj\n<< /Length ${pad} >>\nstream\n`;
    const padTail = '\nendstream\nendobj\n';
    const xref = body.length + padHead.length + pad + padTail.length;
    let tail = `xref\n0 5\n0000000000 65535 f \n`;
    for (const off of offsets) tail += `${String(off).padStart(10, '0')} 00000 n \n`;
    tail += `trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
    return [body + padHead, padTail + tail];
  };
  // The pad's digit count sits in the header, so settle it by iterating (converges in two rounds).
  let pad = 1000, a, b;
  for (let round = 0; round < 3; round++) { [a, b] = build(pad); pad = total - a.length - b.length; }
  [a, b] = build(pad);
  assert.equal(a.length + pad + b.length, total, 'padded size');
  return ab(Buffer.concat([Buffer.from(a, 'latin1'), Buffer.alloc(pad, 0x20), Buffer.from(b, 'latin1')]));
}
/** The four-op journal-free proof (§0): revision, dirty and canUndo as `engine.journalState` reads them. */
const journal = s => engine.journalState(s);

test('compare (a): compare-render reads the compare doc, not the main doc; the identical pair diffs empty', async () => {
  const s = await session('fixtures/hello.pdf');
  const opened = await compareOpen(s, 'fixtures/nav.pdf');
  assert.equal(opened.ok, true, opened.error);
  assert.equal(opened.name, 'nav.pdf');
  assert.equal(opened.pageCount, 3);
  assert.equal(opened.pages.length, 3);
  near([opened.pages[0].width, opened.pages[0].height], [612, 792], 0.01, 'nav p0 size');

  const b2 = op(s, { op: 'compare-render', page: 2, scale: 1 });
  assert.equal(b2.ok, true, b2.error);
  assert.equal(b2.lines[0].text, 'three');
  const other = await session('fixtures/nav.pdf');
  const main2 = op(other, { op: 'render', page: 2, scale: 1 });
  assert.equal(hash(b2.pixels), hash(main2.pixels), 'compare-render p2 = a separate session\'s render of nav p2');
  assert.equal(b2.pixelWidth, main2.pixelWidth);
  assert.equal(b2.pixelHeight, main2.pixelHeight);

  const b0 = op(s, { op: 'compare-render', page: 0, scale: 1 });
  assert.equal(b0.lines[0].text, 'one');
  const a0 = op(s, { op: 'render', page: 0, scale: 1 });
  assert.equal(a0.lines[0].text, 'PDF4All');
  assert.equal(b0.pixelWidth, a0.pixelWidth);
  assert.equal(b0.pixelHeight, a0.pixelHeight);

  // The identical case: nav.pdf as main too (the open keeps the slot, §1.3(f)/D1).
  assert.equal(op(s, { op: 'open', name: 'nav.pdf', pdf: await fixture('fixtures/nav.pdf') }).ok, true);
  const same = op(s, { op: 'compare-text', pageA: 0, pageB: 0 });
  assert.equal(same.ok, true, same.error);
  assert.deepEqual({ same: same.same, removed: same.removed, added: same.added, moved: same.moved },
    { same: 1, removed: [], added: [], moved: 0 });
  const mainAgain = op(s, { op: 'render', page: 0, scale: 1 });
  const cmpAgain = op(s, { op: 'compare-render', page: 0, scale: 1 });
  assert.equal(hash(mainAgain.pixels), hash(cmpAgain.pixels), 'identical pages: differing 0');
});

test('compare (b): hello vs nav lists every line with its y-down stext bbox', async () => {
  const s = await session('fixtures/hello.pdf');
  assert.equal((await compareOpen(s, 'fixtures/nav.pdf')).ok, true);
  const diff = op(s, { op: 'compare-text', pageA: 0, pageB: 0 });
  assert.equal(diff.ok, true, diff.error);
  assert.equal(diff.same, 0);
  assert.equal(diff.moved, 0);
  assert.deepEqual(diff.removed.map(l => l.text), ['PDF4All']);
  assert.deepEqual(diff.added.map(l => l.text), ['one']);
  near(diff.removed[0].bbox, [72, 53, 204, 102], 0.5, 'PDF4All bbox');
  near(diff.added[0].bbox, [72, 66, 112, 98], 0.5, 'one bbox');
  const p2 = op(s, { op: 'compare-text', pageA: 0, pageB: 2 });
  assert.deepEqual(p2.added.map(l => l.text), ['three']);
  const bad = op(s, { op: 'compare-text', pageA: 0, pageB: 3 });
  assert.equal(bad.ok, false, 'pageB out of range');
});

test('compare (c): the MAIN doc\'s copy bit gates compare-text; an encrypted compare file is refused, the slot kept', async () => {
  const base = mupdf.Document.openDocument(await fixture('fixtures/hello.pdf'), 'application/pdf');
  const locked = base.asPDF().saveToBuffer('encrypt=rc4-128,owner-password=own,user-password=,permissions=-20').asUint8Array();
  base.destroy();
  const s = engine.createSession();
  const opened = op(s, { op: 'open', name: 'nocopy.pdf', pdf: ab(Buffer.from(locked)) });
  assert.equal(opened.ok, true, opened.error);
  assert.equal(opened.info.permissions.copy, false, 'the in-memory main doc forbids copying');
  assert.equal((await compareOpen(s, 'fixtures/nav.pdf')).ok, true);
  const text = op(s, { op: 'compare-text', pageA: 0, pageB: 0 });
  assert.equal(text.ok, false);
  assert.equal(text.reason, 'read-only');
  assert.equal(op(s, { op: 'compare-render', page: 0, scale: 1 }).ok, true, 'compare-render works without copy');

  const enc = await compareOpen(s, 'fixtures/encrypted.pdf');
  assert.equal(enc.ok, false);
  assert.equal(enc.reason, 'encrypted');
  const kept = op(s, { op: 'compare-render', page: 0, scale: 1 });
  assert.equal(kept.ok, true, 'a refused compare-open keeps the previous slot');
  assert.equal(kept.lines[0].text, 'one');

  // §9.7 store-then-destroy: a successful re-open replaces the slot and destroys the previous WASM doc.
  const previous = s.compare.doc;
  assert.equal((await compareOpen(s, 'fixtures/hello.pdf')).ok, true);
  assert.notEqual(s.compare.doc, previous);
  assert.equal(previous.pointer, 0, 'the previous compare doc is destroyed');
  assert.notEqual(s.compare.doc.pointer, 0, 'the new compare doc is live');
});

test('compare (c2): the combined size cap refuses at MAX_PDF_BYTES + 1 before opening; an oversize main open closes the slot', async () => {
  const s = await session('fixtures/hello.pdf');
  const helloBytes = (await fixture('fixtures/hello.pdf')).byteLength;
  assert.equal((await compareOpen(s, 'fixtures/nav.pdf')).ok, true);
  const over = op(s, { op: 'compare-open', name: 'big.pdf', pdf: new ArrayBuffer(MAX_PDF_BYTES - helloBytes + 1) });
  assert.equal(over.ok, false);
  assert.equal(over.reason, 'too-large');
  assert.equal(op(s, { op: 'compare-render', page: 0, scale: 1 }).lines[0].text, 'one', 'slot kept after too-large');
  // The cap itself opens (§9.3 refuses only above MAX_PDF_BYTES).
  const exact = op(s, { op: 'compare-open', name: 'exact.pdf', pdf: paddedPdf(MAX_PDF_BYTES - helloBytes) });
  assert.equal(exact.ok, true, exact.error);
  // §9.19(d): the pair-cap close followed by a failing open still reports compareClosed; the main doc stays.
  const garbage = Buffer.alloc(4096, 0x41);
  garbage.write('%PDF-1.4 garbage');
  const failed = op(s, { op: 'open', name: 'bad.pdf', pdf: ab(garbage) });
  assert.equal(failed.ok, false);
  assert.equal(failed.compareClosed, true, 'a failing open after the pair-cap close carries compareClosed');
  assert.equal(s.compare, null);
  assert.equal(s.name, 'hello.pdf', 'the main doc is kept');

  // A compare file of MAX - hello + 1 bytes with no main doc, then hello as main: together over the cap.
  const alone = engine.createSession();
  const big = op(alone, { op: 'compare-open', name: 'big.pdf', pdf: paddedPdf(MAX_PDF_BYTES - helloBytes + 1) });
  assert.equal(big.ok, true, big.error);
  const main = op(alone, { op: 'open', name: 'hello.pdf', pdf: await fixture('fixtures/hello.pdf') });
  assert.equal(main.ok, true, main.error);
  assert.equal(main.compareClosed, true);
  assert.equal(alone.compare, null);
  assert.equal(op(alone, { op: 'compare-render', page: 0, scale: 1 }).reason, 'no-compare-document');
});

test('compare (d): opening another main doc keeps the slot; compare-close empties it', async () => {
  const s = await session('fixtures/hello.pdf');
  assert.equal((await compareOpen(s, 'fixtures/nav.pdf')).ok, true);
  assert.equal(op(s, { op: 'open', name: 'nav.pdf', pdf: await fixture('fixtures/nav.pdf') }).ok, true);
  assert.equal(s.compare?.name, 'nav.pdf', 'open keeps session.compare');
  assert.equal(op(s, { op: 'compare-render', page: 1, scale: 1 }).lines[0].text, 'two');
  const closed = op(s, { op: 'compare-close' });
  assert.equal(closed.ok, true, closed.error);
  assert.equal(s.compare, null);
  const render = op(s, { op: 'compare-render', page: 0, scale: 1 });
  assert.equal(render.ok, false);
  assert.equal(render.reason, 'no-compare-document');
  assert.equal(op(s, { op: 'compare-text', pageA: 0, pageB: 0 }).reason, 'no-compare-document');
  assert.equal(op(s, { op: 'compare-close' }).ok, true, 'closing an empty slot is a no-op');
});

test('compare (e): no compare op journals, dirties or moves the revision; one undo restores rotation 0', async () => {
  const s = await session('fixtures/hello.pdf');
  const rotated = op(s, { op: 'page-rotate', page: 0, by: 90 });
  assert.equal(rotated.ok, true, rotated.error);
  const before = journal(s);
  assert.deepEqual(before, { revision: 1, dirty: true, canUndo: true, canRedo: false });
  const steps = [
    { op: 'compare-open', name: 'nav.pdf', pdf: await fixture('fixtures/nav.pdf') },
    { op: 'compare-render', page: 0, scale: 1 },
    { op: 'compare-text', pageA: 0, pageB: 0 },
    { op: 'compare-close' },
    { op: 'compare-open', name: 'nav.pdf', pdf: await fixture('fixtures/nav.pdf') },
  ];
  for (const step of steps) {
    const reply = op(s, step);
    assert.equal(reply.ok, true, `${step.op}: ${reply.error}`);
    assert.deepEqual(journal(s), before, `${step.op} leaves the journal alone`);
  }
  assert.equal(op(s, { op: 'render', page: 0, scale: 1 }).width, 792, 'rotated');
  assert.equal(op(s, { op: 'undo' }).ok, true);
  assert.equal(op(s, { op: 'render', page: 0, scale: 1 }).width, 612, 'one undo restores rotation 0');
  assert.equal(journal(s).canUndo, false, 'the compare ops left no journal step');
});

test('compare (f): closing the main doc keeps the slot; compare-text then refuses no-document', async () => {
  const s = await session('fixtures/hello.pdf');
  assert.equal((await compareOpen(s, 'fixtures/nav.pdf')).ok, true);
  assert.equal(op(s, { op: 'close' }).ok, true);
  assert.equal(s.compare?.name, 'nav.pdf');
  assert.equal(op(s, { op: 'compare-render', page: 0, scale: 1 }).ok, true);
  assert.equal(op(s, { op: 'compare-text', pageA: 0, pageB: 0 }).reason, 'no-document');
  // compare-open needs no main doc either.
  const empty = engine.createSession();
  assert.equal((await compareOpen(empty, 'fixtures/nav.pdf')).ok, true);
});

test('compare (g): line LCS on normalised text', async () => {
  const s = engine.createSession();
  assert.equal(op(s, { op: 'open', name: 'a.pdf', pdf: linesPdf(['alpha', 'beta', 'gamma']) }).ok, true);
  assert.equal(op(s, { op: 'compare-open', name: 'b.pdf', pdf: linesPdf(['alpha', 'gamma', 'delta']) }).ok, true);
  const diff = op(s, { op: 'compare-text', pageA: 0, pageB: 0 });
  assert.equal(diff.ok, true, diff.error);
  assert.equal(diff.same, 2);
  assert.deepEqual(diff.removed.map(l => l.text), ['beta']);
  assert.deepEqual(diff.added.map(l => l.text), ['delta']);
  assert.equal(diff.moved, 0);

  const t = engine.createSession();
  assert.equal(op(t, { op: 'open', name: 'a.pdf', pdf: linesPdf(['a  b']) }).ok, true);
  assert.equal(op(t, { op: 'compare-open', name: 'b.pdf', pdf: linesPdf(['a b']) }).ok, true);
  const spaces = op(t, { op: 'compare-text', pageA: 0, pageB: 0 });
  assert.deepEqual({ same: spaces.same, removed: spaces.removed, added: spaces.added }, { same: 1, removed: [], added: [] });
});

test('compare §9.1/§9.7: a signed main doc compares; a locked main doc and a 2001-line page refuse', async () => {
  const signed = await session('fixtures/p0b/signed-intact.pdf');
  assert.equal((await compareOpen(signed, 'fixtures/nav.pdf')).ok, true);
  const text = op(signed, { op: 'compare-text', pageA: 0, pageB: 0 });
  assert.equal(text.ok, true, text.error);

  const locked = await session('fixtures/encrypted.pdf');
  assert.equal((await compareOpen(locked, 'fixtures/nav.pdf')).ok, true);
  assert.equal(op(locked, { op: 'compare-text', pageA: 0, pageB: 0 }).reason, 'read-only');

  const many = engine.createSession();
  const lines = Array.from({ length: 2001 }, (_, i) => `l${i}`);
  assert.equal(op(many, { op: 'open', name: 'many.pdf', pdf: linesPdf(lines, 0.3, 0.36) }).ok, true);
  assert.equal(op(many, { op: 'compare-open', name: 'b.pdf', pdf: linesPdf(['x']) }).ok, true);
  const refused = op(many, { op: 'compare-text', pageA: 0, pageB: 0 });
  assert.equal(refused.ok, false);
  assert.equal(refused.reason, 'too-many');
});

// ------------------------------------------------------------------ drive half (§3.1/§3.2, §9.16)

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
const sha = async path => createHash('sha256').update(await readFile(join(root, path))).digest('hex');
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, contents: step.contents, compare: step.state?.compare, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error });

/** §9.8: pixels.total is the letter-page grid at pixels.scale, ratio is differing/total to 4 dp. */
function pixelsOk(pixels, label) {
  assert.ok(pixels && typeof pixels.scale === 'number', `${label}: pixels ${JSON.stringify(pixels)}`);
  assert.equal(pixels.scale, Math.min(1, Math.floor(4096 / 792 * 100) / 100), `${label}: S`);
  assert.equal(pixels.total, Math.ceil(612 * pixels.scale) * Math.ceil(792 * pixels.scale), `${label}: total`);
  assert.equal(pixels.ratio, Math.round(pixels.differing / pixels.total * 10000) / 10000, `${label}: ratio`);
}

test('drive Compare: hello vs nav side by side, the slot survives undo and open, close empties it', { skip: CI_NO_ELECTRON, timeout: 300_000 }, async () => {
  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} before`);
  const text = await readFile(join(root, SCRIPT), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.deepEqual([Number(header[1]), Number(header[2])], [12, 0]);
  assert.equal(lines.length, 12, 'header step count matches the file');
  assert.equal(lines.filter(line => JSON.parse(line).cmd === 'click').length, 0, 'header click count matches the file');

  const { code, report } = await drive(SCRIPT, 150_000);
  const steps = report.steps.filter(step => step.cmd !== undefined);
  assert.equal(steps.length, 12);
  assert.deepEqual(red(steps).map(brief), [], 'every expect holds');
  assert.equal(code, 0);

  // §9.16 ledger: labels journal once (rev 1), no compare step moves it, undo is rev 2, the open resets to 0.
  assert.deepEqual(steps.map(step => step.state.revision), [0, 1, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0]);
  assert.deepEqual(steps.map(step => step.state.compare.open), [false, false, true, true, true, true, true, true, true, true, true, false]);
  const opened = pick(report, 'compare', { from: 'fixtures/nav.pdf' });
  assert.ok(opened.state.compare.pixels.differing > 0, 'hello vs nav p0 differs');
  pixelsOk(opened.state.compare.pixels, 'S3');
  assert.equal(opened.state.announcement, `Page 1 vs 1: 0 lines same, 1 removed, 1 added; ${(opened.state.compare.pixels.ratio * 100).toFixed(1)} % of pixels differ`);
  const b2 = steps[3].state.compare.pixels;
  assert.ok(b2.differing > 0, 'hello vs nav p1 differs');
  pixelsOk(b2, 'S4');
  assert.deepEqual([steps[3].state.compare.pageB, steps[4].state.compare.pageB], [1, 2], 'S5: pageB 9 clamps to 2 (§9.19(a))');
  assert.ok(steps[4].state.compare.pixels.differing > 0, 'hello vs nav p2 differs');
  const refused = pick(report, 'compare', { from: 'fixtures/encrypted.pdf' });
  assert.ok(refused.state.error.includes('encrypted.pdf'), `S7 error names the file: ${refused.state.error}`);
  const identical = steps[8].state.compare.pixels;
  assert.equal(identical.differing, 0, 'nav vs nav: identical');
  pixelsOk(identical, 'S9');
  assert.equal(steps[9].state.compare.pixels.differing, 0, 'nav p1 vs p1');
  assert.ok(steps[10].state.compare.pixels.differing > 0, 'nav p1 vs p2 differs');
  assert.equal(steps[11].state.compare.showDiff, false);

  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} after`);
});
