// Untyped page-tree nodes under every page edit; MuPDF reads each save.
import { mu, rs, rsR2, rawPdf, stream, call, ab, view, fmt, fixture } from './lib2.mjs';
const page = (c, parent) => `<< /Type /Page /Parent ${parent} 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 9 0 R >> >> /Contents ${c} 0 R >>`;
const C = t => stream(`BT /F1 24 Tf 20 200 Td (${t}) Tj ET`);
const F = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>';
// pages: 3 Alpha 4 Beta (under node 10), 5 Gamma under root 2; 11 Delta under nested 12 in 10 (some shapes)
const shapes = {
  untyped: [`<< /Type /Pages /Kids [10 0 R 5 0 R] /Count 3 >>`, `<< /Parent 2 0 R /Kids [3 0 R 4 0 R] /Count 2 >>`],
  untypedMB: [`<< /Type /Pages /Kids [10 0 R 5 0 R] /Count 3 >>`, `<< /Parent 2 0 R /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>`],
  typeNull: [`<< /Type /Pages /Kids [10 0 R 5 0 R] /Count 3 >>`, `<< /Type null /Parent 2 0 R /Kids [3 0 R 4 0 R] /Count 2 >>`],
  kidsRef: [`<< /Type /Pages /Kids [10 0 R 5 0 R] /Count 3 >>`, `<< /Parent 2 0 R /Kids 13 0 R /Count 2 >>`],
  untypedRoot: [`<< /Kids [10 0 R 5 0 R] /Count 3 >>`, `<< /Type /Pages /Parent 2 0 R /Kids [3 0 R 4 0 R] /Count 2 >>`],
  bothUntyped: [`<< /Kids [10 0 R 5 0 R] /Count 3 >>`, `<< /Parent 2 0 R /Kids [3 0 R 4 0 R] /Count 2 >>`],
  noCount: [`<< /Type /Pages /Kids [10 0 R 5 0 R] /Count 3 >>`, `<< /Parent 2 0 R /Kids [3 0 R 4 0 R] >>`],
  nested: [`<< /Type /Pages /Kids [10 0 R 5 0 R] /Count 4 >>`, `<< /Parent 2 0 R /Kids [3 0 R 12 0 R] /Count 3 >>`],
};
function build(name) {
  const [two, ten] = shapes[name];
  const nested = name === 'nested';
  return rawPdf([
    '<< /Type /Catalog /Pages 2 0 R >>', two,
    page(6, 10), page(7, nested ? 12 : 10), page(8, 2),
    C('Alpha'), C('Beta'), C('Gamma'), F, ten,
    nested ? page(14, 12) : F,
    nested ? '<< /Parent 10 0 R /Kids [4 0 R 11 0 R] /Count 2 >>' : F,
    '[3 0 R 4 0 R]',
    C('Delta'),
  ]);
}
const edits = [
  null,
  { op: 'page-rotate', page: 0, by: 90 }, { op: 'page-rotate', page: 1, by: 90 },
  { op: 'page-delete', page: 0 }, { op: 'page-delete', page: 1 },
  { op: 'page-move', page: 0, to: 2 }, { op: 'page-move', page: 2, to: 0 },
  { op: 'insert-pages', at: 1, name: 'h.pdf', pdf: null },
  { op: 'extract-pages', pages: [1] },
];
const mrot = bytes => { const s = mu.createSession(); const o = call(mu, s, { op: 'open', name: 'x', pdf: ab(bytes) }); if (!o.ok) return o.error; return (o.rotations ?? []).join(','); };
for (const name of Object.keys(shapes)) {
  const bytes = build(name);
  console.log(`== ${name}`);
  for (const [n, e] of [['MU', mu], ['RS', rs], ['R2', rsR2]]) {
    if (!e) continue;
    for (const edit of edits) {
      const s = e.createSession();
      const o = call(e, s, { op: 'open', name: 'u.pdf', pdf: ab(bytes) });
      if (!o.ok) { console.log(`  ${n} open ERR ${o.error}`); break; }
      let r = { ok: true };
      if (edit) {
        const req = { ...edit };
        if (req.op === 'insert-pages') req.pdf = ab(fixture('hello.pdf'));
        r = call(e, s, req);
      }
      const ids = call(e, s, { op: 'page-ids' }).ids;
      const t = call(e, s, { op: 'text-all' });
      let out = `  ${n} ${edit ? JSON.stringify({ ...edit, pdf: undefined }) : 'open'} -> ${r.ok ? 'ok' : 'ERR ' + r.error} ids=${JSON.stringify(ids)} text=${(t.text ?? t.error).replace(/\s/g, '')}`;
      if (edit?.op === 'extract-pages' && r.ok && r.bytes) { const m = view(mu, Buffer.from(r.bytes)); out += ` | extract@MU ${fmt(m)}`; }
      else {
        const sv = call(e, s, { op: 'save' });
        if (sv.ok) { const m = view(mu, Buffer.from(sv.bytes)); out += ` | save@MU p=${m.pages} ${m.text} rot=${mrot(Buffer.from(sv.bytes))}`; } else out += ` | save ERR ${sv.error}`;
      }
      console.log(out);
    }
  }
}
