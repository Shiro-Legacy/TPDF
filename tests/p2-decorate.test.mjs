/**
 * P2 unit 8 §A — page decoration: watermark/header/footer, page labels, layers.
 * Runs against src/renderer/engine.ts exactly as the worker does.
 *
 * The unit's own gate (`.stage/gate-u8.mjs`, written in `p2-decorate.md`):
 * `setLayerVisible` is **not journaled** and `saveToBuffer` does **not** write it, so a
 * layer toggle is a view-only change — no operation, no revision, no dirty bit — and
 * the renderer says so. mupdf also lists layers in the reverse of `/OCGs`, and reports
 * the *displayed* box for `getBounds("MediaBox")`, which is why the stamp reads the raw
 * `/CropBox`/`/MediaBox` array for its placement.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import { createEngine, MAX_STAMP_CHARS } from '../build/renderer/engine.js';
import { assertWithin } from './timing-budget.mjs';

const engine = createEngine(mupdf);

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

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
  return toArrayBuffer(Buffer.from(parts.join(''), 'latin1'));
}

/** The layers fixture shape (grok's `fixtures/layers.pdf`): Base + Notes, both visible. */
function layersPdf() {
  const content = '/OC /ocA BDC BT /F1 24 Tf 72 700 Td (base) Tj ET EMC\n'
    + '/OC /ocB BDC BT /F1 24 Tf 72 600 Td (notes) Tj ET EMC\n';
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [6 0 R 7 0 R] /D << /Order [6 0 R 7 0 R] /ON [6 0 R 7 0 R] >> >> >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] '
      + '/Resources << /Font << /F1 4 0 R >> /Properties << /ocA 6 0 R /ocB 7 0 R >> >> /Contents 5 0 R >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    `<< /Length ${content.length} >>\nstream\n${content}endstream`,
    '<< /Type /OCG /Name (Base) >>',
    '<< /Type /OCG /Name (Notes) >>',
  ]);
}

/** A 400x200 blank page with `/Rotate`, for the visual-placement checks. */
function rotatedPagePdf(rotate) {
  return buildPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Rotate ${rotate} >>`,
  ]);
}

async function openBytes(bytes, name = 'scratch.pdf', session = engine.createSession()) {
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: bytes }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

async function open(id, session = engine.createSession()) {
  return openBytes(toArrayBuffer(await readFile(`fixtures/${id}.pdf`)), `${id}.pdf`, session);
}

function op(session, request) {
  return engine.dispatch({ id: 900 + (request.id ?? 0), ...request }, session).reply;
}

function pageText(doc, page) {
  const loaded = doc.loadPage(page);
  try {
    return loaded.toStructuredText().asText().trim();
  } finally {
    loaded.destroy();
  }
}

function labelsOf(session) {
  const reply = op(session, { op: 'labels' });
  assert.equal(reply.ok, true, reply.error);
  return reply.items;
}

/** The bytes of the last content stream of a page, as latin1 text. */
function lastStream(doc, page) {
  const contents = doc.asPDF().findPage(page).get('Contents');
  assert.equal(contents.isArray(), true, 'the stamp makes /Contents an array');
  return Buffer.from(contents.get(contents.length - 1).readStream().asUint8Array()).toString('latin1');
}

/** The ink rows of a rendered page, 1 px per point. */
function inkRows(doc, page) {
  const loaded = doc.loadPage(page);
  try {
    const pixmap = loaded.toPixmap(mupdf.Matrix.scale(1, 1), mupdf.ColorSpace.DeviceRGB, false);
    try {
      const pixels = pixmap.getPixels();
      const components = pixmap.getNumberOfComponents();
      const rows = [];
      for (let y = 0; y < pixmap.getHeight(); y++) {
        for (let x = 0; x < pixmap.getWidth(); x++) {
          if (pixels[(y * pixmap.getWidth() + x) * components] < 128) {
            rows.push(y);
            break;
          }
        }
      }
      return { rows, width: pixmap.getWidth(), height: pixmap.getHeight() };
    } finally {
      pixmap.destroy();
    }
  } finally {
    loaded.destroy();
  }
}

// ------------------------------------------------------------------ stamps

test('a watermark adds one appended stream per page, and one undo removes them all', async () => {
  const session = await open('nav');
  try {
    const before = pageText(session.doc, 0);
    const stamped = op(session, { op: 'stamp', kind: 'watermark', text: 'DRAFT' });
    assert.equal(stamped.ok, true, stamped.error);
    assert.equal(stamped.revision, 1);
    assert.equal(stamped.dirty, true);

    for (const page of [0, 1, 2]) {
      assert.match(pageText(session.doc, page), /DRAFT/, `page ${page + 1} carries the stamp`);
    }
    const contents = session.doc.asPDF().findPage(0).get('Contents');
    assert.equal(contents.isArray(), true);
    assert.equal(Buffer.from(contents.get(0).readStream().asUint8Array()).toString('latin1'), 'q\n',
      'the first stream saves the graphics state');
    assert.match(lastStream(session.doc, 0), /^Q\n\/PDF4All <<\/Kind \/Watermark>> BDC /);
    assert.match(lastStream(session.doc, 0), /0\.5 g BT \/PDF4AllF 48 Tf /, '50% grey at 48 pt');
    // The page's own resources keep their fonts; ours is one more key.
    const fonts = session.doc.asPDF().findPage(0).get('Resources').get('Font');
    const keys = [];
    fonts.forEach((value, key) => keys.push(key));
    assert.deepEqual(keys.sort(), ['F1', 'PDF4AllF']);

    const undone = op(session, { op: 'undo' });
    assert.equal(undone.step, 'add watermark');
    for (const page of [0, 1, 2]) assert.equal(pageText(session.doc, page), before === 'one' && page === 0 ? 'one' : pageText(session.doc, page));
    assert.deepEqual([0, 1, 2].map(page => pageText(session.doc, page)), ['one', 'two', 'three'],
      'one undo takes every page back');
    const redone = op(session, { op: 'redo' });
    assert.equal(redone.step, 'add watermark');
    assert.match(pageText(session.doc, 0), /DRAFT/);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('header and footer land at the visual top and bottom of every rotation', async () => {
  for (const [rotate, expected] of [
    [0, /1 0 0 1 36 164 Tm/],
    [90, /0 1 -1 0 36 36 Tm/],
    [180, /-1 0 0 -1 364 36 Tm/],
    [270, /0 -1 1 0 364 164 Tm/],
  ]) {
    const session = await openBytes(rotatedPagePdf(rotate), `rot${rotate}.pdf`);
    try {
      // 400x200 page: R=0/180 keep H=200, R=90/270 swap it.
      const header = op(session, { op: 'stamp', kind: 'header', text: 'HEADER' });
      assert.equal(header.ok, true, header.error);
      assert.equal(header.rotations[0], rotate);
      assert.match(lastStream(session.doc, 0), expected, `R=${rotate}: the header text matrix`);
      const footer = op(session, { op: 'stamp', kind: 'footer', text: 'FOOTER' });
      assert.equal(footer.ok, true, footer.error);

      const ink = inkRows(session.doc, 0);
      const top = ink.rows.filter(row => row < 60).length;
      const bottom = ink.rows.filter(row => row > ink.height - 60).length;
      assert.ok(top > 0, `R=${rotate}: the header paints at the visual top`);
      assert.ok(bottom > 0, `R=${rotate}: the footer paints at the visual bottom`);
      assert.equal(ink.rows.some(row => row > 60 && row < ink.height - 60), false,
        `R=${rotate}: nothing in the middle`);
    } finally {
      engine.closeDocument(session.doc);
    }
  }
});

test('header placeholders fill per page from the index, the count and the label', async () => {
  const session = await open('nav');
  try {
    assert.equal(op(session, { op: 'stamp', kind: 'header', text: 'Page {page}/{pages} {label}', pages: [1] }).ok, true);
    assert.match(pageText(session.doc, 1), /Page 2\/3 2/);
    assert.equal(op(session, { op: 'labels-set', from: 0, style: 'r' }).ok, true);
    assert.equal(op(session, { op: 'stamp', kind: 'footer', text: 'leaf {label}', pages: [1, 2] }).ok, true);
    assert.match(pageText(session.doc, 1), /leaf ii/);
    assert.match(pageText(session.doc, 2), /leaf iii/);
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ------------------------------------------------------------------ labels

test('labels set, clear, undo and redo, and `getLabel` follows', async () => {
  const session = await open('nav');
  try {
    assert.deepEqual(labelsOf(session), ['1', '2', '3'], 'the index is the default label');

    const roman = op(session, { op: 'labels-set', from: 0, style: 'r' });
    assert.equal(roman.ok, true, roman.error);
    assert.deepEqual(roman.items, ['i', 'ii', 'iii']);
    assert.equal(roman.dirty, true);
    assert.deepEqual(labelsOf(session), ['i', 'ii', 'iii']);

    const mixed = op(session, { op: 'labels-set', from: 1, style: 'D', prefix: 'A-', start: 5 });
    assert.equal(mixed.ok, true, mixed.error);
    assert.deepEqual(mixed.items, ['i', 'A-5', 'A-6']);

    const undone = op(session, { op: 'undo' });
    assert.equal(undone.step, 'set page labels');
    assert.deepEqual(labelsOf(session), ['i', 'ii', 'iii']);
    assert.equal(op(session, { op: 'redo' }).step, 'set page labels');
    assert.deepEqual(labelsOf(session), ['i', 'A-5', 'A-6']);

    const cleared = op(session, { op: 'labels-clear', from: 1 });
    assert.equal(cleared.ok, true, cleared.error);
    assert.deepEqual(cleared.items, ['i', 'ii', 'iii'],
      'clearing the range that started at page 2 leaves the earlier roman range applying');
    assert.equal(op(session, { op: 'undo' }).step, 'clear page labels');
    assert.deepEqual(labelsOf(session), ['i', 'A-5', 'A-6']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ------------------------------------------------------------------ layers

test('layers list, toggle as a view-only change, and are never written on save', async () => {
  const session = await openBytes(layersPdf(), 'layers.pdf');
  try {
    const listed = op(session, { op: 'layers' });
    assert.equal(listed.ok, true, listed.error);
    // mupdf lists OCGs in the reverse of /OCGs: [Base, Notes] reads back as [Notes, Base].
    assert.deepEqual(listed.items, [
      { id: 0, name: 'Notes', visible: true },
      { id: 1, name: 'Base', visible: true },
    ]);
    assert.equal(pageText(session.doc, 0).replace(/\s+/g, ' ').trim(), 'base notes');

    const revision = session.revision;
    const hidden = op(session, { op: 'layer-set', layerId: 1, visible: false });
    assert.equal(hidden.ok, true, hidden.error);
    assert.equal(hidden.persisted, false, 'measured: mupdf does not journal or save a layer toggle');
    assert.deepEqual(hidden.items[1], { id: 1, name: 'Base', visible: false });
    assert.equal(session.revision, revision, 'no revision bump: nothing was journaled');
    assert.equal(op(session, { op: 'undo' }).ok, false, 'and there is nothing to undo');
    assert.equal(pageText(session.doc, 0).replace(/\s+/g, ' ').trim(), 'notes', 'the in-session render honours it');

    const save = engine.dispatch({ id: 20, op: 'save' }, session).reply;
    assert.equal(save.ok, true, save.error);
    assert.equal(save.dirty, false, 'the document was never dirty');
    const re = mupdf.Document.openDocument(save.bytes, 'application/pdf');
    try {
      assert.deepEqual(Array.from({ length: re.countLayers() }, (_, index) => [re.getLayerName(index), re.isLayerVisible(index)]),
        [['Notes', true], ['Base', true]], 'the saved copy still has every layer visible');
      assert.equal(pageText(re, 0).replace(/\s+/g, ' ').trim(), 'base notes', 'and the hidden layer is back');
    } finally {
      re.destroy();
    }

    // A document without layers lists none, and refuses a toggle.
    const plain = await open('hello');
    try {
      assert.deepEqual(op(plain, { op: 'layers' }).items, []);
      const refused = op(plain, { op: 'layer-set', layerId: 0, visible: false });
      assert.equal(refused.ok, false);
      assert.match(refused.error, /layer must be an integer in 0\.\.0/);
    } finally {
      engine.closeDocument(plain.doc);
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

// --------------------------------------------------------------- refusals

test('stamps and labels refuse what cannot be written, without dirtying the document', async () => {
  const session = await open('nav');
  try {
    const refusals = [
      [op(session, { op: 'stamp', kind: 'watermark', text: '' }), /1\.\.200 characters/],
      [op(session, { op: 'stamp', kind: 'header', text: 'x'.repeat(MAX_STAMP_CHARS + 1) }), /1\.\.200 characters/],
      [op(session, { op: 'stamp', kind: 'header', text: 'plain \u{1F600}' }), /U\+1F600\) is not a WinAnsi/],
      [op(session, { op: 'stamp', kind: 'banner', text: 'x' }), /kind must be one of watermark, header, footer/],
      [op(session, { op: 'stamp', kind: 'header', text: 'x', pages: [] }), /1\.\.2000/],
      [op(session, { op: 'stamp', kind: 'header', text: 'x', pages: [3] }), /page 0 must be an integer in 0\.\.2/],
      [op(session, { op: 'labels-set', from: 0, style: 'X' }), /style must be one of/],
      [op(session, { op: 'labels-set', from: 9, style: 'D' }), /0\.\.2/],
      [op(session, { op: 'labels-set', from: 0, style: 'D', start: 0 }), /start must be an integer of at least 1/],
      [op(session, { op: 'labels-set', from: 0, style: 'D', prefix: 'x'.repeat(33) }), /at most 32 characters/],
      [op(session, { op: 'labels-clear', from: -1 }), /0\.\.2/],
      [op(session, { op: 'layer-set', layerId: 0, visible: false }), /layer must be an integer in 0\.\.0/],
    ];
    for (const [reply, pattern] of refusals) {
      assert.equal(reply.ok, false, JSON.stringify(reply));
      assert.match(reply.error, pattern);
    }
    assert.equal(op(session, { op: 'undo' }).ok, false, 'nothing was journaled');
    assert.deepEqual([0, 1, 2].map(page => pageText(session.doc, page)), ['one', 'two', 'three']);
    // A WinAnsi extra works: the euro sign is one byte in WinAnsiEncoding.
    assert.equal(op(session, { op: 'stamp', kind: 'footer', text: 'Costs 5\u20ac' }).ok, true);
    assert.match(lastStream(session.doc, 0), /Costs 5\x80/);
  } finally {
    engine.closeDocument(session.doc);
  }

  // Read-only documents refuse every decoration, and a signed one still lists labels.
  const signed = await openBytes(buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>',
    '<< /Length 2 >>\nstream\nq \nendstream',
    '<< /Fields [6 0 R] /SigFlags 3 >>',
    '<< /Type /Annot /Subtype /Widget /FT /Sig /T (sig1) /Rect [72 72 300 120] /P 3 0 R /V 7 0 R >>',
    '<< /Type /Sig /Filter /Adobe.PPKLite >>',
  ]), 'signed.pdf');
  try {
    for (const request of [
      { op: 'stamp', kind: 'watermark', text: 'x' },
      { op: 'labels-set', from: 0, style: 'r' },
      { op: 'labels-clear', from: 0 },
    ]) {
      const refused = op(signed, request);
      assert.equal(refused.ok, false, request.op);
      assert.equal(refused.reason, 'read-only');
    }
    assert.deepEqual(labelsOf(signed), ['1'], 'reading labels is still fine');
    assert.deepEqual(op(signed, { op: 'layers' }).items, []);
  } finally {
    engine.closeDocument(signed.doc);
  }
});

// ------------------------------------------------------------ round trip

test('save, validate and reopen keep the stamps and the labels', async () => {
  const session = await open('nav');
  try {
    assert.equal(op(session, { op: 'stamp', kind: 'watermark', text: 'PDF4All decorate' }).ok, true);
    assert.equal(op(session, { op: 'stamp', kind: 'header', text: 'Page {page} of {pages}' }).ok, true);
    assert.equal(op(session, { op: 'labels-set', from: 0, style: 'r' }).ok, true);

    const save = engine.dispatch({ id: 20, op: 'save' }, session).reply;
    assert.equal(save.ok, true, save.error);
    const check = engine.dispatch({ id: 21, op: 'validate', pdf: save.bytes }, session).reply;
    assert.equal(check.ok, true, check.error);
    assert.equal(check.pageCount, 3);

    const re = mupdf.Document.openDocument(check.bytes, 'application/pdf');
    try {
      assert.deepEqual([0, 1, 2].map(page => re.loadPage(page).getLabel()), ['i', 'ii', 'iii']);
      const first = pageText(re, 0);
      assert.match(first, /PDF4All decorate/, 'the watermark survives');
      assert.match(first, /Page 1 of 3/, 'the header survives, with its placeholders filled');
      assert.match(first, /one/, 'and so does the page');
      const reopened = mupdf.Document.openDocument(check.bytes, 'application/pdf');
      assert.equal(reopened.asPDF().findPage(0).get('Contents').isArray(), true);
      reopened.destroy();
    } finally {
      re.destroy();
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});

// -------------------------------------------------------------------- cost

test('large.pdf: a watermark on all 500 pages stays inside the budget', async () => {
  const session = await open('large');
  try {
    assert.equal(session.doc.countPages(), 500);
    const plain = engine.dispatch({ id: 10, op: 'save' }, session).reply;
    const started = performance.now();
    const stamped = op(session, { op: 'stamp', kind: 'watermark', text: 'CONFIDENTIAL' });
    const stampMs = performance.now() - started;
    assert.equal(stamped.ok, true, stamped.error);

    const saved = engine.dispatch({ id: 11, op: 'save' }, session).reply;
    assert.equal(saved.ok, true, saved.error);
    const growth = saved.bytes.byteLength - plain.bytes.byteLength;
    console.log(`U8_LARGE ${JSON.stringify({
      stampMs: Math.round(stampMs), growth, stampedPages: 500,
    })}`);
    assertWithin(stampMs, 2000, '500 stamps');
    assert.ok(growth > 0 && growth < 5 * 1024 * 1024, `the stamp stream grows the file sanely, not ${growth} bytes`);
    assert.match(pageText(session.doc, 499), /CONFIDENTIAL/, 'the last page too');
  } finally {
    engine.closeDocument(session.doc);
  }
});

// ------------------------------------------------------------ Bates (contract-bates.md §1.5 + §9)

/** The stext line whose text matches `pattern`, with its bbox right edge, on one page. */
function lineMatching(doc, page, pattern) {
  const loaded = doc.loadPage(page);
  try {
    const json = JSON.parse(loaded.toStructuredText().asJSON());
    for (const block of json.blocks) {
      for (const line of block.lines ?? []) {
        if (pattern.test(line.text)) return { text: line.text, x: line.bbox.x, edge: line.bbox.x + line.bbox.w, right: loaded.getBounds()[2], cx: line.bbox.x + line.bbox.w / 2, cy: line.bbox.y + line.bbox.h / 2 };
      }
    }
    return null;
  } finally {
    loaded.destroy();
  }
}

test('bates (a): a footer {bates} counts 100..102 over three pages, one undo takes all back', async () => {
  const session = await open('nav');
  try {
    const reply = op(session, { op: 'stamp', kind: 'footer', text: 'ACME-{bates}', batesStart: 100, batesPad: 6, pages: [0, 1, 2] });
    assert.equal(reply.ok, true, reply.error);
    assert.deepEqual(reply.bates, { from: 100, to: 102 });
    assert.match(pageText(session.doc, 0), /ACME-000100/);
    assert.match(pageText(session.doc, 1), /ACME-000101/);
    assert.match(pageText(session.doc, 2), /ACME-000102/);
    assert.equal(op(session, { op: 'undo' }).step, 'add footer');
    assert.deepEqual([0, 1, 2].map(page => pageText(session.doc, page)), ['one', 'two', 'three']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('bates (b): the count follows the stamped-page order, not the page number', async () => {
  const session = await open('nav');
  try {
    const reply = op(session, { op: 'stamp', kind: 'footer', text: 'ACME-{bates}', batesStart: 100, pages: [2, 0] });
    assert.equal(reply.ok, true, reply.error);
    assert.deepEqual(reply.bates, { from: 100, to: 101 });
    assert.match(pageText(session.doc, 0), /ACME-000100/);
    assert.match(pageText(session.doc, 2), /ACME-000101/);
    assert.doesNotMatch(pageText(session.doc, 1), /ACME/);
    // No {bates} in the text: no bates key even when a start is sent.
    const plain = op(session, { op: 'stamp', kind: 'header', text: 'plain', batesStart: 5 });
    assert.equal(plain.ok, true, plain.error);
    assert.equal('bates' in plain, false);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('bates (c): missing start, bad start/pad and bad align refuse with a token, nothing written', async () => {
  const session = await open('nav');
  try {
    const revision = session.revision;
    for (const [request, token] of [
      [{ text: '{bates}' }, 'bates-start-required'],
      [{ text: '{bates}', batesStart: -1 }, 'invalid-bates'],
      [{ text: '{bates}', batesStart: 1.5 }, 'invalid-bates'],
      [{ text: '{bates}', batesStart: '100' }, 'invalid-bates'],
      [{ text: '{bates}', batesStart: 1, batesPad: 0 }, 'invalid-bates'],
      [{ text: '{bates}', batesStart: 1, batesPad: 16 }, 'invalid-bates'],
      [{ text: 'plain', batesPad: 16 }, 'invalid-bates'],
      [{ text: 'x', align: 'centre' }, 'invalid-align'],
    ]) {
      const reply = op(session, { op: 'stamp', kind: 'footer', ...request });
      assert.equal(reply.ok, false, JSON.stringify(request));
      assert.equal(reply.reason, 'invalid', JSON.stringify(reply));
      assert.match(reply.error, new RegExp(`^stamp: ${token}:`));
      assert.equal(reply.dirty, false);
      assert.equal(session.revision, revision, 'revision unchanged');
    }
    assert.equal(op(session, { op: 'undo' }).ok, false, 'nothing was journaled');
    assert.deepEqual([0, 1, 2].map(page => pageText(session.doc, page)), ['one', 'two', 'three']);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('bates (d): a right-aligned footer ends at the right margin on every rotation and on nav', async () => {
  const cases = [[0, 364], [90, 164], [180, 364], [270, 164]];
  for (const [rotate, target] of cases) {
    const session = await openBytes(rotatedPagePdf(rotate), `rot${rotate}.pdf`);
    try {
      const right = op(session, { op: 'stamp', kind: 'footer', text: 'ACME-{bates}', batesStart: 100, batesPad: 6, align: 'right' });
      assert.equal(right.ok, true, right.error);
      const left = op(session, { op: 'stamp', kind: 'header', text: 'LEFT-CONTROL', align: 'left' });
      assert.equal(left.ok, true, left.error);
      const line = lineMatching(session.doc, 0, /ACME-000100/);
      assert.ok(line, `R=${rotate}: the Bates line is found`);
      assert.equal(line.right - 36, target);
      assert.ok(Math.abs(line.edge - target) <= 10, `R=${rotate}: right edge ${line.edge} within 10 of ${target}`);
      const control = lineMatching(session.doc, 0, /LEFT-CONTROL/);
      assert.ok(Math.abs(control.x - 36) <= 2, `R=${rotate}: left control at x 36, not ${control.x}`);
    } finally {
      engine.closeDocument(session.doc);
    }
  }
  const session = await open('nav');
  try {
    assert.equal(op(session, { op: 'stamp', kind: 'footer', text: 'ACME-{bates}', batesStart: 100, align: 'right', pages: [0] }).ok, true);
    const line = lineMatching(session.doc, 0, /ACME-000100/);
    assert.ok(Math.abs(line.edge - 576) <= 10, `nav: right edge ${line.edge} within 10 of 576`);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('bates (e): {bates} inside a watermark expands too, align ignored', async () => {
  const session = await open('nav');
  try {
    const reply = op(session, { op: 'stamp', kind: 'watermark', text: 'W{bates}', batesStart: 7, batesPad: 3, align: 'right', pages: [1] });
    assert.equal(reply.ok, true, reply.error);
    assert.deepEqual(reply.bates, { from: 7, to: 7 });
    assert.match(pageText(session.doc, 1), /W007/);
    // §9.5: the watermark is centred on the expanded width, not the template's.
    const wide = op(session, { op: 'stamp', kind: 'watermark', text: '{bates}{bates}{bates}', batesStart: 1, batesPad: 15, pages: [0] });
    assert.equal(wide.ok, true, wide.error);
    const mark = lineMatching(session.doc, 0, /\d{3}/);
    assert.ok(mark, 'the watermark line is found');
    assert.ok(Math.hypot(mark.cx - 306, mark.cy - 396) <= 30, `watermark centre (${mark.cx}, ${mark.cy}) within 30 of (306, 396)`);
    // §9.5: {bates} is replaced before {label}, so a label holding {bates} stays literal.
    const labels = op(session, { op: 'labels-set', from: 0, style: 'D', prefix: 'L{bates}-' });
    assert.equal(labels.ok, true, labels.error);
    const header = op(session, { op: 'stamp', kind: 'header', text: '[{label}] {bates}', batesStart: 9, batesPad: 2, pages: [1] });
    assert.equal(header.ok, true, header.error);
    assert.match(pageText(session.doc, 1), /\[L\{bates\}-2\] 09/);
  } finally {
    engine.closeDocument(session.doc);
  }
});

test('bates (f): pad 15 with start 10^15-1 is the ceiling; past it refuses', async () => {
  const session = await open('nav');
  try {
    const top = op(session, { op: 'stamp', kind: 'footer', text: '{bates}', batesStart: 1e15 - 1, batesPad: 15, pages: [0] });
    assert.equal(top.ok, true, top.error);
    assert.match(pageText(session.doc, 0), /999999999999999/);
    const revision = session.revision;
    for (const request of [
      { batesStart: 1e15, batesPad: 15, pages: [0] },
      { batesStart: 1e15 - 2, batesPad: 15, pages: [0, 1, 2] },
    ]) {
      const reply = op(session, { op: 'stamp', kind: 'footer', text: '{bates}', ...request });
      assert.equal(reply.ok, false, JSON.stringify(request));
      assert.match(reply.error, /^stamp: invalid-bates:/);
      assert.equal(session.revision, revision);
    }
  } finally {
    engine.closeDocument(session.doc);
  }
});
