// Probe 7: mutation fuzz of open/authenticate/page-ids/security-set/save/validate/close on small inputs.
// Rust: no trap, bounded time. MuPDF: compare open outcome (Rust refusing what MuPDF opens).
import { mu, rs, call, fixture, ab, mupdf, pages } from './lib.mjs';
import { writeFileSync } from 'node:fs';
const DUMP = `${process.env.FUZZ_DUMP ?? `${(await import('node:os')).tmpdir()}/tpdf-fuzz`}/fz`;
const dump = (tag, i, bytes) => writeFileSync(`${DUMP}/${tag}-${seed0}-${i}.pdf`, Buffer.from(bytes));

function mupdfWrite(bytes, options) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  try { return doc.asPDF().saveToBuffer(options).asUint8Array().slice().buffer; } finally { doc.destroy(); }
}
let seed = Number(process.argv[2] ?? 1);
const seed0 = seed;
let x32 = (seed * 2654435761) >>> 0 || 1;
const rand = () => { x32 ^= x32 << 13; x32 >>>= 0; x32 ^= x32 >>> 17; x32 ^= x32 << 5; x32 >>>= 0; return x32 / 4294967296; };
const pick = n => Math.floor(rand() * n);

const seeds = {
  hello: fixture('hello.pdf'),
  enc: fixture('encrypted.pdf'),
  aes: fixture('encrypted-aes256.pdf'),
  noedit: fixture('encrypted-noedit.pdf'),
  form: fixture('form.pdf'),
  nav: fixture('nav.pdf'),
  r4objstm: mupdfWrite(ab(pages(2)), 'encrypt=aes-128,user-password=user,owner-password=owner,permissions=4,objstms=yes,compress=yes'),
  r6objstm: mupdfWrite(ab(pages(2)), 'encrypt=aes-256,user-password=user,owner-password=owner,permissions=4,objstms=yes'),
  r2: mupdfWrite(ab(pages(2)), 'encrypt=rc4-40,user-password=,owner-password=owner,permissions=4'),
  objstm: mupdfWrite(ab(pages(3)), 'objstms=yes,compress=yes'),
};
const TOKENS = ['/Encrypt', '/R 6', '/R 2', '/V 4', '/V 5', '/Length 999999', '/P -1', '/P 0', ' obj', 'endobj', 'stream', 'endstream', 'xref', 'trailer', 'startxref', '/Kids [', '/Count 0', '/Count 99999999', '/Root', '/Prev 0', '/XRefStm 9', '/Type /ObjStm', '/N 999999', '/First 9', ' 0 R', '<<', '>>', '[', ']', '(', ')', '/CFM /AESV3', '/CFM /None', '/StmF /Identity', '/EncryptMetadata false', '%%EOF', '/Linearized 1'];

function mutate(buf) {
  const b = Buffer.from(buf);
  const n = 1 + pick(4);
  let out = b;
  for (let i = 0; i < n; i++) {
    const kind = pick(6);
    const at = pick(out.length);
    if (kind === 0) { out = Buffer.from(out); out[at] = pick(256); }
    else if (kind === 1) out = out.subarray(0, Math.max(5, at));
    else if (kind === 2) out = Buffer.concat([out.subarray(0, at), Buffer.from(TOKENS[pick(TOKENS.length)], 'latin1'), out.subarray(at)]);
    else if (kind === 3) out = Buffer.concat([out.subarray(0, at), out.subarray(Math.min(out.length, at + 1 + pick(40)))]);
    else if (kind === 4) { const s = out.toString('latin1'); const d = String(pick(100000)); const m = s.replace(/\d+/g, (x) => (rand() < 0.02 ? d : x)); out = Buffer.from(m, 'latin1'); }
    else { const from = pick(out.length); const len = pick(200); out = Buffer.concat([out.subarray(0, at), out.subarray(from, from + len), out.subarray(at)]); }
  }
  return out;
}

const N = Number(process.argv[3] ?? 500);
const stats = { rustOpen: 0, mupdfOpen: 0, rustOnly: 0, mupdfOnly: 0, traps: 0, slow: 0, saveFail: 0, validateFail: 0, lockedR: 0, lockedM: 0, pwDiff: 0, pagesDiff: 0 };
const examples = {};
const names = Object.keys(seeds);
for (let i = 0; i < N; i++) {
  const name = names[pick(names.length)];
  const bytes = ab(mutate(seeds[name]));
  const sm = mu.createSession();
  const sr = rs.createSession();
  const a = call(mu, sm, { op: 'open', name, pdf: bytes.slice(0) });
  const t = performance.now();
  const b = call(rs, sr, { op: 'open', name, pdf: bytes.slice(0) });
  const seq = [b];
  if (a.ok) stats.mupdfOpen++;
  if (b.ok) stats.rustOpen++;
  if (a.ok && !b.ok) { dump('mupdfOnly', i, bytes); stats.mupdfOnly++; (examples.mupdfOnly ??= []).push(`${name} i=${i} ${b.error.slice(0, 90)}`); }
  if (!a.ok && b.ok) stats.rustOnly++;
  if (a.ok && b.ok) {
    if (a.info.needsPassword !== b.info.needsPassword) { dump(`pw-${a.info.needsPassword}-${b.info.needsPassword}`, i, bytes); stats.pwDiff++; (examples.pwDiff ??= []).push(`${name} i=${i} mu=${a.info.needsPassword} rs=${b.info.needsPassword}`); }
    else if (a.pageCount !== b.pageCount) { dump('pages', i, bytes); stats.pagesDiff++; (examples.pagesDiff ??= []).push(`${name} i=${i} mu=${a.pageCount} rs=${b.pageCount}`); }
  }
  if (b.ok) {
    if (b.info.needsPassword) stats.lockedR++;
    seq.push(call(rs, sr, { op: 'page-ids' }));
    for (const password of ['wrong', 'user', 'pdf4all-owner', 'owner']) seq.push(call(rs, sr, { op: 'authenticate', password }));
    seq.push(call(rs, sr, { op: 'page-ids' }));
    seq.push(call(rs, sr, { op: 'security-set', mode: 'none' }));
    const saved = call(rs, sr, { op: 'save' });
    seq.push(saved);
    if (saved.ok) {
      const v = call(rs, sr, { op: 'validate', pdf: saved.bytes.slice(0) });
      seq.push(v);
      if (!v.ok) { stats.validateFail++; (examples.validateFail ??= []).push(`${name} i=${i} ${v.error.slice(0, 90)}`); }
    } else if (saved.reason !== 'read-only' && saved.reason !== 'encrypted') { stats.saveFail++; (examples.saveFail ??= []).push(`${name} i=${i} ${saved.error.slice(0, 90)}`); }
    seq.push(call(rs, sr, { op: 'security-set', mode: 'aes-256', userPassword: 'x' }));
    const s2 = call(rs, sr, { op: 'save' });
    if (s2.ok) seq.push(call(rs, sr, { op: 'validate', pdf: s2.bytes.slice(0) }));
  }
  seq.push(call(rs, sr, { op: 'close' }));
  const ms = performance.now() - t;
  if (ms > 2000) { stats.slow++; (examples.slow ??= []).push(`${name} i=${i} ${Math.round(ms)}ms`); }
  if (seq.some(r => r.reason === 'worker')) { stats.traps++; (examples.traps ??= []).push(`${name} i=${i} ${seq.find(r => r.reason === 'worker').error.slice(0, 120)}`); }
  call(mu, sm, { op: 'close' });
}
console.log(JSON.stringify(stats));
for (const [k, v] of Object.entries(examples)) console.log(k, v.slice(0, 8));
