// Round-2 probe helpers: engine.ts on MuPDF, the fix build (c4d2181), and
// optional baselines (f04e094 built in base-f04/pkg, ed62a16 in the main checkout).
import { readFileSync, existsSync } from 'node:fs';
const APP = process.env.TPDF_APP ?? new URL('../../..', import.meta.url).pathname.replace(/\/$/, '');
export const mupdf = (await import(`${APP}/node_modules/mupdf/dist/mupdf.js`)).default;
const { createEngine } = await import(`${APP}/build-oracle/engine/oracle/app-engine/engine.js`);
const { createRustEngine } = await import(`${APP}/build/renderer/engine-rust.js`);

async function loadRust(dir) {
  if (!existsSync(`${dir}/tandem_rust_workflow.js`)) return null;
  const wasm = await import(`${dir}/tandem_rust_workflow.js`);
  wasm.initSync({ module: readFileSync(`${dir}/tandem_rust_workflow_bg.wasm`) });
  return createRustEngine(wasm);
}

export const mu = createEngine(mupdf);
export const rs = await loadRust(`${APP}/engine/web/pkg`);
export const rsF04 = process.env.NO_BASE ? null : await loadRust(process.env.RS_BASE_F04 ?? '-');
export const rsEd62 = process.env.NO_BASE ? null : await loadRust(process.env.RS_BASE_R2 ?? '-');
export const rsR2 = rsEd62;

export function fixture(name) {
  const b = readFileSync(`${APP}/fixtures/${name}`);
  return Buffer.from(b);
}
export function ab(buf) {
  const b = Buffer.isBuffer(buf) ? buf : Buffer.from(buf);
  const copy = Buffer.from(b);
  return copy.buffer.slice(copy.byteOffset, copy.byteOffset + copy.byteLength);
}

let nextId = 1;
export function call(engine, session, request) {
  const req = { id: nextId++, ...request };
  const { reply } = engine.dispatch(req, session);
  return reply;
}

export function norm(reply) {
  return JSON.parse(JSON.stringify(reply, (k, v) => {
    if (k === 'id') return undefined;
    if (k === 'openMs' || k === 'saveMs' || k === 'paintMs' || k === 'renderMs' || k === 'searchMs') return 0;
    if (v instanceof ArrayBuffer) return { $bytes: v.byteLength };
    if (k === 'ticket') return undefined;
    return v;
  }));
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

const LATIN = 'latin1';
export function rawPdf(objects, trailerExtra = '', { header = '%PDF-1.4\n', gens = {}, order = null, raw = {} } = {}) {
  const parts = [Buffer.from(header, LATIN)];
  const offsets = new Array(objects.length).fill(null);
  let size = parts[0].length;
  for (const index of (order ?? objects.map((_, i) => i))) {
    const body = objects[index];
    if (body === null) { continue; }
    if (raw[index + 1] !== undefined) { const bytes = Buffer.from(`${index + 1} ${gens[index + 1] ?? 0} obj\n${raw[index + 1]}`, LATIN); offsets[index] = [size, gens[index + 1] ?? 0]; parts.push(bytes); size += bytes.length; continue; }
    const gen = gens[index + 1] ?? 0;
    const bytes = Buffer.isBuffer(body) ? Buffer.concat([Buffer.from(`${index + 1} ${gen} obj\n`, LATIN), body, Buffer.from('\nendobj\n', LATIN)]) : Buffer.from(`${index + 1} ${gen} obj\n${body}\nendobj\n`, LATIN);
    offsets[index] = [size, gen];
    parts.push(bytes);
    size += bytes.length;
  }
  let tail = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const o of offsets) tail += o === null ? '0000000000 65535 f \n' : `${String(o[0]).padStart(10, '0')} ${String(o[1]).padStart(5, '0')} n \n`;
  tail += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R ${trailerExtra}>>\nstartxref\n${size}\n%%EOF\n`;
  parts.push(Buffer.from(tail, LATIN));
  return Buffer.concat(parts);
}
export const stream = (body, dict = '') => `<< ${dict} /Length ${Buffer.byteLength(body, LATIN)} >>\nstream\n${body}\nendstream`;
export const HELV = '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>';
export const broken = b => Buffer.from(b.toString('latin1').replace(/startxref\n\d+(?![\s\S]*startxref)/, 'startxref\n999999'), 'latin1');

/** Open, text, annots; optional save and MuPDF reading of the save. */
export function view(engine, bytes, { save = false, password } = {}) {
  if (!engine) return { ok: false, error: 'no engine' };
  const s = engine.createSession();
  const o = call(engine, s, { op: 'open', name: 'f.pdf', pdf: ab(bytes) });
  if (!o.ok) return { ok: false, error: o.error, s };
  let auth = null;
  if (password !== undefined && o.info.needsPassword) auth = call(engine, s, { op: 'authenticate', password });
  const t = call(engine, s, { op: 'text-all' });
  const ids = call(engine, s, { op: 'page-ids' });
  const pages = auth?.pageCount ?? o.pageCount;
  const annots = [];
  for (let p = 0; p < pages; p++) { const a = call(engine, s, { op: 'annots', page: p }); annots.push(a.ok ? a.items.length : 'E'); }
  const r = { ok: true, pages, rep: o.info.repaired, versions: o.info.versions, needs: o.info.needsPassword, auth: auth ? (auth.authenticated ?? auth.error) : null, text: t.ok ? t.text.replace(/\s/g, '') : `E:${t.error}`, ids: ids.ids, annots: annots.join(','), s, open: o };
  if (save) {
    const sv = call(engine, s, { op: 'save' });
    r.saveOk = sv.ok;
    if (sv.ok) {
      const v = call(engine, s, { op: 'validate', pdf: sv.bytes.slice(0) });
      r.validate = v.ok ? 'ok' : v.error;
      r.saved = Buffer.from(sv.bytes);
    } else r.saveErr = sv.error;
  }
  return r;
}
export const fmt = v => !v ? '-' : v.ok ? `p=${v.pages} rep=${v.rep} v=${v.versions} ids=${JSON.stringify(v.ids)} an=${v.annots} ${v.text.slice(0, 80)}` : `ERR ${String(v.error).slice(0, 100)}`;
