#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Widget rendering of the form ops, Rust against MuPDF: each scenario runs
// the same app requests through engine.ts on MuPDF and through the Rust
// engine (tests/rust-engine.mjs), renders every page with the `render` op
// and compares the pixels inside each widget's rect, per field kind.
// usage (repo root, after npm run build and tools/build-workflow.sh):
//   node engine/tools/app-forms-oracle.mjs [summary.json] [--dump DIR]
//   node engine/tools/app-forms-oracle.mjs --synthesis synthesis.json
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const mupdf = (await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"))).default;
const { createEngine: createMupdf } = await import(path.join(root, "build/renderer/engine.js"));
const { createEngine: createRust } = await import(path.join(root, "tests/rust-engine.mjs"));
const args = process.argv.slice(2);
const dumpAt = args.indexOf("--dump");
const dump = dumpAt >= 0 ? args.splice(dumpAt, 2)[1] : null;
const out = args[0];
const SCALE = 2;

const ab = (b) => b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength);
const fixture = (name) => ab(fs.readFileSync(path.join(root, "fixtures", name)));

/** Object-numbering-safe PDF from a 1-based list of indirect object bodies. */
function buildPdf(objects) {
  const parts = ["%PDF-1.4\n"];
  const offsets = [];
  objects.forEach((body, index) => {
    offsets.push(parts.join("").length);
    parts.push(`${index + 1} 0 obj\n${body}\nendobj\n`);
  });
  const xref = parts.join("").length;
  parts.push(`xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`);
  for (const offset of offsets) parts.push(`${String(offset).padStart(10, "0")} 00000 n \n`);
  parts.push(`trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`);
  return ab(Buffer.from(parts.join(""), "latin1"));
}

/** One page of `widgets` (bodies, numbered from 7) with an AcroForm; `acro` adds keys. */
function formPdf(widgets, { acro = "", page = "/MediaBox [0 0 612 792]" } = {}) {
  const n = widgets.length;
  const refs = widgets.map((_w, i) => `${7 + i} 0 R`).join(" ");
  return buildPdf([
    "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    `<< /Type /Page /Parent 2 0 R ${page} /Contents 4 0 R /Annots [${refs}] >>`,
    "<< /Length 2 >>\nstream\nq \nendstream",
    `<< /Fields [${refs}] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 6 0 R /ZaDb ${7 + n} 0 R /TiRo ${8 + n} 0 R /Cour ${9 + n} 0 R >> >> ${acro} >>`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    ...widgets.map((body) => `<< /Type /Annot /Subtype /Widget /F 4 /P 3 0 R ${body} >>`),
    "<< /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>",
  ]);
}

const kids = (y) => [{ on: "A", rect: [300, y, 316, y + 16] }, { on: "B", rect: [330, y, 354, y + 24] }];
const longText = "The quick brown fox jumps over the lazy dog, then wraps onto the next line of a multiline field.";

/** Each scenario: a document and the app requests that build and fill its fields. */
const scenarios = [
  {
    name: "designer: add each kind, empty",
    pdf: () => fixture("hello.pdf"),
    steps: [
      { op: "field-add", kind: "text", name: "t1", rect: [72, 300, 252, 324] },
      { op: "field-add", kind: "text", name: "t2", rect: [72, 340, 252, 400], multiline: true },
      { op: "field-add", kind: "checkbox", name: "c1", rect: [72, 420, 88, 436] },
      { op: "field-add", kind: "combo", name: "d1", options: ["one", "two", "three"], rect: [72, 460, 252, 484] },
      { op: "field-add", kind: "radio", name: "g1", kids: kids(500) },
    ],
  },
  {
    name: "designer: add with defaults, then fill",
    pdf: () => fixture("hello.pdf"),
    steps: [
      { op: "field-add", kind: "text", name: "t1", rect: [72, 300, 252, 324], default: "Ann" },
      { op: "field-add", kind: "text", name: "t2", rect: [72, 340, 252, 420], multiline: true },
      { op: "field-add", kind: "text", name: "t3", rect: [72, 430, 252, 454], maxLen: 8 },
      { op: "field-add", kind: "checkbox", name: "c1", rect: [72, 460, 88, 476], default: true },
      { op: "field-add", kind: "checkbox", name: "c2", rect: [100, 460, 130, 490] },
      { op: "field-add", kind: "combo", name: "d1", options: ["one", "two", "three"], rect: [72, 500, 252, 524], default: "two" },
      { op: "field-add", kind: "radio", name: "g1", kids: kids(540), default: "B" },
      { set: "t2", value: longText },
      { set: "t3", value: "Short" },
      { set: "c2", value: true },
      { set: "d1", value: "three" },
      { set: "g1", on: "A", value: true },
    ],
  },
  {
    name: "designer: props (rect, multiline, maxLen, options, edit)",
    pdf: () => fixture("hello.pdf"),
    steps: [
      { op: "field-add", kind: "text", name: "t1", rect: [72, 300, 252, 324] },
      { set: "t1", value: "Hello world" },
      { props: "t1", rect: [72, 330, 300, 380] },
      { props: "t1", multiline: true },
      { op: "field-add", kind: "text", name: "t2", rect: [320, 300, 500, 324] },
      { set: "t2", value: "abc" },
      { props: "t2", maxLen: 5 },
      { op: "field-add", kind: "combo", name: "d1", options: ["one", "two"], rect: [72, 400, 252, 424] },
      { props: "d1", options: ["alpha", "beta"] },
      { props: "d1", edit: true },
      { set: "d1", value: "typed" },
      { op: "field-add", kind: "checkbox", name: "c1", rect: [72, 440, 88, 456] },
      { set: "c1", value: true },
      { props: "c1", rect: [100, 440, 140, 480] },
    ],
  },
  {
    name: "designer: CropBox page and /Rotate 90 page",
    pdf: () => fixture("p3/form-design.pdf"),
    pages: [0, 1],
    steps: [0, 1].flatMap((page) => [
      { op: "field-add", page, kind: "text", name: `t${page}`, rect: [72, 72, 252, 96], default: "Rotated" },
      { op: "field-add", page, kind: "checkbox", name: `c${page}`, rect: [72, 120, 88, 136], default: true },
      { op: "field-add", page, kind: "radio", name: `g${page}`, kids: [{ on: "A", rect: [100, 160, 116, 176] }, { on: "B", rect: [130, 160, 146, 176] }], default: "A" },
    ]),
  },
  {
    name: "fill: form2.pdf",
    pdf: () => fixture("form2.pdf"),
    steps: [
      { set: "name", value: "PDF4All P2 form" },
      { set: "notes", value: "line one\nline two is a much longer line that should wrap around" },
      { set: "agree", value: true },
      { set: "size", on: "M", value: true },
      { set: "choice", value: "two" },
      { set: "pick", value: "beta" },
    ],
  },
  {
    name: "fill: form.pdf",
    pdf: () => fixture("form.pdf"),
    steps: [{ set: "name", value: "PDF4All P0" }],
  },
  {
    // Counted as kind "text-intl": MuPDF's Greek, Cyrillic and CID fallback
    // fonts, and a value no font has (Polish: lossy, left to the viewer).
    name: "fill: text outside WinAnsi",
    pdf: () => fixture("hello.pdf"),
    kindSuffix: "-intl",
    steps: [
      ["cyr", "Привет мир"],
      ["grk", "Ελληνικά"],
      ["cjk", "日本語の文字"],
      ["kor", "한국어"],
      ["mix", "Hi Привет 日本 Ελλ x"],
      ["pol", "Łódź"],
    ].flatMap(([name, value], i) => [
      { op: "field-add", kind: "text", name, rect: [72, 300 + 32 * i, 352, 324 + 32 * i] },
      { set: name, value },
    ]),
  },
  {
    name: "view: widgets without /AP",
    pdf: () => formPdf([
      "/FT /Tx /T (a) /V (Hello world) /Rect [72 700 252 724] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [1] >>",
      "/FT /Tx /T (b) /V (Auto sized text) /Rect [72 660 172 684] /DA (/Helv 0 Tf 0 0 1 rg)",
      `/FT /Tx /Ff 4096 /T (c) /V (${longText}) /Rect [72 560 252 650] /DA (/TiRo 11 Tf 0 g) /MK << /BC [1 0 0] >>`,
      "/FT /Tx /Ff 16777216 /MaxLen 6 /T (d) /V (abc123) /Rect [72 520 252 544] /DA (/Cour 12 Tf 0 g) /MK << /BC [0] >>",
      "/FT /Tx /Q 1 /T (e) /V (centred) /Rect [300 700 480 724] /DA (/Helv 12 Tf 0 g) /BS << /W 2 >> /MK << /BC [0 0 1] /BG [0.9] >>",
      "/FT /Tx /Q 2 /T (f) /V (right) /Rect [300 660 480 684] /DA (/Helv 12 Tf 0 g)",
      "/FT /Ch /Ff 131072 /T (g) /V (two) /Opt [(one) (two)] /Rect [300 620 480 644] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [1] >>",
      "/FT /Ch /T (h) /V (beta) /Opt [(alpha) (beta) (gamma)] /Rect [300 520 480 600] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] >>",
      "/FT /Btn /T (i) /V /Yes /AS /Yes /Rect [72 470 88 486] /DA (/ZaDb 0 Tf 0 g) /MK << /BC [0] /BG [1] >>",
      "/FT /Btn /T (j) /V /Off /AS /Off /Rect [100 470 116 486] /DA (/ZaDb 0 Tf 0 g) /MK << /BC [0] /BG [1] >>",
      "/FT /Btn /Ff 32768 /T (k) /V /A /AS /A /Rect [130 470 150 490] /DA (/ZaDb 0 Tf 0 g) /MK << /BC [0] /BG [1] >>",
      "/FT /Btn /Ff 65536 /T (l) /Rect [300 460 400 490] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /CA (Push) >>",
      "/FT /Tx /T (m) /V (Rotated) /Rect [500 520 524 700] /DA (/Helv 12 Tf 0 g) /MK << /R 90 /BC [0] >>",
    ]),
    steps: [],
  },
  {
    name: "view: /NeedAppearances",
    pdf: () => formPdf([
      "/FT /Tx /T (a) /V (Needs appearance) /Rect [72 700 252 724] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] >> /AP << /N 99 0 R >>",
      "/FT /Tx /Ff 4096 /T (b) /V (two\\nlines) /Rect [72 640 252 690] /DA (/Helv 10 Tf 0 g)",
      "/FT /Btn /T (c) /V /Yes /AS /Yes /Rect [72 600 92 620] /DA (/ZaDb 0 Tf 0 g) /MK << /BC [0] >>",
      "/FT /Ch /Ff 131072 /T (d) /V (b) /Opt [(a) (b)] /Rect [72 560 252 584] /DA (/Helv 12 Tf 0 g)",
    ], { acro: "/NeedAppearances true" }),
    steps: [],
  },
];

/**
 * Synthesis cases: one widget each, with the /DR fonts Helv, TiRo, Cour and
 * ZaDb. `--synthesis FILE` records MuPDF's own appearance streams for them
 * (the value set through setTextValue, setChoiceValue or toggle, then
 * pdf_update_appearance after a setRect); tests/app_forms.rs compares the
 * Rust synthesis with the recorded streams.
 */
const W = (body) => body;
const synthesisCases = [
  ["tx_basic", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Hello"],
  ["tx_empty", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [1] >>"), ""],
  ["tx_auto", W("/FT /Tx /T (x) /Rect [100 650 280 674] /DA (/Helv 0 Tf 0 g)"), "Hello"],
  ["tx_auto_long", W("/FT /Tx /T (x) /Rect [100 700 200 724] /DA (/Helv 0 Tf 0 g)"), "The quick brown fox jumps over the lazy dog"],
  ["tx_auto_tall", W("/FT /Tx /T (x) /Rect [100 600 280 700] /DA (/Helv 0 Tf 0 g)"), "Hi"],
  ["tx_mk", W("/FT /Tx /T (x) /Rect [100 600 280 624] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [1] >>"), "Hello world"],
  ["tx_multi", W("/FT /Tx /Ff 4096 /T (x) /Rect [100 500 280 580] /DA (/Helv 12 Tf 0 g)"), "line one\nline two is a much longer line that should wrap around"],
  ["tx_multi_auto", W("/FT /Tx /Ff 4096 /T (x) /Rect [100 600 280 700] /DA (/Helv 0 Tf 0 g)"), "Hi there this is a long multiline text that wraps a few times over the width of the box"],
  ["tx_multi_breaks", W("/FT /Tx /Ff 4096 /T (x) /Rect [100 600 280 700] /DA (/Helv 10 Tf 0 g)"), "a\r\nb\rc"],
  ["tx_multi_q1", W("/FT /Tx /Ff 4096 /Q 1 /T (x) /Rect [100 600 280 700] /DA (/Helv 12 Tf 0 g)"), "centered\ntext that is long enough to wrap around once"],
  ["tx_multi_q2", W("/FT /Tx /Ff 4096 /Q 2 /T (x) /Rect [100 600 280 700] /DA (/Helv 12 Tf 0 g)"), "right\naligned text"],
  ["tx_longword", W("/FT /Tx /Ff 4096 /T (x) /Rect [100 600 160 700] /DA (/Helv 12 Tf 0 g)"), "Supercalifragilisticexpialidocious word"],
  ["tx_times", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/TiRo 12 Tf 0 g)"), "Hello"],
  ["tx_courier_multi", W("/FT /Tx /Ff 4096 /T (x) /Rect [100 600 280 700] /DA (/Cour 10 Tf 0 g)"), "a\nb"],
  ["tx_comb", W("/FT /Tx /Ff 16777216 /MaxLen 5 /T (x) /Rect [100 450 280 474] /DA (/Helv 12 Tf 0 g)"), "abc"],
  ["tx_comb_lines", W("/FT /Tx /Ff 16777216 /MaxLen 6 /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] >>"), "ab(c"],
  ["tx_comb_auto", W("/FT /Tx /Ff 16777216 /MaxLen 6 /Q 1 /T (x) /Rect [100 700 280 724] /DA (/Helv 0 Tf 0 g)"), "ab"],
  ["tx_q1", W("/FT /Tx /Q 1 /T (x) /Rect [100 400 280 424] /DA (/Helv 12 Tf 0 g)"), "Hi"],
  ["tx_q2_red", W("/FT /Tx /Q 2 /T (x) /Rect [100 350 280 374] /DA (/Helv 12 Tf 1 0 0 rg)"), "Hi"],
  ["tx_cmyk", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 0 0 1 k) /MK << /BC [1 0 0 0] /BG [0 0 1 0] >>"), "C"],
  ["tx_rgb", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /MK << /BC [1 0 0] /BG [0 0 1] >>"), "C"],
  ["tx_bs0", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /BS << /W 0 >> /MK << /BC [0] /BG [1] >>"), "C"],
  ["tx_bs3", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /BS << /W 3 >> /MK << /BC [0] /BG [1] >>"), "C"],
  ["tx_border", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /Border [0 0 2] /MK << /BC [0] >>"), "C"],
  ["tx_r90", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /MK << /R 90 /BC [0] >>"), "Rot"],
  ["tx_r180", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /MK << /R 180 /BC [0] >>"), "Rot"],
  ["tx_gray_da", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (0.5 g /Helv 9 Tf)"), "G"],
  ["tx_no_da", W("/FT /Tx /T (x) /Rect [100 700 280 724]"), "NoDA"],
  ["tx_missing_font", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Foo 12 Tf 0 g)"), "Foo"],
  ["tx_escapes", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "a(b)c\\d é"],
  ["tx_tiny", W("/FT /Tx /T (x) /Rect [100 700 110 706] /DA (/Helv 12 Tf 0 g)"), "tiny"],
  ["tx_tiny_auto", W("/FT /Tx /T (x) /Rect [100 700 110 706] /DA (/Helv 0 Tf 0 g)"), "tiny"],
  ["tx_password", W("/FT /Tx /Ff 8192 /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "secret"],
  ["ch_combo", W("/FT /Ch /Ff 131072 /T (x) /V (one) /Opt [(one) (two) (three)] /Rect [100 450 280 474] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [1] >>"), "three"],
  ["ch_combo_auto", W("/FT /Ch /Ff 131072 /T (x) /V (a) /Opt [(a) (b)] /Rect [100 700 280 724] /DA (/Helv 0 Tf 0 g)"), "b"],
  ["ch_combo_q1", W("/FT /Ch /Ff 131072 /Q 1 /T (x) /V (a) /Opt [(a) (b)] /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "b"],
  ["ch_combo_pairs", W("/FT /Ch /Ff 131072 /T (x) /V (e1) /Opt [[(e1) (Display One)] [(e2) (Display Two)]] /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "e2"],
  ["ch_list", W("/FT /Ch /T (x) /V (beta) /Opt [(alpha) (beta) (gamma)] /Rect [100 350 280 410] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [1] >>"), "gamma"],
  ["ch_list_ti", W("/FT /Ch /T (x) /V (b) /TI 1 /Opt [(a) (b) (c) (d)] /Rect [100 600 280 700] /DA (/Helv 12 Tf 0 g)"), "c"],
  ["ch_list_long", W("/FT /Ch /T (x) /V (b) /Opt [(a long option that is wider than the box itself)] /Rect [100 600 200 700] /DA (/Helv 12 Tf 0 g)"), "a long option that is wider than the box itself"],
  ["btn_check16", W("/FT /Btn /T (x) /V /Off /AS /Off /Rect [100 700 116 716] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [1] >>"), true],
  ["btn_check30x20", W("/FT /Btn /T (x) /V /Off /AS /Off /Rect [100 700 130 720] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [1] >>"), true],
  ["btn_check_plain", W("/FT /Btn /T (x) /V /Off /AS /Off /Rect [100 600 200 700] /DA (/ZaDb 0 Tf 0 g)"), true],
  ["btn_check_b0", W("/FT /Btn /T (x) /V /Off /AS /Off /Rect [100 700 124 724] /DA (/Helv 12 Tf 0 g) /BS << /W 0 >> /MK << /BC [0] /BG [1] >>"), true],
  ["btn_check_b3", W("/FT /Btn /T (x) /V /Off /AS /Off /Rect [100 700 124 724] /DA (/Helv 12 Tf 0 g) /BS << /W 3 >> /MK << /BC [0] /BG [1] >>"), true],
  ["btn_check_r90", W("/FT /Btn /T (x) /V /Off /AS /Off /Rect [100 700 140 724] /DA (/Helv 12 Tf 0 g) /MK << /R 90 /BC [0] /BG [1] >>"), true],
  ["btn_check_red", W("/FT /Btn /T (x) /V /Off /AS /Off /Rect [100 700 124 724] /DA (/ZaDb 0 Tf 1 0 0 rg) /MK << /BC [0] /BG [1] >>"), true],
  ["btn_check_ca", W("/FT /Btn /T (x) /V /Off /AS /Off /Rect [100 700 124 724] /DA (/ZaDb 0 Tf 0 g) /MK << /BC [0] /BG [1] /CA (8) >>"), true],
  ["btn_radio_red", W("/FT /Btn /Ff 32768 /T (x) /V /Off /AS /Off /Rect [100 700 124 724] /DA (/ZaDb 0 Tf 1 0 0 rg) /MK << /BC [0] /BG [1] >>"), true],
  ["btn_radio_ca", W("/FT /Btn /Ff 32768 /T (x) /V /Off /AS /Off /Rect [100 700 124 724] /DA (/ZaDb 0 Tf 0 g) /MK << /BC [0] /BG [1] /CA (4) >>"), true],
  ["btn_radio16", W("/FT /Btn /Ff 32768 /T (x) /V /Off /AS /Off /Rect [100 700 116 716] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [1] >>"), true],
  ["btn_radio_plain", W("/FT /Btn /Ff 32768 /T (x) /V /Off /AS /Off /Rect [100 700 130 720] /DA (/Helv 12 Tf 0 g)"), true],
  ["btn_radio_b3", W("/FT /Btn /Ff 32768 /T (x) /V /Off /AS /Off /Rect [100 700 124 724] /DA (/Helv 12 Tf 0 g) /BS << /W 3 >> /MK << /BC [0] /BG [1] >>"), true],
  ["btn_radio_colors", W("/FT /Btn /Ff 32768 /T (x) /V /Off /AS /Off /Rect [100 700 124 724] /DA (/Helv 12 Tf 0 g) /MK << /BC [1 0 0] /BG [0 0 1 0] >>"), true],
  ["btn_push", W("/FT /Btn /Ff 65536 /T (x) /Rect [100 700 200 730] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /CA (Go) >>"), null],
  ["btn_push_plain", W("/FT /Btn /Ff 65536 /T (x) /Rect [100 700 200 730] /DA (/Helv 12 Tf 0 g)"), null],
  ["btn_push_auto", W("/FT /Btn /Ff 65536 /T (x) /Rect [100 700 200 740] /DA (/Helv 0 Tf 0 g) /MK << /CA (Go) >>"), null],
  ["btn_push_auto_wide", W("/FT /Btn /Ff 65536 /T (x) /Rect [100 700 140 740] /DA (/Helv 0 Tf 0 g) /MK << /CA (Wide caption) >>"), null],
  ["btn_push_bg", W("/FT /Btn /Ff 65536 /T (x) /Rect [100 700 200 730] /DA (/Helv 12 Tf 0 g) /BS << /W 3 >> /MK << /BC [0] /BG [0.75] /CA (Go) >>"), null],
  // Comb fields without a positive /MaxLen are one line; multiline wins.
  ["tx_comb_nomax", W("/FT /Tx /Ff 16777216 /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "abc"],
  ["tx_comb_max0", W("/FT /Tx /Ff 16777216 /MaxLen 0 /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] >>"), "abc"],
  ["tx_comb_nomax_q1", W("/FT /Tx /Ff 16777216 /Q 1 /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "abc"],
  ["tx_comb_nomax_auto", W("/FT /Tx /Ff 16777216 /Q 2 /T (x) /Rect [100 700 280 724] /DA (/Helv 0 Tf 0 g) /MK << /BC [0] >>"), "abc"],
  ["tx_comb_max1", W("/FT /Tx /Ff 16777216 /MaxLen 1 /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] >>"), "a"],
  ["tx_comb_multi", W("/FT /Tx /Ff 16781312 /MaxLen 6 /T (x) /Rect [100 700 280 760] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] >>"), "abc"],
  // Text outside WinAnsi: MuPDF's Greek, Cyrillic and CID fallback fonts.
  ["tx_cyrillic", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Привет мир"],
  ["tx_greek", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Ελληνικά"],
  ["tx_cjk", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "日本語"],
  ["tx_hangul", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "한국어 ㄱㅏ"],
  ["tx_kana", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "ひらがなｱｲ"],
  ["tx_mixed", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Hi Привет 日本 Ελλ x"],
  ["tx_scripts_switch", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "ひ日한ㄅ"],
  ["tx_initial_common", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "123 Привет"],
  ["tx_greek_euro", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "€ Ελλ"],
  ["tx_cyr_symbols", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Д≤ Д° Д\u00a0Дé"],
  ["tx_cjk_common", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "日 1€≤"],
  ["tx_lang_ja", W("/FT /Tx /T (x) /Lang (ja) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "日本"],
  ["tx_lang_ko", W("/FT /Tx /T (x) /Lang (ko-KR) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "日本"],
  ["tx_lang_tw", W("/FT /Tx /T (x) /Lang (zh-TW) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "日本"],
  ["tx_lang_cn", W("/FT /Tx /T (x) /Lang (zh-CN) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "日本"],
  ["tx_lang_en", W("/FT /Tx /T (x) /Lang (en) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "日本"],
  ["tx_cyr_times", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/TiRo 12 Tf 0 g)"), "Привет"],
  ["tx_grk_times", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/TiRo 12 Tf 0 g)"), "Ελλ"],
  ["tx_cyr_courier", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Cour 12 Tf 0 g)"), "Привет"],
  ["tx_grk_courier", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Cour 12 Tf 0 g)"), "Ελλ"],
  ["tx_cyr_q1", W("/FT /Tx /Q 1 /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Привет"],
  ["tx_cyr_auto", W("/FT /Tx /T (x) /Rect [100 700 140 724] /DA (/Helv 0 Tf 0 g)"), "Привет мир"],
  ["tx_cjk_q2", W("/FT /Tx /Q 2 /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "日 1Ab"],
  ["tx_cjk_auto", W("/FT /Tx /T (x) /Rect [100 700 140 724] /DA (/Helv 0 Tf 0 g)"), "日本語日本語"],
  ["tx_multi_cyr", W("/FT /Tx /Ff 4096 /T (x) /Rect [100 650 160 724] /DA (/Helv 12 Tf 0 g)"), "Привет мир и всё\nabc"],
  ["tx_multi_cjk", W("/FT /Tx /Ff 4096 /T (x) /Rect [100 650 280 724] /DA (/Helv 12 Tf 0 g)"), "日本\nПри\r\nab"],
  ["tx_multi_context", W("/FT /Tx /Ff 4096 /T (x) /Rect [100 650 280 724] /DA (/Helv 12 Tf 0 g)"), "123\nПривет"],
  ["tx_multi_context_lines", W("/FT /Tx /Ff 4096 /T (x) /Rect [100 650 160 724] /DA (/Helv 12 Tf 0 g)"), "Привет 123 456 789 000"],
  ["tx_single_newline", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Пр\nab"],
  ["tx_comb_cyr", W("/FT /Tx /Ff 16777216 /MaxLen 5 /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Пр a"],
  ["tx_comb_cjk", W("/FT /Tx /Ff 16777216 /MaxLen 4 /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "日本"],
  ["ch_combo_cyr", W("/FT /Ch /Ff 131072 /T (x) /V (a) /Opt [(a) (b)] /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Да"],
  ["ch_list_cyr", W("/FT /Ch /T (x) /Opt [<FEFF0414043000200061> (b)] /Rect [100 650 280 724] /DA (/Helv 12 Tf 0 g)"), "b"],
  // MuPDF draws a bullet for a character the current script's font lacks.
  ["tx_cyr_euro", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Привет – «x» €"],
  ["tx_latin_le", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "a≤b"],
  // MuPDF embeds a font program for these; Rust draws none.
  ["tx_polish", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "Łódź"],
  ["tx_arrow", W("/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)"), "a→b"],
];

/** The synthesis-case PDF: the widget is object 6, /DR fonts 7..10. */
function synthesisPdf(widget) {
  return buildPdf([
    "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [6 0 R] >>",
    "<< /Length 2 >>\nstream\nq \nendstream",
    "<< /Fields [6 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 7 0 R /TiRo 8 0 R /Cour 9 0 R /ZaDb 10 0 R >> >> >>",
    `<< /Type /Annot /Subtype /Widget /F 4 /P 3 0 R ${widget} >>`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >>",
  ]);
}

/** The fonts a form names: each dictionary, and a CID font's descendant and descriptor. */
function fontsOf(obj) {
  const fonts = {};
  const res = obj.get("Resources");
  const dict = res && res.isDictionary() ? res.get("Font") : null;
  if (!dict || !dict.isDictionary()) return fonts;
  dict.forEach((v, k) => {
    const font = v.resolve();
    const record = { font: font.toString() };
    const descendants = font.get("DescendantFonts");
    if (descendants && descendants.isArray()) {
      const descendant = descendants.get(0).resolve();
      record.descendant = descendant.toString();
      record.descriptor = descendant.get("FontDescriptor").resolve().toString();
    }
    fonts[k] = record;
  });
  return fonts;
}

function streamOf(obj) {
  if (!obj || obj.isNull() || !obj.isStream()) return null;
  const bbox = obj.get("BBox");
  const matrix = obj.get("Matrix");
  const nums = (a) => (a && a.isArray() ? [...Array(a.length)].map((_v, i) => a.get(i).asNumber()) : null);
  return { bbox: nums(bbox), matrix: nums(matrix), content: Buffer.from(obj.readStream().asUint8Array()).toString("latin1"), fonts: fontsOf(obj) };
}

function recordSynthesis(file) {
  const cases = [];
  for (const [name, widget, value] of synthesisCases) {
    const pdf = new mupdf.PDFDocument(Buffer.from(synthesisPdf(widget)));
    pdf.enableJournal();
    const page = pdf.loadPage(0);
    const w = page.getWidgets()[0];
    pdf.beginOperation("x");
    if (typeof value === "boolean") w.toggle();
    else if (typeof value === "string" && w.isText()) w.setTextValue(value);
    else if (typeof value === "string") w.setChoiceValue(value);
    w.setRect(w.getRect());
    w.update();
    pdf.endOperation();
    const ap = w.getObject().get("AP");
    const record = { name, widget, value };
    const normal = ap.get("N");
    if (normal.isStream()) record.N = streamOf(normal);
    else {
      record.states = {};
      normal.forEach((v, k) => { record.states[k] = streamOf(v); });
    }
    const down = ap.get("D");
    if (down && !down.isNull() && down.isStream()) record.D = streamOf(down);
    record.V = (() => { const v = w.getObject().get("V"); return v && !v.isNull() ? (v.isName() ? { name: v.asName() } : { text: v.asString() }) : null; })();
    cases.push(record);
  }
  fs.writeFileSync(file, `${JSON.stringify({ date: "2026-10-15", mupdf: "1.28.1", cases }, null, 1)}\n`);
  console.log(`${cases.length} synthesis cases -> ${file}`);
}

const synthAt = args.indexOf("--synthesis");
if (synthAt >= 0) {
  recordSynthesis(args[synthAt + 1]);
  process.exit(0);
}

function run(engine, scenario) {
  const session = engine.createSession();
  const op = (request) => engine.dispatch({ id: 1, ...request }, session).reply;
  const opened = op({ op: "open", name: "oracle.pdf", pdf: scenario.pdf() });
  if (!opened.ok) throw new Error(`open: ${opened.error}`);
  const find = (name, page, on) => op({ op: "fields", page }).items.find((item) => item.name === name && (on === undefined || item.on === on));
  for (const step of scenario.steps) {
    const page = step.page ?? 0;
    let reply;
    if (step.set) reply = op({ op: "field-set", page, widgetId: find(step.set, page, step.on)?.id, value: step.value });
    else if (step.props) {
      const { props, ...rest } = step;
      reply = op({ op: "field-props", page, widgetId: find(props, page)?.id, ...rest });
    } else reply = op({ page, ...step });
    if (!reply.ok) throw new Error(`${JSON.stringify(step)}: ${reply.error}`);
  }
  const pages = {};
  for (const page of scenario.pages ?? [0]) {
    const render = op({ op: "render", page, scale: SCALE });
    if (!render.ok) throw new Error(`render: ${render.error}`);
    pages[page] = { width: render.pixelWidth, height: render.pixelHeight, pixels: new Uint8Array(render.pixels), fields: op({ op: "fields", page }).items };
  }
  return { pages, save: op({ op: "save" }) };
}

const stats = () => ({ widgets: 0, pixels: 0, exact: 0, within16: 0, over64: 0 });
const kinds = {};
const savedKinds = {};
const rows = [];

/** Page `page` of saved bytes drawn by MuPDF at SCALE, as RGBA. */
function mupdfRender(bytes, page) {
  const doc = mupdf.Document.openDocument(Buffer.from(bytes), "application/pdf");
  const pix = doc.loadPage(page).toPixmap(mupdf.Matrix.scale(SCALE, SCALE), mupdf.ColorSpace.DeviceRGB, false, true);
  const width = pix.getWidth();
  const height = pix.getHeight();
  const rgb = pix.getPixels();
  const stride = pix.getStride();
  const pixels = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      for (let c = 0; c < 3; c++) pixels[(y * width + x) * 4 + c] = rgb[y * stride + x * 3 + c];
      pixels[(y * width + x) * 4 + 3] = 255;
    }
  }
  pix.destroy();
  doc.destroy();
  return { width, height, pixels };
}

/** Per-pixel agreement of two RGBA images inside a page-space rect. */
function compare(a, b, rect) {
  const s = { ...stats(), widgets: 1 };
  const [x0, y0, x1, y1] = rect.map((v) => Math.round(v * SCALE));
  for (let y = Math.max(0, y0); y < Math.min(b.height, y1); y++) {
    for (let x = Math.max(0, x0); x < Math.min(b.width, x1); x++) {
      const i = (y * b.width + x) * 4;
      const d = Math.max(...[0, 1, 2].map((c) => Math.abs(a.pixels[i + c] - b.pixels[i + c])));
      s.pixels++;
      if (d === 0) s.exact++;
      if (d <= 16) s.within16++;
      if (d > 64) s.over64++;
    }
  }
  return s;
}
for (const scenario of scenarios) {
  const ref = run(createMupdf(mupdf), scenario);
  const rust = run(createRust(mupdf), scenario);
  for (const [page, a] of Object.entries(ref.pages)) {
    const b = rust.pages[page];
    if (a.width !== b.width || a.height !== b.height) {
      rows.push({ scenario: scenario.name, page, error: `size ${a.width}x${a.height} vs ${b.width}x${b.height}` });
      continue;
    }
    if (dump) {
      fs.mkdirSync(dump, { recursive: true });
      const base = path.join(dump, `${scenarios.indexOf(scenario)}-${page}`);
      for (const [tag, img] of [["mupdf", a], ["rust", b]]) {
        const rgb = Buffer.alloc(img.width * img.height * 3);
        for (let i = 0; i < img.width * img.height; i++) for (let c = 0; c < 3; c++) rgb[i * 3 + c] = img.pixels[i * 4 + c];
        fs.writeFileSync(`${base}-${tag}.ppm`, Buffer.concat([Buffer.from(`P6\n${img.width} ${img.height}\n255\n`), rgb]));
      }
    }
    // The saved files drawn by MuPDF alone: the appearances, not the renderers.
    const saved = scenario.steps.length && ref.save.ok && rust.save.ok
      ? [mupdfRender(ref.save.bytes, Number(page)), mupdfRender(rust.save.bytes, Number(page))]
      : null;
    for (const field of b.fields) {
      const twin = a.fields.find((item) => item.id === field.id) ?? a.fields.find((item) => item.name === field.name && item.on === field.on);
      const kind = field.kind + (scenario.kindSuffix ?? "");
      const s = compare(a, b, field.rect);
      const total = (kinds[kind] ??= stats());
      for (const key of Object.keys(s)) total[key] += s[key];
      const pct = (n, d) => (d ? Math.round((10000 * n) / d) / 100 : 100);
      const row = {
        scenario: scenario.name, page: Number(page), name: field.name, on: field.on, kind,
        mupdfValue: twin?.value, rustValue: field.value,
        exactPct: pct(s.exact, s.pixels), within16Pct: pct(s.within16, s.pixels), over64Px: s.over64,
      };
      if (saved) {
        const t = compare(saved[0], saved[1], field.rect);
        const savedTotal = (savedKinds[kind] ??= stats());
        for (const key of Object.keys(t)) savedTotal[key] += t[key];
        row.savedExactPct = pct(t.exact, t.pixels);
        row.savedWithin16Pct = pct(t.within16, t.pixels);
      }
      rows.push(row);
    }
  }
}
const pct = (n, d) => (d ? Math.round((10000 * n) / d) / 100 : 100);
const table = (totals) => Object.fromEntries(Object.entries(totals).map(([kind, s]) => [kind, {
  widgets: s.widgets, pixels: s.pixels, exactPct: pct(s.exact, s.pixels), within16Pct: pct(s.within16, s.pixels), over64Px: s.over64,
}]));
const perKind = table(kinds);
const savedPerKind = table(savedKinds);
const summary = {
  date: "2026-10-15",
  subject: "widget rendering after the form ops: Rust app engine (hayro view) against engine.ts on MuPDF 1.28.1, both through the render op",
  command: "node engine/tools/app-forms-oracle.mjs engine/reference/app-forms-20261015/summary.json",
  scale: SCALE,
  scenarios: scenarios.map((s) => s.name),
  perKind,
  savedPerKindNote: "the two engines' saved files after the op scenarios, both drawn by MuPDF: the appearance streams alone, without the renderer difference",
  savedPerKind,
  rows,
};
const text = JSON.stringify(summary, null, 1);
if (out) fs.writeFileSync(out, `${text}\n`);
console.log(JSON.stringify({ perKind, savedPerKind }, null, 1));
