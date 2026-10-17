#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// The generated pages of reference/app-view-20261015: base 14 Latin per face,
// MuPDF's own Greek, Cyrillic and CJK fallback fonts (written by mupdf.js, so
// exactly the fonts the forms group writes), and the view-cost stress files.
// usage (repo root): node engine/oracle/app-view-cases.mjs OUT_DIR
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const mupdf = (await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"))).default;
const out = process.argv[2];
if (!out) throw new Error("usage: app-view-cases.mjs OUT_DIR");
fs.mkdirSync(out, { recursive: true });

/** A classic-xref PDF of object bodies (Buffers or latin1 strings), numbered from 1. */
function pdf(objects) {
  const parts = [Buffer.from("%PDF-1.7\n")];
  const offsets = [];
  let at = parts[0].length;
  objects.forEach((body, i) => {
    offsets.push(at);
    const chunk = Buffer.concat([Buffer.from(`${i + 1} 0 obj\n`), Buffer.from(body, "latin1"), Buffer.from("\nendobj\n")]);
    parts.push(chunk);
    at += chunk.length;
  });
  const xref = [`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`];
  for (const o of offsets) xref.push(`${String(o).padStart(10, "0")} 00000 n \n`);
  xref.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${at}\n%%EOF\n`);
  parts.push(Buffer.from(xref.join("")));
  return Buffer.concat(parts);
}
const stream = (data) => `<< /Length ${Buffer.byteLength(data, "latin1")} >>\nstream\n${data}\nendstream`;

// Latin in each base 14 text face (WinAnsi), eight sizes.
const faces = ["Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique", "Times-Roman", "Times-Bold",
  "Times-Italic", "Times-BoldItalic", "Courier", "Courier-Bold", "Courier-Oblique", "Courier-BoldOblique"];
const lines = ["The quick brown fox jumps over the lazy dog.", "PACK MY BOX WITH FIVE DOZEN LIQUOR JUGS 0123456789",
  "\\(Hello\\), world! @#$%&*+=?[]{}|~ \\251 \\256 \\341\\351\\361\\374"];
for (const face of faces) {
  const content = [];
  let y = 560;
  for (const size of [7, 9, 10, 12, 14, 18, 24, 36]) {
    for (const text of lines) {
      content.push(`BT /F1 ${size} Tf 20 ${y} Td (${text}) Tj ET`);
      y -= size * 1.25;
    }
  }
  fs.writeFileSync(path.join(out, `face-${face}.pdf`), pdf([
    "<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 700 580] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
    stream(content.join("\n")),
    `<< /Type /Font /Subtype /Type1 /BaseFont /${face} /Encoding /WinAnsiEncoding >>`,
  ]));
}

// MuPDF's simple fallback fonts: Greek (ISO 8859-7) and Cyrillic (KOI8-U).
{
  const doc = new mupdf.PDFDocument();
  const res = doc.newDictionary();
  const fonts = doc.newDictionary();
  res.put("Font", fonts);
  const koi8 = "юабцдефгхийклмнопярстужвьызшэщчъЮАБЦДЕФГХИЙКЛМНОПЯРСТУЖВЬЫЗШЭЩЧЪ";
  const koi = (s) => Buffer.from([...s].map((c) => (koi8.includes(c) ? 0xc0 + koi8.indexOf(c) : c.charCodeAt(0)))).toString("hex");
  const iso7 = (s) => Buffer.from([...s].map((c) => {
    const u = c.codePointAt(0);
    return u >= 0x0386 && u <= 0x03ce ? u - 0x0386 + 0xb6 : u;
  })).toString("hex");
  let content = "";
  let y = 760;
  ["Helvetica", "Helvetica-Bold", "Helvetica-BoldOblique", "Times-Roman", "Helvetica-Oblique", "Times-Bold", "Times-Italic", "Courier"]
    .forEach((face, i) => {
      const font = new mupdf.Font(face);
      fonts.put(`L${i}`, doc.addSimpleFont(font, "Latin"));
      fonts.put(`C${i}`, doc.addSimpleFont(font, "Cyrillic"));
      fonts.put(`G${i}`, doc.addSimpleFont(font, "Greek"));
      content += `BT /L${i} 14 Tf 40 ${y} Td (${face}) Tj ET\n`;
      y -= 22;
      content += `BT /C${i} 18 Tf 40 ${y} Td <${koi("Привет мир АБВГДЕЖЗИЙКЛМН абвгдежзийклмн")}> Tj ET\n`;
      y -= 22;
      content += `BT /G${i} 18 Tf 40 ${y} Td <${iso7("Ελληνικά ΑΒΓΔΕΖΗΘ αβγδεζηθικλμν")}> Tj ET\n`;
      y -= 30;
    });
  doc.insertPage(-1, doc.addPage([0, 0, 612, 792], 0, res, content));
  fs.writeFileSync(path.join(out, "mupdf-simple.pdf"), doc.saveToBuffer("").asUint8Array());
}

// MuPDF's CJK fonts (pdf_add_cjk_font: Type0, Uni*-UTF16-H, no program).
{
  const doc = new mupdf.PDFDocument();
  const res = doc.newDictionary();
  const fonts = doc.newDictionary();
  res.put("Font", fonts);
  let content = "";
  let y = 740;
  [["zh-Hans", "日本語中文简体字测试"], ["ja", "日本語のひらがなカタカナ"], ["ko", "한국어 문자 시험"],
    ["zh-Hant", "繁體中文測試"], ["zh-Hans", "日 1Ab€≤"]].forEach(([lang, text], i) => {
    fonts.put(`K${i}`, doc.addCJKFont(new mupdf.Font(lang), lang, 0, true));
    content += `BT /K${i} 24 Tf 40 ${y} Td <${Buffer.from(text, "utf16le").swap16().toString("hex")}> Tj ET\n`;
    y -= 50;
  });
  doc.insertPage(-1, doc.addPage([0, 0, 612, 792], 0, res, content));
  fs.writeFileSync(path.join(out, "mupdf-cjk.pdf"), doc.saveToBuffer("").asUint8Array());
}

// N pages sharing one /Annots array object of N Square annotations.
for (const n of [1000, 4000]) {
  const first = 7;
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    `<< /Type /Pages /Kids [${Array.from({ length: n }, (_, i) => `${first + i} 0 R`).join(" ")}] /Count ${n} >>`,
    stream("BT /F1 24 Tf 72 700 Td (Shared annots) Tj ET"),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    `[${Array.from({ length: n }, (_, i) => `${first + n + i} 0 R`).join(" ")}]`,
    "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Length 23 >>\nstream\n1 0 0 rg 0 0 10 10 re f\nendstream",
  ];
  for (let i = 0; i < n; i++) {
    objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 3 0 R /Annots 5 0 R >>");
  }
  for (let i = 0; i < n; i++) {
    const x = 20 + (i % 50) * 11;
    const y = 20 + (Math.floor(i / 50) % 60) * 11;
    objects.push(`<< /Type /Annot /Subtype /Square /Rect [${x} ${y} ${x + 10} ${y + 10}] /F 4 /AP << /N 6 0 R >> >>`);
  }
  fs.writeFileSync(path.join(out, `shared-annots-${n}.pdf`), pdf(objects));
}

// One page and 300,000 small unreferenced objects.
{
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
    stream("BT /F1 24 Tf 72 700 Td (Many objects) Tj ET"), "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  for (let i = 0; i < 300000; i++) objects.push(`<< /K [${i} [1 2 3] << /A (x) >>] >>`);
  fs.writeFileSync(path.join(out, "many-objects.pdf"), pdf(objects));
}
