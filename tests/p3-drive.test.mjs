/**
 * `--drive=<script.jsonl>`: generic replay over the menu IPC with one STATE per command
 * (design-agent-drive.md). Scripts under tests/drive/ are the reviewer's acceptance runs.
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdtemp as makeTempDir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const skip = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;

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

async function savedBytes(dir, name) {
  return (await stat(join(dir, name))).size;
}

test('drive: open → note → undo → redo → save-to, one STATE per step', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/note-undo-save.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.ok, true);
  assert.deepEqual(report.steps.map(step => step.cmd), ['open', 'note', 'undo', 'redo', 'save-to', undefined]);
  assert.ok(report.steps.every(step => step.ok === true), JSON.stringify(report.steps.filter(step => step.ok !== true)));
  assert.equal(await savedBytes(dir, 'note.pdf'), report.steps[4].state.save.bytes);
});

test('drive: content edit by target text, overflow refusal, save-to', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/content-edit.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.steps[1].state.content.edited.to, 'BETA');
  assert.equal(await savedBytes(dir, 'content.pdf'), report.steps[3].state.save.bytes);
});

test('drive: password unlock, page op, wait, print-to; no password in the report', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir, stdout } = await drive('tests/drive/password-pages.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.steps[1].state.doc.needsPassword, false);
  assert.equal(report.steps[4].state.print.ok, true);
  assert.ok((await readFile(join(dir, 'printed.pdf'))).subarray(0, 5).toString() === '%PDF-');
  assert.ok(!stdout.includes('pdf4all-user'), 'report must not echo the password');
});

test('drive: a failed expect exits 1 and names the step; a bad open lands in state.error', { skip, timeout: 120_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-drive-neg-'));
  const script = join(dir, 'neg.jsonl');
  await writeFile(script, [
    '{"cmd": "open", "path": "fixtures/does-not-exist.pdf", "expect": {"doc": {"name": "hello.pdf"}}}',
    '{"cmd": "note", "contents": "x", "expect": {"notes": 2}}',
  ].join('\n') + '\n');
  const { code, report } = await drive(script);
  assert.equal(code, 1);
  assert.equal(report.ok, false);
  assert.equal(report.steps[0].ok, true, 'a missing file leaves the fixture open');
  assert.match(report.steps[0].state.error, /does-not-exist\.pdf/);
  assert.equal(report.steps[1].ok, false);
  assert.equal(report.steps[1].state.notes, 1);
});

test('drive: U1b copy-on-write edit inside a Form drawn twice, saved copy keeps the other placement', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/cow-edit.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.steps[1].state.content.edited.to, 'BRAV');
  assert.equal(await savedBytes(dir, 'cow.pdf'), report.steps[4].state.save.bytes);
});

test('drive: U3 subset CID edit by target text, repertoire refusal, undo/redo/save-to', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/subset-edit.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.steps[1].state.content.edited.to, 'テスト日本');
  assert.deepEqual(report.steps[2].state.content.refused, [{ text: '隣の行', detail: 'unsupported-encoding' }]);
  assert.equal(await savedBytes(dir, 'subset.pdf'), report.steps[5].state.save.bytes);
});

test('drive: U4 move/delete/replace on placements, undo/redo all three, save-to', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/object-edit.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.deepEqual(report.steps.slice(1, 4).map(step => step.state.content.edited), [
    { kind: 'image', action: 'move', name: 'Im1' },
    { kind: 'form', action: 'delete', name: 'Fm2' },
    { kind: 'image', action: 'replace', name: 'Im1' },
  ]);
  assert.ok(report.steps.slice(1, 4).every(step => step.state.content.refused.length === 0), JSON.stringify(report.steps.slice(1, 4).map(step => step.state.content.refused)));
  assert.equal(report.steps[6].state.dirty, false, 'three undos return to the open bytes');
  assert.equal(report.steps[9].state.dirty, true, 'three redos reapply them');
  assert.equal(await savedBytes(dir, 'objects.pdf'), report.steps[10].state.save.bytes);
});

test('drive: U5 page-scoped find/replace, one undo reverts the set, page 1 intact', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/replace-page.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.steps[1].state.content.replaced.applied, 4);
  assert.equal(report.steps[1].state.content.replaced.matches, 4);
  assert.deepEqual(report.steps[1].state.content.replaced.rejected.map(item => item.detail),
    ['ambiguous-text-association']);
  assert.equal(report.steps[2].state.dirty, false, 'one undo reverts the whole set');
  assert.equal(await savedBytes(dir, 'replace.pdf'), report.steps[4].state.save.bytes);
  assert.equal(report.steps[5].state.content.replaced.matches, 1, 'page 1 still has ALPHA');
  assert.equal(report.steps[5].state.content.replaced.applied, 0, 'a preview-only line writes nothing');
  assert.equal(report.steps[6].state.content.replaced.applied, 1, 'a selected-run scope applies (review U5)');
});

test('drive: text-add, growth edit, TJ edit and save-to', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/text-add-grow.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  const add = report.steps[1].state.content.lastAdd;
  assert.ok(add?.targetId);
  assert.ok(add.bounds?.length === 4);
  assert.equal(report.steps[2].state.content.edited.to, 'Added longer');
  assert.equal(report.steps[3].state.content.replaced.applied, 1);
  assert.ok((await savedBytes(dir, 'text-add.pdf')) > 0);
});

test('drive: U6 form designer — create, fill, rename, resize, delete, undo/redo, save, refusals', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/field-design.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.ok, true, JSON.stringify(report.steps.filter(step => step.ok !== true)));

  const byStep = index => report.steps[index].state.fields;
  assert.equal(byStep(1).defs.n1.kind, 'text');
  assert.equal(byStep(3).defs.d1.kind, 'combo');
  assert.deepEqual(byStep(4).defs['g.B'].rect, [110, 180, 126, 196]);
  assert.deepEqual(byStep(5).defs['g2.A'].rect, [72, 180, 88, 196], '§7.11: a group rename re-keys every kid');
  assert.equal(byStep(5).defs['g2.B'].kind, 'radio');
  assert.equal(byStep(6).values.n1, 'abc', 'the same-session fill lands');
  assert.equal(byStep(7).values.c1, true);
  assert.equal(byStep(8).values.d1, 'two');
  assert.equal(byStep(9).values.g2, 'B');
  assert.deepEqual(byStep(11).defs.c1.rect, [72, 110, 104, 142], 'a resize reports the new rect');
  assert.equal(byStep(12).last.action, 'delete');
  assert.equal(byStep(13).count, 5, 'one undo brings the deleted field back');
  assert.equal(byStep(14).count, 4, 'redo takes it away again');
  assert.equal(await savedBytes(dir, 'designed.pdf'), report.steps[15].state.save.bytes);
  assert.deepEqual(byStep(16).values, { n2: 'abc', c1: true, g2: 'B' }, 'the saved copy reopens with the values');
  assert.deepEqual(byStep(18).last, { action: 'add', name: 'name', detail: 'duplicate-name' });
  assert.equal(byStep(19).last.detail, 'scripted');
  assert.equal(byStep(20).last.detail, 'signature');
  assert.equal(byStep(22).last.detail, 'xfa');
  assert.equal(report.steps[17].state.dirty, false, 'no refusal dirtied the reopened document');
});

test('drive: U7 help dialog + label/read-only/required/default authored, refused, undone, saved', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/help-fields.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.ok, true, JSON.stringify(report.steps.filter(step => step.ok !== true)));

  assert.deepEqual(report.steps[1].state.help, { open: true, rows: 11 }, 'the Help menu opens the matrix');
  assert.equal(report.steps[3].state.help.open, false, 'and close closes it');
  assert.equal(report.steps[5].state.help.open, false, 'a dialog does not stay open across edits');
  const defs = index => report.steps[index].state.fields.defs;
  assert.deepEqual(defs(5).t1, { kind: 'text', rect: [72, 300, 252, 324], label: 'Full name', required: true, default: 'Ann' });
  assert.equal(defs(6).c1.default, true, 'a checkbox default is its checked state');
  assert.equal(defs(7).d1.default, 'two', 'a dropdown default is its value');
  assert.equal(defs(8).g1.default, 'B', 'a radio default names the kid');
  assert.equal(defs(9).t1.readOnly, true);
  assert.deepEqual(report.steps[10].state.fields.last, { action: 'fill', name: 't1', detail: 'field-read-only' });
  assert.equal(report.steps[10].state.fields.values.t1, 'Ann', 'the refused fill changed nothing');
  assert.equal(report.steps[12].state.fields.values.t1, 'Bob', 'once cleared, the fill lands');
  assert.equal(report.steps[13].state.fields.values.t1, 'Ann', 'undo takes the fill back');
  assert.equal(report.steps[14].state.fields.defs.t1.readOnly, true, 'the second undo restores the read-only step');
  assert.equal(report.steps[16].state.fields.values.t1, 'Bob', 'two redos put both back');
  assert.equal(await savedBytes(dir, 'help-fields.pdf'), report.steps[18].state.save.bytes);
  const reopened = report.steps[19].state.fields;
  assert.deepEqual(reopened.values, { t1: 'Bob', c1: true, d1: 'two', g1: 'B' }, 'the saved copy reopens with the values');
  assert.deepEqual(reopened.defs.t1, { kind: 'text', rect: [72, 300, 252, 324], label: 'Full name', required: true, readOnly: true, default: 'Ann' });
  assert.deepEqual(reopened.defs.g1, { kind: 'radio', rect: [72, 420, 88, 436], required: true, default: 'B' }, 'the group carries them for its kids');
});

test('drive P4: search redaction applies to a new file and a shared placement is copied (R3)', { skip, timeout: 180_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/redact.jsonl', 150_000);
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.ok, true, JSON.stringify(report.steps.filter(step => step.ok !== true)));
  assert.ok(await savedBytes(dir, 'redacted.pdf') > 0);
  assert.equal(report.steps[3].state.redact.last.ok, true);
  assert.equal(report.steps[7].state.redact.last.ok, true);
  assert.ok(await savedBytes(dir, 'shared.pdf') > 0);
});

// ------------------------------------------------------------------- A2 driveability
// One script per closed gap (contract-a2-driveability.md §6, §8). Each proves its result
// through STATE, not through a log line. Steps are addressed by cmd + line keys, not by
// position: the scripts are grok's and grow steps as values are filled in.

const FORM2_SHA256 = 'f2d77d007f947e7daab0c2b9800b0a91df3ee843ef32f183b53b200b9d941164';

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

test('drive A2: goto/zoom/find/tab and the page-id reorder', { skip, timeout: 120_000 }, async () => {
  const { code, report } = await drive('tests/drive/nav-zoom-find.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.ok, true, JSON.stringify(report.steps.filter(step => step.ok !== true)));

  const opened = pick(report, 'open', { 'state.doc.name': 'nav.pdf' }).state;
  assert.equal(opened.page, 0, 'open shows page 0');
  assert.equal(opened.zoom.mode, 'fit-width');
  assert.deepEqual(opened.search, { query: '', count: 0, index: -1 });
  assert.equal(opened.tab, 'tab-links');
  assert.deepEqual(opened.pages.ids, [3, 4, 5], 'nav.pdf page objects');
  assert.equal(pick(report, 'goto', { contents: { page: 2 } }).state.page, 2, 'goto moves the shown page');
  const refusedGoto = pick(report, 'goto', { contents: { page: 9 } }).state;
  assert.equal(refusedGoto.page, 2, 'an out-of-range goto is refused before goTo');
  assert.match(refusedGoto.error, /goto: page must be an integer/);
  assert.equal(pick(report, 'zoom', { contents: { mode: 'fit-page' } }).state.zoom.mode, 'fit-page');
  assert.deepEqual(pick(report, 'zoom', { contents: { mode: 'fit-width' } }).state.zoom, { mode: 'fit-width' });
  assert.deepEqual(pick(report, 'zoom', { contents: { scale: 1.5 } }).state.zoom, { mode: 'fixed', scale: 1.5 });
  const refusedZoom = pick(report, 'zoom', { contents: { scale: 0.6 } }).state;
  assert.deepEqual(refusedZoom.zoom, { mode: 'fixed', scale: 1.5 }, '0.6 is not a ZOOM_STEP');
  assert.match(refusedZoom.error, /zoom: scale must be one of/);
  assert.deepEqual(pick(report, 'move-up').state.pages.ids, [4, 3, 5], 'move-up reorders the page objects');
  assert.deepEqual(pick(report, 'move-up').state.page, 0);
  assert.deepEqual(pick(report, 'delete-page').state.pages.ids, [3, 5], 'delete-page drops one id');
  assert.equal(pick(report, 'delete-page').state.pages.count, 2);
  assert.equal(pick(report, 'tab', { contents: { id: 'tab-notes' } }).state.tab, 'tab-notes');
  assert.equal(pick(report, 'tab', { contents: { id: 'tab-links' } }).state.tab, 'tab-links');
  const refusedTab = pick(report, 'tab', { contents: { id: 'tab-nope' } }).state;
  assert.equal(refusedTab.tab, 'tab-links', 'an unknown tab id is refused');
  assert.match(refusedTab.error, /tab: no inspector tab/);
  assert.deepEqual(pick(report, 'find', { contents: { query: 'PDF4All' } }).state.search, { query: 'PDF4All', count: 500, index: 0 });
  assert.equal(pick(report, 'find', { contents: { dir: 'next' } }).state.search.index, 1, 'next moves the hit index');
  assert.equal(pick(report, 'find', { contents: { dir: 'prev' } }).state.search.index, 0, 'prev moves it back');
  assert.deepEqual(pick(report, 'find', { contents: { query: 'zzq-no-such-string-a2' } }).state.search,
    { query: 'zzq-no-such-string-a2', count: 0, index: -1 });
  assert.equal(pick(report, 'goto', { contents: { page: 499 } }).state.page, 499);
  assert.deepEqual(pick(report, 'open', { 'state.doc.name': 'large.pdf' }).state.pages.ids.slice(0, 2), [4, 6]);
});

test('drive A2: comment set/delete, underline/strikeout, undo/redo, save/reopen', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/annot-edit.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.ok, true, JSON.stringify(report.steps.filter(step => step.ok !== true)));

  const added = pick(report, 'note', { contents: 'A2 note one' }).state;
  assert.deepEqual(added.comments, [{ index: 0, type: 'Text', contents: 'A2 note one', editable: true }]);
  const edited = pick(report, 'annot', { contents: { action: 'set', index: 0, contents: 'A2 note edited' } }).state;
  assert.deepEqual(edited.annotLast, { action: 'set', index: 0 });
  assert.equal(edited.comments[0].contents, 'A2 note edited');
  const deleted = pick(report, 'annot', { contents: { action: 'delete', index: 0 } }).state;
  assert.deepEqual(deleted.annotLast, { action: 'delete', index: 0 });
  assert.deepEqual(deleted.comments, [], 'the row is gone');
  assert.equal(pick(report, 'underline').state.annots.Underline, 1);
  assert.equal(pick(report, 'strikeout').state.annots.StrikeOut, 1);
  assert.equal(pick(report, 'undo').state.annots.StrikeOut, 0, 'undo takes the last markup back');
  assert.equal(pick(report, 'redo').state.annots.StrikeOut, 1, 'redo puts it back');
  const saved = pick(report, 'save-to', { 'state.save.name': 'annot-edit.pdf' });
  assert.equal(saved.state.save.ok, true);
  assert.equal(await savedBytes(dir, 'annot-edit.pdf'), saved.state.save.bytes);
  const reopened = pick(report, 'open', { 'state.doc.name': 'annot-edit.pdf' }).state;
  assert.equal(reopened.annots.Underline, 1);
  assert.equal(reopened.annots.StrikeOut, 1, 'the saved copy carries both markups');
  assert.equal(reopened.notes, 0, 'no Text note survived the delete');
});

test('drive A2: insert with a range, watermark/header ranges, labels set/clear and refusals', { skip, timeout: 120_000 }, async () => {
  const { code, report } = await drive('tests/drive/dialogs.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.ok, true, JSON.stringify(report.steps.filter(step => step.ok !== true)));

  const inserted = pick(report, 'insert', { from: 'fixtures/hello.pdf', contents: { after: 1, range: '1' } }).state;
  assert.equal(inserted.pages.count, 4, 'one page inserted after page 1');
  assert.deepEqual(inserted.assemble, { inserted: 1, source: 'hello.pdf', fieldNames: [], last: { action: 'insert', ok: true } });
  const badRange = pick(report, 'insert', { contents: { after: 1, range: '9-12' } }).state;
  assert.equal(badRange.pages.count, 4, 'a range past the source is refused');
  assert.deepEqual(badRange.assemble.last, { action: 'insert', ok: false, detail: 'invalid' });
  const missing = pick(report, 'insert', { from: 'fixtures/missing-a2.pdf' }).state;
  assert.equal(missing.pages.count, 4, 'a missing source is refused');
  assert.equal(missing.assemble.last.detail, 'unreadable');
  assert.deepEqual(pick(report, 'watermark').state.decorate.last, { action: 'stamp', kind: 'watermark', pages: [0, 1], ok: true });
  assert.deepEqual(pick(report, 'header-footer').state.decorate.last, { action: 'stamp', kind: 'header', pages: [0, 1, 2], ok: true });
  const set = pick(report, 'labels', { contents: { from: 1, style: 'A', prefix: '', start: 1 } }).state;
  assert.deepEqual(set.decorate.last, { action: 'labels-set', ok: true });
  assert.deepEqual(set.decorate.labels, ['A']);
  const invalid = pick(report, 'labels', { contents: { from: 1, style: 'Q', prefix: '', start: 1 } }).state;
  assert.deepEqual(invalid.decorate.last, { action: 'labels-set', ok: false, detail: 'invalid-style' });
  assert.deepEqual(invalid.decorate.labels, ['A'], 'an unknown style never writes');
  assert.equal(invalid.dirty, true, 'and never dirties beyond the set before it');
  const cleared = pick(report, 'labels', { contents: { clear: true, from: 1 } }).state;
  assert.deepEqual(cleared.decorate.last, { action: 'labels-clear', ok: true });
  assert.deepEqual(cleared.decorate.labels, ['1'], 'clear restores the index labels');
});

test('drive A2: page/uri link, link delete, bookmark add/rename/delete/go', { skip, timeout: 120_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/links-bookmarks.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.ok, true, JSON.stringify(report.steps.filter(step => step.ok !== true)));

  const pageLinks = report.steps.filter(step => step.cmd === 'link'
    && JSON.stringify(step.contents) === JSON.stringify({ kind: 'page', page: 1 }));
  assert.equal(pageLinks.length, 2, 'one refusal before the find, one add after it');
  assert.deepEqual(pageLinks[0].state.nav.last, { action: 'link-add', detail: 'no-anchor' }, 'no find hit, no link');
  assert.equal(pageLinks[0].state.dirty, false, 'the refusal never dirtied');
  assert.deepEqual(pageLinks[1].state.nav.links['0'], [{ kind: 'page', page: 0 }]);
  const uri = pick(report, 'link', { contents: { kind: 'uri', uri: 'https://example.com/a2' } }).state;
  assert.deepEqual(uri.nav.links['0'], [{ kind: 'page', page: 0 }, { kind: 'uri', uri: 'https://example.com/a2' }]);
  const dropped = pick(report, 'link', { contents: { action: 'delete', index: 0 } }).state;
  assert.deepEqual(dropped.nav.links['0'], [{ kind: 'uri', uri: 'https://example.com/a2' }], 'the page link is deleted');
  assert.deepEqual(pick(report, 'bookmark', { contents: { action: 'add', title: 'A2 mark' } }).state.nav.outline, [{ title: 'A2 mark', page: 0 }]);
  assert.deepEqual(pick(report, 'bookmark', { contents: { action: 'rename', index: 0, title: 'A2 renamed' } }).state.nav.outline,
    [{ title: 'A2 renamed', page: 0 }]);
  assert.deepEqual(pick(report, 'bookmark', { contents: { action: 'delete', index: 0 } }).state.nav.outline, []);
  const saved = pick(report, 'save-to', { 'state.save.name': 'links-bookmarks.pdf' });
  assert.equal(await savedBytes(dir, 'links-bookmarks.pdf'), saved.state.save.bytes);
  const reopened = pick(report, 'open', { 'state.doc.name': 'links-bookmarks.pdf' });
  assert.deepEqual(reopened.state.nav.links['0'], [{ kind: 'uri', uri: 'https://example.com/a2' }], 'the saved copy keeps the web link');
  assert.deepEqual(pick(report, 'bookmark', { contents: { action: 'go', index: 3 } }).state.page, 2, 'bookmark go follows Three to page 2');
});

test('drive A2: save-in-place rewrites the copy, never the fixture', { skip, timeout: 120_000 }, async () => {
  const before = createHash('sha256').update(await readFile(join(root, 'fixtures/form2.pdf'))).digest('hex');
  const { code, report, dir } = await drive('tests/drive/save-in-place.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.ok, true, JSON.stringify(report.steps.filter(step => step.ok !== true)));

  const inPlace = pick(report, 'save-in-place');
  assert.equal(inPlace.state.save.inPlace, true, 'save-in-place is the in-place path');
  assert.equal(inPlace.state.dirty, false, 'the write is the new clean baseline');
  assert.equal(await savedBytes(dir, 'save-in-place.pdf'), inPlace.state.save.bytes);
  const reopened = report.steps[report.steps.length - 1].state;
  assert.equal(reopened.notes, 2, 'the rewritten copy reopens with both notes');
  assert.equal(reopened.dirty, false);
  const after = createHash('sha256').update(await readFile(join(root, 'fixtures/form2.pdf'))).digest('hex');
  assert.equal(after, before, 'a drive run must never rewrite a fixture');
});

test('drive P5: signature verification, imported trust, invalid and partial coverage', { skip, timeout: 180_000 }, async () => {
  const { code, report } = await drive('tests/drive/verify.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.equal(report.ok, true, JSON.stringify(report.steps.filter(step => step.ok !== true)));
  assert.equal(report.steps[1].state.verify.last.reason, 'encrypted', 'a worker refusal reaches STATE');
  assert.equal(report.steps[3].state.verify.last.signatures[0].trust, 'untrusted');
  assert.equal(report.steps[4].state.verify.last.signatures[0].trust, 'trusted');
  assert.equal(report.steps[6].state.verify.last.signatures[0].status, 'invalid');
  assert.equal(report.steps[8].state.verify.last.signatures[0].trust, 'untrusted');
  assert.equal(report.steps[10].state.verify.last.signatures[0].cell, 'valid (weak SHA-1 digest)');
  assert.equal(report.steps[12].state.verify.last.signatures[0].coverage, 'partial');
  assert.match(report.steps[12].state.verify.last.summary, /; later changes were not verified$/);
  assert.equal(report.steps[12].state.dirty, false);
  assert.equal(report.steps[16].state.verify.last.signatures[0].trust, 'trusted');
  assert.equal(report.steps[16].state.verify.last.signatures[0].status, 'valid');
  assert.equal(report.steps[16].state.dirty, false);
});

test('drive P5: visible signature, second incremental signature, and certify', { skip, timeout: 180_000 }, async () => {
  const { code, report, dir } = await drive('tests/drive/sign.jsonl');
  assert.equal(code, 0, JSON.stringify(report));
  assert.ok(report.steps.filter(step => step.expect !== undefined).every(step => step.ok), JSON.stringify(report.steps.filter(step => step.expect !== undefined && !step.ok)));
  assert.equal(report.steps[1].state.sign.last.revisions, 1);
  assert.equal(report.steps[3].state.sign.last.revisions, 2);
  assert.equal(report.steps[5].state.sign.last.ok, true);
  for (const file of ['signed.pdf', 'signed-twice.pdf', 'certified.pdf']) {
    assert.equal((await readFile(join(dir, file))).subarray(0, 5).toString(), '%PDF-');
  }
});

const DIRECT_FIXTURES = {
  'fixtures/form2.pdf': 'f2d77d007f947e7daab0c2b9800b0a91df3ee843ef32f183b53b200b9d941164',
  'fixtures/p0b/edit-transformed.pdf': '638f1ac768ac8f14fd3ec1ff21b0efe5b87fc1bcd788e721782d523c96a0034b',
};

test('drive P6 U2: field/text gestures undo, reflow, cancel and restore refusals', { skip, timeout: 120_000 }, async () => {
  const { code, report } = await drive('tests/drive/direct-edit2.jsonl', 100_000);
  assert.equal(report.steps.length, 50);
  const rounded = rect => rect.map(Math.round);
  const moved = report.steps[2].state.fields.defs.drag.rect, resized = report.steps[5].state.fields.defs.drag.rect, nudged = report.steps[9].state.fields.defs.drag.rect;
  assert.deepEqual(rounded(moved), [82, 80, 262, 104]);
  assert.deepEqual(rounded(report.steps[3].state.fields.defs.drag.rect), [72, 72, 252, 96]);
  assert.deepEqual(rounded(report.steps[4].state.fields.defs.drag.rect), rounded(moved));
  assert.deepEqual(rounded(resized), [82, 80, 272, 104]);
  assert.deepEqual(rounded(report.steps[6].state.fields.defs.drag.rect), rounded(moved));
  assert.deepEqual(rounded(report.steps[7].state.fields.defs.drag.rect), rounded(resized));
  assert.deepEqual(rounded(nudged), [83, 80, 273, 104]);
  assert.deepEqual(rounded(report.steps[11].state.fields.defs.drag.rect), rounded(resized));
  assert.deepEqual(rounded(report.steps[12].state.fields.defs.drag.rect), rounded(nudged));
  const zoomedField = report.steps[15].state, fieldBox = zoomedField.direct.active.box, fieldRect = zoomedField.direct.active.rect, fieldScale = zoomedField.zoom.scale;
  assert.deepEqual(rounded([fieldBox.left, fieldBox.top, fieldBox.width, fieldBox.height]), rounded([fieldRect[0] * fieldScale, fieldRect[1] * fieldScale, (fieldRect[2] - fieldRect[0]) * fieldScale, (fieldRect[3] - fieldRect[1]) * fieldScale]));
  const originalParagraph = report.steps[20].state.direct.active;
  const reflowedParagraph = report.steps[22].state.direct.active;
  assert.equal(originalParagraph.lines, 4);
  assert.ok(reflowedParagraph.lines > originalParagraph.lines);
  assert.ok(reflowedParagraph.width < originalParagraph.width);
  assert.equal(report.steps[24].state.dirty, false, 'undo restored the longer draft + width as one gesture');
  assert.equal(report.steps[28].state.dirty, false, 'undo restored the shorter paragraph edit');
  assert.deepEqual(report.steps[29].state.direct.active.rect, report.steps[30].state.direct.active.rect, 'Esc canceled the live run resize');
  assert.equal(report.steps[29].state.revision, report.steps[30].state.revision);
  assert.match(report.steps[36].state.error, /would leave the page/);
  assert.deepEqual(report.steps[35].state.direct.active.rect, report.steps[36].state.direct.active.rect, 'refusal restored the open editor box');
  assert.equal(report.steps[35].state.direct.active.targetId, report.steps[38].state.direct.active.targetId, 'the original click still finds the same run');
  const movedText = report.steps[32].state.content.bounds.find(item => item.targetId === report.steps[32].state.content.bounds.find(candidate => candidate.bounds[0] === 80 && candidate.bounds[1] > 650)?.targetId);
  assert.ok(movedText && Math.abs(movedText.bounds[0] - 80) < 1 && Math.abs(movedText.bounds[1] - 650) < 1);
  const resizedText = report.steps[39].state.direct.active;
  assert.ok(resizedText.fontSize >= 100 && resizedText.fontSize < 400);
  assert.ok(resizedText.fontPx >= 100);
  const textBox = resizedText.box, textRect = resizedText.rect, textScale = report.steps[43].state.zoom.scale;
  assert.deepEqual(rounded([textBox.left, textBox.top, textBox.width, textBox.height]), rounded([textRect[0] * textScale, textRect[1] * textScale, (textRect[2] - textRect[0]) * textScale, (textRect[3] - textRect[1]) * textScale]));
  assert.equal(report.steps[49].state.fields.values.choice, '', 'undo reverted the editable combo value');
  const red = report.steps.filter(step => step.expect !== undefined && !step.ok);
  assert.equal(red.length, 0, JSON.stringify(red.map(step => ({ cmd: step.cmd, state: step.state }))));
  assert.equal(code, 0);
});

test('drive P6: click-to-fill, click-to-edit, tab, armed-tool pass-through', { skip, timeout: 180_000 }, async () => {
  const before = Object.fromEntries(await Promise.all(Object.keys(DIRECT_FIXTURES).map(async path => [path, createHash('sha256').update(await readFile(join(root, path))).digest('hex')])));
  const { code, report, dir } = await drive('tests/drive/direct-edit.jsonl', 150_000);
  assert.equal(report.steps.length, 75);
  assert.equal(report.steps.filter(step => step.cmd === 'click').length, 19);
  const red = report.steps.filter(step => step.expect !== undefined && !step.ok);
  assert.equal(red.length, 0, JSON.stringify(red.map(step => ({ notes: step.state?.notes, active: step.state?.direct?.active, error: step.state?.error }))));
  assert.equal(code, 0);
  const rotated = report.steps.find(step => step.cmd === 'click' && step.contents?.x === 712);
  assert.ok(rotated, 'rotate click step missing');
  // §8.9: the expect on this step holds the editor-on-name outcome.
  assert.ok(rotated.state.pages.rotate[0] === 90);
  assert.equal((await readFile(join(dir, 'direct-text.pdf'))).subarray(0, 5).toString(), '%PDF-');
  for (const [path, hash] of Object.entries(DIRECT_FIXTURES)) assert.equal(before[path], hash);
  for (const [path, hash] of Object.entries(before)) {
    assert.equal(createHash('sha256').update(await readFile(join(root, path))).digest('hex'), hash, path);
  }
});
