// Probe 25: realistic damage model: 1-2 random byte replacements (or a deleted/duplicated byte)
// inside the object dictionaries (not stream data) of a 3-page file; intact and broken xref.
// Counts Rust saves that lose pages, text or annotations MuPDF reads in the source.
import { mu, rs, call, ab, rawPdf, stream } from './lib.mjs';
import { writeFileSync, mkdirSync } from 'node:fs';
const DUMP = `${process.env.FUZZ_DUMP ?? `${(await import('node:os')).tmpdir()}/tpdf-fuzz`}/fz25${process.env.TAG ?? ''}`;
mkdirSync(DUMP, { recursive: true });
let seed = Number(process.argv[2] ?? 1); const seed0 = seed;
let x32 = (seed * 2654435761) >>> 0 || 1;
const rand = () => { x32 ^= x32 << 13; x32 >>>= 0; x32 ^= x32 >>> 17; x32 ^= x32 << 5; x32 >>>= 0; return x32 / 4294967296; };
const pick = n => Math.floor(rand() * n);
const FORMBODY = 'BT /F1 18 Tf 72 400 Td (Delta form) Tj ET';
const base = rawPdf([
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 9 0 R >> >> /Contents 6 0 R /Annots [10 0 R] >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 9 0 R >> /XObject << /Fm 11 0 R >> >> /Contents 7 0 R >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 400] /Resources << /Font << /F1 9 0 R >> >> /Contents [8 0 R] >>',
  stream('BT /F1 24 Tf 72 700 Td (Alpha page) Tj ET'),
  stream('BT /F1 24 Tf 72 700 Td (Beta page) Tj ET q /Fm Do Q'),
  stream('BT /F1 24 Tf 72 300 Td (Gamma page) Tj ET'),
  '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  '<< /Type /Annot /Subtype /Square /Rect [100 100 200 200] /C [1 0 0] /F 4 /Contents (sq) >>',
  `<< /Type /XObject /Subtype /Form /BBox [0 0 500 500] /Resources << /Font << /F1 9 0 R >> >> /Length ${FORMBODY.length} >>\nstream\n${FORMBODY}\nendstream`,
]);
// byte ranges of dictionary text (between "obj\n" and "\nendobj" or "stream")
const s = base.toString('latin1');
const ranges = [];
for (const m of s.matchAll(/ 0 obj\n(<<[^]*?)(\nendobj|\nstream)/g)) {
  const start = m.index + ' 0 obj\n'.length; ranges.push([start, start + m[1].length]);
}
const view = (engine, bytes) => {
  const se = engine.createSession();
  const o = call(engine, se, { op: 'open', name: 'f', pdf: ab(bytes) });
  if (!o.ok) return { ok: false, error: o.error, s: se };
  const t = call(engine, se, { op: 'text-all' });
  let an = 0; for (let p = 0; p < o.pageCount; p++) { const a = call(engine, se, { op: 'annots', page: p }); if (a.ok) an += a.items.length; }
  return { ok: true, pages: o.pageCount, rep: o.info.repaired, text: t.ok ? t.text.replace(/\s/g, '') : '', an, s: se };
};
const N = Number(process.argv[3] ?? 400);
const st = { n: 0, muOpens: 0, rsRefuses: 0, savePagesLess: 0, saveTextLess: 0, saveAnnotsLess: 0, viewTextLess: 0, saveFails: 0 };
const ex = [];
for (let i = 0; i < N; i++) {
  let b = Buffer.from(base);
  const k = 1 + pick(2);
  const edits = [];
  for (let j = 0; j < k; j++) {
    const [lo, hi] = ranges[pick(ranges.length)];
    const at = lo + 2 + pick(hi - lo - 4); // keep the outer << >>
    const kind = pick(4);
    const byte = rand() < 0.5 ? pick(256) : ' /<>[]()0123456789-+.ReAPgtyx'.charCodeAt(pick(29));
    if (kind <= 1) { edits.push(`set@${at}:${JSON.stringify(String.fromCharCode(b[at]))}->${JSON.stringify(String.fromCharCode(byte))}`); b = Buffer.from(b); b[at] = byte; }
    else if (kind === 2) { edits.push(`del@${at}:${JSON.stringify(String.fromCharCode(b[at]))}`); b = Buffer.concat([b.subarray(0, at), Buffer.from(' '), b.subarray(at + 1)]); b[at] = 0x20; }
    else { edits.push(`ins@${at}:${JSON.stringify(String.fromCharCode(byte))}`); b = Buffer.from(b); b[at] = byte === 0x20 ? 0x2d : byte; }
  }
  const brokenX = rand() < 0.5;
  if (brokenX) b = Buffer.from(b.toString('latin1').replace(/startxref\n\d+/, 'startxref\n999999'), 'latin1');
  st.n++;
  const a = view(mu, b), r = view(rs, b);
  if (!a.ok) continue;
  st.muOpens++;
  const tag = `${brokenX ? 'broken' : 'intact'} ${edits.join(' ')}`;
  const keep = (k2, msg) => { st[k2]++; if (ex.filter(e => e.startsWith(k2)).length < 8) { ex.push(`${k2} i=${i} ${tag} :: ${msg}`); writeFileSync(`${DUMP}/${k2}-${seed0}-${i}.pdf`, b); } };
  if (!r.ok) { keep('rsRefuses', r.error); continue; }
  if (r.text.length < a.text.length) st.viewTextLess++;
  const sv = call(rs, r.s, { op: 'save' });
  if (!sv.ok) { keep('saveFails', sv.error); continue; }
  const c = view(mu, Buffer.from(sv.bytes));
  if (!c.ok) { keep('saveFails', c.error); continue; }
  if (c.pages < a.pages) keep('savePagesLess', `src=${a.pages} save=${c.pages}`);
  if (c.text.length < a.text.length) keep('saveTextLess', `src=${a.text} save=${c.text}`);
  if (c.an < a.an) keep('saveAnnotsLess', `src=${a.an} save=${c.an}`);
  for (const v of [a, r, c]) call(v === r ? rs : mu, v.s, { op: 'close' });
}
console.log(JSON.stringify(st));
for (const e of ex) console.log(e);
