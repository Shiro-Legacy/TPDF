#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Pixel agreement of annotation appearances, Rust against MuPDF: the same
// requests run through engine.ts on MuPDF (node_modules/mupdf) and through
// the Rust engine behind the app's adapter (engine/web/pkg), then each
// engine renders the page with its own `render` op. Also: the Rust save
// drawn by MuPDF against MuPDF's own save (appearance stream fidelity), and
// files whose annotations have no appearance, drawn by each engine.
// usage: node engine/tools/compare-annots.mjs [OUT.json]   (from the app root, after npm run build)
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const mupdf = (await import(path.join(root, "node_modules/mupdf/dist/mupdf.js"))).default;
const { createEngine } = await import(path.join(root, "build/renderer/engine.js"));
const { createRustEngine } = await import(path.join(root, "build/renderer/engine-rust.js"));
const wasm = await import(path.join(root, "engine/web/pkg/tandem_rust_workflow.js"));
wasm.initSync({ module: readFileSync(path.join(root, "engine/web/pkg/tandem_rust_workflow_bg.wasm")) });

const engines = { mupdf: createEngine(mupdf), rust: createRustEngine(wasm) };
const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);

function pdf(objects, pageExtra = "", annots = [], content = "") {
  const bodies = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R ${pageExtra} ${annots.length ? `/Annots [${annots.map((_, i) => `${5 + i} 0 R`).join(" ")}]` : ""} >>`,
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    ...annots,
    ...objects,
  ];
  let s = "%PDF-1.7\n";
  const offsets = [];
  bodies.forEach((body, i) => { offsets.push(s.length); s += `${i + 1} 0 obj\n${body}\nendobj\n`; });
  const xref = s.length;
  s += `xref\n0 ${bodies.length + 1}\n0000000000 65535 f \n${offsets.map(o => `${String(o).padStart(10, "0")} 00000 n \n`).join("")}`;
  s += `trailer\n<< /Size ${bodies.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(s, "latin1");
}

const gray = readFileSync(path.join(root, "fixtures/img-gray.png"));
const rgb = readFileSync(path.join(root, "fixtures/img-rgb.png"));
const q = (x0, y0, x1, y1) => [x0, y0, x1, y0, x0, y1, x1, y1];

/** Created annotations: requests on a blank page (rotated where named). */
const created = [
  ["Text", [{ op: "annot-add", type: "Text", point: [100, 100], contents: "note" }]],
  ["Highlight", [{ op: "annot-add", type: "Highlight", quads: [q(100, 100, 260, 116), q(100, 120, 180, 136)], contents: "" }]],
  ["Underline", [{ op: "annot-add", type: "Underline", quads: [q(100, 100, 260, 116), q(100, 120, 180, 136)], contents: "" }]],
  ["StrikeOut", [{ op: "annot-add", type: "StrikeOut", quads: [q(100, 100, 260, 116), q(100, 120, 180, 136)], contents: "" }]],
  ["FreeText", [{ op: "annot-add", type: "FreeText", point: [100, 100], contents: "free text, wrapped across the box width" }]],
  ["FreeText fit", [{ op: "annot-add", type: "FreeText", point: [100, 300], contents: "Jane Q. Public\nsecond line", fit: true }]],
  ["Square", [{ op: "annot-add", type: "Square", rect: [100, 100, 260, 200], contents: "" }]],
  ["Circle", [{ op: "annot-add", type: "Circle", rect: [100, 100, 260, 200], contents: "" }]],
  ["Line", [{ op: "annot-add", type: "Line", points: [[100, 100], [300, 180]], contents: "" }]],
  ["Ink", [{ op: "annot-add", type: "Ink", strokes: [[[100, 100], [150, 160], [200, 110]], [[220, 120], [300, 150]]], contents: "" }]],
  ["Ink signature", [{ op: "annot-add", type: "Ink", strokes: [[[100, 100], [150, 160], [300, 110]]], contents: "", signature: true }]],
  ...["Approved", "Draft", "Confidential", "Final", "NotApproved", "Void"].map(icon => [`Stamp ${icon}`,
    [{ op: "annot-add", type: "Stamp", rect: [100, 100, 300, 160], icon, contents: "" }]]),
  ["Stamp image", [{ op: "annot-add", type: "Stamp", rect: [100, 100, 300, 233], image: { name: "rgb.png", bytes: rgb }, contents: "" }]],
  ["Stamp signature", [{ op: "annot-add", type: "Stamp", rect: [100, 100, 300, 160], image: { name: "gray.png", bytes: gray }, signature: true, contents: "" }]],
  ["FreeText set", [{ op: "annot-add", type: "FreeText", point: [100, 100], contents: "before" },
    { op: "annot-set", annotId: "$0", contents: "after the edit, longer" }]],
  ["FreeText move", [{ op: "annot-add", type: "FreeText", point: [100, 100], contents: "moved", fit: true },
    { op: "annot-move", annotId: "$0", point: [300.5, 400.25] }]],
  ["Ink signature resize", [{ op: "annot-add", type: "Ink", strokes: [[[100, 100], [150, 160], [300, 110]]], contents: "", signature: true },
    { op: "annot-move", annotId: "$0", point: [200, 300], width: 120 }]],
];
const rotated = [
  ["Text rot90", [{ op: "annot-add", type: "Text", point: [100, 100], contents: "note" }], 90],
  ["FreeText rot90", [{ op: "annot-add", type: "FreeText", point: [100, 100], contents: "rotated text box" }], 90],
  ["Square rot90", [{ op: "annot-add", type: "Square", rect: [100, 100, 260, 200], contents: "" }], 90],
  ["Highlight rot90", [{ op: "annot-add", type: "Highlight", quads: [q(100, 100, 260, 116)], contents: "" }], 90],
  // Stamps are drawn upright on a turned page (MuPDF turns them with the
  // page), so their rows differ from MuPDF's by design; viewOfOwnSave is
  // what measures them.
  ...[90, 180, 270].flatMap(rotate => [
    [`Stamp Approved rot${rotate}`, [{ op: "annot-add", type: "Stamp", rect: [100, 100, 300, 160], icon: "Approved", contents: "" }], rotate],
    [`Stamp image rot${rotate}`, [{ op: "annot-add", type: "Stamp", rect: [100, 100, 300, 233], image: { name: "rgb.png", bytes: rgb }, contents: "" }], rotate],
    [`Stamp signature rot${rotate}`, [{ op: "annot-add", type: "Stamp", rect: [100, 100, 300, 160], image: { name: "gray.png", bytes: gray }, signature: true, contents: "" }], rotate],
  ]),
];

/** Foreign annotations edited by requests (annotation 5). */
const edited = [
  ["FreeText /RD set", "/Subtype /FreeText /Rect [100 600 300 700] /RD [20 10 30 10] /DA (/Helv 12 Tf 0 0 0 rg) /Contents (x)",
    [{ op: "annot-set", annotId: 5, contents: "changed inside the margins" }]],
  // ISO 32000 /RD order (left top right bottom); MuPDF reads it as a rect.
  ["FreeText asymmetric /RD set", "/Subtype /FreeText /Rect [100 600 300 700] /RD [20 10 30 40] /DA (/Helv 12 Tf 0 0 0 rg) /Contents (x)",
    [{ op: "annot-set", annotId: 5, contents: "changed" }]],
  ["Stamp bare set", "/Subtype /Stamp /Rect [100 400 300 600] /Name /Approved", [{ op: "annot-set", annotId: 5, contents: "changed" }]],
  ["Text bare set", "/Subtype /Text /Rect [72 400 200 450]", [{ op: "annot-set", annotId: 5, contents: "changed" }]],
];

/** Foreign appearances hayro would place wrongly (a /Matrix, a box off the origin). */
const foreignForms = [
  ["AP turned by /Matrix", "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Matrix [0 1 -1 0 0 0] /Length 12 >>\nstream\n0 0 1 1 re f\nendstream"],
  ["AP box off the origin", "<< /Type /XObject /Subtype /Form /BBox [20 10 120 60] /Length 26 >>\nstream\n0 0 1 rg 20 10 100 50 re f\nendstream"],
  ["AP translated /Matrix", "<< /Type /XObject /Subtype /Form /BBox [0 0 100 50] /Matrix [1 0 0 1 50 20] /Length 25 >>\nstream\n1 0 0 rg 0 0 100 50 re f\nendstream"],
];

/** Annotations without an appearance, drawn as each engine synthesizes them. */
const bare = [
  ["Square bare", "/Subtype /Square /Rect [100 600 200 700] /C [1 0 0] /IC [0 0 1] /BS << /W 3 >>"],
  ["Circle bare", "/Subtype /Circle /Rect [100 600 260 700] /C [0 0.5 0] /BS << /W 2 /S /D /D [4 2] >>"],
  ["Text bare", "/Subtype /Text /Rect [100 600 120 620] /Name /Comment /C [0 0 1]"],
  ["Text bare no colour", "/Subtype /Text /Rect [100 600 130 630]"],
  ["Highlight bare", "/Subtype /Highlight /Rect [0 0 1 1] /QuadPoints [100 700 300 700 100 680 300 680] /C [1 0.5 0]"],
  ["Underline bare", "/Subtype /Underline /Rect [0 0 1 1] /QuadPoints [100 700 300 700 100 680 300 680] /C [0 0 1]"],
  ["StrikeOut bare", "/Subtype /StrikeOut /Rect [0 0 1 1] /QuadPoints [100 700 300 700 100 680 300 680] /C [1 0 0] /CA 0.5"],
  ["Squiggly bare", "/Subtype /Squiggly /Rect [0 0 1 1] /QuadPoints [100 700 300 700 100 680 300 680] /C [1 0 0]"],
  ["FreeText bare", "/Subtype /FreeText /Rect [100 600 300 700] /Contents (Times text that wraps inside its box) /DA (/TiRo 14 Tf 1 0 0 rg) /BS << /W 0 >>"],
  ["Ink bare", "/Subtype /Ink /Rect [0 0 1 1] /InkList [[100 600 150 700 200 620 260 690]] /C [0 0 1] /BS << /W 2 >>"],
  ["Line bare", "/Subtype /Line /Rect [0 0 1 1] /L [100 600 300 700] /C [1 0 0] /BS << /W 2 >> /LE [/OpenArrow /ClosedArrow]"],
  ["Polygon bare", "/Subtype /Polygon /Rect [0 0 1 1] /Vertices [100 600 300 600 200 700] /C [1 0 0] /IC [1 1 0]"],
  ["PolyLine bare", "/Subtype /PolyLine /Rect [0 0 1 1] /Vertices [100 600 300 600 200 700] /C [0 0 1]"],
  ["Stamp bare", "/Subtype /Stamp /Rect [100 600 290 650] /Name /Approved /C [0 0.6 0]"],
  ["Stamp bare, square rect", "/Subtype /Stamp /Rect [100 400 300 600] /Name /Approved"],
  ["Text bare, large rect", "/Subtype /Text /Rect [72 400 200 450]"],
  ["Caret bare", "/Subtype /Caret /Rect [100 600 140 640] /C [0 0 1]"],
  ["Redact bare", "/Subtype /Redact /Rect [100 600 300 700]"],
  ["Redact bare with quads", "/Subtype /Redact /Rect [100 600 300 700] /QuadPoints [110 690 200 690 110 680 200 680]"],
];

function open(kind, bytes) {
  const engine = engines[kind];
  const session = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: "open", name: "a.pdf", pdf: buffer(bytes) }, session).reply;
  if (!reply.ok) throw new Error(`${kind} open: ${reply.error}`);
  return { engine, session };
}

function run(kind, bytes, requests) {
  const { engine, session } = open(kind, bytes);
  const ids = [];
  for (const request of requests) {
    const resolved = { id: 2, page: 0, ...request };
    if (typeof resolved.annotId === "string") resolved.annotId = ids[Number(resolved.annotId.slice(1))];
    if (resolved.image) resolved.image = { name: resolved.image.name, bytes: buffer(resolved.image.bytes) };
    const reply = engine.dispatch(resolved, session).reply;
    if (!reply.ok) throw new Error(`${kind} ${request.op}: ${reply.error}`);
    if (request.op === "annot-add") {
      const items = engine.dispatch({ id: 3, op: "annots", page: 0 }, session).reply.items;
      ids.push(items[items.length - 1].id);
    }
  }
  return { engine, session };
}

function render(engine, session, scale) {
  const reply = engine.dispatch({ id: 4, op: "render", page: 0, scale }, session).reply;
  if (!reply.ok) throw new Error(reply.error);
  return { w: reply.pixelWidth, h: reply.pixelHeight, px: new Uint8Array(reply.pixels) };
}

function pixmap(bytes, scale) {
  const doc = mupdf.Document.openDocument(bytes, "application/pdf");
  const pix = doc.loadPage(0).toPixmap(mupdf.Matrix.scale(scale, scale), mupdf.ColorSpace.DeviceRGB, false, true);
  const w = pix.getWidth(), h = pix.getHeight(), rgb = pix.getPixels();
  const px = new Uint8Array(w * h * 4);
  for (let i = 0; i < w * h; i++) px.set([rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2], 255], i * 4);
  return { w, h, px };
}

/** Exact and near agreement over the region where either image is not white. */
function compare(a, b, label) {
  if (a.w !== b.w || a.h !== b.h) return { size: [a.w, a.h], other: [b.w, b.h] };
  const ink = (img, i) => img.px[i] < 255 || img.px[i + 1] < 255 || img.px[i + 2] < 255;
  let x0 = Infinity, y0 = Infinity, x1 = -1, y1 = -1;
  for (let y = 0; y < a.h; y++) for (let x = 0; x < a.w; x++) {
    const i = (y * a.w + x) * 4;
    if (ink(a, i) || ink(b, i)) { x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y); }
  }
  if (x1 < 0) return { label, region: null, exactPct: 100 };
  x0 = Math.max(0, x0 - 2); y0 = Math.max(0, y0 - 2); x1 = Math.min(a.w - 1, x1 + 2); y1 = Math.min(a.h - 1, y1 + 2);
  let n = 0, exact = 0, near = 0, far = 0;
  for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) {
    const i = (y * a.w + x) * 4;
    const d = Math.max(Math.abs(a.px[i] - b.px[i]), Math.abs(a.px[i + 1] - b.px[i + 1]), Math.abs(a.px[i + 2] - b.px[i + 2]));
    n++; if (d === 0) exact++; if (d <= 16) near++; if (d > 64) far++;
  }
  const pct = v => Math.round(10000 * v / n) / 100;
  return { region: [x0, y0, x1 + 1, y1 + 1], exactPct: pct(exact), within16Pct: pct(near), over64Px: far };
}

function save(engine, session) {
  const reply = engine.dispatch({ id: 5, op: "save" }, session).reply;
  if (!reply.ok) throw new Error(reply.error);
  return new Uint8Array(reply.bytes);
}

const rows = [];
const blank = pdf([]);
const turned = rotate => pdf([], `/Rotate ${rotate}`);
const edits = edited.map(([n, entries, r]) => [n, r, pdf([], "", [`<< /Type /Annot ${entries} /F 4 >>`]), "edited"]);
for (const [name, requests, bytes, source] of [...created.map(([n, r]) => [n, r, blank, "created"]),
  ...rotated.map(([n, r, rotate]) => [n, r, turned(rotate), "created"]), ...edits]) {
  try {
    const m = run("mupdf", bytes, requests), r = run("rust", bytes, requests);
    const row = { kind: name, source };
    for (const scale of [1, 2]) row[`render@${scale}`] = compare(render(r.engine, r.session, scale), render(m.engine, m.session, scale));
    const own = save(r.engine, r.session);
    row.mupdfDrawsSaves = compare(pixmap(own, 2), pixmap(save(m.engine, m.session), 2));
    // Rust's view against MuPDF drawing Rust's own save: the view's fidelity
    // to what it saves, whatever the deliberate differences from MuPDF.
    row.viewOfOwnSave = compare(render(r.engine, r.session, 2), pixmap(own, 2));
    rows.push(row);
  } catch (err) {
    rows.push({ kind: name, source, error: String(err.message ?? err) });
  }
}
for (const [name, form] of foreignForms) {
  const bytes = pdf([form], "", ["<< /Type /Annot /Subtype /Stamp /Rect [100 400 300 600] /F 4 /AP << /N 6 0 R >> >>"]);
  try {
    const m = open("mupdf", bytes), r = open("rust", bytes);
    const row = { kind: name, source: "foreign appearance" };
    for (const scale of [1, 2]) row[`render@${scale}`] = compare(render(r.engine, r.session, scale), render(m.engine, m.session, scale));
    rows.push(row);
  } catch (err) {
    rows.push({ kind: name, source: "foreign appearance", error: String(err.message ?? err) });
  }
}
for (const [name, entries] of bare) {
  const bytes = pdf([], "", [`<< /Type /Annot ${entries} /F 4 >>`]);
  try {
    const m = open("mupdf", bytes), r = open("rust", bytes);
    const row = { kind: name, source: "no appearance" };
    for (const scale of [1, 2]) row[`render@${scale}`] = compare(render(r.engine, r.session, scale), render(m.engine, m.session, scale));
    rows.push(row);
  } catch (err) {
    rows.push({ kind: name, source: "no appearance", error: String(err.message ?? err) });
  }
}
// Controls: the same text and stamp label as page content, which neither
// engine synthesizes, to separate text rasterization from the appearances.
for (const [name, font, text] of [["control Helvetica 12", "/Helvetica", "/F 12 Tf 100 680 Td (free text, wrapped across the) Tj"],
  ["control Times-Bold 30", "/Times-Bold", "/F 30 Tf 100 680 Td (DRAFT) Tj"]]) {
  const fixed = pdf([`<< /Type /Font /Subtype /Type1 /BaseFont ${font} /Encoding /WinAnsiEncoding >>`],
    "/Resources << /Font << /F 5 0 R >> >>", [], `BT ${text} ET`);
  const m = open("mupdf", fixed), r = open("rust", fixed);
  rows.push({ kind: name, source: "page content", "render@1": compare(render(r.engine, r.session, 1), render(m.engine, m.session, 1)), "render@2": compare(render(r.engine, r.session, 2), render(m.engine, m.session, 2)) });
}
// A note MuPDF made (16 x 16 icon in a 20 x 20 rect), opened by each engine.
{
  const m = run("mupdf", blank, created[0][1]);
  const bytes = save(m.engine, m.session);
  for (const [label, file] of [["Text from MuPDF", bytes]]) {
    const a = open("mupdf", file), b = open("rust", file);
    rows.push({ kind: label, source: "MuPDF appearance", "render@1": compare(render(b.engine, b.session, 1), render(a.engine, a.session, 1)), "render@2": compare(render(b.engine, b.session, 2), render(a.engine, a.session, 2)) });
  }
}
const out = process.argv[2];
const summary = { date: new Date().toISOString().slice(0, 10), mupdf: "1.28.1", rows };
if (out) writeFileSync(out, JSON.stringify(summary, null, 1) + "\n");
for (const row of rows) {
  const cell = c => (c ? (c.exactPct !== undefined ? `${c.exactPct}%/${c.within16Pct ?? "-"}%/${c.over64Px ?? "-"}` : JSON.stringify(c)) : "");
  console.log(`${row.kind.padEnd(30)} ${row.error ?? `r1 ${cell(row["render@1"])}  r2 ${cell(row["render@2"])}  save ${cell(row.mupdfDrawsSaves)}  own ${cell(row.viewOfOwnSave)}`}`);
}
