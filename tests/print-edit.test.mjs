/**
 * print-edit (contract-print-edit.md D1–D9): Edit text on macOS Quartz print-to-PDF pages,
 * where every glyph is its own `q <clip> re W n … q S 0 0 S X Y cm BT … (c) Tj ET Q Q` block.
 * Engine direct except (j), the drive. Fixture: fixtures/p6/quartz.pdf (make-quartz.mjs):
 * /TT1 codes from 33 in first-use order (TT1_CHARS), /TT2 draws only `t`; no space glyph,
 * word gap 0.278 em. Helpers are copies of tests/p3-coded.test.mjs and
 * tests/p3-coded-drive.test.mjs (not shared on purpose).
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
import { createEngine } from './rust-engine.mjs';
import { assertWithin } from './timing-budget.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const engine = createEngine(mupdf);
const FIXTURE = 'fixtures/p6/quartz.pdf';
const FIXTURE_SHA256 = '937d55fe100bae4c13b753da65fc335a22537a8b783b7ee0ac1abd3594dd72c3';
const TT1_CHARS = 'BusineLcAplatoFrmUPdS2hbfg.EvywxkHqRCKI';
const GLYPH_LINES = 25;
const BODY = [
  'Applicants must read these notes before filling',
  'in the form. Every section applies to new owners',
  'and to renewals of an existing business licence.',
  'Print clearly in black ink and sign the last page.',
  'Attach a copy of the lease or the property deed.',
  'Home based operators also need written consent',
  'from the owner of the building where they work.',
  'Fees are payable by cheque or by card at the desk.',
  'A licence is valid for one year from the date shown.',
  'Renewal notices are mailed thirty days in advance.',
  'Change of address must be reported within ten days.',
  'Keep the licence on display at the place of business.',
  'Inspectors may visit during normal opening hours.',
  'Send any other questions to the licensing office.',
];
const TEXTS = ['PDF4All control', 'Business Licence Application Form', 'Unit', 'Postal code', 'Suite and Street address',
  'Business', 't', 'rade name', 'Office use only', 'Passed all assessments', 'Floor 2', 'nd', ...BODY];

function toArrayBuffer(bytes) {
  if (bytes instanceof ArrayBuffer) return bytes;
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
}

let nextId = 900;
async function openFile(path, session = engine.createSession()) {
  const reply = engine.dispatch({ id: nextId++, op: 'open', name: path, pdf: toArrayBuffer(await readFile(path)) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function openBytes(bytes, session = engine.createSession(), name = 'scratch.pdf') {
  const reply = engine.dispatch({ id: nextId++, op: 'open', name, pdf: toArrayBuffer(bytes) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function op(session, request) {
  return engine.dispatch({ id: nextId++, ...request }, session).reply;
}

function scan(session, page = 0) {
  const reply = op(session, { op: 'edit-targets', page });
  assert.equal(reply.ok, true, reply.error);
  return reply;
}

function candidate(session, text) {
  const reply = scan(session);
  const found = reply.items.filter(entry => entry.text === text);
  assert.equal(found.length, 1, `${JSON.stringify(text)} once in ${JSON.stringify(reply.items.map(entry => [entry.text, entry.support, entry.reason]))}`);
  assert.equal(found[0].support, 'candidate', `${text}: ${found[0].reason}`);
  return { item: found[0], generation: reply.generation, revision: reply.revision, page: 0 };
}

function edit(session, target, text, extra = {}) {
  return op(session, {
    op: 'edit-text', page: target.page, targetId: target.item.targetId,
    generation: target.generation, revision: target.revision, ...(text === undefined ? {} : { text }), ...extra,
  });
}

function contentOf(session, page = 0) {
  const target = session.doc.loadPage(page);
  try {
    const buffer = target.getObject().get('Contents').readStream();
    try {
      return Buffer.from(buffer.asUint8Array()).toString('latin1');
    } finally {
      buffer.destroy();
    }
  } finally {
    target.destroy();
  }
}

const journalPosition = session => session.doc.asPDF().getJournal().position;

/** Every glyph block of a content stream: its span, clip, scale, origin and font. */
const BLOCK = /q\s+(?:[-\d.]+\s+[-\d.]+\s+[-\d.]+\s+[-\d.]+\s+re\s+W\s+n\s+)?(?:\/Cs1\s+cs\s+0\s+0\s+0\s+sc\s+)?q\s+([-\d.]+)\s+0\s+0\s+([-\d.]+)\s+([-\d.]+)\s+([-\d.]+)\s+cm\s+BT\s+1\s+0\s+0\s+1\s+0\s+0\s+Tm\s+\/(\w+)\s+1\s+Tf\s+(?:\((?:\\.|[^\\)])*\)|<[0-9a-fA-F]*>)\s*Tj\s+ET\s+Q\s+Q/g;
function blocksOf(content) {
  return [...content.matchAll(BLOCK)].map(match => ({
    start: match.index, end: match.index + match[0].length, clip: /re\s+W\s+n/.test(match[0]),
    s: Number(match[1]), x: Number(match[3]), y: Number(match[4]), font: match[5], source: match[0],
  }));
}
/** The blocks on baseline `y` (user space) whose x lies in [x0, x1). */
const lineBlocks = (content, y, x0 = 0, x1 = 612) => blocksOf(content).filter(block => Math.abs(block.y - y) < 0.001 && block.x >= x0 && block.x < x1);

/** /TT1 advances (1000-unit) by character, read from the fixture's own /Widths. */
async function tt1Widths() {
  const doc = mupdf.PDFDocument.openDocument(await readFile(FIXTURE), 'application/pdf');
  try {
    const font = doc.loadPage(0).getObject().get('Resources').get('Font').get('TT1');
    assert.equal(font.get('FirstChar').asNumber(), 33);
    const widths = font.get('Widths');
    return new Map([...TT1_CHARS].map((ch, index) => [ch, widths.get(index).asNumber()]));
  } finally {
    doc.destroy();
  }
}

/** The natural x of every glyph of `text` from `x0` at size `s`: advances plus `space` em per word gap. */
function naturalX(widths, text, x0, s, space = 0.278) {
  const xs = [];
  let pen = x0;
  let gap = false;
  for (const ch of text) {
    if (ch === ' ') { gap = true; continue; }
    if (gap) pen += space * s;
    gap = false;
    xs.push(pen);
    pen += widths.get(ch) / 1000 * s;
  }
  return xs;
}

function render(session, zoom = 2) {
  const page = session.doc.loadPage(0);
  try {
    const pixmap = page.toPixmap(mupdf.Matrix.scale(zoom, zoom), mupdf.ColorSpace.DeviceRGB, false, true);
    try {
      return { width: pixmap.getWidth(), pixels: pixmap.getPixels().slice() };
    } finally {
      pixmap.destroy();
    }
  } finally {
    page.destroy();
  }
}

test('print-edit fixture: bytes match the provenance pin, the pre-fix shape (> 512 glyph blocks)', async () => {
  const provenance = JSON.parse(await readFile('fixtures/p6/quartz.provenance.json', 'utf8'));
  const bytes = await readFile(FIXTURE);
  assert.equal(bytes.length, provenance.bytes);
  assert.equal(createHash('sha256').update(bytes).digest('hex'), FIXTURE_SHA256);
  const session = openBytes(bytes);
  const blocks = blocksOf(contentOf(session));
  assert.equal(blocks.length, 703);
  assert.ok(blocks.every(block => block.clip), 'every fixture glyph is clipped');
});

test('print-edit (a): one candidate text target per glyph line, in content order, ≤ 512 items', async () => {
  const session = await openFile(FIXTURE);
  const reply = scan(session);
  assert.deepEqual(reply.items.map(item => item.text), TEXTS);
  assert.ok(reply.items.length <= 512);
  for (const item of reply.items) {
    assert.equal(item.support, 'candidate', `${item.text}: ${item.reason}`);
    if (item.text === 'PDF4All control') {
      assert.equal(item.glyphs, undefined);
      continue;
    }
    assert.equal(item.glyphs, item.text.replaceAll(' ', '').length, item.text);
    assert.ok(item.repertoire.includes(' '), `${item.text}: the repertoire names the space`);
    assert.equal(item.moveable, true, item.text);
    assert.equal(item.tailFollows, false, item.text);
    assert.equal(item.kerned, undefined, item.text);
    assert.ok(Array.isArray(item.bounds) && item.bounds.length === 4, item.text);
  }
  assert.equal(reply.items.filter(item => item.glyphs !== undefined).length, GLYPH_LINES);
  assert.equal(candidate(session, 'Unit').item.fontSize, 10);
  assert.equal(candidate(session, 'nd').item.fontSize, 6);
  // D11: the MacRoman /TT3 line writes only what /TT3 draws on this page, plus space.
  assert.equal(candidate(session, 'Office use only').item.repertoire, ' Ocefilnosuy');
  // The doubled letters (P2's ambiguous members) sit inside their line, not beside it.
  assert.ok(candidate(session, 'Passed all assessments').item.bounds[2] > candidate(session, 'Passed all assessments').item.bounds[0] + 90);
});

test('print-edit (b): Unit -> Suite rewrites one line unclipped at natural advances; undo restores every byte', async () => {
  const widths = await tt1Widths();
  const session = await openFile(FIXTURE);
  const before = contentOf(session);
  const old = lineBlocks(before, 680, 0, 300);
  assert.equal(old.length, 4);
  const reply = edit(session, candidate(session, 'Unit'), 'Suite');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.equal(reply.revision, 1);
  assert.equal(reply.dirty, true);
  const line = candidate(session, 'Suite');
  assert.equal(line.item.glyphs, 5);
  const after = contentOf(session);
  const blocks = lineBlocks(after, 680, 0, 300);
  assert.equal(blocks.length, 5);
  const expected = naturalX(widths, 'Suite', 72, 10);
  assert.ok(Math.abs(blocks[2].x - expected[2]) < 0.01, `${blocks[2].x} vs ${expected[2]}`);
  blocks.forEach((block, index) => assert.ok(Math.abs(block.x - expected[index]) < 0.001, `${index}: ${block.x} vs ${expected[index]}`));
  assert.ok(blocks.every(block => !block.clip && block.s === 10 && block.font === 'TT1'));
  const span = after.slice(blocks[0].start, blocks.at(-1).end);
  assert.equal(/re\s+W\s+n/.test(span), false, span);
  assert.ok(span.includes('/Cs1 cs 0 0 0 sc'), 'the colour group travels verbatim');
  // Neighbours: every byte before and after the line is the original's.
  assert.ok(after.startsWith(before.slice(0, old[0].start)));
  assert.ok(after.endsWith(before.slice(old.at(-1).end)));
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentOf(session), before);
});

test('print-edit D11: a MacRoman simple-font line edits from its drawn codes, refuses others, undo restores', async () => {
  const session = await openFile(FIXTURE);
  const before = contentOf(session);
  // D12: the line's two explicit 0x20 blocks are consumed by the collapse, never targets of their own.
  assert.equal(lineBlocks(before, 640, 300).length, 15);
  assert.equal(scan(session).items.filter(item => item.text.trim() === '').length, 0);
  assert.equal(candidate(session, 'Office use only').item.glyphs, 13);
  const refused = edit(session, candidate(session, 'Office use only'), 'Office use Zone');
  assert.equal(refused.ok, false, JSON.stringify(refused));
  assert.equal(refused.detail, 'unsupported-encoding', refused.error);
  assert.deepEqual(refused.missing, ['Z']);
  assert.equal(contentOf(session), before);
  const reply = edit(session, candidate(session, 'Office use only'), 'only Office use');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.equal(candidate(session, 'only Office use').item.glyphs, 13);
  const blocks = lineBlocks(contentOf(session), 640, 300);
  assert.equal(blocks.length, 13);
  assert.ok(blocks.every(block => block.font === 'TT3' && block.s === 10));
  const span = contentOf(session).slice(blocks[0].start, blocks.at(-1).end);
  assert.equal(/re\s+W\s+n/.test(span), false, span);
  assert.equal(blocks[0].x, 320);
  assert.equal(op(session, { op: 'undo' }).ok, true);
  assert.equal(contentOf(session), before);
});

test('print-edit (c): a character outside the repertoire, an overflow and an off-page move refuse, nothing written', async () => {
  const session = await openFile(FIXTURE);
  const before = contentOf(session);
  const check = (reply, detail) => {
    assert.equal(reply.ok, false, JSON.stringify(reply));
    assert.equal(reply.detail, detail, reply.error);
    assert.equal(contentOf(session), before);
    assert.equal(journalPosition(session), 0);
    const state = scan(session);
    assert.equal(state.revision, 0);
  };
  const missing = edit(session, candidate(session, 'Unit'), 'UniZ');
  check(missing, 'unsupported-encoding');
  assert.deepEqual(missing.missing, ['Z']);
  const long = 'Suite and Street address and Suite and Street address and Suite';
  check(edit(session, candidate(session, 'Unit'), long), 'overflow');
  check(edit(session, candidate(session, 'Unit'), undefined, { move: { dx: 0, dy: -800 } }), 'outside-page');
  check(edit(session, candidate(session, 'Unit'), undefined, { size: 0 }), 'invalid');
  check(edit(session, candidate(session, 'Unit'), undefined, { move: { dx: 'a', dy: 0 } }), 'invalid');
});

test('print-edit (d): word gaps keep the line space, whitespace normalises, empty text deletes the line', async () => {
  const widths = await tt1Widths();
  const session = await openFile(FIXTURE);
  const reply = edit(session, candidate(session, 'Suite and Street address'), 'Street and Suite');
  assert.equal(reply.ok, true, JSON.stringify(reply));
  assert.equal(candidate(session, 'Street and Suite').item.glyphs, 14);
  const content = contentOf(session);
  const blocks = lineBlocks(content, 660);
  assert.equal(blocks.length, 14);
  const expected = naturalX(widths, 'Street and Suite', 72, 10);
  blocks.forEach((block, index) => assert.ok(Math.abs(block.x - expected[index]) < 0.001, `${index}: ${block.x} vs ${expected[index]}`));

  const spaced = await openFile(FIXTURE);
  const messy = edit(spaced, candidate(spaced, 'Suite and Street address'), '  Street \t and   Suite ');
  assert.equal(messy.ok, true, JSON.stringify(messy));
  assert.equal(contentOf(spaced), content, 'the normalised write is the same bytes');

  const same = await openFile(FIXTURE);
  const noop = edit(same, candidate(same, 'Suite and Street address'), ' Suite  and Street address ');
  assert.equal(noop.ok, true, JSON.stringify(noop));
  assert.equal(journalPosition(same), 0, 'the normalised text is the old text: no op');
  assert.equal(noop.dirty, false);

  const gone = await openFile(FIXTURE);
  const deleted = edit(gone, candidate(gone, 'Passed all assessments'), '');
  assert.equal(deleted.ok, true, JSON.stringify(deleted));
  assert.equal(journalPosition(gone), 1);
  assert.equal(scan(gone).items.some(item => item.text === 'Passed all assessments'), false);
  assert.equal(lineBlocks(contentOf(gone), 620).length, 0);
  assert.equal(scan(gone).items.length, TEXTS.length - 1);
});

test('print-edit (e): move and size keep the line spacing; +10 then -10 restores positions; a zero move is a no-op', async () => {
  const session = await openFile(FIXTURE);
  const before = lineBlocks(contentOf(session), 680, 300);
  assert.equal(before.length, 10);
  assert.equal(edit(session, candidate(session, 'Postal code'), undefined, { move: { dx: 10, dy: 5 } }).ok, true);
  const moved = lineBlocks(contentOf(session), 675, 300);
  assert.equal(moved.length, 10);
  moved.forEach((block, index) => assert.ok(Math.abs(block.x - before[index].x - 10) < 0.001, `${index}`));
  assert.equal(candidate(session, 'Postal code').item.glyphs, 10);

  const unit = lineBlocks(contentOf(session), 680, 0, 300);
  assert.equal(edit(session, candidate(session, 'Unit'), undefined, { size: 20 }).ok, true);
  const big = lineBlocks(contentOf(session), 680, 0, 300);
  assert.equal(big.length, 4);
  big.forEach((block, index) => {
    assert.equal(block.s, 20);
    assert.ok(Math.abs(block.x - (72 + (unit[index].x - 72) * 2)) < 0.001, `${index}: ${block.x}`);
  });
  assert.equal(candidate(session, 'Unit').item.fontSize, 20);

  const floor = lineBlocks(contentOf(session), 600);
  assert.equal(edit(session, candidate(session, 'Floor 2'), undefined, { move: { dx: 10, dy: 0 } }).ok, true);
  assert.equal(edit(session, candidate(session, 'Floor 2'), undefined, { move: { dx: -10, dy: 0 } }).ok, true);
  const back = lineBlocks(contentOf(session), 600);
  assert.equal(back.length, floor.length);
  back.forEach((block, index) => assert.ok(Math.abs(block.x - floor[index].x) < 0.001, `${index}`));

  const position = journalPosition(session);
  const zero = edit(session, candidate(session, 'Floor 2'), undefined, { move: { dx: 0, dy: 0 } });
  assert.equal(zero.ok, true, JSON.stringify(zero));
  assert.equal(journalPosition(session), position, 'a zero move writes nothing');
});

test('print-edit (f): an unclipped redraw differs from the clipped original only at the line fringe', async () => {
  const session = await openFile(FIXTURE);
  const bounds = candidate(session, 'Passed all assessments').item.bounds;
  const before = render(session);
  assert.equal(edit(session, candidate(session, 'Passed all assessments'), undefined, { move: { dx: 10, dy: 0 } }).ok, true);
  assert.equal(edit(session, candidate(session, 'Passed all assessments'), undefined, { move: { dx: -10, dy: 0 } }).ok, true);
  assert.ok(lineBlocks(contentOf(session), 620).every(block => !block.clip));
  const after = render(session);
  const grown = [bounds[0] - 2, bounds[1] - 2, bounds[2] + 2, bounds[3] + 2].map(value => value * 2);
  let differing = 0;
  for (let index = 0; index < before.pixels.length; index++) {
    if (before.pixels[index] === after.pixels[index]) continue;
    differing++;
    const pixel = Math.floor(index / 3);
    const x = pixel % before.width, y = Math.floor(pixel / before.width);
    assert.ok(x >= grown[0] - 1 && x <= grown[2] + 1 && y >= grown[1] - 1 && y <= grown[3] + 1, `pixel ${x},${y} outside ${grown}`);
  }
  assert.ok(differing / before.pixels.length < 0.005, `${differing} of ${before.pixels.length}`);
});

test('print-edit (g): Find/Replace hits a glyph line, edit-batch moves two lines as one op, clip-copy refuses', async () => {
  // On the fresh page: "U" is drawn only by "Unit", so once "Unit" is gone the page cannot draw it.
  const session = await openFile(FIXTURE);
  const target = scan(session);
  const preview = op(session, { op: 'edit-replace-preview', page: 0, generation: target.generation, revision: target.revision, find: 'Suite', replace: 'Unit' });
  assert.equal(preview.ok, true, preview.error);
  assert.deepEqual(preview.matches.map(match => match.text), ['Suite and Street address'], JSON.stringify(preview.rejected));
  const applied = op(session, {
    op: 'edit-replace-apply', page: 0, generation: preview.generation, revision: preview.revision,
    find: 'Suite', replace: 'Unit', previewToken: preview.previewToken, targetIds: preview.matches.map(match => match.targetId),
  });
  assert.equal(applied.ok, true, applied.error);
  const texts = scan(session).items.map(item => item.text);
  assert.ok(texts.includes('Unit') && texts.includes('Unit and Street address'), JSON.stringify(texts));
  assert.equal(texts.includes('Suite and Street address'), false);
  assert.equal(candidate(session, 'Unit and Street address').item.glyphs, 20);

  const position = journalPosition(session);
  const ndX = lineBlocks(contentOf(session), 604)[0].x;
  const sc = scan(session);
  const id = text => sc.items.find(item => item.text === text).targetId;
  const batch = op(session, { op: 'edit-batch', page: 0, generation: sc.generation, revision: sc.revision, items: [
    { op: 'edit-text', targetId: id('Floor 2'), move: { dx: 5, dy: 0 } },
    { op: 'edit-text', targetId: id('nd'), move: { dx: 5, dy: 0 } },
  ] });
  assert.equal(batch.ok, true, JSON.stringify(batch));
  assert.equal(journalPosition(session), position + 1, 'one journal op');
  assert.equal(lineBlocks(contentOf(session), 600)[0].x, 77);
  assert.ok(Math.abs(lineBlocks(contentOf(session), 604)[0].x - ndX - 5) < 0.001);

  const copy = scan(session);
  const refused = op(session, { op: 'clip-copy', page: 0, generation: copy.generation, revision: copy.revision, targets: [{ kind: 'text', id: copy.items.find(item => item.text === 'Floor 2').targetId }] });
  assert.equal(refused.ok, false);
  assert.equal(refused.detail, 'not-candidate', refused.error);
  assert.match(refused.error, /print-to-PDF line/);
});

/** Review fixes: a page whose /F1 is Helvetica WinAnsi WITH /FirstChar 32 + /Widths (a D11 glyph-block font). */
const HELV = { ' ': 278, a: 556, b: 556, c: 500, d: 556, x: 500, y: 500, z: 500 };
const helvWidth = ch => HELV[ch] ?? 556;
function simplePage(body) {
  const pdf = new mupdf.PDFDocument();
  const font = pdf.newDictionary();
  font.put('Type', pdf.newName('Font'));
  font.put('Subtype', pdf.newName('Type1'));
  font.put('BaseFont', pdf.newName('Helvetica'));
  font.put('Encoding', pdf.newName('WinAnsiEncoding'));
  font.put('FirstChar', 32);
  font.put('LastChar', 126);
  const widths = pdf.newArray();
  for (let code = 32; code <= 126; code++) widths.push(helvWidth(String.fromCharCode(code)));
  font.put('Widths', widths);
  const fonts = pdf.newDictionary();
  fonts.put('F1', pdf.addObject(font));
  const resources = pdf.newDictionary();
  resources.put('Font', fonts);
  pdf.insertPage(-1, pdf.addPage([0, 0, 612, 792], 0, resources, body));
  return pdf.saveToBuffer('').asUint8Array().slice();
}
/** Quartz-shaped blocks for `text` at (x, y), size s, no colour; spaces are 0x20 blocks when `spaceBlocks`, else 0.278 em gaps. */
function quartzLine(text, x, y, s = 10, { spaceBlocks = false, d = s } = {}) {
  let out = '';
  let pen = x;
  for (const ch of text) {
    if (ch === ' ' && !spaceBlocks) { pen += 0.278 * s; continue; }
    out += `q ${Math.floor(pen)} ${Math.floor(y - 0.25 * s)} ${Math.ceil(helvWidth(ch) / 1000 * s) + 1} ${Math.ceil(1.35 * s)} re W n q ${s} 0 0 ${d} ${pen.toFixed(4)} ${y} cm BT 1 0 0 1 0 0 Tm /F1 1 Tf (${ch}) Tj ET Q Q\n`;
    pen += helvWidth(ch) / 1000 * s;
  }
  return out;
}

test('print-edit (h): 4097 real glyph blocks hit the raw ceiling; 513 plain runs still hit the 512-target ceiling', () => {
  let glyphs = '';
  for (let index = 0; index < 4097; index++) {
    const x = 10 + (index % 64) * 9, y = 780 - Math.floor(index / 64) * 12;
    glyphs += quartzLine('abcdxyz'[index % 7], x, y, 6);
  }
  const session = openBytes(simplePage(glyphs));
  const raw = op(session, { op: 'edit-targets', page: 0 });
  assert.equal(raw.ok, false);
  assert.equal(raw.reason, 'too-large');
  assert.match(raw.error, /more than 4096 targets/);

  let runs = 'BT /F1 6 Tf ';
  for (let index = 0; index < 513; index++) runs += `1 0 0 1 ${20 + (index % 32) * 18} ${40 + Math.floor(index / 32) * 30} Tm (a) Tj `;
  const plain = op(openBytes(simplePage(`${runs}ET`)), { op: 'edit-targets', page: 0 });
  assert.equal(plain.ok, false);
  assert.equal(plain.reason, 'too-large');
  assert.match(plain.error, /more than 512 targets/);
});

test('print-edit review 1: an 80,000-digit comment beside a one-byte Tj scans in under 1 s', () => {
  const session = openBytes(simplePage(`BT /F1 12 Tf 72 700 Td (a) Tj ET\n% q q ${'1'.repeat(80_000)}\n`));
  const started = performance.now();
  const reply = op(session, { op: 'edit-targets', page: 0 });
  const ms = performance.now() - started;
  assert.equal(reply.ok, true, reply.error);
  assertWithin(ms, 1000, 'the scan');
});

test('print-edit review 2: adjacent space blocks are one word gap; identical text is a no-op; a pure move keeps relative x', () => {
  const bytes = simplePage(quartzLine('ab  cd', 72, 700, 10, { spaceBlocks: true }));
  const glyphXs = session => blocksOf(contentOf(session)).filter(block => !block.source.includes('( ) Tj')).map(block => block.x);
  const line = session => {
    const items = scan(session).items;
    assert.equal(items.length, 1, JSON.stringify(items.map(item => item.text)));
    return items[0];
  };
  const same = openBytes(bytes);
  assert.equal(line(same).text, 'ab cd');
  assert.equal(line(same).glyphs, 4);
  const before = contentOf(same);
  const noop = edit(same, candidate(same, 'ab cd'), 'ab cd');
  assert.equal(noop.ok, true, JSON.stringify(noop));
  assert.equal(noop.dirty, false);
  assert.equal(journalPosition(same), 0);
  assert.equal(contentOf(same), before);

  const moved = openBytes(bytes);
  const xs = glyphXs(moved);
  assert.equal(xs.length, 4);
  const reply = edit(moved, candidate(moved, 'ab cd'), undefined, { move: { dx: 10, dy: 0 } });
  assert.equal(reply.ok, true, JSON.stringify(reply));
  const after = glyphXs(moved);
  assert.equal(after.length, 4);
  after.forEach((x, index) => assert.ok(Math.abs(x - xs[index] - 10) < 0.001, `${index}: ${x} vs ${xs[index]} + 10`));
  assert.equal(line(moved).text, 'ab cd');
});

test('print-edit review 3: 4096 plain one-byte runs refuse too-large in under 500 ms', () => {
  let runs = 'BT /F1 6 Tf ';
  for (let index = 0; index < 4096; index++) runs += `1 0 0 1 ${20 + (index % 32) * 18} ${10 + Math.floor(index / 32) * 6} Tm (a) Tj `;
  const session = openBytes(simplePage(`${runs}ET`));
  const started = performance.now();
  const reply = op(session, { op: 'edit-targets', page: 0 });
  const ms = performance.now() - started;
  assert.equal(reply.ok, false);
  assert.equal(reply.reason, 'too-large');
  assert.match(reply.error, /more than 512 targets/);
  assertWithin(ms, 500, 'the refusal');
});

test('print-edit review 4: Find/Replace whose result is the same text leaves the glyph line bytes and clips', async () => {
  const session = await openFile(FIXTURE);
  const before = contentOf(session);
  const target = scan(session);
  const preview = op(session, { op: 'edit-replace-preview', page: 0, generation: target.generation, revision: target.revision, find: 'Unit', replace: 'Unit' });
  assert.equal(preview.ok, true, preview.error);
  assert.deepEqual(preview.matches.map(match => match.text), ['Unit']);
  const applied = op(session, {
    op: 'edit-replace-apply', page: 0, generation: preview.generation, revision: preview.revision,
    find: 'Unit', replace: 'Unit', previewToken: preview.previewToken, targetIds: preview.matches.map(match => match.targetId),
  });
  assert.equal(applied.ok, true, applied.error);
  assert.equal(contentOf(session), before);
  assert.ok(lineBlocks(contentOf(session), 680, 0, 300).every(block => block.clip));
});

test('print-edit D1 negatives: a non-uniform scale beyond tolerance and a two-byte operand stay ordinary items', () => {
  const twoByte = quartzLine('a', 72, 600).replace('(a) Tj', '(xy) Tj');
  const session = openBytes(simplePage(quartzLine('abc', 72, 700, 10, { d: 10.02 }) + twoByte));
  const items = scan(session).items;
  assert.deepEqual(items.map(item => item.text), ['a', 'b', 'c', 'xy']);
  assert.ok(items.every(item => item.glyphs === undefined), JSON.stringify(items));
});

test('print-edit D12 boundary: |a − d| just inside 1e-3·a joins, just outside does not', () => {
  const session = openBytes(simplePage(quartzLine('abc', 72, 700, 10, { d: 10.0099 }) + quartzLine('xyz', 72, 600, 10, { d: 10.0101 })));
  const items = scan(session).items;
  assert.deepEqual(items.map(item => item.text), ['abc', 'x', 'y', 'z']);
  assert.equal(items[0].glyphs, 3);
  assert.ok(items.slice(1).every(item => item.glyphs === undefined));
});

test('print-edit (i): the saved file reopens and extracts the edited text', async () => {
  const session = await openFile(FIXTURE);
  assert.equal(edit(session, candidate(session, 'Unit'), 'Suite').ok, true);
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const doc = mupdf.PDFDocument.openDocument(new Uint8Array(saved.bytes), 'application/pdf');
  try {
    const page = doc.loadPage(0);
    const structured = page.toStructuredText();
    try {
      assert.ok(structured.asText().includes('Suite'), structured.asText());
    } finally {
      structured.destroy();
      page.destroy();
    }
  } finally {
    doc.destroy();
  }
  const reopened = openBytes(new Uint8Array(saved.bytes));
  assert.equal(candidate(reopened, 'Suite').item.glyphs, 5);
});

// ------------------------------------------------------------------- (j) drive

const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const SCRIPT = 'tests/drive/print-edit.jsonl';

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

const sha = async path => createHash('sha256').update(await readFile(join(root, path))).digest('hex');
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, contents: step.contents, edited: step.state?.content?.edited, refused: step.state?.content?.refused, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error });

test('drive: print-to-PDF lines edit by content, click + type + Enter, grip drag, undo/redo, save-to', { skip: CI_NO_ELECTRON, timeout: 300_000 }, async () => {
  assert.equal(await sha(FIXTURE), FIXTURE_SHA256, 'fixture before');
  const text = await readFile(join(root, SCRIPT), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.deepEqual([Number(header[1]), Number(header[2])], [11, 1]);
  assert.equal(lines.length, 11);
  assert.equal(lines.filter(line => JSON.parse(line).cmd === 'click').length, 1);

  const { code, report, dir } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 11, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  assert.deepEqual(steps[2].state.content.refused.map(entry => entry.detail), ['unsupported-encoding']);
  // The grip drag moved "Floor 2" (view top ~183 at x 72, the only line at x 72 there) by (20, 10) view pt.
  const floorAt = (step, x, top) => step.state.content.bounds.filter(entry => Math.abs(entry.bounds[0] - x) < 1 && Math.abs(entry.bounds[1] - top) < 6);
  const floor = floorAt(steps[6], 72, 185);
  assert.equal(floor.length, 1, JSON.stringify(steps[6].state.content.bounds));
  assert.equal(floorAt(steps[7], 72, 185).length, 0);
  assert.equal(floorAt(steps[7], 92, floor[0].bounds[1] + 10).length, 1, JSON.stringify(steps[7].state.content.bounds));

  assert.ok(dir.startsWith(tmpdir()));
  const saved = openBytes(await readFile(join(dir, 'quartz-edited.pdf')));
  const texts = scan(saved).items.map(item => item.text);
  assert.ok(texts.includes('Suite') && texts.includes('Post code') && texts.includes('Floor 2'), JSON.stringify(texts));
  assert.equal(texts.includes('Unit'), false);
  const dragged = blocksOf(contentOf(saved)).filter(block => Math.abs(block.y - 590) < 0.5);
  assert.equal(dragged.length, 6, 'the dragged line sits on baseline 590');
  assert.ok(Math.abs(dragged[0].x - 92) < 0.5, `${dragged[0].x}`);
  assert.equal(await sha(FIXTURE), FIXTURE_SHA256, 'fixture after');
});
