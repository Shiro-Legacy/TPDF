import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';

const PINNED_SHA256 = {
  'redact.pdf': '1601df8c6cdeed038b57dbbb85d7c264d15f413e1efe7e82387e95e9dd444f28',
  'redact-shared-only.pdf': '2392df3856e5138802e07e0f6b512d2998103e9dcfff9bce3b0b0cdec4c15571',
  'edit-tj.pdf': '18e6b8a742172d7109d5e3464fc3746e8c8af05def1285d81ee828896ae48eee',
  'p3/field-defs.pdf': '43782ca55aa415cc9fc09406d95e9300326ad281fac5a0176b14c88a295f4250',
  'p5/empty-sigfield.pdf': '07e46851060fc7aa179c0f89a7d08ebdac26f682f4322edf20c57dad0523a206',
  'p6/direct3.pdf': 'e3ae5a43d9fb17e5b164c00b529d5be50ba4d9178de6c11b66252cc240297466',
  'p6/links.pdf': '0ac413b0fa9b42062542afbc970f0245bceacab0a789017c90a413733962d0bc',
  'p6/objects6.pdf': 'ca0a5988eb81cd6b35916918695cd8c9881b8ebba82ae590e38cdf8a97fb2ab8',
  'p6/quartz.pdf': '937d55fe100bae4c13b753da65fc335a22537a8b783b7ee0ac1abd3594dd72c3',
  'p6/tabs.pdf': 'b90c5db2ed9d4b1221b453bc9df02682fd8a6c678724263738a68d24bc047326',
};

const IDS = ['hello', 'rotated', 'cjk', 'rtl', 'large'];

async function fixture(id) {
  const bytes = await readFile(`fixtures/${id}.pdf`);
  return { id, bytes, doc: mupdf.Document.openDocument(bytes, 'application/pdf') };
}

function nonWhiteChannels(page) {
  const pixmap = page.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  try {
    const pixels = pixmap.getPixels();
    let count = 0;
    for (const value of pixels) if (value < 250) count++;
    return { count, total: pixels.length };
  } finally {
    pixmap.destroy();
  }
}

/** Union bbox of a search hit: search() returns one quad per matched character. */
function bounds(hit) {
  const xs = hit.flatMap((quad) => [quad[0], quad[2], quad[4], quad[6]]);
  const ys = hit.flatMap((quad) => [quad[1], quad[3], quad[5], quad[7]]);
  return {
    x0: Math.min(...xs), y0: Math.min(...ys), x1: Math.max(...xs), y1: Math.max(...ys),
    width: Math.max(...xs) - Math.min(...xs), height: Math.max(...ys) - Math.min(...ys),
  };
}

function hits(page, needle) {
  const text = page.toStructuredText();
  try {
    return text.search(needle);
  } finally {
    text.destroy();
  }
}

test('pinned fixtures match sha256', async () => {
  for (const [name, sha] of Object.entries(PINNED_SHA256)) {
    const bytes = await readFile(`fixtures/${name}`);
    assert.equal(createHash('sha256').update(bytes).digest('hex'), sha, name);
  }
});

test('p6/quartz.pdf: provenance matches the bytes and the generator (print-edit D8)', async () => {
  const provenance = JSON.parse(await readFile('fixtures/p6/quartz.provenance.json', 'utf8'));
  const bytes = await readFile('fixtures/p6/quartz.pdf');
  assert.equal(provenance.file, 'fixtures/p6/quartz.pdf');
  assert.equal(provenance.bytes, bytes.length);
  assert.equal(provenance.sha256, createHash('sha256').update(bytes).digest('hex'));
  assert.equal(provenance.sha256, PINNED_SHA256['p6/quartz.pdf']);
  assert.equal(provenance.generator.sha256, createHash('sha256').update(await readFile(provenance.generator.path)).digest('hex'));
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  assert.equal(doc.countPages(), provenance.pages);
  doc.destroy();
});

test('every fixture renders a non-blank first page', async () => {
  for (const id of IDS) {
    const { doc } = await fixture(id);
    const page = doc.loadPage(0);
    const { count, total } = nonWhiteChannels(page);
    assert.ok(count > 100, `${id}: only ${count} non-white channels, page looks blank`);
    assert.ok(count < total / 2, `${id}: ${count}/${total} non-white channels, unexpected coverage`);
    page.destroy();
    doc.destroy();
  }
});

test('rotated.pdf: "PDF4All" is found three times with upright, 90deg and 45deg baselines', async () => {
  const { doc } = await fixture('rotated');
  const page = doc.loadPage(0);
  const found = hits(page, 'PDF4All');
  assert.equal(found.length, 3, 'known limitation: rotated text must yield one hit per run');

  const shapes = found.map((hit) => {
    // quad = [ul.x, ul.y, ur.x, ur.y, ll.x, ll.y, lr.x, lr.y]
    const quad = hit[0];
    const baseline = [quad[2] - quad[0], quad[3] - quad[1]];
    const box = bounds(hit);
    const rx = Math.abs(baseline[0]);
    const ry = Math.abs(baseline[1]);
    const shape = rx > 10 * ry ? 'upright' : ry > 10 * rx ? 'vertical' : 'diagonal';
    return { shape, baseline, box, aspect: box.width / box.height };
  });
  const upright = shapes.filter((s) => s.shape === 'upright');
  const vertical = shapes.filter((s) => s.shape === 'vertical');
  const diagonal = shapes.filter((s) => s.shape === 'diagonal');
  const summary = JSON.stringify(shapes);
  assert.equal(upright.length, 1, `expected one 0deg run, got ${summary}`);
  assert.equal(vertical.length, 1, `expected one 90deg run, got ${summary}`);
  assert.equal(diagonal.length, 1, `expected one 45deg run, got ${summary}`);

  assert.ok(upright[0].aspect > 2, `0deg hit bbox should be wide: ${summary}`);
  assert.ok(vertical[0].aspect < 0.5, `90deg hit bbox should be tall: ${summary}`);
  assert.ok(
    diagonal[0].aspect > 0.9 && diagonal[0].aspect < 1.1,
    `45deg hit bbox should be about square: ${summary}`,
  );
  assert.ok(
    Math.abs(Math.abs(diagonal[0].baseline[0]) - Math.abs(diagonal[0].baseline[1])) < 1,
    `45deg baseline should be at 45deg: ${summary}`,
  );
  console.log(JSON.stringify({ fixture: 'rotated', shapes }));
  page.destroy();
  doc.destroy();
});

test('cjk.pdf: bound, unbound searches and rendering of an embedded subset font', async () => {
  const { bytes, doc } = await fixture('cjk');
  const page = doc.loadPage(0);
  const text = page.toStructuredText();
  try {
    const body = text.asText();
    for (const needle of ['日本語テスト', '中文测试']) {
      assert.ok(body.includes(needle), `known limitation: asText() dropped ${needle}: ${JSON.stringify(body)}`);
      const found = text.search(needle);
      assert.equal(found.length, 1, `known limitation: search(${needle}) returned ${found.length} hits`);
      console.log(JSON.stringify({ fixture: 'cjk', needle, bbox: bounds(found[0]) }));
    }
  } finally {
    text.destroy();
  }
  // The whole file is a couple of kB: the font program really is a subset.
  assert.ok(bytes.length < 400_000, `cjk.pdf is ${bytes.length} bytes, expected a subset under 400 kB`);
  page.destroy();
  doc.destroy();
});

test('rtl.pdf: Hebrew and Arabic strings survive embedding and extraction', async () => {
  const { bytes, doc } = await fixture('rtl');
  const page = doc.loadPage(0);
  const found = { hebrew: hits(page, 'שלום'), arabic: hits(page, 'مرحبا'), marker: hits(page, 'PDF4All') };
  assert.equal(found.hebrew.length, 1, `known limitation: Hebrew search returned ${found.hebrew.length} hits`);
  assert.equal(found.arabic.length, 1, `known limitation: Arabic search returned ${found.arabic.length} hits`);
  assert.equal(found.marker.length, 3, 'the Latin marker must survive next to both RTL runs');
  console.log(JSON.stringify({
    fixture: 'rtl',
    hebrew: bounds(found.hebrew[0]),
    arabic: bounds(found.arabic[0]),
  }));

  const text = page.toStructuredText();
  try {
    const body = text.asText();
    // Visual-order glyphs in the file, logical order out of the extractor.
    assert.ok(body.includes('שלום'), `known limitation: Hebrew extraction returned ${JSON.stringify(body)}`);
    assert.ok(body.includes('مرحبا'), `known limitation: Arabic extraction returned ${JSON.stringify(body)}`);
  } finally {
    text.destroy();
  }
  assert.ok(bytes.length < 400_000, `rtl.pdf is ${bytes.length} bytes, expected subsets under 400 kB`);
  page.destroy();
  doc.destroy();
});

test('large.pdf: 500 pages, two "PDF4All" hits per sampled page, last page renders', async () => {
  const { doc } = await fixture('large');
  assert.equal(doc.countPages(), 500);
  for (const index of [0, 250, 499]) {
    const page = doc.loadPage(index);
    // header line + the paragraph opener
    assert.equal(hits(page, 'PDF4All').length, 2, `page ${index + 1}`);
    page.destroy();
  }
  const last = doc.loadPage(499);
  const text = last.toStructuredText();
  try {
    assert.ok(text.asText().includes('page 500 of 500'), 'last page must carry its page number');
  } finally {
    text.destroy();
  }
  const { count } = nonWhiteChannels(last);
  assert.ok(count > 100, `last page looks blank (${count} non-white channels)`);
  last.destroy();
  doc.destroy();
});
