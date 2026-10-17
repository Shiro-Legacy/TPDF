// Cut damage: one object's text loses a random tail (its closing >> and/or
// endobj, as a partial write or a hand edit leaves it) and the next object
// header follows. Intact and broken xref. Counts, per build, Rust saves that
// lose pages or text MuPDF (engine.ts) shows in the source.
import { mu, rs, rsF04, rawPdf, stream, view, call } from './lib2.mjs';
let seed = Number(process.argv[2] ?? 1);
let x32 = (seed * 2654435761) >>> 0 || 1;
const rand = () => { x32 ^= x32 << 13; x32 >>>= 0; x32 ^= x32 >>> 17; x32 ^= x32 << 5; x32 >>>= 0; return x32 / 4294967296; };
const pick = n => Math.floor(rand() * n);
const FORMBODY = 'BT /F1 18 Tf 72 400 Td (Delta form) Tj ET';
const objs = [
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 9 0 R >> >> /Contents 6 0 R /Annots [10 0 R] >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 9 0 R >> /XObject << /Fm 11 0 R >> >> /Contents 7 0 R >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 400] /Resources << /Font << /F1 9 0 R >> >> /Contents [8 0 R] >>',
  stream('BT /F1 24 Tf 72 700 Td (Alpha page) Tj ET'),
  stream('BT /F1 24 Tf 72 700 Td (Beta page) Tj ET q /Fm Do Q'),
  stream('BT /F1 24 Tf 72 300 Td (Gamma page) Tj ET'),
  '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
  '<< /Type /Annot /Subtype /Square /Rect [100 100 200 200] /C [1 0 0] /F 4 /Contents (sq) >>',
  `<< /Type /XObject /Subtype /Form /BBox [0 0 500 500] /Resources << /Font << /F1 9 0 R >> >> /Length ${FORMBODY.length} >>\nstream\n${FORMBODY}\nendstream`,
  '<< /Title (Doc) /Producer (probe) /Count 7 >>',
];
const N = Number(process.argv[3] ?? 200);
const builds = [['RS', rs], ['F04', rsF04]].filter(([, e]) => e);
const st = Object.fromEntries(builds.map(([n]) => [n, { pagesLess: 0, textLess: 0, refuses: 0, saveFails: 0 }]));
let muOpens = 0;
const ex = [];
for (let i = 0; i < N; i++) {
  // a random object (not a stream: 6-8, 11) cut short, written at a random place
  const cands = [0, 1, 2, 3, 4, 8, 9, 11];
  const victim = cands[pick(cands.length)];
  const body = objs[victim];
  const full = `${body}\nendobj\n`;
  const cutAt = 2 + pick(full.length - 2);
  const nl = rand() < 0.5;
  const raw = { [victim + 1]: full.slice(0, cutAt) + (nl ? '\n' : '') };
  const order = objs.map((_, k) => k);
  // move the victim before a random other object
  order.splice(order.indexOf(victim), 1);
  const at = pick(order.length + 1);
  order.splice(at, 0, victim);
  let b = rawPdf(objs, '/Info 12 0 R ', { order, raw });
  const brokenX = rand() < 0.5;
  if (brokenX) b = Buffer.from(b.toString('latin1').replace(/startxref\n\d+/, 'startxref\n999999'), 'latin1');
  const a = view(mu, b);
  if (!a.ok) continue;
  muOpens++;
  const tag = `i=${i} ${brokenX ? 'broken' : 'intact'} ${nl ? 'nl' : 'glued'} obj${victim + 1} cut ${JSON.stringify(full.slice(cutAt, cutAt + 30))} before ${at < order.length - 1 ? 'obj' + (order[at + 1] + 1) : 'xref'}`;
  const res = {};
  for (const [name, e] of builds) {
    const r = view(e, b, { save: true });
    if (!r.ok) { st[name].refuses++; res[name] = 'refuses'; continue; }
    if (!r.saved) { st[name].saveFails++; res[name] = 'savefail'; continue; }
    const c = view(mu, r.saved);
    if (!c.ok) { st[name].saveFails++; res[name] = 'savefail'; continue; }
    const bad = [];
    if (c.pages < a.pages) { st[name].pagesLess++; bad.push(`pages ${c.pages}<${a.pages}`); }
    if (c.text.length < a.text.length) { st[name].textLess++; bad.push(`text ${c.text}`); }
    res[name] = bad.join(' ') || 'ok';
  }
  if (res.RS !== 'ok') { const cls = res.F04 === 'ok' ? 'REGRESSION' : 'pre-existing'; st.cls ??= {}; st.cls[cls + (nl ? '-nl' : '-glued')] = (st.cls[cls + (nl ? '-nl' : '-glued')] ?? 0) + 1; ex.push(`${cls} ${tag} :: MU p=${a.pages} ${a.text} | RS ${res.RS} | F04 ${res.F04}`); }
}
console.log(JSON.stringify({ seed, N, muOpens, st }));
for (const e of ex.slice(0, 40)) console.log(e);
