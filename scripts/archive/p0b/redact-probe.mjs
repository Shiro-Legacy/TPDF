#!/usr/bin/env node
/**
 * P0-b probe 2: mupdf redaction apply + sanitise, save incremental=no.
 *
 * Usage: node scripts/p0b/redact-probe.mjs <in.pdf> <out.pdf>
 * Tools: mupdf npm 1.28.1 (AGPL-3.0-or-later) wasm; Node (process.version).
 * Never shipped. Writes <out.pdf> and prints JSON (hits, snapshots, sanitise log).
 */
import { readFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import mupdf from 'mupdf';

const TOKEN = 'P0B-SECRET-7731';
const NEEDLE = new TextEncoder().encode(TOKEN);

function u8has(u8, n = NEEDLE) {
  outer: for (let i = 0; i <= u8.length - n.length; i++) {
    for (let j = 0; j < n.length; j++) if (u8[i + j] !== n[j]) continue outer;
    return true;
  }
  return false;
}

function u8count(u8, n = NEEDLE) {
  let c = 0;
  outer: for (let i = 0; i <= u8.length - n.length; i++) {
    for (let j = 0; j < n.length; j++) if (u8[i + j] !== n[j]) continue outer;
    c++;
  }
  return c;
}

function strHas(s) {
  return typeof s === 'string' && s.includes(TOKEN);
}

function quadRect(q) {
  const xs = [q[0], q[2], q[4], q[6]];
  const ys = [q[1], q[3], q[5], q[7]];
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}

function unionRects(quads) {
  let r = quadRect(quads[0]);
  for (const q of quads.slice(1)) {
    const s = quadRect(q);
    r = [Math.min(r[0], s[0]), Math.min(r[1], s[1]), Math.max(r[2], s[2]), Math.max(r[3], s[3])];
  }
  return r;
}

/** readStream on the indirect ref — resolve() drops isStream(). */
function streamU8(obj) {
  if (!obj || obj.isNull()) return new Uint8Array();
  try {
    if (obj.isStream()) return obj.readStream().asUint8Array();
  } catch { /* fall through */ }
  return new Uint8Array();
}

function objHasToken(obj, seen = new Set()) {
  if (!obj || obj.isNull()) return false;
  if (obj.isIndirect()) {
    const n = obj.asIndirect();
    if (seen.has(n)) return false;
    seen.add(n);
  }
  try {
    if (obj.isString()) return strHas(obj.asString());
    if (obj.isName()) return strHas(obj.asName());
    if (obj.isStream() && u8has(streamU8(obj))) return true;
    if (obj.isDictionary() || obj.isArray() || obj.isStream()) {
      let hit = false;
      obj.forEach((val) => { if (objHasToken(val, seen)) hit = true; });
      return hit;
    }
  } catch { return false; }
  return false;
}

function snapshot(pdf) {
  const trailer = pdf.getTrailer();
  const root = trailer.get('Root');
  const pages = [];
  for (let i = 0; i < pdf.countPages(); i++) {
    const page = pdf.loadPage(i);
    let text = '';
    try { text = page.toStructuredText().asText(); } catch { text = ''; }
    pages.push({
      i,
      text,
      textHas: strHas(text),
      contentsHas: objHasToken(page.getObject().get('Contents')),
    });
    page.destroy();
  }
  const xobjects = {};
  {
    const p0 = pdf.loadPage(0);
    const res = p0.getObject().get('Resources');
    const xo = res.isNull() ? null : res.get('XObject');
    if (xo && !xo.isNull()) {
      xo.forEach((val, key) => { xobjects[key] = objHasToken(val); });
    }
    p0.destroy();
  }
  const annots = [];
  const links = [];
  for (let i = 0; i < pdf.countPages(); i++) {
    const page = pdf.loadPage(i);
    for (const a of page.getAnnotations()) {
      annots.push({ type: a.getType(), contents: a.getContents(), contentsHas: strHas(a.getContents()), objHas: objHasToken(a.getObject()) });
    }
    for (const l of page.getLinks()) links.push({ uri: l.getURI(), has: strHas(l.getURI()) });
    page.destroy();
  }
  let outline = '';
  try {
    const walk = (xs) => (xs || []).forEach((it) => { outline += (it.title || '') + '\n'; if (it.down) walk(it.down); });
    walk(pdf.loadOutline());
  } catch { /* ignore */ }
  let embedNames = [];
  let embedBytesHas = false;
  try {
    const files = pdf.getEmbeddedFiles() || {};
    embedNames = Object.keys(files);
    for (const name of embedNames) {
      try {
        const buf = pdf.getEmbeddedFileContents(files[name]);
        if (buf && u8has(buf.asUint8Array())) embedBytesHas = true;
      } catch { /* ignore */ }
    }
  } catch { /* ignore */ }
  return {
    countVersions: pdf.countVersions(),
    infoHas: objHasToken(trailer.get('Info')),
    metadataHas: objHasToken(root.get('Metadata')),
    outlineHas: strHas(outline) || objHasToken(root.get('Outlines')),
    ocgHas: objHasToken(root.get('OCProperties')),
    structHas: objHasToken(root.get('StructTreeRoot')),
    acroHas: objHasToken(root.get('AcroForm')),
    namesHas: objHasToken(root.get('Names')),
    embedNamesHas: embedNames.some((n) => n.includes(TOKEN)),
    embedBytesHas,
    pages,
    xobjects,
    annots,
    links,
    extractHas: pages.some((p) => p.textHas),
  };
}

function redactPages(pdf) {
  const hits = [];
  for (let i = 0; i < pdf.countPages(); i++) {
    const page = pdf.loadPage(i);
    let found = [];
    try { found = page.search(TOKEN) || []; } catch (e) { hits.push({ page: i, searchError: String(e) }); }
    for (const quads of found) {
      if (!quads.length) continue;
      const rect = unionRects(quads);
      const annot = page.createAnnotation('Redact');
      annot.setRect(rect);
      try { annot.setQuadPoints(quads); } catch { /* rect-only */ }
      hits.push({ page: i, rect, quads: quads.length });
    }
    page.applyRedactions(
      true,
      mupdf.PDFPage.REDACT_IMAGE_PIXELS,
      mupdf.PDFPage.REDACT_LINE_ART_REMOVE_IF_TOUCHED,
      mupdf.PDFPage.REDACT_TEXT_REMOVE,
    );
    page.destroy();
  }
  return hits;
}

function sanitise(pdf) {
  const log = [];
  const seen = new Set();
  function sweep(obj, label) {
    if (!obj || obj.isNull()) return;
    if (obj.isIndirect()) {
      const n = obj.asIndirect();
      if (seen.has(n)) return;
      seen.add(n);
    }
    try {
      if (obj.isStream()) {
        const u = streamU8(obj);
        if (u8has(u)) {
          const next = new TextDecoder('latin1').decode(u).split(TOKEN).join('');
          obj.writeStream(next);
          log.push(`stream ${label}`);
        }
      }
    } catch (e) { log.push(`fail stream ${label}: ${e}`); }
    try {
      if (obj.isDictionary() || obj.isArray() || obj.isStream()) {
        obj.forEach((val, key) => {
          try {
            if (val.isString() && strHas(val.asString())) {
              obj.put(key, pdf.newString(val.asString().split(TOKEN).join('')));
              log.push(`string ${label}/${key}`);
              return;
            }
            if (val.isName() && strHas(val.asName())) {
              obj.put(key, pdf.newName('redacted'));
              log.push(`name ${label}/${key}`);
              return;
            }
          } catch (e) { log.push(`fail put ${label}/${key}: ${e}`); }
          sweep(val, `${label}/${key}`);
        });
      }
    } catch (e) { log.push(`fail walk ${label}: ${e}`); }
  }
  sweep(pdf.getTrailer(), 'trailer');
  return log;
}

function shareMap(pdf) {
  const refs = {};
  for (let i = 0; i < pdf.countPages(); i++) {
    const page = pdf.loadPage(i);
    const res = page.getObject().get('Resources');
    const xo = res.isNull() ? null : res.get('XObject');
    if (xo && !xo.isNull()) {
      xo.forEach((val, key) => {
        const n = val.isIndirect() ? val.asIndirect() : key;
        (refs[n] ||= []).push({ page: i, key });
      });
    }
    page.destroy();
  }
  return Object.fromEntries(Object.entries(refs).filter(([, v]) => v.length > 1));
}

function pixmapUniform(pdf, hits) {
  const out = [];
  const inset = 2; // ponytail: skip AA on the search-quad edge
  for (let i = 0; i < pdf.countPages(); i++) {
    const page = pdf.loadPage(i);
    const pix = page.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
    const w = pix.getWidth();
    const h = pix.getHeight();
    const n = pix.getNumberOfComponents();
    const pixels = pix.getPixels();
    for (const hit of hits.filter((x) => x.page === i && x.rect)) {
      const [x0, y0, x1, y1] = hit.rect;
      const px0 = Math.max(0, Math.floor(x0) + inset);
      const px1 = Math.min(w, Math.ceil(x1) - inset);
      const py0 = Math.max(0, Math.floor(y0) + inset);
      const py1 = Math.min(h, Math.ceil(y1) - inset);
      let first = null;
      let same = true;
      let sampled = 0;
      for (let y = py0; y < py1 && same; y++) {
        for (let x = px0; x < px1; x++) {
          const off = (y * w + x) * n;
          const key = `${pixels[off]},${pixels[off + 1]},${pixels[off + 2]}`;
          sampled++;
          if (first === null) first = key;
          else if (key !== first) { same = false; break; }
        }
      }
      const cx = Math.floor((x0 + x1) / 2);
      const cy = Math.floor((y0 + y1) / 2);
      const coff = (cy * w + cx) * n;
      out.push({
        page: i, rect: hit.rect, oneColour: same, colour: first, sampled,
        center: `${pixels[coff]},${pixels[coff + 1]},${pixels[coff + 2]}`,
      });
    }
    pix.destroy();
    page.destroy();
  }
  return out;
}

async function main() {
  const inp = process.argv[2];
  const outp = process.argv[3];
  if (!inp || !outp) {
    console.error('usage: node scripts/p0b/redact-probe.mjs <in.pdf> <out.pdf>');
    process.exit(2);
  }
  const pdf = mupdf.Document.openDocument(await readFile(inp), 'application/pdf');
  const before = snapshot(pdf);
  const sharedBefore = shareMap(pdf);
  const hits = redactPages(pdf);
  const afterMupdf = snapshot(pdf);
  const sanitiseLog = sanitise(pdf);
  const afterSan = snapshot(pdf);
  const pix = pixmapUniform(pdf, hits);
  const shared = shareMap(pdf);
  const noGc = pdf.saveToBuffer('incremental=no').asUint8Array();
  const saved = pdf.saveToBuffer('incremental=no,garbage=compact').asUint8Array();
  await writeFile(outp, saved);
  const reopened = mupdf.Document.openDocument(await readFile(outp), 'application/pdf');
  const report = {
    mupdf: '1.28.1',
    node: process.version,
    token: TOKEN,
    in: inp,
    out: outp,
    saveOptions: 'incremental=no,garbage=compact',
    saveOptionsNote: 'incremental=no alone keeps unreferenced original streams',
    searchHits: hits,
    pixmapRects: pix,
    sharedXObjectsBefore: sharedBefore,
    sharedXObjectsAfter: shared,
    savedBytes: saved.length,
    savedRawCount: u8count(saved),
    incrementalNoRawCount: u8count(noGc),
    incrementalNoBytes: noGc.length,
    before,
    afterMupdf,
    sanitiseLog,
    afterSan,
    afterSave: snapshot(reopened),
  };
  console.log(JSON.stringify(report, null, 2));
}

const isMain = process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url;
if (isMain) main().catch((e) => { console.error(e); process.exit(1); });
