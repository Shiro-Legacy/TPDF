// P6 U4 adversarial review, ENGINE + FIXTURES lens. Throwaway probe; run from the repo root
// under the lock after `npm run build`. Reads fixtures, never writes them.
import { readFile } from 'node:fs/promises';
import mupdf from 'mupdf';
import { createEngine, OP_PERMISSION } from '../../../../build/renderer/engine.js';

const engine = createEngine(mupdf);
const out = [];
const log = (...a) => { out.push(a.map(v => typeof v === 'string' ? v : JSON.stringify(v)).join(' ')); };
const ab = b => b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);
function buildPdf(objects) {
  const parts = ['%PDF-1.4\n']; const offsets = [];
  objects.forEach((body, i) => { offsets.push(parts.join('').length); parts.push(`${i + 1} 0 obj\n${body}\nendobj\n`); });
  const xref = parts.join('').length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const o of offsets) parts.push(`${String(o).padStart(10, '0')} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  return ab(Buffer.from(parts.join(''), 'latin1'));
}
function openBytes(bytes) {
  const s = engine.createSession();
  const r = engine.dispatch({ id: 1, op: 'open', name: 'x.pdf', pdf: bytes }, s).reply;
  if (!r.ok) throw new Error(r.error);
  return s;
}
const op = (s, req) => engine.dispatch({ id: 9, ...req }, s).reply;
const links = (s, p) => op(s, { op: 'links', page: p }).items;
function rt(s) { const b = op(s, { op: 'save' }).bytes; engine.closeDocument(s.doc); return openBytes(b); }
function rawAnnots(s, p) {
  const pg = s.doc.asPDF().findPage(p); const a = pg.get('Annots');
  const res = []; for (let i = 0; i < a.length; i++) res.push(a.get(i).resolve().toString());
  return res;
}
const near = (a, b) => a.length === b.length && a.every((v, i) => Math.abs(v - b[i]) <= 0.01);
const check = (name, ok, extra) => log(ok ? 'PASS' : 'FAIL', name, extra ?? '');

// 1. two consecutive link-set rect calls: the second sees the first (no stale handle)
{
  let s = openBytes(ab(await readFile('fixtures/p6/links.pdf')));
  const a = op(s, { op: 'link-set', page: 0, annotId: 10, rect: [80, 300, 200, 320] });
  const b = op(s, { op: 'link-set', page: 0, annotId: 10, rect: [80, 300, 200, 320] });
  check('second identical link-set is noop (sees the first write)', a.ok && b.ok && b.noop === true && b.revision === 1, [a.revision, b.noop, b.revision]);
  const c = op(s, { op: 'link-set', page: 1, annotId: 14, rect: [600, 100, 640, 300] });
  const d = op(s, { op: 'link-set', page: 1, annotId: 14, rect: [600, 100, 640, 300] });
  check('rotated page: second identical is noop', c.ok && d.noop === true, [c.error, d.noop]);
  s = rt(s);
  check('p0 + p1 rects after reopen', near(links(s, 0)[0].rect, [80, 300, 200, 320]) && near(links(s, 1)[0].rect, [600, 100, 640, 300]), [links(s, 0)[0].rect, links(s, 1)[0].rect]);
  engine.closeDocument(s.doc);
}
// 2. refused target with a valid rect: no partial write
{
  const s = openBytes(ab(await readFile('fixtures/p6/links.pdf')));
  const before = rawAnnots(s, 0);
  for (const t of [{ uri: 'javascript:alert(1)' }, { uri: 'file:///etc/hosts' }, { page: 9 }, { uri: 'x'.repeat(10) }, { uri: 'https://' + 'x'.repeat(2041) }]) {
    const r = op(s, { op: 'link-set', page: 0, annotId: 11, rect: [80, 300, 200, 320], target: t });
    check(`refused ${JSON.stringify(t).slice(0, 30)} dirty false rev 0`, !r.ok && r.dirty === false && r.revision === 0, [r.detail, r.error]);
  }
  check('raw /Annots unchanged after refusals', JSON.stringify(rawAnnots(s, 0)) === JSON.stringify(before));
  check('undo refused (nothing journaled)', op(s, { op: 'undo' }).ok === false);
  // whitespace-padded uri
  const w = op(s, { op: 'link-set', page: 0, annotId: 10, target: { uri: '  https://example.org/pad ' } });
  log('INFO padded uri reply', w.ok, w.detail ?? '', 'stored:', links(s, 0)[0].uri);
  engine.closeDocument(s.doc);
}
// 3. /A /GoTo link retarget, plus /Dest + /A both present; interleaved widgets; offset MediaBox; /Rotate 270
{
  const bytes = buildPdf([
    '<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R 7 0 R] >> >>',
    '<< /Type /Pages /Kids [3 0 R 10 0 R 12 0 R] /Count 3 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R 5 0 R 6 0 R 7 0 R 8 0 R] >>',
    '<< /Type /Annot /Subtype /Link /Rect [72 700 200 720] /A << /S /GoTo /D [10 0 R /Fit] >> >>',
    '<< /Type /Annot /Subtype /Widget /FT /Tx /T (w1) /Rect [72 600 200 620] /F 4 >>',
    '<< /Type /Annot /Subtype /Link /Rect [72 650 200 670] /Dest [12 0 R /Fit] /A << /S /URI /URI (https://example.org/shadow) >> >>',
    '<< /Type /Annot /Subtype /Widget /FT /Tx /T (w2) /Rect [72 500 200 520] /F 4 >>',
    '<< /Type /Annot /Subtype /Link /Rect [72 550 200 570] /A << /S /URI /URI (https://example.org/c) >> >>',
    '<< >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [100 100 712 892] /Annots [11 0 R] >>',
    '<< /Type /Annot /Subtype /Link /Rect [172 800 300 820] /A << /S /URI /URI (https://example.org/off) >> >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 270 /Annots [13 0 R] >>',
    '<< /Type /Annot /Subtype /Link /Rect [72 700 200 720] /A << /S /URI /URI (https://example.org/r270) >> >>',
  ]);
  let s = openBytes(bytes);
  const l0 = links(s, 0);
  log('INFO interleaved links', l0.map(i => [i.id, i.annotId, i.kind, i.page ?? i.uri]));
  check('interleaved widgets: annotIds [4,6,8]', JSON.stringify(l0.map(i => i.annotId)) === '[4,6,8]');
  check('widget obj 5 -> not-link', op(s, { op: 'link-set', page: 0, annotId: 5, rect: [80, 80, 90, 90] }).detail === 'not-link');
  check('non-annot obj 9 (in no /Annots) -> no-such-annot', op(s, { op: 'link-set', page: 0, annotId: 9, rect: [80, 80, 90, 90] }).detail === 'no-such-annot');
  const g = op(s, { op: 'link-set', page: 0, annotId: 4, target: { uri: 'https://example.org/was-goto' } });
  const dshadow = op(s, { op: 'link-set', page: 0, annotId: 6, target: { uri: 'https://example.org/unshadowed' } });
  log('INFO /Dest+/A shadow link listed as', l0[1].kind, l0[1].page ?? l0[1].uri, 'set ->', dshadow.ok, dshadow.noop ?? '', dshadow.error ?? '');
  const pg = op(s, { op: 'link-set', page: 0, annotId: 8, target: { page: 2 } });
  const off = op(s, { op: 'link-set', page: 1, annotId: 11, rect: [100, 100, 250, 130] });
  const before270 = links(s, 2)[0].rect;
  const r270 = op(s, { op: 'link-set', page: 2, annotId: 13, rect: [100, 300, 140, 450] });
  log('INFO replies', [g.ok, g.error], [pg.ok, pg.error], [off.ok, off.error], [r270.ok, r270.error], 'p1 before', links(s, 1)[0].rect, 'p2 before', before270);
  s = rt(s);
  const a = links(s, 0);
  check('GoTo -> uri survives reopen', a[0].kind === 'uri' && a[0].uri === 'https://example.org/was-goto', a[0]);
  check('/Dest+/A -> uri survives reopen', a[1].kind === 'uri' && a[1].uri === 'https://example.org/unshadowed', a[1]);
  check('uri -> page 2 survives reopen', a[2].kind === 'page' && a[2].page === 2, a[2]);
  check('annotIds stable after writes', JSON.stringify(a.map(i => i.annotId)) === '[4,6,8]');
  check('offset MediaBox rect round-trips', near(links(s, 1)[0].rect, [100, 100, 250, 130]), links(s, 1)[0].rect);
  check('/Rotate 270 rect round-trips', near(links(s, 2)[0].rect, [100, 300, 140, 450]), links(s, 2)[0].rect);
  const fields = op(s, { op: 'fields', page: 0 });
  check('widgets untouched', fields.items.length === 2, fields.items.map(f => [f.name, f.rect]));
  // link-add then link-set the new link by its annotId
  const add = op(s, { op: 'link-add', page: 0, rect: [300, 300, 400, 330], target: { uri: 'https://example.org/new' } });
  const nl = links(s, 0).find(i => i.uri === 'https://example.org/new');
  const setNew = op(s, { op: 'link-set', page: 0, annotId: nl.annotId, rect: [310, 310, 410, 340] });
  check('link-add -> link-set by new annotId', add.ok && nl.annotId > 0 && setNew.ok, [nl.annotId, setNew.error]);
  engine.closeDocument(s.doc);
}
// 4. undo reads back from a fresh page; redo
{
  const s = openBytes(ab(await readFile('fixtures/p6/links.pdf')));
  op(s, { op: 'link-set', page: 1, annotId: 14, rect: [600, 100, 640, 300], target: { page: 0 } });
  op(s, { op: 'undo' });
  const u = links(s, 1)[0];
  check('rotated undo restores rect + target', near(u.rect, [700, 72, 720, 200]) && u.uri === 'https://example.org/rot', u);
  engine.closeDocument(s.doc);
}
// 5. /Tabs names and structIndex bounds
function tabsPdf(tabs, kExpr, extra = []) {
  return buildPdf([
    `<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 5 0 R] >> /StructTreeRoot 6 0 R >>`,
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] ${tabs} /Annots [4 0 R 5 0 R] >>`,
    '<< /Type /Annot /Subtype /Widget /FT /Tx /T (f1) /Rect [72 700 192 720] /F 4 >>',
    '<< /Type /Annot /Subtype /Widget /FT /Tx /T (f2) /Rect [220 700 340 720] /F 4 >>',
    `<< /Type /StructTreeRoot /K ${kExpr} >>`,
    ...extra,
  ]);
}
const idx = s => op(s, { op: 'fields', page: 0 });
for (const [tabs, want] of [['/Tabs /R', 'R'], ['/Tabs /W', 'W'], ['/Tabs /X', null], ['/Tabs (S)', null], ['', null]]) {
  const s = openBytes(tabsPdf(tabs, '[]'));
  check(`tabs ${tabs || '(absent)'} -> ${want}`, idx(s).tabs === want, idx(s).tabs);
  engine.closeDocument(s.doc);
}
const objr = n => `<< /Type /OBJR /Obj ${n} 0 R >>`;
function nested(depth) { let k = `[${objr(5)} ${objr(4)}]`; for (let i = 0; i < depth; i++) k = `<< /S /Div /K ${k} >>`; return k; }
for (const depth of [20, 30, 31, 32, 62, 63, 64, 70]) {
  const s = openBytes(tabsPdf('/Tabs /S', nested(depth)));
  const r = idx(s);
  log('INFO nested-dict depth', depth, r.items.map(i => [i.name, i.structIndex]));
  engine.closeDocument(s.doc);
}
function nestedArr(depth) { let k = `[${objr(5)} ${objr(4)}]`; for (let i = 0; i < depth; i++) k = `[<< /S /Div /K ${k} >>]`; return k; }
for (const depth of [20, 31, 32, 40]) {
  const s = openBytes(tabsPdf('/Tabs /S', nestedArr(depth)));
  log('INFO nested dict-in-array depth', depth, idx(s).items.map(i => [i.name, i.structIndex]));
  engine.closeDocument(s.doc);
}
{ // 50 000 nodes: integers (MCIDs) then the OBJRs
  const ints = n => Array.from({ length: n }, (_, i) => i % 10).join(' ');
  for (const n of [49990, 49997, 49998, 50000]) {
    const s = openBytes(tabsPdf('/Tabs /S', `[${ints(n)} ${objr(5)} ${objr(4)}]`));
    log('INFO', n, 'MCIDs before OBJRs', idx(s).items.map(i => [i.name, i.structIndex]));
    engine.closeDocument(s.doc);
  }
}
{ // cycle through /K, /P not followed
  const s = openBytes(tabsPdf('/Tabs /S', '[7 0 R]', [`<< /S /Div /P 6 0 R /K [7 0 R ${objr(5)} << /S /P /K ${objr(4)} >>] >>`]));
  const r = idx(s);
  check('cyclic /K terminates, order f2 f1', r.items.find(i => i.name === 'f2')?.structIndex === 0 && r.items.find(i => i.name === 'f1')?.structIndex === 1, r.items.map(i => [i.name, i.structIndex]));
  engine.closeDocument(s.doc);
}
log('INFO OP_PERMISSION link-set', OP_PERMISSION['link-set']);
console.log(out.join('\n'));
