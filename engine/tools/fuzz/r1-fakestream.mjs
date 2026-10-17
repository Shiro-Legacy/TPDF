// Fake object headers inside stream data; damaged stream shapes; intact / broken xref.
import { mu, rs, rsR2, rawPdf, broken, view, fmt } from './lib2.mjs';
const L = s => Buffer.byteLength(s, 'latin1');
const page = (c, mb = '0 0 612 792') => `<< /Type /Page /Parent 2 0 R /MediaBox [${mb}] /Resources << /Font << /F1 9 0 R >> >> /Contents ${c} 0 R >>`;
const cs = t => { const b = `BT /F1 24 Tf 72 700 Td (${t}) Tj ET`; return `<< /Length ${L(b)} >>\nstream\n${b}\nendstream`; };
const FAKE = `\n4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 50 50] /Contents 6 0 R >>\nendobj\n9 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\ntrailer\n<< /Root 1 0 R /Info 3 0 R >>\n<< /Root 4 0 R >>\n`;
const S = {
  ok: `<< /Length ${L(FAKE)} >>\nstream\n${FAKE}\nendstream`,
  shortlen: `<< /Length 5 >>\nstream\n${FAKE}\nendstream`,
  longlen: `<< /Length ${L(FAKE) + 400} >>\nstream\n${FAKE}\nendstream`,
  reflen: `<< /Length 99 0 R >>\nstream\n${FAKE}\nendstream`,
  nolen: `<< >>\nstream\n${FAKE}\nendstream`,
  badendstream: `<< /Length ${L(FAKE)} >>\nstream\n${FAKE}\nendstrean`,
  badstreamkw: `<< /Length ${L(FAKE)} >>\nstrean\n${FAKE}\nendstream`,
  dictbroken: `<< /Length ${L(FAKE)} \nstream\n${FAKE}\nendstream`,
};
function build(s10, force) {
  const objs = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>',
    page(6), page(7, '0 0 595 842'), page(8, '0 0 400 400'),
    cs('Alpha'), cs('Beta'), cs('Gamma'),
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
    s10,
    force ? '<< /Title (x) /Foo 1\n' : '<< /Title (x) >>',
  ];
  return rawPdf(objs, '/Info 11 0 R ', { order: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10], raw: force ? { 11: '<< /Title (x) /Foo 1\n' } : {} });
}
for (const [name, s10] of Object.entries(S)) {
  for (const force of [false, true]) {
    for (const [lab, bytes] of [['intact', build(s10, force)], ['broken', broken(build(s10, force))]]) {
      const row = [];
      for (const [en, e] of [['MU', mu], ['RS', rs], ['R2', rsR2]]) {
        if (!e) continue;
        const v = view(e, bytes, { save: en !== 'MU' });
        let s = `${en} ${fmt(v)}`;
        if (v.saved) { const m = view(mu, v.saved); s += ` [save->MU p=${m.pages} ${m.text}]`; }
        row.push(s);
      }
      console.log(`${name.padEnd(13)} force=${force ? 1 : 0} ${lab}\n   ${row.join('\n   ')}`);
    }
  }
}
