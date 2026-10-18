#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Measures, through MuPDF's structured text (the app's node_modules copy),
// the Unicode MuPDF gives glyph names, the built-in encodings of its Symbol
// and ZapfDingbats faces, and its TrueType-UCS2 CMap, with synthetic PDFs.
// Candidate glyph names are the strings of MuPDF's WASM binary and their
// suffixes (the linker merges string tails) plus a few TeX names.
// usage: probe-text-unicode.mjs OUT.json
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const mupdf = (await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"))).default;
mupdf.setLog?.({ warning: () => {}, error: () => {} });

function pdf(objects) {
  const parts = [Buffer.from("%PDF-1.7\n")];
  let at = parts[0].length;
  const offsets = [];
  objects.forEach((o, i) => {
    offsets.push(at);
    const body = typeof o === "string"
      ? Buffer.from(`${i + 1} 0 obj\n${o}\nendobj\n`, "latin1")
      : Buffer.from(`${i + 1} 0 obj\n<< /Length ${o.length} >>\nstream\n${o.toString()}\nendstream\nendobj\n`, "latin1");
    parts.push(body);
    at += body.length;
  });
  let xref = `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const o of offsets) xref += `${String(o).padStart(10, "0")} 00000 n \n`;
  parts.push(Buffer.from(`${xref}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${at}\n%%EOF\n`));
  return Buffer.concat(parts);
}

/** Shows each code of `fonts` (font dictionaries, code lists) between brackets drawn in Helvetica. */
function bracketed(fonts) {
  const objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "", "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>", "<< /Length 14 >>\nstream\n0 0 0 0 0 0 d1\nendstream"];
  const procs = objects.length;
  let content = "";
  let line = 0;
  const names = [];
  for (const [dict, codes, hex] of fonts) {
    objects.push(dict.replaceAll("PROC", `${procs} 0 R`));
    const f = `F${objects.length}`;
    names.push(`/${f} ${objects.length} 0 R`);
    for (const c of codes) {
      content += `BT /H 2 Tf 0 ${-4 * line++} Td ([) Tj /${f} 2 Tf <${c.toString(16).padStart(hex, "0")}> Tj /H 2 Tf (\\]) Tj ET\n`;
    }
  }
  objects.push({ length: Buffer.byteLength(content, "latin1"), toString: () => content });
  objects[2] = `<< /Type /Page /Parent 2 0 R /MediaBox [0 ${-4 * line - 4} 100 4] /Resources << /Font << /H 4 0 R ${names.join(" ")} >> >> /Contents ${objects.length} 0 R >>`;
  const doc = mupdf.Document.openDocument(pdf(objects), "application/pdf");
  // One line per probe: "[", the probed characters, "]".
  const lines = [];
  doc.loadPage(0).toStructuredText("preserve-whitespace").walk({
    beginLine: () => lines.push([]),
    onChar: c => lines[lines.length - 1].push(...[...c].map(ch => ch.codePointAt(0))),
  });
  // MuPDF reorders right-to-left characters, so drop the first "[" and the last "]".
  const out = lines.map(l => {
    const open = l.indexOf(0x5b);
    const close = l.lastIndexOf(0x5d);
    return open < 0 || close <= open ? null : l.filter((_, i) => i !== open && i !== close);
  });
  if (out.length !== line || out.includes(null)) throw new Error(`expected ${line} bracketed lines, read ${out.length}`);
  return out;
}

const esc = n => [...Buffer.from(n, "latin1")].map(b => (b > 0x20 && b < 0x7f && !"#()<>[]{}/%".includes(String.fromCharCode(b)) ? String.fromCharCode(b) : `#${b.toString(16).padStart(2, "0")}`)).join("");

// Glyph names: Type3 fonts naming codes 128-255, where an unknown name reads U+FFFD.
const wasm = fs.readFileSync(path.join(root, "node_modules/mupdf/dist/mupdf-wasm.wasm"));
const candidates = new Set(["square", "triangle", "circlecopyrt"]);
for (const m of wasm.toString("latin1").matchAll(/\0([A-Za-z0-9]{1,40})(?=\0)/g)) {
  for (let i = 0; i < m[1].length; i++) if (/^[A-Za-z]/.test(m[1].slice(i))) candidates.add(m[1].slice(i));
}
const names = [...candidates].sort();
const glyphNames = {};
for (let start = 0; start < names.length; start += 128 * 40) {
  const fonts = [];
  for (let at = start; at < Math.min(names.length, start + 128 * 40); at += 128) {
    const chunk = names.slice(at, at + 128);
    fonts.push([`<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << ${chunk.map(n => `/${esc(n)} PROC`).join(" ")} >> /Encoding << /Differences [128 ${chunk.map(n => `/${esc(n)}`).join(" ")}] >> /FirstChar 128 /LastChar 255 /Widths [${Array(128).fill(500).join(" ")}] >>`, chunk.map((_, i) => 128 + i), 2]);
  }
  const got = bracketed(fonts);
  names.slice(start, start + 128 * 40).forEach((n, i) => {
    const cps = got[i];
    if (!(cps.length === 1 && cps[0] === 0xfffd)) glyphNames[n] = cps;
  });
}

// Built-in encodings of the non-embedded Symbol and ZapfDingbats faces.
const all = Array.from({ length: 256 }, (_, i) => i);
const [symbol, dingbats] = [["Symbol"], ["ZapfDingbats"]].map(([base]) => bracketed([[`<< /Type /Font /Subtype /Type1 /BaseFont /${base} >>`, all, 2]]));

// TrueType-UCS2: a non-embedded Identity-H CID font without ToUnicode.
const cids = Array.from({ length: 300 }, (_, i) => i);
const ttucs2 = bracketed([[`<< /Type /Font /Subtype /Type0 /BaseFont /Arial /Encoding /Identity-H /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Arial /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor << /Type /FontDescriptor /FontName /Arial /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >> >>] >>`, cids, 4]]);

// Characters structured text rewrites: presentation forms, one ToUnicode value per page.
const forms = [];
for (let v = 0xfb00; v <= 0xfdff; v++) forms.push(v);
for (let v = 0xfe70; v <= 0xfeff; v++) forms.push(v);
const rewritten = {};
{
  const map = forms.map((v, i) => `<${(i + 1).toString(16).padStart(4, "0")}> <${v.toString(16)}>`);
  let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /X def 1 begincodespacerange <0000> <FFFF> endcodespacerange\n";
  for (let i = 0; i < map.length; i += 100) cmap += `${Math.min(100, map.length - i)} beginbfchar\n${map.slice(i, i + 100).join("\n")}\nendbfchar\n`;
  cmap += "endcmap CMapName currentdict /CMap defineresource pop end end";
  const objects = ["<< /Type /Catalog /Pages 2 0 R >>", "", "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>", { length: cmap.length, toString: () => cmap },
    "<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding /Identity-H /ToUnicode 4 0 R /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /X /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor << /Type /FontDescriptor /FontName /X /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >> >>] >>"];
  const kids = [];
  forms.forEach((_, i) => {
    const content = `BT /F 10 Tf 10 10 Td <${(i + 1).toString(16).padStart(4, "0")}> Tj ET`;
    objects.push({ length: content.length, toString: () => content });
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 30] /Resources << /Font << /F 5 0 R >> >> /Contents ${objects.length} 0 R >>`);
    kids.push(`${objects.length} 0 R`);
  });
  objects[1] = `<< /Type /Pages /Kids [${kids.join(" ")}] /Count ${kids.length} >>`;
  const doc = mupdf.Document.openDocument(pdf(objects), "application/pdf");
  forms.forEach((v, i) => {
    const got = [...doc.loadPage(i).toStructuredText("preserve-whitespace").asText()].map(c => c.codePointAt(0)).filter(c => c !== 10);
    if (!(got.length === 1 && got[0] === v)) rewritten[v] = got;
  });
}

const out = {
  mupdf: JSON.parse(fs.readFileSync(path.join(root, "node_modules/mupdf/package.json"), "utf8")).version,
  candidates: names.length,
  glyphNames,
  symbol,
  dingbats,
  trueTypeUcs2: ttucs2,
  rewritten,
};
fs.writeFileSync(process.argv[2], JSON.stringify(out));
console.log(`${names.length} candidates, ${Object.keys(glyphNames).length} names read`);
