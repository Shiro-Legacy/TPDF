// Probe helpers: both engines the way tests/rust-engine.mjs builds them,
// without the test harness's session.doc copies.
import { readFileSync } from 'node:fs';
const APP = process.env.TPDF_APP ?? new URL('../../..', import.meta.url).pathname.replace(/\/$/, '');
export const mupdf = (await import(`${APP}/node_modules/mupdf/dist/mupdf.js`)).default;
const { createEngine } = await import(`${APP}/build-oracle/engine/oracle/app-engine/engine.js`);
const { createRustEngine } = await import(`${APP}/build/renderer/engine-rust.js`);
const PKG = process.env.RS_PKG ?? `${APP}/engine/web/pkg`;
const wasm = await import(`${PKG}/tandem_rust_workflow.js`);
wasm.initSync({ module: readFileSync(`${PKG}/tandem_rust_workflow_bg.wasm`) });

export const mu = createEngine(mupdf);
export const rs = createRustEngine(wasm);
export { wasm };

export function fixture(name) {
  const b = readFileSync(`${APP}/fixtures/${name}`);
  return b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);
}
export function ab(buf) {
  const b = Buffer.isBuffer(buf) ? buf : Buffer.from(buf);
  return b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);
}

let nextId = 1;
export function call(engine, session, request) {
  const req = { id: nextId++, ...request };
  // both engines may mutate the request (pdf = null), so each gets its own copy
  const { reply } = engine.dispatch(req, session);
  return reply;
}

/** Strip timing and buffers for comparison. */
export function norm(reply) {
  return JSON.parse(JSON.stringify(reply, (k, v) => {
    if (k === 'id') return undefined;
    if (k === 'openMs' || k === 'saveMs' || k === 'paintMs' || k === 'renderMs' || k === 'searchMs') return 0;
    if (v instanceof ArrayBuffer) return { $bytes: v.byteLength };
    if (k === 'ticket') return undefined;
    return v;
  }));
}

/** Runs the same request sequence on both engines; returns pairs. */
export function both(steps, { log = false } = {}) {
  const sm = mu.createSession();
  const sr = rs.createSession();
  const out = [];
  for (const step of steps) {
    const r1 = call(mu, sm, typeof step === 'function' ? step() : clone(step));
    const r2 = call(rs, sr, typeof step === 'function' ? step() : clone(step));
    out.push({ step: describe(step), mu: r1, rs: r2 });
    if (log) console.log(describe(step), JSON.stringify(norm(r1)), JSON.stringify(norm(r2)));
  }
  return { out, sm, sr };
}
function clone(step) {
  const copy = {};
  for (const [k, v] of Object.entries(step)) copy[k] = v instanceof ArrayBuffer ? v.slice(0) : v;
  return copy;
}
function describe(step) {
  const s = typeof step === 'function' ? step() : step;
  return JSON.stringify(s, (k, v) => (v instanceof ArrayBuffer ? `<${v.byteLength}B>` : v));
}

export function diff(a, b, path = '') {
  const out = [];
  if (typeof a !== typeof b || Array.isArray(a) !== Array.isArray(b) || (a === null) !== (b === null)) {
    out.push(`${path}: ${JSON.stringify(a)} vs ${JSON.stringify(b)}`);
    return out;
  }
  if (a && typeof a === 'object') {
    const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
    for (const k of keys) out.push(...diff(a[k], b[k], `${path}.${k}`));
    return out;
  }
  if (a !== b) out.push(`${path}: ${JSON.stringify(a)} vs ${JSON.stringify(b)}`);
  return out;
}

export function report(pairs, title) {
  let same = 0;
  for (const p of pairs) {
    const d = diff(norm(p.mu), norm(p.rs));
    if (d.length) {
      console.log(`  DIFF ${title} ${p.step.slice(0, 160)}`);
      for (const line of d.slice(0, 12)) console.log(`     ${line}`);
    } else same++;
  }
  console.log(`  ${title}: ${same}/${pairs.length} same`);
}

/** MuPDF reading of bytes: pages, needsPassword, auth with password, repaired, encryption meta. */
export function mupdfRead(bytes, password) {
  const doc = mupdf.Document.openDocument(bytes, 'application/pdf');
  try {
    const pdf = doc.asPDF();
    const needs = doc.needsPassword();
    let auth = null;
    if (needs && password !== undefined) auth = doc.authenticatePassword(password);
    let pages = -1;
    try { pages = doc.countPages(); } catch (e) { pages = `err ${e.message}`; }
    const enc = (() => { try { return doc.getMetaData('encryption'); } catch { return '?'; } })();
    let text = '';
    try { if (!needs || auth) { const p = doc.loadPage(0); text = p.toStructuredText().asText().trim(); p.destroy(); } } catch (e) { text = `err ${e.message}`; }
    const perms = ['print', 'copy', 'edit', 'annotate', 'form'].filter(n => { try { return doc.hasPermission(n); } catch { return false; } });
    return { needs, auth, pages, repaired: pdf?.wasRepaired(), enc, text: text.slice(0, 60), perms };
  } finally {
    doc.destroy();
  }
}

// ---- tiny PDF builders
const LATIN = 'latin1';
export function rawPdf(objects, trailerExtra = '', { header = '%PDF-1.4\n' } = {}) {
  const parts = [Buffer.from(header, LATIN)];
  const offsets = [];
  let size = parts[0].length;
  for (const [index, body] of objects.entries()) {
    if (body === null) { offsets.push(null); continue; }
    const bytes = Buffer.isBuffer(body) ? Buffer.concat([Buffer.from(`${index + 1} 0 obj\n`, LATIN), body, Buffer.from('\nendobj\n', LATIN)]) : Buffer.from(`${index + 1} 0 obj\n${body}\nendobj\n`, LATIN);
    offsets.push(size);
    parts.push(bytes);
    size += bytes.length;
  }
  let tail = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) tail += offset === null ? '0000000000 65535 f \n' : `${String(offset).padStart(10, '0')} 00000 n \n`;
  tail += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R ${trailerExtra}>>\nstartxref\n${size}\n%%EOF\n`;
  parts.push(Buffer.from(tail, LATIN));
  return Buffer.concat(parts);
}
export const stream = (body, dict = '') => `<< ${dict} /Length ${Buffer.byteLength(body, LATIN)} >>\nstream\n${body}\nendstream`;
export const HELV = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>';
export function pages(n, { kidsInline = false } = {}) {
  const objs = ['<< /Type /Catalog /Pages 2 0 R >>'];
  const kids = [];
  const pageObjs = [];
  for (let i = 0; i < n; i++) {
    const pageNum = 3 + i * 2;
    kids.push(`${pageNum} 0 R`);
    pageObjs.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 ${3 + n * 2} 0 R >> >> /Contents ${pageNum + 1} 0 R >>`);
    pageObjs.push(stream(`BT /F1 24 Tf 72 700 Td (Page ${i + 1}) Tj ET`));
  }
  objs.push(`<< /Type /Pages /Kids [${kids.join(' ')}] /Count ${n} >>`);
  objs.push(...pageObjs, HELV);
  return rawPdf(objs);
}
