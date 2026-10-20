import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';

// P6 U2 engine (contract-p6-unit2.md §2–§4, §9): run move/size, paragraphs, editable combos.
// Every write is saved, re-opened, re-scanned and rendered (§6.3).
const engine = createEngine(mupdf);
const PARAGRAPH_SHA = '9f6681e7c4e25429ea1bac1ee6367b1b92cba141dcb17cb462877b2100a46a91';
const PARA = 'The quick brown fox jumps over the lazy dog while the cat watches from a sunny window and the birds sing.';
const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);

async function open(name = 'fixtures/p6/paragraph.pdf') {
  const s = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: buffer(await readFile(name)) }, s).reply;
  assert.equal(reply.ok, true, reply.error);
  return s;
}
const request = (s, req) => engine.dispatch({ id: 2, ...req }, s).reply;
const scan = (s, page) => request(s, { op: 'edit-targets', page });
const text = (sc, value) => sc.items.find(item => item.text === value);
function reopen(s) {
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const fresh = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'saved.pdf', pdf: saved.bytes }, fresh).reply.ok, true);
  return fresh;
}
/** Dark pixels inside a page-pt box (y down) of the page rendered at 72 dpi. */
function ink(s, page, box) {
  const view = s.doc.loadPage(page);
  const pixmap = view.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceGray, false);
  try {
    const width = pixmap.getWidth(), pixels = pixmap.getPixels();
    let count = 0;
    for (let y = Math.max(0, Math.floor(box[1])); y < Math.min(pixmap.getHeight(), Math.ceil(box[3])); y++) {
      for (let x = Math.max(0, Math.floor(box[0])); x < Math.min(width, Math.ceil(box[2])); x++) if (pixels[y * width + x] < 128) count++;
    }
    return count;
  } finally { pixmap.destroy(); view.destroy(); }
}
function editParagraph(s, page, index, extra) {
  const sc = scan(s, page);
  const paragraph = sc.paragraphs[index];
  return { before: sc, paragraph, reply: request(s, { op: 'edit-paragraph', page, paragraphId: paragraph.paragraphId, generation: sc.generation, revision: sc.revision, ...extra }) };
}
function editRun(s, page, value, extra) {
  const sc = scan(s, page);
  const item = text(sc, value);
  return { before: sc, item, reply: request(s, { op: 'edit-text', page, targetId: item.targetId, generation: sc.generation, revision: sc.revision, ...extra }) };
}
const same = (left, right) => assert.deepEqual(left.map(v => Math.round(v * 10)), right.map(v => Math.round(v * 10)));

test('P6 U2 fixture is the generated one', async () => {
  const bytes = await readFile('fixtures/p6/paragraph.pdf');
  assert.equal(createHash('sha256').update(bytes).digest('hex'), PARAGRAPH_SHA);
});

test('§3.1/§9.10–12 paragraph detection on every fixture block', async () => {
  const s = await open();
  const p0 = scan(s, 0);
  assert.equal(p0.paragraphs.length, 1);
  const [para] = p0.paragraphs;
  assert.equal(para.lines, 4);
  assert.equal(para.paragraphId, `para.${para.targetIds[0]}`);
  assert.deepEqual([para.left, para.leading, para.fontSize, para.kerned, para.tagged], [72, 14, 11, false, false]);
  for (const id of para.targetIds) assert.equal(p0.items.find(item => item.targetId === id).paragraphId, para.paragraphId);
  assert.equal(text(p0, 'Below the paragraph').paragraphId, undefined);
  const p1 = scan(s, 1);
  const lines = p1.paragraphs.map(p => p.targetIds.map(id => p1.items.find(item => item.targetId === id).text));
  assert.deepEqual(lines, [
    ['Tagged line one', 'Tagged line two', 'Tagged line three'],
    ['Leading one', 'Leading two', 'Leading three'],
    ['Kerned line one', 'Kerned line two'],
    ['TD one', 'TD two'],
  ]);
  assert.deepEqual(p1.paragraphs.map(p => [p.leading, p.tagged, p.kerned]), [[14, true, false], [13, false, false], [14, false, true], [14, false, false]]);
  // Not left-aligned, and a non-whitelisted `0 g` before `After TD`: single runs only.
  for (const value of ['Middle block', 'Centred text', 'After TD']) assert.equal(text(p1, value).paragraphId, undefined, value);
});

test('§2.4/§9.3 fontSize, moveable and the tail reasons', async () => {
  const s = await open();
  const sc = scan(s, 0);
  assert.deepEqual([text(sc, 'Move me').fontSize, text(sc, 'Move me').moveable], [11, true]);
  assert.deepEqual([text(sc, 'Head run').moveable, text(sc, 'Head run').moveReason], [false, 'tail-would-move']);
  assert.deepEqual([text(sc, 'tail run').moveable, text(sc, 'tail run').moveReason], [false, 'tail']);
  const tail = editRun(s, 0, 'tail run', { move: { dx: 5, dy: 0 } }).reply;
  assert.deepEqual([tail.ok, tail.detail, tail.reason], [false, 'not-candidate', 'tail']);
  assert.equal(editRun(s, 0, 'Head run', { move: { dx: 5, dy: 0 } }).reply.detail, 'tail-would-move');
  assert.equal(request(s, { op: 'undo' }).ok, false, 'refusals opened no op');
  // A tail may still be resized (Tf does not reposition).
  const sized = editRun(s, 0, 'tail run', { size: 12 }).reply;
  assert.equal(sized.ok, true, sized.error);
});

test('§2.2 move: Td before, exact inverse after; later text unchanged; pixels follow', async () => {
  const s = await open();
  const { item, before, reply } = editRun(s, 0, 'Move me', { move: { dx: 50, dy: -30 } });
  assert.equal(reply.ok, true, reply.error);
  const fresh = reopen(s);
  const after = scan(fresh, 0);
  same(text(after, 'Move me').bounds, [item.bounds[0] + 50, item.bounds[1] - 30, item.bounds[2] + 50, item.bounds[3] - 30]);
  same(text(after, 'Head run').bounds, text(before, 'Head run').bounds);
  assert.ok(ink(fresh, 0, text(after, 'Move me').bounds) > 20);
  assert.equal(ink(fresh, 0, item.bounds), 0);
  const view = s.doc.loadPage(0);
  assert.match(Buffer.from(view.getObject().get('Contents').readStream().asUint8Array()).toString('latin1'), /50 30 Td \(Move me\) Tj -50 -30 Td/);
  view.destroy();
});

test('§2.3/§9.2/§9.6 size in page pt; overflow and outside-page refusals; no-op writes nothing', async () => {
  const s = await open();
  const { item, reply } = editRun(s, 0, 'Move me', { size: 30 });
  assert.equal(reply.ok, true, reply.error);
  const fresh = reopen(s);
  const moved = text(scan(fresh, 0), 'Move me');
  assert.equal(moved.fontSize, 30);
  assert.ok(moved.bounds[2] - moved.bounds[0] > (item.bounds[2] - item.bounds[0]) * 2.5);
  assert.ok(ink(fresh, 0, moved.bounds) > 200);
  const big = editRun(s, 0, 'Move me', { size: 300 }).reply;
  assert.deepEqual([big.ok, big.detail], [false, 'overflow']);
  const off = editRun(s, 0, 'Move me', { move: { dx: 0, dy: -600 } }).reply;
  assert.deepEqual([off.ok, off.detail], [false, 'outside-page']);
  const revision = scan(s, 0).revision;
  assert.equal(editRun(s, 0, 'Move me', { move: { dx: 0, dy: 0 } }).reply.revision, revision);
  assert.equal(editRun(s, 0, 'Move me', { size: 30 }).reply.revision, revision);
});

test('§3.4–3.5 paragraph grows to the obstacle, then overflows', async () => {
  const s = await open();
  const grown = editParagraph(s, 0, 0, { text: `${PARA} Then the sun sets.` });
  assert.equal(grown.reply.ok, true, grown.reply.error);
  assert.equal(grown.reply.lines, 5);
  const stray = editParagraph(await open(), 0, 0, { text: `\n${PARA} Then the sun sets.\n\n` }).reply;
  assert.deepEqual([stray.ok, stray.lines], [true, 5], stray.error);
  const fresh = reopen(s);
  const after = scan(fresh, 0);
  assert.equal(after.paragraphs[0].lines, 5);
  same(text(after, 'Below the paragraph').bounds, text(grown.before, 'Below the paragraph').bounds);
  const lastLine = after.items.find(item => item.targetId === after.paragraphs[0].targetIds[4]);
  assert.ok(ink(fresh, 0, lastLine.bounds) > 50);
  const over = editParagraph(await open(), 0, 0, { text: `${PARA} Then the sun sets behind the quiet hills far away tonight.` }).reply;
  assert.deepEqual([over.ok, over.detail, over.lines, over.maxLines], [false, 'overflow', 6, 5]);
});

test('§3.5 shrink drops lines; §9.9 TD leaves TL restored for the text after', async () => {
  const s = await open();
  const shrunk = editParagraph(s, 0, 0, { text: 'Short now.' });
  assert.equal(shrunk.reply.ok, true, shrunk.reply.error);
  const fresh = reopen(s);
  const after = scan(fresh, 0);
  assert.equal(after.paragraphs.length, 0);
  same(text(after, 'Short now.').bounds.slice(0, 2), shrunk.paragraph.bounds.slice(0, 2));
  same(text(after, 'Below the paragraph').bounds, text(shrunk.before, 'Below the paragraph').bounds);
  assert.equal(ink(fresh, 0, [72, 110, 300, 140]), 0, 'dropped lines are gone');
  const t = await open();
  const td = editParagraph(t, 1, 3, { text: 'Only' });
  assert.equal(td.reply.ok, true, td.reply.error);
  const tdAfter = scan(reopen(t), 1);
  same(text(tdAfter, 'After TD').bounds, text(td.before, 'After TD').bounds);
});

test('§3.3 width re-wraps, size scales leading, move shifts; no-op keeps the revision', async () => {
  const wide = await open();
  const w = editParagraph(wide, 0, 0, { text: PARA, width: 300 });
  assert.equal(w.reply.lines, 2);
  assert.ok(scan(reopen(wide), 0).paragraphs[0].width <= 300);
  const small = await open();
  assert.equal(editParagraph(small, 0, 0, { text: PARA, size: 9 }).reply.ok, true);
  const sp = scan(reopen(small), 0).paragraphs[0];
  assert.deepEqual([sp.fontSize, sp.leading], [9, 11.45]);
  const moved = await open();
  const m = editParagraph(moved, 0, 0, { text: PARA, move: { dx: 10, dy: -20 } });
  assert.equal(m.reply.ok, true, m.reply.error);
  const mp = scan(reopen(moved), 0).paragraphs[0];
  same(mp.bounds, [m.paragraph.bounds[0] + 10, m.paragraph.bounds[1] - 20, m.paragraph.bounds[2] + 10, m.paragraph.bounds[3] - 20]);
  const still = await open();
  const n = editParagraph(still, 0, 0, { text: `  ${PARA.replace(/ /g, '   ')} ` });
  assert.deepEqual([n.reply.ok, n.reply.revision, n.reply.dirty], [true, 0, false]);
});

test('§4.1/§9.14 editable combos accept typed values and drop /I; plain combos still refuse', async () => {
  const s = await open('fixtures/form2.pdf');
  const combo = request(s, { op: 'fields', page: 0 }).items.find(item => item.kind === 'combo');
  assert.equal(combo.edit, false);
  const plain = request(s, { op: 'field-set', page: 0, widgetId: combo.id, value: 'Zebra' });
  assert.equal(plain.ok, false);
  assert.equal(request(s, { op: 'field-props', page: 0, widgetId: combo.id, edit: true }).ok, true);
  assert.equal(request(s, { op: 'fields', page: 0 }).items.find(item => item.id === combo.id).edit, true);
  const typed = request(s, { op: 'field-set', page: 0, widgetId: combo.id, value: 'Zebra' });
  assert.equal(typed.ok, true, typed.error);
  const fresh = reopen(s);
  const item = request(fresh, { op: 'fields', page: 0 }).items.find(i => i.id === combo.id);
  assert.deepEqual([item.value, item.edit], ['Zebra', true]);
  const view = fresh.doc.loadPage(0);
  const widget = view.getWidgets().find(w => w.getObject().asIndirect() === combo.id);
  assert.equal(widget.getObject().get('I').isNull(), true);
  assert.match(widget.getObject().get('AP').get('N').readStream().asString(), /Zebra/);
  view.destroy();
  // Clearing the flag puts `/V` back on an option.
  assert.equal(request(fresh, { op: 'field-props', page: 0, widgetId: combo.id, edit: false }).ok, true);
  assert.ok(combo.options.includes(request(fresh, { op: 'fields', page: 0 }).items.find(i => i.id === combo.id).value));
  assert.equal(request(fresh, { op: 'field-props', page: 0, widgetId: request(fresh, { op: 'fields', page: 0 }).items.find(i => i.kind === 'text').id, edit: true }).ok, false);
});

test('§2.6 text-add takes sizes up to 400 pt', async () => {
  const s = await open('fixtures/hello.pdf');
  const add = size => request(s, { op: 'text-add', page: 0, point: [20, 500], text: 'Big', font: 'Helvetica', size, color: [0, 0, 0] });
  assert.equal(add(300).ok, true);
  assert.equal(add(401).ok, false);
});

function rawPdf(content, extra = '') {
  const L = 'latin1', body = Buffer.from(content, L);
  const objects = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] ${extra} /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    Buffer.concat([Buffer.from(`<< /Length ${body.length} >>\nstream\n`, L), body, Buffer.from('endstream', L)])];
  const parts = [Buffer.from('%PDF-1.4\n', L)], offsets = [];
  let size = parts[0].length;
  for (const [i, o] of objects.entries()) {
    const chunk = Buffer.concat([Buffer.from(`${i + 1} 0 obj\n`, L), Buffer.isBuffer(o) ? o : Buffer.from(o, L), Buffer.from('\nendobj\n', L)]);
    offsets.push(size); parts.push(chunk); size += chunk.length;
  }
  let xref = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const o of offsets) xref += `${String(o).padStart(10, '0')} 00000 n \n`;
  parts.push(Buffer.from(`${xref}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${size}\n%%EOF\n`, L));
  return Buffer.concat(parts);
}

test('§9.1–2 move and size through any invertible matrix; singular Tm refuses `matrix`', () => {
  for (const [label, content, extra] of [
    ['Tm-scaled 1 Tf', 'BT /F1 1 Tf 12 0 0 12 72 700 Tm (Hello) Tj 0 -1.2 Td (World) Tj ET', ''],
    ['rotated page', 'BT /F1 12 Tf 72 700 Td (Hello) Tj 0 -14 Td (World) Tj ET', '/Rotate 90'],
    ['rotated text + cm', 'q 0.5 0 0 0.5 0 0 cm BT /F1 24 Tf 0 1 -1 0 300 800 Tm (Hello) Tj 1 0 Td (World) Tj ET Q', ''],
  ]) {
    const s = engine.createSession();
    assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'x.pdf', pdf: buffer(rawPdf(content, extra)) }, s).reply.ok, true);
    const { item, before, reply } = editRun(s, 0, 'Hello', { move: { dx: 10, dy: 20 } });
    assert.deepEqual([item.fontSize, reply.ok], [12, true], label);
    const after = scan(s, 0);
    same(text(after, 'Hello').bounds, [item.bounds[0] + 10, item.bounds[1] + 20, item.bounds[2] + 10, item.bounds[3] + 20]);
    same(text(after, 'World').bounds, text(before, 'World').bounds);
    assert.equal(editRun(s, 0, 'Hello', { size: 30 }).reply.ok, true, label);
    assert.equal(text(scan(reopen(s), 0), 'Hello').fontSize, 30, label);
  }
  const s = engine.createSession();
  engine.dispatch({ id: 1, op: 'open', name: 'x.pdf', pdf: buffer(rawPdf('BT /F1 12 Tf 1 0 0 0 72 700 Tm (Flat) Tj ET')) }, s);
  const flat = text(scan(s, 0), 'Flat');
  if (flat.support === 'candidate') {
    assert.deepEqual([flat.moveable, flat.moveReason], [false, 'matrix']);
    assert.equal(editRun(s, 0, 'Flat', { move: { dx: 1, dy: 1 } }).reply.reason, 'matrix');
  }
});
