import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from '../build/renderer/engine.js';

// P6 U3 engine (contract-p6-unit3.md §1.1–1.3, §6.1, §9.1–9.5): the page transform in
// `edit-targets`, the edit-object no-op, obstacles in the view space, text-add where you
// click. Every write is saved, re-opened, re-scanned and rendered. `redact-set` lives in
// tests/p4-redact.test.mjs.
const engine = createEngine(mupdf);
const DIRECT3 = 'fixtures/p6/direct3.pdf';
const DIRECT3_SHA = 'e3ae5a43d9fb17e5b164c00b529d5be50ba4d9178de6c11b66252cc240297466';
const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);

async function open(name = DIRECT3) {
  const s = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: buffer(await readFile(name)) }, s).reply;
  assert.equal(reply.ok, true, reply.error);
  return s;
}
const request = (s, req) => engine.dispatch({ id: 2, ...req }, s).reply;
const scan = (s, page) => request(s, { op: 'edit-targets', page });
const text = (sc, value) => sc.items.find(item => item.text === value);
const object = (sc, name) => sc.items.find(item => item.kind !== 'text' && item.objectName === name);
function reopen(s) {
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const fresh = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'saved.pdf', pdf: saved.bytes }, fresh).reply.ok, true);
  return fresh;
}
/** Dark pixels inside a page-pt box (y down) of the page rendered at 72 dpi (§9.3: < 200, so gray Im1 counts). */
function ink(s, page, box) {
  const view = s.doc.loadPage(page);
  const pixmap = view.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceGray, false);
  try {
    const width = pixmap.getWidth(), pixels = pixmap.getPixels();
    let count = 0;
    for (let y = Math.max(0, Math.floor(box[1])); y < Math.min(pixmap.getHeight(), Math.ceil(box[3])); y++) {
      for (let x = Math.max(0, Math.floor(box[0])); x < Math.min(width, Math.ceil(box[2])); x++) if (pixels[y * width + x] < 200) count++;
    }
    return count;
  } finally { pixmap.destroy(); view.destroy(); }
}
function contents(s, page = 0) {
  const view = s.doc.loadPage(page);
  try {
    const value = view.getObject().get('Contents');
    const elements = value.isArray() ? [...Array(value.length)].map((_, index) => value.get(index)) : [value];
    return elements.map(element => {
      const data = element.readStream();
      try { return Buffer.from(data.asUint8Array()).toString('latin1'); } finally { data.destroy(); }
    }).join('\n');
  } finally { view.destroy(); }
}
/** The renderer's `viewBox`: map the 4 corners of a y-up box through the transform, min/max. */
function viewBox(box, m) {
  const points = [[box[0], box[1]], [box[2], box[1]], [box[0], box[3]], [box[2], box[3]]]
    .map(([x, y]) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]);
  const xs = points.map(p => p[0]), ys = points.map(p => p[1]);
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}
/** The renderer's `pdfBox`: the exact inverse of the 2×2 linear part plus translation. */
function pdfBox(rect, m) {
  const det = m[0] * m[3] - m[1] * m[2];
  const inv = ([x, y]) => [(m[3] * (x - m[4]) - m[2] * (y - m[5])) / det, (-m[1] * (x - m[4]) + m[0] * (y - m[5])) / det];
  const [a, b] = [inv([rect[0], rect[1]]), inv([rect[2], rect[3]])];
  return [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[0], b[0]), Math.max(a[1], b[1])];
}
function editObject(s, sc, name, action, box) {
  const item = object(sc, name);
  return request(s, { op: 'edit-object', page: sc.page, targetId: item.targetId, generation: sc.generation, revision: sc.revision, action, box });
}
const near = (actual, expected, tolerance = 0.01) => {
  assert.equal(actual.length, expected.length);
  actual.forEach((value, index) => assert.ok(Math.abs(value - expected[index]) <= tolerance, `${JSON.stringify(actual)} vs ${JSON.stringify(expected)}`));
};

test('P6 U3 fixture is the generated one', async () => {
  const bytes = await readFile(DIRECT3);
  assert.equal(createHash('sha256').update(bytes).digest('hex'), DIRECT3_SHA);
});

test('§1.1/§9.1 edit-targets carries the page transform; object bounds stay in user space', async () => {
  const s = await open();
  const p0 = scan(s, 0);
  assert.deepEqual(p0.transform.map(v => v + 0), [1, 0, 0, -1, 0, 792]);
  const im1 = object(p0, 'Im1');
  assert.deepEqual(im1.bounds, [72, 600, 172, 650]);
  assert.deepEqual(im1.placement, { box: [72, 600, 172, 650], orient: { swap: false, sx: 1, sy: 1 } }, 'the internal view box is never sent');
  assert.deepEqual(im1.objectOps, ['move', 'resize', 'delete', 'replace', 'duplicate', 'transform', 'crop']);
  assert.deepEqual(viewBox(im1.placement.box, p0.transform), [72, 142, 172, 192]);
  assert.deepEqual(viewBox(object(p0, 'Im2').placement.box, p0.transform), [400, 52, 460, 92]);
  const fm1 = object(p0, 'Fm1');
  assert.equal(fm1.kind, 'form');
  assert.deepEqual(viewBox(fm1.placement.box, p0.transform), [400, 262, 500, 292]);
  assert.deepEqual(pdfBox([72, 142, 172, 192], p0.transform), [72, 600, 172, 650]);
  // Page 1 is /Rotate 90: its transform is not a flip, and no placement is editable there.
  assert.deepEqual(scan(s, 1).transform.map(v => v + 0), [0, 1, 1, 0, 0, 0]);
  // Page 2: the background image covers the page (the renderer's page-cover rule hides it).
  const p2 = scan(s, 2);
  assert.deepEqual(p2.transform.map(v => v + 0), [1, 0, 0, -1, 0, 792]);
  assert.deepEqual(viewBox(object(p2, 'Bg').placement.box, p2.transform), [0, 0, 612, 792]);
  assert.equal(text(p2, 'On top').support, 'candidate');
  // A non-zero MediaBox origin ([20 30 632 822]) shows up as the translation.
  const shifted = scan(await open('fixtures/edit-tj.pdf'), 3);
  assert.deepEqual(shifted.transform.map(v => v + 0), [1, 0, 0, -1, -20, 822]);
  assert.deepEqual(pdfBox([10, 10, 50, 40], shifted.transform), [30, 782, 70, 812]);
  assert.deepEqual(viewBox([30, 782, 70, 812], shifted.transform), [10, 10, 50, 40]);
});

test('§1.2/§9.2 a gesture-equivalent move lands, undoes, and a same-box move is a no-op', async () => {
  const s = await open();
  const before = contents(s);
  const sc = scan(s, 0);
  // Drag Im1's view box by (50, 30) on the page: the renderer sends pdfBox of the new rect.
  const box = pdfBox([122, 172, 222, 222], sc.transform);
  assert.deepEqual(box, [122, 570, 222, 620]);
  // No-op first, on a clean document: the same box, and a box within the 4-decimal rounding.
  for (const same of [[72, 600, 172, 650], [72.00001, 600, 172.00001, 650]]) {
    const noop = editObject(s, sc, 'Im1', 'move', same);
    assert.deepEqual([noop.ok, noop.noop, noop.revision, noop.dirty, noop.canUndo], [true, true, 0, false, false]);
  }
  assert.equal(editObject(s, sc, 'Im1', 'resize', [72, 600, 172, 650]).noop, true);
  // A bad box still refuses before the no-op test.
  assert.equal(editObject(s, sc, 'Im1', 'move', [72, 600, 72, 650]).detail, 'out-of-bounds');
  assert.equal(contents(s), before);

  const moved = editObject(s, sc, 'Im1', 'move', box);
  assert.deepEqual([moved.ok, moved.noop, moved.revision, moved.dirty], [true, undefined, 1, true], moved.error);
  // A no-op on a dirty document leaves revision and dirty as they are.
  const again = scan(s, 0);
  const noop = editObject(s, again, 'Im1', 'move', box);
  assert.deepEqual([noop.noop, noop.revision, noop.dirty], [true, 1, true]);

  const saved = reopen(s);
  const rescanned = scan(saved, 0);
  assert.deepEqual(object(rescanned, 'Im1').placement.box, [122, 570, 222, 620]);
  assert.deepEqual(viewBox(object(rescanned, 'Im1').placement.box, rescanned.transform), [122, 172, 222, 222]);
  assert.ok(ink(saved, 0, [124, 174, 220, 220]) > 4000, 'Im1 is drawn at its new place');
  assert.equal(ink(saved, 0, [74, 144, 120, 170]), 0, 'and no longer at its old place');
  assert.ok(ink(s, 0, [74, 144, 120, 170]) === 0 && ink(await open(), 0, [74, 144, 120, 170]) > 1000);

  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.equal(contents(s), before, 'undo restores the old bytes');
});

test('§1.3/§9.3 obstacles are compared in the view space (y down)', async () => {
  // (a) Paragraph line 1 (y-down 80.18..95.29) shares Im2's band (y-down 52..92, x 400..460):
  // its growth limit is Im2's left edge − 1 = 399. Before the fix Im2's y-up box (700..740)
  // missed the band and the limit was the widest text's right edge (≈ 220), so the first
  // edit below refused `overflow`.
  const s = await open();
  const sc = scan(s, 0);
  const line1 = text(sc, 'Alpha bravo charlie delta echo');
  near(line1.bounds, [72, 80.18, 219.96, 95.29]);
  const grow = request(s, { op: 'edit-text', page: 0, targetId: line1.targetId, generation: sc.generation, revision: sc.revision, text: 'Alpha bravo charlie delta echo foxtrot golf hotel' });
  assert.equal(grow.ok, true, grow.error);
  const grown = scan(s, 0);
  const wide = text(grown, 'Alpha bravo charlie delta echo foxtrot golf hotel');
  assert.ok(wide.bounds[2] > 300 && wide.bounds[2] < 399, `right edge ${wide.bounds[2]}`);
  const tooWide = request(s, { op: 'edit-text', page: 0, targetId: wide.targetId, generation: grown.generation, revision: grown.revision, text: 'Alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike' });
  assert.deepEqual([tooWide.ok, tooWide.detail, tooWide.limit, tooWide.revision], [false, 'overflow', 399, 1]);
  const saved = reopen(s);
  const again = text(scan(saved, 0), 'Alpha bravo charlie delta echo foxtrot golf hotel');
  assert.ok(again, 'the grown line survives save and reopen');
  assert.ok(ink(saved, 0, [230, 82, again.bounds[2], 94]) > 20, 'the grown text is drawn');

  // (b) Paragraph bottom y-down 134.89; Im1's top is y-down 142 (x 72..172 overlaps the
  // paragraph). A 5th line (leading 13.2) would end at ≈ 148.09, into Im1 → overflow with
  // maxLines 4. Before the fix Im1 read as y-down 600..650 and the 5-line edit landed.
  const t = await open();
  const para = scan(t, 0).paragraphs[0];
  near(para.bounds, [72, 80.18, 219.96, 134.89]);
  assert.equal(para.leading, 13.2);
  const fifthBottom = para.bounds[3] + para.leading;
  assert.ok(fifthBottom + 1 > 142 && para.bounds[3] + 1 < 142, `5-line bottom ${fifthBottom}`);
  const lines = ['Alpha bravo charlie delta echo', 'foxtrot golf hotel india', 'juliet kilo lima mike november', 'oscar papa quebec'];
  const five = (() => {
    const p = scan(t, 0);
    return request(t, { op: 'edit-paragraph', page: 0, paragraphId: p.paragraphs[0].paragraphId, generation: p.generation, revision: p.revision, text: [...lines, 'romeo'].join('\n') });
  })();
  assert.deepEqual([five.ok, five.detail, five.lines, five.maxLines, five.revision, five.dirty], [false, 'overflow', 5, 4, 0, false]);
  const p = scan(t, 0);
  const four = request(t, { op: 'edit-paragraph', page: 0, paragraphId: p.paragraphs[0].paragraphId, generation: p.generation, revision: p.revision, text: [...lines.slice(0, 3), 'oscar papa quebec romeo'].join('\n') });
  assert.equal(four.ok, true, four.error);
  assert.equal(four.lines, 4);
  const reopened = reopen(t);
  const after = scan(reopened, 0);
  assert.ok(text(after, 'oscar papa quebec romeo'), 'the re-worded 4th line survives save and reopen');
  assert.ok(after.paragraphs[0].bounds[3] + 1 < 142, 'and still clears Im1 by 1 pt');
  assert.ok(ink(reopened, 0, [72, 124, 219, 137]) > 20, 'line 4 is drawn');
});

test('§2.3/§9.5 text-add at a click point lands a candidate run; a rotated page refuses', async () => {
  const s = await open();
  // A click at the empty (300, 500), Helvetica 12: baseline = top + 0.8 · size.
  const point = [300, 500 + 0.8 * 12];
  const added = request(s, { op: 'text-add', page: 0, point, text: 'Hello there', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.equal(added.ok, true, added.error);
  assert.ok(added.targetId, 'the reply names the new run');
  assert.deepEqual([added.revision, added.dirty], [1, true]);
  assert.ok(Math.abs(added.bounds[0] - 300) < 0.5 && added.bounds[1] < point[1] && added.bounds[3] > point[1] - 1, JSON.stringify(added.bounds));
  const saved = reopen(s);
  const run = text(scan(saved, 0), 'Hello there');
  assert.equal(run.support, 'candidate');
  assert.equal(run.fontSize, 12);
  assert.ok(ink(saved, 0, [300, 500, 362, 512]) > 30, 'the added text is drawn under the click');

  // §9.5: page 1 is /Rotate 90 (shown 792×612); a point with y ≤ 612 reaches the rotate check.
  const t = await open();
  const refused = request(t, { op: 'text-add', page: 1, point: [300, 300], text: 'No', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.deepEqual([refused.ok, refused.reason, refused.revision, refused.dirty], [false, 'unsupported-transform', 0, false]);
});

test('§9.5 a landed text-add replies ok even when the lookup scan is over budget', () => {
  // 512 runs is the scan ceiling: the pre-write scan passes, the 513th (the add) makes the
  // lookup scan refuse `too-large`. The add has landed, so the reply is ok with no target.
  const pdf = new mupdf.PDFDocument();
  const font = pdf.addSimpleFont(new mupdf.Font('Helvetica'), 'Latin');
  const resources = pdf.addObject({ Font: { F1: font } });
  let body = 'BT /F1 6 Tf ';
  for (let i = 0; i < 512; i++) body += `1 0 0 1 ${20 + (i % 32) * 18} ${40 + Math.floor(i / 32) * 30} Tm (a) Tj `;
  pdf.insertPage(-1, pdf.addPage([0, 0, 612, 792], 0, resources, `${body}ET`));
  const bytes = pdf.saveToBuffer('').asUint8Array().slice();
  const s = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'crowded.pdf', pdf: buffer(bytes) }, s).reply.ok, true);
  assert.equal(scan(s, 0).items.length, 512);
  const added = request(s, { op: 'text-add', page: 0, point: [300, 60], text: 'Late', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.deepEqual([added.ok, added.targetId, added.bounds, added.revision, added.dirty], [true, '', null, 1, true], added.error);
  assert.equal(scan(s, 0).reason, 'too-large', 'the page is now over the scan budget');
});

test('§2.3 review S1: added text that is a substring of another run still gets its own bounds', async () => {
  // editMatchRuns used to call a second indexOf hit ambiguous; the run's own Tm origin now picks it.
  for (const value of ['Alpha', 'golf', 'me', 'a']) {
    const s = await open();
    const added = request(s, { op: 'text-add', page: 0, point: [300, 520], text: value, font: 'Helvetica', size: 12, color: [0, 0, 0] });
    assert.equal(added.ok, true, added.error);
    assert.ok(added.targetId && added.bounds, `${value}: the reply names a target with bounds`);
    assert.ok(Math.abs(added.bounds[0] - 300) < 1 && added.bounds[1] < 520 && added.bounds[3] > 520, `${value}: bounds sit at the click`);
    const item = scan(s, 0).items.find(i => i.targetId === added.targetId);
    assert.deepEqual([item.support, item.bounds], ['candidate', added.bounds]);
    const host = scan(s, 0).items.find(i => i.kind === 'text' && i.text !== value && i.text.includes(value));
    assert.ok(host.bounds && host.bounds[0] < 100, `${value}: the host run keeps its own bounds`);
  }
});
