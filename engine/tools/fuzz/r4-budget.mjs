// (a) Loose-dictionary failures share the scan's reparse budget: can junk between
// objects starve a later reread round 2 made? (b) timing of loose dicts / rereads.
import { mu, rs, rsR2, rawPdf, stream, broken, view, fmt, call, ab } from './lib2.mjs';
const page = (c, extra = '') => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 9 0 R >> >> /Contents ${c} 0 R ${extra}>>`;
const C = t => stream(`BT /F1 24 Tf 72 700 Td (${t}) Tj ET`);
function build(junkBytes, markers, pageExtra) {
  const filler = 'x'.repeat(Math.floor(junkBytes / markers));
  const junk = Array.from({ length: markers }, () => `<< /A (${filler}`).join('');
  const objs = [
    '<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>',
    page(6), page(7, pageExtra), page(8), C('Alpha'), C('Beta'), C('Gamma'),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>', `<< /X 1 >>\nendobj\n${junk}\n`,
  ];
  // object 10 (with junk after it) written before page 4
  return rawPdf(objs, '', { order: [0, 1, 9, 2, 3, 4, 5, 6, 7, 8], raw: { 10: `<< /X 1 >>\nendobj\n${junk}\n` } });
}
const extras = {
  strTrailer: '/PieceInfo (a\ntrailer\nb) ',
  strHeader: '/PieceInfo (a\n12 0 obj\nb) ',
};
for (const [en, extra] of Object.entries(extras)) {
  for (const [junkBytes, markers] of [[0, 1], [2000, 4], [20000, 8]]) {
    for (const [lab, bytes] of [['intact', build(junkBytes, markers, extra)], ['broken', broken(build(junkBytes, markers, extra))]]) {
      const row = [];
      for (const [n, e] of [['MU', mu], ['RS', rs], ['R2', rsR2]]) {
        if (!e) continue;
        const v = view(e, bytes, { save: n !== 'MU' });
        let s = `${n} ${fmt(v)}`;
        if (v.saved) { const m = view(mu, v.saved); s += ` [save->MU p=${m.pages} ${m.text}]`; }
        row.push(s);
      }
      console.log(`${en} junk=${junkBytes}x${markers} ${lab} size=${bytes.length}\n   ${row.join('\n   ')}`);
    }
  }
}
// timing: n KB of unterminated dictionaries / nested `<<` in gaps; overlapping headers in strings with an xref
const time = (label, bytes) => {
  for (const [n, e] of [['RS', rs], ['R2', rsR2]]) {
    const s = e.createSession(); const t0 = performance.now();
    const o = call(e, s, { op: 'open', name: 'x.pdf', pdf: ab(bytes) });
    console.log(`time ${label} ${n} ${(performance.now() - t0).toFixed(0)} ms ok=${o.ok} p=${o.pageCount ?? o.error}`);
  }
};
for (const kb of [64, 256, 1024]) {
  const n = kb * 1024;
  const base = rawPdf(['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>', page(4), C('Alpha'), '<< /X 1 >>']);
  const s = base.toString('latin1');
  const at = s.indexOf('endobj', s.indexOf('5 0 obj')) + 6;
  // gap junk shapes
  const shapes = {
    nested: '<< /A '.repeat(n / 6),
    openarr: '<< /A ['.repeat(n / 7),
    manyDicts: '<< /Root 1 0 R >>'.repeat(n / 17),
    lt: '<'.repeat(n),
  };
  for (const [k, junk] of Object.entries(shapes)) {
    time(`${k}-${kb}KB`, broken(Buffer.from(s.slice(0, at) + '\n' + junk + '\n' + s.slice(at), 'latin1')));
  }
  // overlapping objects: headers inside strings, each pointed at by the xref (read_at path)
  let body = '%PDF-1.4\n'; const offs = []; const m = Math.floor(n / 40);
  body += '1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n';
  for (let i = 0; i < m; i++) { offs.push(body.length); body += `${i + 10} 0 obj\n<< /S (`; }
  body += ') >>\nendobj\n';
  const x = body.length;
  body += `xref\n0 3\n0000000000 65535 f \n0000000009 00000 n \n${String(body.indexOf('2 0 obj')).padStart(10, '0')} 00000 n \n10 ${m}\n` + offs.map(o => `${String(o).padStart(10, '0')} 00000 n \n`).join('');
  body += `trailer\n<< /Size ${m + 10} /Root 1 0 R >>\nstartxref\n${x}\n%%EOF\n`;
  time(`nestedStrings-${kb}KB m=${m}`, Buffer.from(body, 'latin1'));
}
