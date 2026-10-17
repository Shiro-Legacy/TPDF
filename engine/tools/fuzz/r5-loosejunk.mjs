// Loose dictionaries in gaps of a file whose xref is intact: do they override the xref's trailer?
import { mupdf, mu, rs, rsR2, rawPdf, stream, call, ab, view, fmt, fixture } from './lib2.mjs';
const T = b => { try { return mupdf.Document.openDocument(Buffer.from(b), 'application/pdf').getMetaData('info:Title'); } catch (e) { return 'ERR'; } };
const page = c => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 9 0 R >> >> /Contents ${c} 0 R >>`;
const C = t => stream(`BT /F1 24 Tf 72 700 Td (${t}) Tj ET`);
const base = rawPdf([
  '<< /Type /Catalog /Pages 2 0 R >>', '<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>', page(5), page(6),
  C('Alpha'), C('Beta'), '<< /Title (Wrong) >>', '<< /Type /Catalog /Pages 8 0 R >>', '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  '<< /Title (Right) >>', '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
], '/Info 10 0 R ');
// objects: 7 = wrong Info, 8 = another catalog (1 page) -> wait 8 is catalog naming 8? fix below
const s0 = base.toString('latin1');
const enc = '<< /Filter /Standard /V 1 /R 2 /Length 40 /P -4 /O <' + '28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A' + '> /U <' + '00'.repeat(32) + '> >>';
const junks = {
  none: '',
  foo: '<< /Foo 1 >>\n',
  info: '<< /Info 7 0 R >>\n',
  infoKw: 'trailer\n<< /Info 7 0 R >>\n',
  root: '<< /Root 8 0 R >>\n',
  encrypt: `<< /Encrypt ${enc} /ID [<01><01>] >>\n`,
};
for (const [name, junk] of Object.entries(junks)) {
  for (const where of ['afterEOF', 'betweenObjects']) {
    let s;
    if (where === 'afterEOF') s = s0 + junk;
    else { const at = s0.indexOf('endobj', s0.indexOf('7 0 obj')) + 7; s = s0.slice(0, at) + junk + s0.slice(at); }
    let bytes = Buffer.from(s, 'latin1');
    if (where === 'betweenObjects') { // keep xref offsets intact: shift entries after the insertion
      const at = s0.indexOf('endobj', s0.indexOf('7 0 obj')) + 7, d = junk.length;
      let t = s.replace(/(\d{10}) 00000 n /g, (m, o) => `${String(+o >= at ? +o + d : +o).padStart(10, '0')} 00000 n `);
      t = t.replace(/startxref\n(\d+)/, (m, o) => `startxref\n${+o + d}`);
      bytes = Buffer.from(t, 'latin1');
    }
    const row = [];
    for (const [n, e] of [['MU', mu], ['RS', rs], ['R2', rsR2]]) {
      const v = view(e, bytes, { save: n !== 'MU' });
      let r = `${n} ${fmt(v)} needs=${v.needs} title=${n === 'MU' ? T(bytes) : ''}`;
      if (v.saved) { const m = view(mu, v.saved); r += ` [save->MU p=${m.pages} ${m.text} title=${T(v.saved)}]`; }
      row.push(r);
    }
    console.log(`${name} ${where}\n   ${row.join('\n   ')}`);
  }
}
