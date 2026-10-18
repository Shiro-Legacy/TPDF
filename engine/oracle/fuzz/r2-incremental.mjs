// Incremental updates with damaged later sections; which trailer wins.
import { mupdf, mu, rs, rsR2, rawPdf, stream, call, ab, view, fmt } from './lib2.mjs';
const T = b => { try { const d = mupdf.Document.openDocument(Buffer.from(b), 'application/pdf'); return d.getMetaData('info:Title'); } catch (e) { return 'ERR'; } };
const page = (c, extra = '') => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 9 0 R >> >> /Contents ${c} 0 R ${extra}>>`;
const C = t => stream(`BT /F1 24 Tf 72 700 Td (${t}) Tj ET`);
const base = rawPdf([
  '<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>',
  page(6), page(7), page(8), C('Alpha'), C('Beta'), C('Gamma'),
  '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>', '<< /Title (Old) >>',
], '/Info 10 0 R ');
const baseXref = base.toString('latin1').lastIndexOf('xref\n0 ');
function update({ obj4 = page(7, '/Annots [12 0 R] '), glue = false, xrefkw = 'xref', trailerkw = 'trailer', off4 = 0, cut = false, junk = '', badStart = false, prev = baseXref } = {}) {
  let s = base.toString('latin1');
  const o4 = s.length; s += `4 0 obj\n${obj4}\nendobj\n`;
  const o12 = s.length + (glue ? -1 : 0);
  if (glue) s = s.slice(0, -1); // drop the newline after endobj: "endobj12 0 obj"
  s += `12 0 obj\n<< /Type /Annot /Subtype /Text /Rect [100 100 120 120] /Contents (Note) /P 4 0 R >>\nendobj\n`;
  const o11 = s.length; s += `11 0 obj\n<< /Title (New) >>\nendobj\n`;
  const x = s.length;
  const e = n => `${String(n).padStart(10, '0')} 00000 n \n`;
  s += `${xrefkw}\n0 1\n0000000000 65535 f \n4 1\n${e(o4 + off4)}11 2\n${e(o11)}${e(o12)}`;
  if (cut) return Buffer.from(s, 'latin1');
  s += `${trailerkw}\n<< /Size 13 /Root 1 0 R /Info 11 0 R /Prev ${prev} >>\nstartxref\n${badStart ? 999999 : x}\n%%EOF\n${junk}`;
  return Buffer.from(s, 'latin1');
}
const cases = {
  ok: {}, xrefkw: { xrefkw: 'xreg' }, trailerkw: { trailerkw: '8railer' }, badStart: { badStart: true },
  off4plus1: { off4: 1 }, off4toOld: { off4: -999999 }, cut: { cut: true }, glued: { glue: true },
  obj4NoClose: { obj4: page(7, '/Annots [12 0 R] ').slice(0, -2) }, obj4NoCloseBroken: { obj4: page(7, '/Annots [12 0 R] ').slice(0, -2), badStart: true },
  junkAfterEOF: { junk: '<< /Root 9 0 R /Info 10 0 R >>\n', badStart: true },
  junkAfterEOFintact: { junk: '<< /Root 9 0 R /Info 10 0 R >>\n' },
  trailerkwJunk: { trailerkw: '8railer', junk: '<< /Info 10 0 R >>\n' },
  badPrev: { prev: 7 },
};
for (const [name, opt] of Object.entries(cases)) {
  let bytes = update(opt);
  if (name === 'off4toOld') { // point the update's entry for 4 at the base revision's object 4
    const s = bytes.toString('latin1'); const old = s.indexOf('4 0 obj'); const ix = s.lastIndexOf('4 1\n');
    bytes = Buffer.from(s.slice(0, ix + 4) + `${String(old).padStart(10, '0')} 00000 n \n` + s.slice(ix + 4 + 20), 'latin1');
  }
  const row = [];
  for (const [n, e] of [['MU', mu], ['RS', rs], ['R2', rsR2]]) {
    if (!e) continue;
    const v = view(e, bytes, { save: n !== 'MU' });
    const title = n === 'MU' ? T(bytes) : '';
    let s = `${n} ${fmt(v)} title=${JSON.stringify(title)}`;
    if (v.saved) { const m = view(mu, v.saved); s += ` [save->MU p=${m.pages} an=${m.annots} ${m.text} title=${JSON.stringify(T(v.saved))}]`; }
    row.push(s);
  }
  console.log(`${name}\n   ${row.join('\n   ')}`);
}
