#!/usr/bin/env node
/**
 * P0-b probe 1: replace <old> with <new> in an existing content stream.
 *
 * Usage: node scripts/p0b/edit-probe.mjs <pdf> <old> <new> <out>
 * Tools: mupdf 1.28.1 wasm (AGPL-3.0-or-later); Node (process.version). Never shipped.
 *
 * Finds the run via toStructuredText, rewrites Tj/TJ literal or hex in the page
 * /Contents and Form XObjects, writeStream, save incremental=no,garbage=compact.
 * Refuses the rewrite if any char of <new> has no glyph in the run's font.
 */
import { readFile, writeFile } from 'node:fs/promises';
import mupdf from 'mupdf';

function pdfEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/\(/g, '\\(').replace(/\)/g, '\\)');
}

function streamText(obj) {
  if (!obj || obj.isNull()) return '';
  try {
    if (obj.isStream()) return new TextDecoder('latin1').decode(obj.readStream().asUint8Array());
  } catch { /* ignore */ }
  if (obj.isArray()) {
    let s = '';
    for (let i = 0; i < obj.length; i++) s += streamText(obj.get(i));
    return s;
  }
  if (obj.isIndirect()) {
    try { if (obj.isStream()) return new TextDecoder('latin1').decode(obj.readStream().asUint8Array()); } catch { /* ignore */ }
  }
  return '';
}

function writeStreamText(obj, text) {
  if (obj.isArray()) {
    // ponytail: concatenate into the first stream
    writeStreamText(obj.get(0), text);
    return;
  }
  obj.writeStream(text);
}

function runsOf(page) {
  const lines = [];
  let cur = null;
  page.toStructuredText().walk({
    beginLine(bbox) { cur = { bbox, chars: [] }; },
    onChar(c, origin, font, size, quad) { cur.chars.push({ c, origin, font, size, quad }); },
    endLine() { if (cur) lines.push(cur); cur = null; },
  });
  return lines;
}

function findOld(lines, old) {
  for (const line of lines) {
    const text = line.chars.map((ch) => ch.c).join('');
    const at = text.indexOf(old);
    if (at < 0) continue;
    const slice = line.chars.slice(at, at + [...old].length);
    return { line, slice, text, origin: slice[0]?.origin, font: slice[0]?.font, size: slice[0]?.size };
  }
  return null;
}

function encodeOf(font, text) {
  if (!font) return { missing: [...text], broken: true };
  const missing = [];
  let any = false;
  for (const ch of text) {
    let code = 0;
    try { code = font.encodeCharacter(ch); } catch { code = 0; }
    if (code) any = true;
    else missing.push(ch);
  }
  return { missing, broken: !any && text.length > 0 };
}

function hexFor(font, text) {
  return [...text].map((ch) => font.encodeCharacter(ch).toString(16).padStart(4, '0')).join('');
}

function rewriteOps(src, old, neu, font) {
  const hits = [];
  let out = src;
  const litOld = `(${pdfEscape(old)})`;
  const litNew = `(${pdfEscape(neu)})`;
  if (out.includes(litOld)) {
    const n = out.split(litOld).length - 1;
    out = out.split(litOld).join(litNew);
    hits.push({ kind: 'literal', n });
  }
  if (font) {
    try {
      const hOld = hexFor(font, old);
      const hNew = hexFor(font, neu);
      const a = `<${hOld}>`;
      const b = `<${hNew}>`;
      if (hOld && out.toLowerCase().includes(a.toLowerCase())) {
        const re = new RegExp(a.replace(/[<>]/g, '\\$&'), 'ig');
        const n = out.match(re)?.length || 0;
        out = out.replace(re, b);
        hits.push({ kind: 'hex', n, hOld, hNew });
      }
    } catch { /* encode failed */ }
  }
  return { out, hits, changed: out !== src };
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
  return refs;
}

function linkRect(page) {
  try {
    const links = page.getLinks() || [];
    if (links.length) return { uri: links[0].getURI(), bounds: links[0].getBounds() };
  } catch { /* COS fallback */ }
  try {
    const annots = page.getObject().get('Annots');
    if (!annots || annots.isNull()) return null;
    for (let i = 0; i < annots.length; i++) {
      const o = annots.get(i).resolve();
      if (o.get('Subtype').asName() !== 'Link') continue;
      const r = o.get('Rect');
      const A = o.get('A');
      let uri = '';
      if (A && !A.isNull()) {
        const u = A.get('URI');
        if (u && !u.isNull() && u.isString()) uri = u.asString();
      }
      return { uri, rect: [0, 1, 2, 3].map((k) => r.get(k).asNumber()) };
    }
  } catch { return null; }
  return null;
}

function actualText(pdf) {
  const root = pdf.getTrailer().get('Root').get('StructTreeRoot');
  if (root.isNull()) return [];
  const out = [];
  const seen = new Set();
  function walk(obj) {
    if (!obj || obj.isNull()) return;
    if (obj.isIndirect()) {
      const n = obj.asIndirect();
      if (seen.has(n)) return;
      seen.add(n);
    }
    try {
      const t = obj.get?.('ActualText');
      if (t && !t.isNull() && t.isString()) out.push(t.asString());
    } catch { /* ignore */ }
    try {
      if (obj.isDictionary() || obj.isArray()) obj.forEach((v) => walk(v));
    } catch { /* ignore */ }
  }
  walk(root);
  return out;
}

function pixmap(page) {
  const pix = page.toPixmap(mupdf.Matrix.identity, mupdf.ColorSpace.DeviceRGB, false);
  return { w: pix.getWidth(), h: pix.getHeight(), n: pix.getNumberOfComponents(), pixels: pix.getPixels(), pix };
}

async function main() {
  const [, , inp, old, neu, outp] = process.argv;
  if (!inp || !old || !neu || !outp) {
    console.error('usage: node scripts/p0b/edit-probe.mjs <pdf> <old> <new> <out>');
    process.exit(2);
  }
  const pdf = mupdf.Document.openDocument(await readFile(inp), 'application/pdf');
  const sharing = shareMap(pdf);
  const shared = Object.fromEntries(Object.entries(sharing).filter(([, v]) => v.length > 1));
  const before = { pages: [], extract: [], links: [], actual: actualText(pdf) };
  let font = null;
  let missing = [];
  let encodeBroken = false;
  const found = [];
  for (let i = 0; i < pdf.countPages(); i++) {
    const page = pdf.loadPage(i);
    const lines = runsOf(page);
    before.pages.push(lines.map((l) => ({
      text: l.chars.map((c) => c.c).join(''),
      origin: l.chars[0]?.origin,
      bbox: l.bbox,
    })));
    before.extract.push(page.toStructuredText().asText());
    before.links.push(linkRect(page));
    const hit = findOld(lines, old);
    if (hit) {
      font = hit.font;
      const enc = encodeOf(hit.font, neu);
      encodeBroken = enc.broken;
      missing = enc.missing;
      if (enc.broken) {
        const present = new Set(before.extract.join(''));
        missing = [...neu].filter((ch) => !present.has(ch));
      }
      found.push({ page: i, origin: hit.origin, size: hit.size, text: hit.text });
    }
    page.destroy();
  }

  const rewrites = [];
  if (missing.length === 0 && found.length) {
    for (let i = 0; i < pdf.countPages(); i++) {
      const page = pdf.loadPage(i);
      const contents = page.getObject().get('Contents');
      const src = streamText(contents);
      const r = rewriteOps(src, old, neu, font);
      if (r.changed) {
        writeStreamText(contents, r.out);
        rewrites.push({ page: i, where: 'contents', hits: r.hits });
      }
      const res = page.getObject().get('Resources');
      const xo = res.isNull() ? null : res.get('XObject');
      if (xo && !xo.isNull()) {
        xo.forEach((val, key) => {
          const srcX = streamText(val);
          if (!srcX) return;
          const rx = rewriteOps(srcX, old, neu, font);
          if (rx.changed) {
            writeStreamText(val, rx.out);
            rewrites.push({ page: i, where: `XObject/${key}`, hits: rx.hits, obj: val.isIndirect() ? val.asIndirect() : null });
          }
        });
      }
      page.destroy();
    }
    // ActualText
    const root = pdf.getTrailer().get('Root').get('StructTreeRoot');
    if (!root.isNull()) {
      const seen = new Set();
      (function walk(obj) {
        if (!obj || obj.isNull()) return;
        if (obj.isIndirect()) {
          const n = obj.asIndirect();
          if (seen.has(n)) return;
          seen.add(n);
        }
        try {
          const t = obj.get?.('ActualText');
          if (t && !t.isNull() && t.isString() && t.asString() === old) {
            obj.put('ActualText', pdf.newString(neu));
            rewrites.push({ where: 'ActualText' });
          }
        } catch { /* ignore */ }
        try { if (obj.isDictionary() || obj.isArray()) obj.forEach((v) => walk(v)); } catch { /* ignore */ }
      })(root);
    }
  }

  const buf = pdf.saveToBuffer('incremental=no,garbage=compact');
  const saved = buf.asUint8Array();
  await writeFile(outp, saved);
  const afterDoc = mupdf.Document.openDocument(await readFile(outp), 'application/pdf');
  const after = { pages: [], extract: [], links: [], actual: actualText(afterDoc), countVersions: afterDoc.countVersions() };
  for (let i = 0; i < afterDoc.countPages(); i++) {
    const page = afterDoc.loadPage(i);
    const lines = runsOf(page);
    after.pages.push(lines.map((l) => ({
      text: l.chars.map((c) => c.c).join(''),
      origin: l.chars[0]?.origin,
      bbox: l.bbox,
    })));
    after.extract.push(page.toStructuredText().asText());
    after.links.push(linkRect(page));
    page.destroy();
  }

  const neighbours = [];
  const nPages = Math.min(before.pages.length, after.pages.length);
  for (let i = 0; i < nPages; i++) {
    for (const bl of before.pages[i]) {
      if (bl.text.includes(old)) continue;
      const match = after.pages[i].find((al) => al.text === bl.text);
      if (!match) {
        neighbours.push({ page: i, text: bl.text, status: 'missing-after' });
        continue;
      }
      const dx = (match.origin?.[0] ?? 0) - (bl.origin?.[0] ?? 0);
      const dy = (match.origin?.[1] ?? 0) - (bl.origin?.[1] ?? 0);
      neighbours.push({ page: i, text: bl.text, dx, dy, ok: Math.abs(dx) <= 0.5 && Math.abs(dy) <= 0.5 });
    }
  }

  const raw = Buffer.from(saved);
  const report = {
    mupdf: '1.28.1',
    node: process.version,
    in: inp,
    out: outp,
    old,
    new: neu,
    saveOptions: 'incremental=no,garbage=compact',
    found,
    missingGlyphs: missing,
    encodeCharacterBroken: encodeBroken,
    rewrote: missing.length === 0 && found.length > 0,
    rewrites,
    sharedXObjects: shared,
    extractBefore: before.extract,
    extractAfter: after.extract,
    extractHasNew: after.extract.some((t) => t.includes(neu)),
    extractHasOld: after.extract.some((t) => t.includes(old)),
    rawHasOld: raw.includes(old),
    rawHasNew: raw.includes(neu),
    countVersions: after.countVersions,
    neighbours,
    actualBefore: before.actual,
    actualAfter: after.actual,
    linksBefore: before.links,
    linksAfter: after.links,
    savedBytes: saved.length,
  };
  console.log(JSON.stringify(report, null, 2));
}

main().catch((e) => { console.error(e); process.exit(1); });
