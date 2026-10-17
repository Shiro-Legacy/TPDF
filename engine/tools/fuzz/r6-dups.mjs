import { mu, rs, rsR2, rawPdf, stream, broken, view, fmt } from './lib2.mjs';
const page = c => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 9 0 R >> >> /Contents ${c} 0 R >>`;
const C = t => stream(`BT /F1 24 Tf 72 700 Td (${t}) Tj ET`);
const objs = ['<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>', page(6), page(7), page(8), C('Alpha'), C('Beta'), C('Gamma'), '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'];
const b0 = rawPdf(objs).toString('latin1');
const xr = b0.lastIndexOf('xref');
const cases = {
  dupGen1Later: b0.slice(0, xr) + '4 1 obj\n<< /Foo 1 >>\nendobj\n' + b0.slice(xr),
  dupGen0Later: b0.slice(0, xr) + '4 0 obj\n<< /Foo 1 >>\nendobj\n' + b0.slice(xr),
  dupGen1Earlier: b0.replace('4 0 obj', '4 1 obj\n<< /Foo 1 >>\nendobj\n4 0 obj'),
};
const midObj = (() => { const s = b0; const o4 = s.indexOf('4 0 obj'); const at = s.indexOf('<<', o4); const line = xr + 'xref\n0 10\n'.length + 20 * 4; return s.slice(0, line) + `${String(at).padStart(10, '0')} 00000 n \n` + s.slice(line + 20); })();
cases.xrefMidObject = midObj;
for (const [name, s] of Object.entries(cases)) {
  const bytes = Buffer.from(s, 'latin1');
  for (const [lab, b] of [['asis', bytes], ['broken', broken(bytes)]]) {
    const row = [];
    for (const [n, e] of [['MU', mu], ['RS', rs], ['R2', rsR2]]) {
      const v = view(e, b, { save: n !== 'MU' });
      let r = `${n} ${fmt(v)}`;
      if (v.saved) { const m = view(mu, v.saved); r += ` [save->MU p=${m.pages} ${m.text}]`; }
      row.push(r);
    }
    console.log(`${name} ${lab}\n   ${row.join('\n   ')}`);
  }
}
