import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { chmod, copyFile, mkdtemp as makeTempDir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import { makeThreePagePdf } from '../fixtures/textpages.mjs';
import { appBinary } from './app-bin.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
/** Windows has no POSIX permission bits (stat reports 0666 or 0444). */
const WIN = process.platform === 'win32';
const python = join(root, WIN ? '.venv-qa/Scripts/python.exe' : '.venv-qa/bin/python3');
/** The smoke tests launch the app (tests/app-bin.mjs); TANDEM_NO_APP=1 skips them. */
const skipApp = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;
// The file-safety unit tests of Electron's safe-write.ts are Rust tests now: crates/tandem-app/tests/safe_write.rs.

function verifyTextNote(pdf) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, '--text-notes', '1', '--contents', 'PDF4All P2 note',
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  assert.equal(verdict.count, 1);
  assert.equal(verdict.no_author, true);
}

async function runSmoke(args, timeoutMs = 60_000) {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-p1-save-'));
  const reportPath = join(dir, 'report.json');
  const child = spawn(appBinary, [`--user-data-dir=${join(dir, 'udata')}`, '--smoke', `--report=${reportPath}`, ...args], {
    cwd: root,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => { stdout += chunk; });
  let timedOut = false;
  const timer = setTimeout(() => { timedOut = true; child.kill(); }, timeoutMs);
  const code = await new Promise(resolve => child.on('close', resolve));
  clearTimeout(timer);
  let report = null;
  try { report = JSON.parse(await readFile(reportPath, 'utf8')); } catch { /* no report */ }
  return { code, stdout, report, timedOut, dir };
}

test('smoke --open hello --save-to writes a copy and leaves the source', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/hello.pdf');
  const before = await stat(fixture);
  const original = await readFile(fixture);
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-save-to-'));
  const dest = join(dir, 'hello-copy.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.save?.name, 'hello-copy.pdf');
  assert.equal(report?.save?.validated, true);
  assert.equal(report?.sourceUnchanged, true);
  const written = await readFile(dest);
  assert.equal(written.byteLength, report?.save?.bytes);
  assert.ok(written.byteLength >= 5);
  const after = await stat(fixture);
  assert.equal(after.mtimeMs, before.mtimeMs);
  assert.deepEqual(await readFile(fixture), original);
});

test('smoke --edit=note --save-to writes one Text note and no /T', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/hello.pdf');
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-save-'));
  const dest = join(dir, 'hello-note.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=note', `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.notes, 1);
  assert.equal(report?.save?.validated, true);
  verifyTextNote(dest);
});

test('smoke --edit=note --save-in-place rewrites a scratch copy', { skip: skipApp, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-in-place-'));
  const scratchPath = join(dir, 'hello.pdf');
  await copyFile(join(root, 'fixtures/hello.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const before = await stat(scratchPath);
  const beforeBytes = await readFile(scratchPath);
  const { code, report, timedOut } = await runSmoke([`--open=${scratchPath}`, '--edit=note', '--save-in-place']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.save?.validated, true);
  const after = await stat(scratchPath);
  const afterBytes = await readFile(scratchPath);
  assert.notDeepEqual(afterBytes, beforeBytes);
  assert.equal(after.mode & 0o777, before.mode & 0o777);
  verifyTextNote(scratchPath);
});

test('smoke --edit=note then undo reports notes 0', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/hello.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=note']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.undo?.notes, 0);
});

function verifyAnnots(pdf, map, contents, type) {
  const args = [join(root, 'scripts/verify-roundtrip.py'), pdf, '--annots', JSON.stringify(map)];
  if (contents) args.push('--contents', contents);
  if (type) args.push('--appearance', type);
  const checked = spawnSync(python, args, { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('smoke --edit=highlight --save-to writes one Highlight', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/hello.pdf');
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-hl-'));
  const dest = join(dir, 'hello-highlight.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=highlight', `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.annots?.Highlight, 1);
  assert.equal(report?.save?.validated, true);
  const verdict = verifyAnnots(dest, { Highlight: 1 }, undefined, 'Highlight');
  assert.ok(verdict.appearance.inside_frac > 0.3);
});

test('smoke --edit=freetext --save-in-place rewrites a scratch copy', { skip: skipApp, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-ft-'));
  const scratchPath = join(dir, 'hello.pdf');
  await copyFile(join(root, 'fixtures/hello.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const before = await stat(scratchPath);
  const beforeBytes = await readFile(scratchPath);
  const { code, report, timedOut } = await runSmoke([`--open=${scratchPath}`, '--edit=freetext', '--save-in-place']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.save?.validated, true);
  const after = await stat(scratchPath);
  const afterBytes = await readFile(scratchPath);
  assert.notDeepEqual(afterBytes, beforeBytes);
  assert.equal(after.mode & 0o777, before.mode & 0o777);
  const verdict = verifyAnnots(scratchPath, { FreeText: 1 }, 'PDF4All P2 freetext', 'FreeText');
  assert.ok(verdict.appearance.inside_frac > 0.01);
});

test('smoke --edit=highlight then undo reports Highlight 0', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/hello.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=highlight']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.undo?.annots?.Highlight, 0);
});

const PAGES_AFTER = { count: 2, rotate: [0, 90], order: ['three', 'one'] };

async function makePagesPdf(dest) {
  await writeFile(dest, makeThreePagePdf());
}

function verifyPages(pdf, expected) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, '--pages', JSON.stringify(expected),
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

function tabPeakMiB(report) {
  const tabs = (report?.memory ?? []).filter(m => m.type === 'Tab');
  const rss = (tabs.length ? tabs : report?.memory ?? []).map(m => m.peakWorkingSetSize ?? m.workingSetSize ?? 0);
  return Math.max(0, ...rss) / 1024; // Electron MemoryInfo is KiB
}

test('smoke --edit=pages --save-in-place rewrites a 3-page scratch', { skip: skipApp, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-pages-'));
  const scratchPath = join(dir, 'pages.pdf');
  await makePagesPdf(scratchPath);
  await chmod(scratchPath, 0o644);
  const { code, report, timedOut } = await runSmoke([`--open=${scratchPath}`, '--edit=pages', '--save-in-place']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  assert.deepEqual(report?.edit?.pages?.rotate, [0, 90]);
  assert.equal(report?.save?.validated, true);
  verifyPages(scratchPath, PAGES_AFTER);
});

test('smoke --edit=pages then undo restores count 3', { skip: skipApp, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-pages-undo-'));
  const scratchPath = join(dir, 'pages.pdf');
  await makePagesPdf(scratchPath);
  const { code, report, timedOut } = await runSmoke([`--open=${scratchPath}`, '--edit=pages']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  assert.equal(report?.undo?.pages?.count, 3);
});

test('smoke --edit=pages --save-to on large.pdf leaves 499 pages', { skip: skipApp, timeout: 120_000 }, async () => {
  const fixture = join(root, 'fixtures/large.pdf');
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-pages-large-'));
  const dest = join(dir, 'large-pages.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=pages', `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 499);
  assert.equal(report?.save?.validated, true);
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

// fill-anywhere D7: a filled document's Save writes a "(filled)" copy, so a smoke save-in-place
// (no path to give) is cancelled and the scratch is never rewritten. The old field/appearance
// assertions moved to tests/fill-anywhere.test.mjs (c), where the copy holds them.
test('smoke --edit=form --save-in-place is cancelled on a filled form2 scratch (Save writes a copy)', { skip: skipApp, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-form-'));
  const scratchPath = join(dir, 'form2.pdf');
  await copyFile(join(root, 'fixtures/form2.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const before = await readFile(scratchPath);
  const { report, timedOut } = await runSmoke([`--open=${scratchPath}`, '--edit=form', '--save-in-place']);
  assert.equal(timedOut, false);
  assert.equal(report?.ok, false);
  assert.match(String(report?.error), /P1_SAVE_ERROR .*cancelled/);
  assert.deepEqual(await readFile(scratchPath), before, 'the scratch bytes are unchanged');
  assert.deepEqual((await readdir(dir)).sort(), ['form2.pdf'], 'nothing else was written');
});

test('smoke --edit=form then undo reverts size', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/form2.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=form']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.fields?.values?.size, 'M');
  assert.equal(report?.undo?.fields?.values?.size, '');
});

const NAV_OUTLINE_BEFORE = [['One', 0], ['Two', 1], ['Two-A', 1], ['Three', 2]];
const NAV_LINKS_BEFORE = {
  0: [{ page: 1 }, { uri: 'https://example.com/' }],
  1: [],
  2: [{ page: 1 }, { blocked: 'file' }],
};
const NAV_OUTLINE_AFTER = [['One', 0], ['Two', null], ['Two-A', null], ['Three', 1], ['PDF4All P2 nav', 1]];
const NAV_LINKS_AFTER = {
  0: [{ uri: 'https://example.com/' }, { page: 1 }],
  1: [{ blocked: 'file' }],
};

function verifyNav(pdf, flag, expected) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, flag, JSON.stringify(expected),
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('nav.pdf --outline/--links match the contract fixture', () => {
  const fixture = join(root, 'fixtures/nav.pdf');
  verifyNav(fixture, '--outline', NAV_OUTLINE_BEFORE);
  verifyNav(fixture, '--links', NAV_LINKS_BEFORE);
});

test('smoke --edit=nav --save-in-place rewrites a nav scratch', { skip: skipApp, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-nav-'));
  const scratchPath = join(dir, 'nav.pdf');
  await copyFile(join(root, 'fixtures/nav.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const before = await stat(scratchPath);
  const { code, report, timedOut, stdout } = await runSmoke([`--open=${scratchPath}`, '--edit=nav', '--save-in-place']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  const last = report?.edit?.nav?.outline?.at(-1);
  assert.equal(last?.title, 'PDF4All P2 nav');
  assert.equal(last?.page, 1);
  assert.equal(report?.save?.validated, true);
  const after = await stat(scratchPath);
  assert.equal(after.mode & 0o777, before.mode & 0o777);
  verifyNav(scratchPath, '--outline', NAV_OUTLINE_AFTER);
  verifyNav(scratchPath, '--links', NAV_LINKS_AFTER);
  const links = [...stdout.matchAll(/P2_LINK ({.*})/g)].map(m => JSON.parse(m[1]));
  assert.ok(links.some(item => item.allowed === false && String(item.uri).startsWith('file:')), stdout);
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('smoke --edit=nav then undo restores page 2', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/nav.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=nav']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  assert.equal(report?.undo?.pages?.count, 3);
  assert.equal(report?.undo?.nav?.outline?.find(item => item.title === 'Two')?.page, 1);
});

function verifyFieldsUnique(pdf) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, '--fields-unique',
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('form2.pdf --fields-unique reports 9 top-level names', () => {
  const verdict = verifyFieldsUnique(join(root, 'fixtures/form2.pdf'));
  assert.equal(verdict.keys, 9);
  assert.equal(verdict.top, 9);
});

test('smoke --edit=assemble --insert-from=form2 --save-in-place merges unique fields', { skip: skipApp, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-assemble-'));
  const scratchPath = join(dir, 'form2.pdf');
  const source = join(root, 'fixtures/form2.pdf');
  await copyFile(source, scratchPath);
  await chmod(scratchPath, 0o644);
  const before = await stat(scratchPath);
  const { code, report, timedOut } = await runSmoke([
    `--open=${scratchPath}`, '--edit=assemble', `--insert-from=${source}`, '--save-in-place',
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  assert.equal(report?.edit?.assemble?.inserted, 1);
  assert.equal(new Set(report?.edit?.assemble?.fieldNames ?? []).size, 18);
  assert.equal(report?.save?.validated, true);
  const after = await stat(scratchPath);
  assert.equal(after.mode & 0o777, before.mode & 0o777);
  verifyPages(scratchPath, { count: 2 });
  const unique = verifyFieldsUnique(scratchPath);
  assert.equal(unique.keys, 18);
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('smoke --edit=assemble then undo restores page count', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/form2.pdf');
  const { code, report, timedOut } = await runSmoke([
    `--open=${fixture}`, '--edit=assemble', `--insert-from=${fixture}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  assert.equal(report?.undo?.pages?.count, 1);
});

test('smoke --edit=extract --save-to writes page 2 of nav', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/nav.pdf');
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-extract-'));
  const dest = join(dir, 'two.pdf');
  const { code, report, timedOut } = await runSmoke([
    `--open=${fixture}`, '--edit=extract', `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.deepEqual(report?.edit?.extract?.pages, [1]);
  verifyPages(dest, { count: 1, rotate: [0], order: ['two'] });
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

function pngSize(buf) {
  if (buf.length < 24 || buf[0] !== 0x89 || buf[1] !== 0x50) return null;
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
}

function verifyImagePage(pdf, expected) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, '--image-page', JSON.stringify(expected),
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('smoke --edit=export-png --save-to writes a PNG of hello page 1', { skip: skipApp, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-export-png-')), 'hello.png');
  const { code, report, timedOut } = await runSmoke([
    `--open=${join(root, 'fixtures/hello.pdf')}`, '--edit=export-png', `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  const buf = await readFile(dest);
  assert.equal(buf[0], 0x89);
  assert.equal(buf[1], 0x50);
  const size = pngSize(buf);
  assert.ok(size && size.width >= 600 && size.height >= 700, JSON.stringify(size));
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('smoke --edit=export-jpeg --save-to writes a JPEG of hello page 1', { skip: skipApp, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-export-jpeg-')), 'hello.jpg');
  const { code, report, timedOut } = await runSmoke([
    `--open=${join(root, 'fixtures/hello.pdf')}`, '--edit=export-jpeg', `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  const buf = await readFile(dest);
  assert.equal(buf[0], 0xff);
  assert.equal(buf[1], 0xd8);
});

test('smoke --edit=export-txt --save-to writes hello text', { skip: skipApp, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-export-txt-')), 'hello.txt');
  const { code, report, timedOut } = await runSmoke([
    `--open=${join(root, 'fixtures/hello.pdf')}`, '--edit=export-txt', `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  const text = await readFile(dest, 'utf8');
  assert.match(text, /PDF4All/);
});

test('smoke --new-from-images writes a 2-page PDF', { skip: skipApp, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-from-images-')), 'images.pdf');
  const rgb = join(root, 'fixtures/img-rgb.png');
  const gray = join(root, 'fixtures/img-gray.png');
  const { code, report, timedOut } = await runSmoke([
    '--new-from-images', `--images=${rgb},${gray}`, `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.new?.pages, 2);
  verifyImagePage(dest, { page: 0, width: 300, height: 200 });
  verifyImagePage(dest, { page: 1, width: 200, height: 300 });
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('export JPEG of the images PDF then new-from-images that JPEG', { skip: skipApp, timeout: 90_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-jpeg-round-'));
  const pdf = join(dir, 'images.pdf');
  const jpg = join(dir, 'page.jpg');
  const back = join(dir, 'from-jpeg.pdf');
  const rgb = join(root, 'fixtures/img-rgb.png');
  const gray = join(root, 'fixtures/img-gray.png');
  const first = await runSmoke(['--new-from-images', `--images=${rgb},${gray}`, `--save-to=${pdf}`]);
  assert.equal(first.code, 0);
  const exported = await runSmoke([`--open=${pdf}`, '--edit=export-jpeg', `--save-to=${jpg}`]);
  assert.equal(exported.code, 0);
  const again = await runSmoke(['--new-from-images', `--images=${jpg}`, `--save-to=${back}`]);
  assert.equal(again.code, 0);
  assert.equal(again.report?.new?.pages, 1);
});

function verifyDecorate(pdf, flag, expected) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, flag, JSON.stringify(expected),
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('layers.pdf --layers lists Base and Notes both on', () => {
  verifyDecorate(join(root, 'fixtures/layers.pdf'), '--layers', [
    { name: 'Base', on: true }, { name: 'Notes', on: true },
  ]);
});

test('smoke --edit=decorate --save-in-place stamps nav.pdf', { skip: skipApp, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-decorate-'));
  const scratchPath = join(dir, 'nav.pdf');
  await copyFile(join(root, 'fixtures/nav.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const { code, report, timedOut } = await runSmoke([
    `--open=${scratchPath}`, '--edit=decorate', '--save-in-place',
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.decorate?.stamps?.watermark, 3);
  assert.equal(report?.edit?.decorate?.stamps?.header, 3);
  assert.equal(report?.edit?.decorate?.labels?.[0], 'i');
  assert.equal(report?.save?.validated, true);
  verifyDecorate(scratchPath, '--labels', ['i', 'ii', 'iii']);
  verifyDecorate(scratchPath, '--text-has', { 0: ['PDF4All P2 decorate', 'Page 1 of 3'] });
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('smoke --edit=decorate --save-in-place hides Notes on layers.pdf', { skip: skipApp, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-layers-'));
  const scratchPath = join(dir, 'layers.pdf');
  await copyFile(join(root, 'fixtures/layers.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const { code, report, timedOut } = await runSmoke([
    `--open=${scratchPath}`, '--edit=decorate', '--save-in-place',
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.decorate?.layers?.[1]?.visible, false);
  if (report?.edit?.decorate?.layersSaved) {
    verifyDecorate(scratchPath, '--layers', [
      { name: 'Base', on: true }, { name: 'Notes', on: false },
    ]);
  }
});

test('smoke --edit=decorate then undo reverts the last op', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/nav.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=decorate']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.undo?.decorate?.labels?.[0], '1');
});

function verifyShapes(pdf) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf,
    '--annots', JSON.stringify({ Square: 1, Circle: 1, Line: 1, Ink: 1, Stamp: 2 }),
    '--appearance', 'Stamp:2',
    '--no-sig',
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  assert.equal(verdict.no_sig?.ok, true);
  assert.ok(verdict.appearance.inside_frac > 0.15);
  return verdict;
}

test('--no-sig on nav.pdf (no certificate signature)', () => {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), join(root, 'fixtures/nav.pdf'), '--no-sig',
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  assert.equal(JSON.parse(checked.stdout).ok, true);
});

test('smoke --edit=shapes --save-in-place writes six annots and no /Sig', { skip: skipApp, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-shapes-'));
  const scratchPath = join(dir, 'nav.pdf');
  await copyFile(join(root, 'fixtures/nav.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const image = join(root, 'fixtures/img-gray.png');
  const { code, report, timedOut } = await runSmoke([
    `--open=${scratchPath}`, '--edit=shapes', `--images=${image}`, '--save-in-place',
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.annots?.Square, 1);
  assert.equal(report?.edit?.annots?.Circle, 1);
  assert.equal(report?.edit?.annots?.Line, 1);
  assert.equal(report?.edit?.annots?.Ink, 1);
  assert.equal(report?.edit?.annots?.Stamp, 2);
  assert.equal(report?.edit?.sign?.subj, 'Signature appearance');
  assert.equal(report?.save?.validated, true);
  verifyShapes(scratchPath);
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('smoke --edit=shapes then undo drops the signature stamp', { skip: skipApp, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/nav.pdf');
  const image = join(root, 'fixtures/img-gray.png');
  const { code, report, timedOut } = await runSmoke([
    `--open=${fixture}`, '--edit=shapes', `--images=${image}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.undo?.annots?.Stamp, 1);
});

function verifyEncryption(pdf, password) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-save.py'), '--encryption', pdf, '--password', password,
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  return JSON.parse(checked.stdout);
}

function p2EditLine(stdout) {
  const line = stdout.split('\n').find(row => row.includes('P2_EDIT '));
  return line ?? '';
}

test('verify-save --encryption reports RC4 and AES-256 fixtures', () => {
  const rc4 = verifyEncryption(join(root, 'fixtures/encrypted.pdf'), 'pdf4all-user');
  assert.equal(rc4.encrypted, true);
  assert.equal(rc4.userOpens, true);
  assert.equal(rc4.emptyOpens, false);
  const aes = verifyEncryption(join(root, 'fixtures/encrypted-aes256.pdf'), 'pdf4all-user');
  assert.equal(aes.encrypted, true);
  assert.equal(aes.userOpens, true);
  assert.equal(aes.emptyOpens, false);
  const noedit = verifyEncryption(join(root, 'fixtures/encrypted-noedit.pdf'), 'pdf4all-user');
  assert.equal(noedit.permissions.edit, false);
  assert.equal(noedit.permissions.annotate, false);
});

test('smoke --edit=encrypt --new-passwords sets AES-256 print+annotate', { skip: skipApp, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-encrypt-')), 'out.pdf');
  const user = 'p2u-user';
  const owner = 'p2u-owner';
  const { code, report, timedOut, stdout } = await runSmoke([
    `--open=${join(root, 'fixtures/hello.pdf')}`, '--edit=encrypt', `--new-passwords=${user}:${owner}`, `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0, stdout);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.security?.mode, 'aes-256');
  const line = p2EditLine(stdout);
  assert.equal(line.includes(user), false, line);
  assert.equal(line.includes(owner), false, line);
  const enc = verifyEncryption(dest, user);
  assert.equal(enc.encrypted, true);
  assert.equal(enc.userOpens, true);
  assert.equal(enc.emptyOpens, false);
});

test('smoke --edit=encrypt-keep keeps RC4 and logs no password', { skip: skipApp, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-encrypt-keep-')), 'out.pdf');
  const pw = 'pdf4all-user';
  const { code, report, timedOut, stdout } = await runSmoke([
    `--open=${join(root, 'fixtures/encrypted.pdf')}`, '--edit=encrypt-keep', `--password=${pw}`, `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0, stdout);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.security?.mode, 'keep');
  const line = p2EditLine(stdout);
  assert.equal(line.includes(pw), false, line);
  const enc = verifyEncryption(dest, pw);
  assert.equal(enc.encrypted, true);
  assert.equal(enc.userOpens, true);
});
