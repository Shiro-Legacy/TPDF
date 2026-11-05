import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createEngine, inherited } from './rust-engine.mjs';

const engine = createEngine();
const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
async function session() {
  const s = engine.createSession();
  const pdf = await readFile('fixtures/hello.pdf');
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'hello.pdf', pdf: buffer(pdf) }, s).reply.ok, true);
  return s;
}
function request(s, req) { return engine.dispatch({ id: 2, ...req }, s).reply; }
const LATIN = 'latin1';
function stream(body) {
  const bytes = Buffer.from(body, LATIN);
  return Buffer.concat([Buffer.from(`<< /Length ${bytes.length} >>\nstream\n`, LATIN), bytes, Buffer.from('endstream', LATIN)]);
}
function rawPdf(objects) {
  const head = Buffer.from('%PDF-1.4\n%\xe2\xe3\xcf\xd3\n', LATIN), parts = [head], offsets = [];
  let size = head.length;
  for (const [i, object] of objects.entries()) {
    const body = Buffer.isBuffer(object) ? object : Buffer.from(object, LATIN);
    const prefix = Buffer.from(`${i + 1} 0 obj\n`, LATIN), suffix = Buffer.from('\nendobj\n', LATIN);
    offsets.push(size); parts.push(prefix, body, suffix); size += prefix.length + body.length + suffix.length;
  }
  let xref = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) xref += `${String(offset).padStart(10, '0')} 00000 n \n`;
  xref += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${size}\n%%EOF\n`;
  return Buffer.concat([...parts, Buffer.from(xref, LATIN)]);
}
function simplePagePdf(content, mediaBox = '[0 0 612 792]', font = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>') {
  return rawPdf(['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox ${mediaBox} /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>`,
    font, stream(content)]);
}

test('text-add writes WinAnsi page content in one undoable operation and creates an editable target', async () => {
  const s = await session();
  const reply = request(s, { op: 'text-add', page: 0, point: [72, 100], text: 'café — €', font: 'Helvetica-Bold', size: 12, color: [0, 0, 0] });
  assert.equal(reply.ok, true, reply.error);
  assert.equal(typeof reply.targetId, 'string');
  assert.equal(reply.dirty, true);
  const scan = request(s, { op: 'edit-targets', page: 0 });
  assert.ok(scan.items.some(item => item.targetId === reply.targetId && item.text === 'café — €'));
  assert.match(s.doc.text(0), /café — €/);
  const contents = s.doc.page(0).get('Contents');
  const stream = contents.get(contents.length - 1);
  const data = Buffer.from(stream.stream());
  assert.ok(data.includes(0xe9) && data.includes(0x97) && data.includes(0x80));
  assert.equal(request(s, { op: 'undo' }).ok, true);
  assert.equal(request(s, { op: 'edit-targets', page: 0 }).items.some(item => item.targetId === reply.targetId), false);
});

test('3pt off-baseline neighbors block growth and report the measured limit', async () => {
  const s = engine.createSession();
  const pdf = simplePagePdf('BT /F1 12 Tf 72 700 Td (A) Tj ET\nBT /F1 12 Tf 100 697 Td (B) Tj ET\n');
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'near.pdf', pdf: buffer(pdf) }, s).reply.ok, true);
  const scan = request(s, { op: 'edit-targets', page: 0 });
  const a = scan.items.find(item => item.text === 'A');
  const refused = request(s, { op: 'edit-text', page: 0, targetId: a.targetId, generation: scan.generation, revision: scan.revision, text: 'AAAAAAAAAAAA' });
  assert.equal(refused.detail, 'overflow');
  assert.ok(refused.oldWidth > 0 && refused.newWidth > refused.oldWidth && Number.isFinite(refused.limit));
  assert.equal(refused.dirty, false);
});

test('TJ target is editable as one Tj and page-margin growth refuses beyond the last text edge', async () => {
  const s = engine.createSession();
  const pdf = await readFile('fixtures/edit-tj.pdf');
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'edit-tj.pdf', pdf: buffer(pdf) }, s).reply.ok, true);
  const scan = request(s, { op: 'edit-targets', page: 0 });
  const tj = scan.items.find(item => item.text === 'KernedTEXThere');
  assert.equal(tj?.support, 'candidate');
  assert.equal(tj?.kerned, true);
  const edit = request(s, { op: 'edit-text', page: 0, targetId: tj.targetId, generation: scan.generation, revision: scan.revision, text: 'Kerned replacement' });
  assert.equal(edit.ok, true, edit.error);
  const fresh = request(s, { op: 'edit-targets', page: 0 });
  assert.ok(fresh.items.some(item => item.text === 'Kerned replacement' && item.kerned !== true));
  const margin = fresh.items.find(item => item.text === 'MARGIN');
  const overflow = request(s, { op: 'edit-text', page: 0, targetId: margin.targetId, generation: fresh.generation, revision: fresh.revision, text: 'MARGIN THAT CANNOT FIT ON THIS PAGE' });
  assert.equal(overflow.detail, 'overflow');
  assert.ok(Number.isFinite(overflow.limit));
});

test('added WinAnsi text remains editable, while a Differences font is not widened', async () => {
  const s = await session();
  const added = request(s, { op: 'text-add', page: 0, point: [72, 150], text: 'café', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.equal(added.ok, true, added.error);
  const targets = request(s, { op: 'edit-targets', page: 0 });
  const target = targets.items.find(item => item.targetId === added.targetId);
  const edited = request(s, { op: 'edit-text', page: 0, targetId: target.targetId, generation: targets.generation, revision: targets.revision, text: 'cafés — €' });
  assert.equal(edited.ok, true, edited.error); // `—`/`€` exercise the base-14 fallback (review fix)
  assert.match(s.doc.text(0), /cafés — €/);
  const differences = simplePagePdf('BT /F1 12 Tf 72 700 Td (cafe) Tj ET\n', '[0 0 612 792]', '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [32 /space] >> >>');
  const ds = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'differences.pdf', pdf: buffer(differences) }, ds).reply.ok, true);
  const difference = request(ds, { op: 'edit-targets', page: 0 }).items.find(item => item.kind === 'text');
  assert.equal(difference.reason, 'unsupported-encoding');
});

test('text-add handles multiline text with separate Tj lines and returns the first-line target', async () => {
  const s = await session();
  const reply = request(s, { op: 'text-add', page: 0, point: [72, 200], text: 'line one\nline two', font: 'Helvetica', size: 12, color: [0.3, 0, 0] });
  assert.equal(reply.ok, true, reply.error);
  const contents = s.doc.page(0).get('Contents');
  const stream = contents.get(contents.length - 1).stream();
  const raw = Buffer.from(stream).toString('latin1');
  assert.match(raw, /\(line one\) Tj 0 -14\.4 Td \(line two\) Tj/);
  assert.match(s.doc.text(0), /line one[\s\S]*line two/);
  const scan = request(s, { op: 'edit-targets', page: 0 });
  assert.ok(scan.items.some(item => item.targetId === reply.targetId && item.text === 'line one'));
  assert.ok(scan.items.some(item => item.text === 'line two'));
});

test('tail growth returns shift matching the moved tail bbox origin', async () => {
  const s = engine.createSession();
  const pdf = await readFile('fixtures/edit-tj.pdf');
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'edit-tj.pdf', pdf: buffer(pdf) }, s).reply.ok, true);
  const before = request(s, { op: 'edit-targets', page: 0 });
  const short = before.items.find(item => item.text === 'short' && item.tailFollows);
  const tailBefore = before.items.find(item => item.text === 'tail follows' && item.bounds);
  const edited = request(s, { op: 'edit-text', page: 0, targetId: short.targetId, generation: before.generation, revision: before.revision, text: 'a longer short' });
  assert.equal(edited.ok, true, edited.error);
  assert.ok(edited.shift > 0);
  const after = request(s, { op: 'edit-targets', page: 0 });
  const tailAfter = after.items.find(item => item.text === 'tail follows' && item.bounds);
  assert.ok(Math.abs((tailAfter.bounds[0] - tailBefore.bounds[0]) - edited.shift) < 0.1);
});

test('text-add maps nonzero MediaBox page points into the added target bounds', async () => {
  const s = engine.createSession();
  const pdf = await readFile('fixtures/edit-tj.pdf');
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'edit-tj.pdf', pdf: buffer(pdf) }, s).reply.ok, true);
  assert.deepEqual(inherited(s.doc.page(3), 'MediaBox').numbers(), [20, 30, 632, 822], 'the page box does not start at 0 0');
  // Page points start at the box's top-left corner, (0, 0), whatever the MediaBox origin.
  const point = [52, 100];
  const reply = request(s, { op: 'text-add', page: 3, point, text: 'origin check', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.equal(reply.ok, true, reply.error);
  const added = request(s, { op: 'edit-targets', page: 3 }).items.find(item => item.targetId === reply.targetId);
  assert.ok(added);
  assert.ok(Math.abs(added.bounds[0] - point[0]) < 0.1);
  assert.ok(Math.abs(added.bounds[3] - point[1]) < 5);
});

test('all twelve base-14 faces add as unembedded WinAnsi fonts', async () => {
  const s = await session();
  const names = ['Helvetica', 'Helvetica-Bold', 'Helvetica-Oblique', 'Helvetica-BoldOblique', 'Times-Roman', 'Times-Bold', 'Times-Italic', 'Times-BoldItalic', 'Courier', 'Courier-Bold', 'Courier-Oblique', 'Courier-BoldOblique'];
  for (let i = 0; i < names.length; i++) {
    const reply = request(s, { op: 'text-add', page: 0, point: [72, 100 + i * 45], text: `face${i}`, font: names[i], size: 12, color: [0, 0, 0] });
    assert.equal(reply.ok, true, `${names[i]}: ${reply.error}`);
  }
  const fonts = s.doc.page(0).get('Resources').get('Font');
  for (let i = 0; i < names.length; i++) {
    const font = fonts.get(`PDF4AllF${i + 1}`);
    assert.ok(font, names[i]);
    assert.equal(font.name('Encoding'), 'WinAnsiEncoding');
    assert.equal(font.name('BaseFont'), names[i]);
  }
});

test('undo preserves existing font bindings and a later font resource resolves correctly', async () => {
  const s = await session();
  const originalFontDict = s.doc.page(0).get('Resources').get('Font');
  const originalFonts = originalFontDict.keys.map(key => [key, String(originalFontDict.raw(key))]);
  const first = request(s, { op: 'text-add', page: 0, point: [72, 100], text: 'Helvetica', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.equal(first.ok, true, first.error);
  assert.equal(request(s, { op: 'undo' }).ok, true);
  const second = request(s, { op: 'text-add', page: 0, point: [72, 130], text: 'Times glyphs', font: 'Times-Bold', size: 12, color: [0, 0, 0] });
  assert.equal(second.ok, true, second.error);
  const fonts = s.doc.page(0).get('Resources').get('Font');
  const current = [];
  for (const key of fonts.keys) current.push([key, String(fonts.raw(key))]);
  for (const [key, value] of originalFonts) assert.equal(current.find(item => item[0] === key)?.[1], value, `preexisting /${key} binding changed`);
  const stream = s.doc.page(0).get('Contents').get(1).stream();
  const fontName = /\/(PDF4AllF\d+)/.exec(Buffer.from(stream).toString('latin1'))?.[1];
  assert.ok(fontName);
  assert.equal(fonts.get(fontName).name('BaseFont'), 'Times-Bold');
  assert.match(s.doc.text(0), /Times glyphs/);
});

test('shared /Contents array is privatized without changing its sibling after save', async () => {
  const pdf = rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents 5 0 R >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 7 0 R >> >> /Contents 5 0 R >>',
    '[6 0 R]', stream('BT /F1 12 Tf 72 700 Td (SHARED) Tj ET\n'),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
  ]);
  const s = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'shared-array.pdf', pdf: buffer(pdf) }, s).reply.ok, true);
  const before = Buffer.from(s.doc.page(1).get('Contents').get(0).stream());
  const added = request(s, { op: 'text-add', page: 0, point: [72, 100], text: 'private', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.equal(added.ok, true, added.error);
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const reopened = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'saved.pdf', pdf: saved.bytes }, reopened).reply.ok, true);
  const p0 = reopened.doc.page(0).get('Contents');
  const p1 = reopened.doc.page(1).get('Contents');
  assert.equal(p0.length, 2);
  assert.equal(p1.length, 1);
  assert.deepEqual(Buffer.from(p1.get(0).stream()), before);

});

test('text-add rejects non-WinAnsi, rotated, and unbalanced pages without dirtying', async () => {
  const s = await session();
  const before = request(s, { op: 'edit-targets', page: 0 }).revision;
  const unsupported = request(s, { op: 'text-add', page: 0, point: [72, 100], text: '漢', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.equal(unsupported.detail, 'unsupported-encoding');
  assert.deepEqual(unsupported.missing, ['漢']);
  assert.equal(request(s, { op: 'edit-targets', page: 0 }).revision, before);
  const tj = await readFile('fixtures/edit-tj.pdf');
  assert.equal(engine.dispatch({ id: 3, op: 'open', name: 'edit-tj.pdf', pdf: buffer(tj) }, s).reply.ok, true);
  const rotated = request(s, { op: 'text-add', page: 1, point: [72, 100], text: 'x', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.equal(rotated.reason, 'unsupported-transform');
  const beforeUnbalanced = request(s, { op: 'edit-targets', page: 2 }).revision;
  const unbalanced = request(s, { op: 'text-add', page: 2, point: [72, 100], text: 'x', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.equal(unbalanced.reason, 'unbalanced-content');
  assert.equal(unbalanced.dirty, false);
  assert.equal(request(s, { op: 'edit-targets', page: 2 }).revision, beforeUnbalanced);
});

test('text-add pins text state, so a prior element\'s top-level Tc/Tw/Tz does not reach the added text (review fix)', async () => {
  const plain = await session();
  const reference = request(plain, { op: 'text-add', page: 0, point: [72, 700], text: 'Plain words', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  const s = engine.createSession();
  const pdf = await readFile('fixtures/edit-tj.pdf');
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'edit-tj.pdf', pdf: buffer(pdf) }, s).reply.ok, true);
  const added = request(s, { op: 'text-add', page: 0, point: [72, 700], text: 'Plain words', font: 'Helvetica', size: 12, color: [0, 0, 0] });
  assert.equal(added.ok, true, added.error);
  const width = bounds => bounds[2] - bounds[0];
  assert.ok(Math.abs(width(added.bounds) - width(reference.bounds)) < 0.1, `${width(added.bounds)} vs ${width(reference.bounds)}`);
});
