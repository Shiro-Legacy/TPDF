// Probe 22: differential fuzz of the repair's lenient token reading.
// A 3-page file (annotation on page 1, a form XObject on page 2) gets 1-3
// damaged tokens placed in dictionary values, array entries or between keys of
// its structural objects; intact xref and broken startxref variants.
// Compared: MuPDF (engine.ts) view of the source vs the Rust view, and MuPDF's
// reading of the Rust save (does the save lose what MuPDF reads in the source?).
import { mu, rs, call, ab, rawPdf, stream } from './lib.mjs';
import { writeFileSync, mkdirSync } from 'node:fs';
const DUMP = `${process.env.FUZZ_DUMP ?? `${(await import('node:os')).tmpdir()}/tpdf-fuzz`}/fz22`;
mkdirSync(DUMP, { recursive: true });

let seed = Number(process.argv[2] ?? 1);
const seed0 = seed;
let x32 = (seed * 2654435761) >>> 0 || 1;
const rand = () => { x32 ^= x32 << 13; x32 >>>= 0; x32 ^= x32 >>> 17; x32 ^= x32 << 5; x32 >>>= 0; return x32 / 4294967296; };
const pick = n => Math.floor(rand() * n);

const TOKENS = ['1e5', '1.0E-4', '--5', '-+5', '+-5', '1.2.3', '5-', '-5-', '12abc', '1,5', '0x10', '.', '-', '+', 'bogus', 'nul', 'NaN', 'true1', 'falsey',
  '<ABQD>', '<A B Q>', '<>', '\xa5', '\x98x', '7g2', '4 0\xa5R', 'R', '0 0 R', '1 70000 R', '-1 0 R', '99999999999 0 R', '4294967296 0 R', '3 0 obj', 'obj', 'endobj',
  'stream', 'endstream', 'xref', 'trailer', 'startxref', ')', '>', '{', '}', '{1}', ']', '>>', '1 2', '1 2 3', '1 bogus', '18446744073709551617', '4294967296.5-',
  '1..5', '1-.5', '.5.', '/A#G0', '/', '//', '/#', 'true', 'false', 'null', '0.000001', '-0', '-.0', '(x)', '(x', 'x)', '[1', '<<', '<< /A 1'];

const FONT = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>';
// objects as arrays of tokens so that damage can be placed between them
const OBJS = [
  ['<<', '/Type', '/Catalog', '/Pages', '2 0 R', '/PageMode', '/UseNone', '>>'],
  ['<<', '/Type', '/Pages', '/Kids', '[', '3 0 R', '4 0 R', '5 0 R', ']', '/Count', '3', '>>'],
  ['<<', '/Type', '/Page', '/Parent', '2 0 R', '/MediaBox', '[', '0', '0', '612', '792', ']', '/Resources', '<<', '/Font', '<<', '/F1', '9 0 R', '>>', '>>', '/Contents', '6 0 R', '/Annots', '[', '10 0 R', ']', '>>'],
  ['<<', '/Type', '/Page', '/Parent', '2 0 R', '/MediaBox', '[', '0', '0', '595', '842', ']', '/Resources', '<<', '/Font', '<<', '/F1', '9 0 R', '>>', '/XObject', '<<', '/Fm', '11 0 R', '>>', '>>', '/Contents', '7 0 R', '>>'],
  ['<<', '/Type', '/Page', '/Parent', '2 0 R', '/MediaBox', '[', '0', '0', '400', '400', ']', '/Rotate', '0', '/Resources', '<<', '/Font', '<<', '/F1', '9 0 R', '>>', '>>', '/Contents', '[', '8 0 R', ']', '>>'],
  null, null, null,
  ['<<', '/Type', '/Font', '/Subtype', '/Type1', '/BaseFont', '/Helvetica', '>>'],
  ['<<', '/Type', '/Annot', '/Subtype', '/Square', '/Rect', '[', '100', '100', '200', '200', ']', '/C', '[', '1', '0', '0', ']', '/F', '4', '/Contents', '(sq)', '>>'],
  null,
];
const CONTENT = {
  6: 'BT /F1 24 Tf 72 700 Td (Alpha page) Tj ET',
  7: 'BT /F1 24 Tf 72 700 Td (Beta page) Tj ET q /Fm Do Q',
  8: 'BT /F1 24 Tf 72 300 Td (Gamma page) Tj ET',
};
const FORM = ['<<', '/Type', '/XObject', '/Subtype', '/Form', '/BBox', '[', '0', '0', '500', '500', ']', '/Matrix', '[', '1', '0', '0', '1', '0', '0', ']', '/Resources', '<<', '/Font', '<<', '/F1', '9 0 R', '>>', '>>'];
const FORMBODY = 'BT /F1 18 Tf 72 400 Td (Delta form) Tj ET';

function build(damaged) {
  const objs = [];
  for (let i = 0; i < 11; i++) {
    const n = i + 1;
    if (CONTENT[n]) { objs.push(stream(CONTENT[n])); continue; }
    if (n === 11) {
      const toks = damaged[11] ?? FORM;
      objs.push(`${toks.join(' ')} /Length ${FORMBODY.length} >>\nstream\n${FORMBODY}\nendstream`);
      continue;
    }
    objs.push((damaged[n] ?? OBJS[i]).join(' '));
  }
  return rawPdf(objs);
}

function mutateOnce(damaged) {
  const targets = [1, 2, 3, 4, 5, 9, 10, 11];
  const n = targets[pick(targets.length)];
  const toks = [...(damaged[n] ?? (n === 11 ? FORM : OBJS[n - 1]))];
  const tok = TOKENS[pick(TOKENS.length)];
  const kind = pick(3);
  // never touch the first '<<'
  const at = 1 + pick(toks.length - 1);
  if (kind === 0) toks.splice(at, 0, tok); // insert
  else if (kind === 1) { if (toks[at] !== '>>' || n === 11) toks[at] = tok; else toks.splice(at, 0, tok); } // replace a token
  else toks.splice(at, 0, `/Xk${pick(9)}`, tok); // a new key with a damaged value
  damaged[n] = toks;
}

const brokenXref = b => Buffer.from(b.toString('latin1').replace(/startxref\n\d+/, 'startxref\n999999'), 'latin1');

function view(engine, bytes) {
  const s = engine.createSession();
  const o = call(engine, s, { op: 'open', name: 'f', pdf: ab(bytes) });
  if (!o.ok) return { ok: false, error: o.error, s };
  const t = call(engine, s, { op: 'text-all' });
  const texts = t.ok ? t.text.split('\f').map(x => x.replace(/\s/g, '')) : null;
  const annots = [];
  for (let p = 0; p < o.pageCount; p++) {
    const a = call(engine, s, { op: 'annots', page: p });
    annots.push(a.ok ? a.items.length : `E`);
  }
  const sizes = o.pages.map(p => `${Math.round(p.width)}x${Math.round(p.height)}`);
  return { ok: true, pages: o.pageCount, repaired: o.info.repaired, text: t.ok ? t.text.replace(/\s/g, '') : `E:${t.error}`, texts, annots, sizes, s };
}

const N = Number(process.argv[3] ?? 300);
const st = { n: 0, muOpens: 0, rsRefuses: 0, rsOpensMuRefuses: 0, viewTextLess: 0, viewTextMore: 0, viewAnnotsLess: 0, viewPagesDiffer: 0, sizesDiffer: 0, saveFails: 0, saveTextLess: 0, saveAnnotsLess: 0, savePagesLess: 0, repairedDiffer: 0 };
const ex = [];
const note = (k, i, msg, bytes) => { st[k]++; if (ex.filter(e => e.startsWith(k)).length < 6) { ex.push(`${k} i=${i} ${msg}`); writeFileSync(`${DUMP}/${k}-${seed0}-${i}.pdf`, bytes); } };
for (let i = 0; i < N; i++) {
  const damaged = {};
  const k = 1 + pick(3);
  for (let j = 0; j < k; j++) mutateOnce(damaged);
  let bytes = build(damaged);
  if (rand() < 0.4) bytes = brokenXref(bytes);
  st.n++;
  const a = view(mu, bytes);
  const t0 = performance.now();
  const b = view(rs, bytes);
  const dt = performance.now() - t0;
  if (dt > 2000) note('slow', i, `${Math.round(dt)} ms`, bytes);
  if (!a.ok && !b.ok) continue;
  if (a.ok) st.muOpens++;
  if (a.ok && !b.ok) { note('rsRefuses', i, `rs=${b.error}`, bytes); continue; }
  if (!a.ok && b.ok) { st.rsOpensMuRefuses++; continue; }
  if (a.repaired !== b.repaired) st.repairedDiffer++;
  if (a.pages !== b.pages) note('viewPagesDiffer', i, `mu=${a.pages} rs=${b.pages}`, bytes);
  else if (a.sizes.join() !== b.sizes.join()) note('sizesDiffer', i, `mu=${a.sizes} rs=${b.sizes}`, bytes);
  if (b.text.length < a.text.length) note('viewTextLess', i, `mu=${JSON.stringify(a.text)} rs=${JSON.stringify(b.text)} rep mu=${a.repaired} rs=${b.repaired}`, bytes);
  if (b.text.length > a.text.length) st.viewTextMore++;
  const sumA = a.annots.reduce((x, y) => x + (typeof y === 'number' ? y : 0), 0), sumB = b.annots.reduce((x, y) => x + (typeof y === 'number' ? y : 0), 0);
  if (sumB < sumA) note('viewAnnotsLess', i, `mu=${a.annots} rs=${b.annots}`, bytes);
  // Rust save as MuPDF reads it
  const sv = call(rs, b.s, { op: 'save' });
  if (!sv.ok) { note('saveFails', i, sv.error, bytes); continue; }
  const v = call(rs, b.s, { op: 'validate', pdf: sv.bytes.slice(0) });
  if (v && v.ok === false) note('saveFails', i, `validate ${v.error}`, bytes);
  const c = view(mu, Buffer.from(sv.bytes));
  if (!c.ok) { note('saveFails', i, `mupdf reopen ${c.error}`, bytes); continue; }
  if (c.pages < a.pages) note('savePagesLess', i, `src=${a.pages} save=${c.pages}`, bytes);
  if (c.text.length < a.text.length) note('saveTextLess', i, `src=${JSON.stringify(a.text)} save=${JSON.stringify(c.text)}`, bytes);
  const sumC = c.annots.reduce((x, y) => x + (typeof y === 'number' ? y : 0), 0);
  if (sumC < sumA) note('saveAnnotsLess', i, `src=${a.annots} save=${c.annots}`, bytes);
  call(mu, a.s, { op: 'close' }); call(rs, b.s, { op: 'close' }); call(mu, c.s, { op: 'close' });
}
console.log(JSON.stringify(st));
for (const e of ex) console.log(e);
