#!/usr/bin/env node
/**
 * P3 U3 gate probe (contract-p3-unit3.md §4). Never shipped.
 * Usage: node scripts/p3/subset-probe.mjs <output-dir>   (run from repo root)
 * mupdf 1.28.1 wasm; oracles: pypdf (.venv-qa), qlmanage+sips, recover.py, verify-edit-region.py.
 */
import { spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import mupdf from 'mupdf';
import { fontBytes } from '../../fixtures/fetch-font.mjs';

const SAVE = 'incremental=no,garbage=yes';
const PY = '.venv-qa/bin/python';
const SUBSET = 'fixtures/p0b/edit-subset.pdf';
const CJK = 'fixtures/cjk.pdf';
const RTL = 'fixtures/rtl.pdf';
const PAGE_H = 792;
const DROID = {
  name: 'DroidSansFallback.ttf',
  url: 'https://github.com/aosp-mirror/platform_frameworks_base/raw/master/data/fonts/DroidSansFallback.ttf',
  sha256: '21b96a0377f067833a93af3082eb28d4ffab7a8cd46bfd513286f1d64b7b0949',
  license: 'Apache-2.0',
};

const HEBREW = {
  name: 'NotoSansHebrew[wdth,wght].ttf',
  url: 'https://github.com/google/fonts/raw/main/ofl/notosanshebrew/NotoSansHebrew%5Bwdth,wght%5D.ttf',
  sha256: '7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88',
  license: 'OFL-1.1',
};

const open = async (f) => mupdf.Document.openDocument(await readFile(f), 'application/pdf');
const hex4 = (n) => n.toString(16).padStart(4, '0');

function contentsObj(page) {
  const c = page.getObject().get('Contents');
  return c.isArray() ? c.get(0) : c;
}

// ToUnicode CMap: 2-byte codes -> string (only single BMP destinations kept; others 'UNMAPPABLE')
function parseToUnicode(src) {
  const map = new Map();
  const dest = (h) => (h.length === 4 ? String.fromCharCode(parseInt(h, 16)) : 'UNMAPPABLE');
  for (const block of src.matchAll(/beginbfchar([\s\S]*?)endbfchar/g)) {
    for (const m of block[1].matchAll(/<([0-9a-fA-F]+)>\s*<([0-9a-fA-F]+)>/g)) map.set(parseInt(m[1], 16), dest(m[2]));
  }
  for (const block of src.matchAll(/beginbfrange([\s\S]*?)endbfrange/g)) {
    for (const m of block[1].matchAll(/<([0-9a-fA-F]+)>\s*<([0-9a-fA-F]+)>\s*(<[0-9a-fA-F]+>|\[[^\]]*\])/g)) {
      const lo = parseInt(m[1], 16);
      const hi = parseInt(m[2], 16);
      if (m[3].startsWith('[')) {
        [...m[3].matchAll(/<([0-9a-fA-F]+)>/g)].forEach((d, i) => map.set(lo + i, dest(d[1])));
      } else {
        const h = m[3].slice(1, -1);
        const base = parseInt(h, 16);
        for (let c = lo; c <= hi; c++) map.set(c, h.length === 4 ? String.fromCharCode(base + c - lo) : 'UNMAPPABLE');
      }
    }
  }
  return map;
}

function parseW(wObj) {
  const w = new Map();
  if (!wObj || wObj.isNull()) return w;
  const arr = wObj.resolve();
  const items = [];
  for (let i = 0; i < arr.length; i++) items.push(arr.get(i).resolve());
  for (let i = 0; i < items.length;) {
    const first = items[i].asNumber();
    if (items[i + 1].isArray()) {
      const a = items[i + 1];
      for (let k = 0; k < a.length; k++) w.set(first + k, a.get(k).resolve().asNumber());
      i += 2;
    } else {
      const last = items[i + 1].asNumber();
      const v = items[i + 2].asNumber();
      for (let c = first; c <= last; c++) w.set(c, v);
      i += 3;
    }
  }
  return w;
}

function cidFont(fontDict) {
  const df = fontDict.get("DescendantFonts"); const d0 = (df.isIndirect() ? df.resolve() : df).get(0); const desc = d0.isIndirect() ? d0.resolve() : d0;
  const dwObj = desc.get('DW');
  return {
    baseFont: fontDict.get('BaseFont').asName(),
    toUnicode: parseToUnicode(fontDict.get('ToUnicode').readStream().asString()),
    widths: parseW(desc.get('W')),
    dw: dwObj && !dwObj.isNull() ? dwObj.asNumber() : 1000,
    cidToGid: desc.get('CIDToGIDMap').isNull() ? 'absent' : desc.get('CIDToGIDMap').toString(),
  };
}

// hex Tj runs in the content stream: [{font, size, hex, codes}]
function hexRuns(stream) {
  const runs = [];
  for (const m of stream.matchAll(/\/(\w+)\s+([\d.]+)\s+Tf[^<(]*?<([0-9a-fA-F]*)>\s*Tj/g)) {
    const codes = [];
    for (let i = 0; i < m[3].length; i += 4) codes.push(parseInt(m[3].slice(i, i + 4), 16));
    runs.push({ font: m[1], size: Number(m[2]), hex: m[3], codes });
  }
  return runs;
}

function stextLines(page) {
  const lines = [];
  let cur = null;
  page.toStructuredText().walk({
    beginLine() { cur = { text: '', chars: [] }; },
    onChar(c, _o, font, size, quad) {
      cur.text += c;
      cur.chars.push({ c, size, font: font.getName(), w: quad[2] - quad[0], quad });
    },
    endLine() { lines.push(cur); },
  });
  return lines;
}

// rect of a stext line in PDF y-up page points
function lineRect(line) {
  const xs = line.chars.flatMap((ch) => [ch.quad[0], ch.quad[2]]);
  const ys = line.chars.flatMap((ch) => [ch.quad[1], ch.quad[5]]);
  return [Math.min(...xs), PAGE_H - Math.max(...ys), Math.max(...xs), PAGE_H - Math.min(...ys)];
}

function py(args) {
  const r = spawnSync(PY, args, { encoding: 'utf8' });
  if (r.status !== 0 && !r.stdout) throw new Error(r.stderr);
  try { return JSON.parse(r.stdout); } catch { return JSON.parse(r.stdout.trim().split('\n').pop()); }
}

const PY_DARK = `
import importlib.util, json, sys, tempfile
from pathlib import Path
spec = importlib.util.spec_from_file_location("ver", "scripts/verify-edit-region.py")
ver = importlib.util.module_from_spec(spec); spec.loader.exec_module(ver)
pdf = Path(sys.argv[1]); x0, y0, x1, y1 = (float(v) for v in sys.argv[2].split(","))
with tempfile.TemporaryDirectory() as t:
    w, h, at = ver.render(pdf, Path(t))
    sx, sy = w / 612.0, h / 792.0
    xa, xb = int(x0 * sx), int(x1 * sx); ya, yb = int((792 - y1) * sy), int((792 - y0) * sy)
    n = dark = 0
    for y in range(ya, yb + 1):
        for x in range(xa, xb + 1):
            n += 1
            r, g, b = at(x, y)
            dark += (r + g + b) < 384
print(json.dumps({"n": n, "dark": dark, "frac": round(dark / n, 4), "pixels": [w, h]}))
`;
const qlDark = (file, rect) => py(['-c', PY_DARK, file, rect.join(',')]);

// mupdf render, dark pixel fraction inside a y-up rect (second renderer)
function mupdfDark(pdf, rect, scale = 2) {
  const page = pdf.loadPage(0);
  const pix = page.toPixmap(mupdf.Matrix.scale(scale, scale), mupdf.ColorSpace.DeviceRGB, false);
  const px = pix.getPixels();
  const W = pix.getWidth();
  const [x0, y0, x1, y1] = rect;
  let n = 0;
  let dark = 0;
  for (let y = Math.floor((PAGE_H - y1) * scale); y <= Math.floor((PAGE_H - y0) * scale); y++) {
    for (let x = Math.floor(x0 * scale); x <= Math.floor(x1 * scale); x++) {
      const i = (y * W + x) * 3;
      n++;
      if (px[i] + px[i + 1] + px[i + 2] < 384) dark++;
    }
  }
  pix.destroy();
  page.destroy();
  return { n, dark, frac: +(dark / n).toFixed(4) };
}

async function save(pdf, file) {
  const buf = pdf.saveToBuffer(SAVE);
  await writeFile(file, Buffer.from(buf.asUint8Array()));
  buf.destroy();
}

function pypdfText(file) {
  return py(['-c', 'import json,sys\nfrom pypdf import PdfReader\nprint(json.dumps([(p.extract_text() or "") for p in PdfReader(sys.argv[1]).pages]))', file]);
}

function rewriteHex(pdf, oldHex, newHex) {
  const c = contentsObj(pdf.loadPage(0));
  const s = c.readStream().asString();
  if (!s.includes(`<${oldHex}>`)) throw new Error(`missing <${oldHex}>`);
  c.writeStream(Buffer.from(s.replace(`<${oldHex}>`, `<${newHex}>`), 'latin1'));
}

// §4.1 /W advance vs stext quad width
async function item1(file) {
  const pdf = await open(file);
  const page = pdf.loadPage(0);
  const fonts = page.getObject().get('Resources').get('Font');
  const stream = contentsObj(page).readStream().asString();
  const flat = stextLines(page).flatMap((l) => l.chars);
  const used = new Set();
  // stext merges same-baseline runs and may reorder RTL: match each run as a contiguous (or reversed) char span
  const locate = (chars) => {
    for (const order of [chars, [...chars].reverse()]) {
      for (let i = 0; i + order.length <= flat.length; i++) {
        if (used.has(i) || !order.every((ch, k) => flat[i + k].c === ch)) continue;
        used.add(i);
        const span = flat.slice(i, i + order.length);
        return order === chars ? span : span.reverse();
      }
    }
    return null;
  };
  const rows = [];
  let maxDiff = 0;
  let unmatched = 0;
  for (const run of hexRuns(stream)) {
    const f = fonts.get(run.font).resolve();
    if (f.get('Subtype').asName() !== 'Type0') continue;
    const cf = cidFont(f);
    const span = locate(run.codes.map((c) => cf.toUnicode.get(c)));
    run.codes.forEach((code, i) => {
      const adv = ((cf.widths.get(code) ?? cf.dw) / 1000) * run.size;
      const q = span ? Math.abs(span[i].w) : null;
      const diff = q == null ? null : Math.abs(adv - q);
      if (diff == null) unmatched++;
      else maxDiff = Math.max(maxDiff, diff);
      rows.push({ font: run.font, fromW: cf.widths.has(code), code: hex4(code), ch: cf.toUnicode.get(code), wAdvance: +adv.toFixed(3), quadW: q == null ? null : +q.toFixed(3), diff: diff == null ? null : +diff.toFixed(3) });
    });
  }
  return { file, pass: unmatched === 0 && maxDiff <= 0.5, maxDiff, unmatched, rows, pages: pdf.countPages() };
}

// §4.2 permutation of existing codes; §4.3 ToUnicode-only code
async function item23(out) {
  const OLD = '1a021b493ebb00f400e700f6'; // 日本語テスト
  const pdf0 = await open(SUBSET);
  const f = pdf0.loadPage(0).getObject().get('Resources').get('Font').get('F2').resolve();
  const cf = cidFont(f);
  const lines = stextLines(pdf0.loadPage(0));
  const rect = lineRect(lines.find((l) => l.text === '日本語テスト'));
  const neighbourRect = lineRect(lines.find((l) => l.text === '隣の行'));
  const codeOf = (ch) => [...cf.toUnicode].find(([, u]) => u === ch)?.[0];
  const NEW = [...'テスト日本'].map((ch) => hex4(codeOf(ch))).join('');
  const before = path.join(out, 'subset-before.pdf');
  await save(pdf0, before);

  const pdf = await open(SUBSET);
  rewriteHex(pdf, OLD, NEW);
  const edited = path.join(out, 'subset-edited.pdf');
  await save(pdf, edited);
  const reopened = await open(edited);
  const mupdfText = reopened.loadPage(0).toStructuredText().asText();
  const pypdf = pypdfText(edited)[0];
  const rec = py([ 'scripts/p0b/recover.py', edited, OLD ]);
  const qlBefore = qlDark(before, rect);
  const qlAfter = qlDark(edited, rect);
  // union of old and new extents: new text is shorter, so the verify rect is the old line rect
  const region = py(['scripts/verify-edit-region.py', before, edited, '--rect', rect.join(',')]);
  const neighbourBefore = qlDark(before, neighbourRect);
  const neighbourAfter = qlDark(edited, neighbourRect);
  const item2 = {
    oldHex: OLD,
    newHex: NEW,
    rect,
    mupdfHasNew: mupdfText.includes('テスト日本'),
    mupdfHasOld: mupdfText.includes('日本語テスト'),
    pypdfHasNew: pypdf.includes('テスト日本'),
    pypdfHasOld: pypdf.includes('日本語テスト'),
    recover: { ok: rec.ok, raw_count: rec.raw_count, object_hit_count: rec.object_hit_count },
    qlDarkBefore: qlBefore,
    qlDarkAfter: qlAfter,
    mupdfDarkAfter: mupdfDark(reopened, rect),
    verifyRegion: region,
    neighbour: { rect: neighbourRect, qlBefore: neighbourBefore, qlAfter: neighbourAfter },
  };
  item2.pass = item2.mupdfHasNew && !item2.mupdfHasOld && item2.pypdfHasNew && !item2.pypdfHasOld
    && rec.raw_count === 0 && rec.object_hit_count === 0 && qlAfter.frac >= 0.01 && region.outside_identical
    && neighbourBefore.dark === neighbourAfter.dark;

  // §4.3: 試 is in the (whole-font) ToUnicode but not drawn on the page
  const pdf3 = await open(SUBSET);
  const shi = codeOf('試');
  const ken = codeOf('験');
  const NEW3 = `1a021b493ebb${hex4(shi)}${hex4(ken)}`; // 日本語試験
  rewriteHex(pdf3, OLD, NEW3);
  const edited3 = path.join(out, 'subset-tounicode-only.pdf');
  await save(pdf3, edited3);
  const re3 = await open(edited3);
  const lines3 = stextLines(re3.loadPage(0));
  const l3 = lines3.find((l) => l.text.startsWith('日本語'));
  const glyphRect = [72 + 3 * 28, rect[1], 72 + 5 * 28, rect[3]]; // the two new glyph cells
  const existingRect = [72, rect[1], 72 + 3 * 28, rect[3]]; // 日本語, still in subset
  const item3 = {
    codes: { '試': hex4(shi), '験': hex4(ken) },
    inToUnicode: shi != null && ken != null,
    extracted: l3?.text,
    glyphRect,
    qlDarkNewCells: qlDark(edited3, glyphRect),
    qlDarkExistingCells: qlDark(edited3, existingRect),
    mupdfDarkNewCells: mupdfDark(re3, glyphRect),
    mupdfDarkExistingCells: mupdfDark(re3, existingRect),
  };
  item3.blank = item3.qlDarkNewCells.frac < 0.01 && item3.mupdfDarkNewCells.frac < 0.01;
  return { item2, item3 };
}

// §4.4 simple subset TrueType: does encodeCharacter refuse a char dropped by subsetting?
// DroidSansFallback (contract's pick) has no Latin glyphs at all, so NotoSansHebrew (has Latin) is the real case.
async function item4(out, meta, tag) {
  const ttf = await fontBytes(meta);
  const full = new mupdf.Font(meta.name, ttf);
  const doc = new mupdf.PDFDocument();
  const fonts = doc.newDictionary();
  fonts.put('F1', doc.addSimpleFont(full, mupdf.Font.SIMPLE_ENCODING_LATIN));
  const res = doc.newDictionary();
  res.put('Font', fonts);
  doc.insertPage(-1, doc.addPage([0, 0, 612, 792], 0, res, Buffer.from('BT /F1 40 Tf 72 600 Td (ALPHA) Tj ET\n', 'latin1')));
  doc.subsetFonts();
  const before = path.join(out, `simple-subset-${tag}-before.pdf`);
  await save(doc, before);
  const pdf = await open(before);
  const page = pdf.loadPage(0);
  const fdict = page.getObject().get('Resources').get('Font').get('F1').resolve();
  const desc = fdict.get('FontDescriptor').resolve();
  let handle = null;
  const quads = [];
  page.toStructuredText().walk({ onChar(_c, _o, font, _s, quad) { handle ??= font; quads.push(quad); } });
  const enc = (f) => Object.fromEntries(['A', 'L', 'P', 'H', 'Z'].map((ch) => [ch, f.encodeCharacter(ch)]));
  const lastX = quads[4][0];
  const zCell = [lastX, 600 - 12, lastX + 40, 600 + 40];
  const pdf2 = await open(before);
  const c = contentsObj(pdf2.loadPage(0));
  c.writeStream(Buffer.from(c.readStream().asString().replace('(ALPHA)', '(ALPHZ)'), 'latin1'));
  const edited = path.join(out, `simple-subset-${tag}-ALPHZ.pdf`);
  await save(pdf2, edited);
  const re = await open(edited);
  const lastA = { ql: qlDark(before, zCell), mupdf: mupdfDark(pdf, zCell) };
  const lastZ = { ql: qlDark(edited, zCell), mupdf: mupdfDark(re, zCell) };
  const wObj = fdict.get('Widths').resolve();
  const firstChar = fdict.get('FirstChar').asNumber();
  const widthOf = (ch) => { const i = ch.charCodeAt(0) - firstChar; return i >= 0 && i < wObj.length ? wObj.get(i).asNumber() : null; };
  const r = {
    font: meta.name,
    baseFont: fdict.get('BaseFont').asName(),
    subtype: fdict.get('Subtype').asName(),
    fontFile: ['FontFile', 'FontFile2', 'FontFile3'].find((k) => !desc.get(k).isNull()),
    fullFontEncode: enc(full),
    subsetHandleEncode: enc(handle),
    subsetAdvanceZ: handle.advanceGlyph(handle.encodeCharacter('Z')),
    widthsA_Z: { A: widthOf('A'), Z: widthOf('Z') },
    extractedAfter: re.loadPage(0).toStructuredText().asText().trim(),
    zCell,
    darkLastCellBefore_A: lastA,
    darkLastCellAfter_Z: lastZ,
    pages: pdf.countPages() + re.countPages(),
  };
  r.aRendersInk = lastA.ql.frac >= 0.01 && lastA.mupdf.frac >= 0.01;
  r.zRendersBlank = lastZ.ql.frac < 0.01 && lastZ.mupdf.frac < 0.01;
  return r;
}

const RTL_RANGES = [[0x0590, 0x08ff], [0xfb1d, 0xfdff], [0xfe70, 0xfeff], [0x0300, 0x036f], [0x1ab0, 0x1aff], [0x1dc0, 0x1dff], [0x20d0, 0x20ff], [0xfe20, 0xfe2f]];
const inRtl = (ch) => RTL_RANGES.some(([a, b]) => ch.charCodeAt(0) >= a && ch.charCodeAt(0) <= b);

// §4.5
async function item5() {
  const pdf = await open(RTL);
  const page = pdf.loadPage(0);
  const fonts = page.getObject().get('Resources').get('Font');
  const runs = hexRuns(contentsObj(page).readStream().asString()).map((run) => {
    const f = fonts.get(run.font).resolve();
    const cf = cidFont(f);
    const chars = run.codes.map((c) => cf.toUnicode.get(c));
    return {
      font: run.font,
      baseFont: cf.baseFont,
      hex: run.hex,
      decoded: chars.join(''),
      codepoints: chars.map((ch) => (ch ? `U+${hex4(ch.charCodeAt(0)).toUpperCase()}` : 'MISSING')),
      allInRtlRanges: chars.every((ch) => ch && inRtl(ch)),
      anyInRtlRanges: chars.some((ch) => ch && inRtl(ch)),
      presentationForms: chars.filter((ch) => ch && ch.charCodeAt(0) >= 0xfb1d).length,
    };
  });
  return { runs, stext: page.toStructuredText().asText(), pages: pdf.countPages() }; // keep pdf alive (wasm GC)
}

async function main() {
  const out = process.argv[2];
  if (!out) {
    console.error('usage: node scripts/p3/subset-probe.mjs <output-dir>');
    process.exit(2);
  }
  const abs = path.resolve(out);
  await mkdir(abs, { recursive: true });
  const report = { mupdf: '1.28.1', save: SAVE };
  report.item1 = [await item1(SUBSET), await item1(CJK), await item1(RTL)]; // rtl.pdf: extra, proportional advances; Arabic stext chars are unshaped so those rows stay unmatched (not gated)
  Object.assign(report, await item23(abs));
  report.item4 = [await item4(abs, DROID, 'droid'), await item4(abs, HEBREW, 'noto')];
  report.item5 = await item5();
  report.gate = report.item1.slice(0, 2).every((r) => r.pass) && report.item2.pass ? 'PASS' : 'FAIL';
  const json = path.join(abs, 'subset-probe.json');
  await writeFile(json, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify({ gate: report.gate, json }));
}

main().catch((e) => { console.error(e); process.exit(1); });
