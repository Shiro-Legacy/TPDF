#!/usr/bin/env node
/**
 * Coded simple-font fixture (contract-coded.md D4): edit-coded.pdf, shaped like a
 * LibreOffice export. /F2 is a symbolic simple TrueType with NO /Encoding, /Flags 4,
 * a 1-byte /ToUnicode and /Widths; its text operands are sequential glyph codes
 * (1, 2, 3 … in first-use order). The embedded program is a mupdf subset of
 * NotoSansHebrew (Latin glyphs, ids kept) whose cmap is replaced by (1,0) and (3,0)
 * format-4 subtables mapping code (and 0xF000 + code) to the glyph id.
 * Line 3 `xx` draws `x` with two different codes (ambiguous in the repertoire).
 *
 * Run: node fixtures/p0b/make-edit-coded.mjs
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import mupdf from 'mupdf';
import { fontBytes } from '../fetch-font.mjs';

const here = import.meta.dirname;
const FONT = {
  name: 'NotoSansHebrew[wdth,wght].ttf',
  url: 'https://github.com/google/fonts/raw/main/ofl/notosanshebrew/NotoSansHebrew%5Bwdth,wght%5D.ttf',
  sha256: '7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88',
  license: 'OFL-1.1',
  licenceText: 'docs/third-party/notosanshebrew-OFL-1.1.txt',
};
const LINES = [['Lorem ipsum', 660], ['Dolor', 600], ['xx', 540]];

const ttf = await fontBytes(FONT);
const handle = new mupdf.Font(FONT.name, ttf);

// Codes in first-use order; the second `x` gets its own code (same glyph).
const codeChars = [];
const lineCodes = LINES.map(([text]) => [...text].map((ch, index) => {
  const reuse = !(text === 'xx' && index === 1) && codeChars.indexOf(ch);
  if (reuse !== false && reuse >= 0) return reuse + 1;
  codeChars.push(ch);
  return codeChars.length;
}));
const gids = codeChars.map((ch) => {
  const gid = handle.encodeCharacter(ch);
  if (!gid) throw new Error(`no glyph for ${ch}`);
  return gid;
});

// 1. A mupdf subset of the font (glyph ids kept): draw the glyphs through Identity-H and subset.
function subsetProgram() {
  const doc = new mupdf.PDFDocument();
  const fonts = doc.newDictionary();
  fonts.put('F1', doc.addFont(handle));
  const resources = doc.newDictionary();
  resources.put('Font', fonts);
  const hex = gids.map((gid) => gid.toString(16).padStart(4, '0')).join('');
  doc.insertPage(-1, doc.addPage([0, 0, 612, 792], 0, resources, Buffer.from(`BT /F1 12 Tf 72 700 Td <${hex}> Tj ET\n`, 'latin1')));
  doc.subsetFonts();
  const descriptor = doc.loadPage(0).getObject().get('Resources').get('Font').get('F1')
    .get('DescendantFonts').get(0).get('FontDescriptor');
  const buffer = descriptor.get('FontFile2').readStream();
  return Uint8Array.from(buffer.asUint8Array());
}

// 2. cmap: (1,0) code -> gid and (3,0) 0xF000+code -> gid, both format 4, one segment per code.
function format4(pairs) {
  const segs = [...pairs, [0xffff, 0]];
  const n = segs.length;
  const pow = 2 ** Math.floor(Math.log2(n));
  const out = Buffer.alloc(16 + n * 8);
  out.writeUInt16BE(4, 0);
  out.writeUInt16BE(out.length, 2);
  out.writeUInt16BE(0, 4);
  out.writeUInt16BE(n * 2, 6);
  out.writeUInt16BE(pow * 2, 8);
  out.writeUInt16BE(Math.log2(pow), 10);
  out.writeUInt16BE(n * 2 - pow * 2, 12);
  segs.forEach(([code, gid], i) => {
    out.writeUInt16BE(code, 14 + i * 2); // endCode
    out.writeUInt16BE(code, 16 + n * 2 + i * 2); // startCode (after reservedPad)
    out.writeUInt16BE(code === 0xffff ? 1 : (gid - code + 0x10000) & 0xffff, 16 + n * 4 + i * 2);
    out.writeUInt16BE(0, 16 + n * 6 + i * 2);
  });
  return out;
}

function cmapTable() {
  const mac = format4(gids.map((gid, i) => [i + 1, gid]));
  const win = format4(gids.map((gid, i) => [0xf000 + i + 1, gid]));
  const head = Buffer.alloc(4 + 2 * 8);
  head.writeUInt16BE(0, 0);
  head.writeUInt16BE(2, 2);
  head.writeUInt16BE(1, 4); head.writeUInt16BE(0, 6); head.writeUInt32BE(head.length, 8);
  head.writeUInt16BE(3, 12); head.writeUInt16BE(0, 14); head.writeUInt32BE(head.length + mac.length, 16);
  return Buffer.concat([head, mac, win]);
}

// 3. Rewrite the sfnt directory with the new cmap (tag order, 4-byte aligned, checksums recomputed).
function withCmap(program) {
  const src = Buffer.from(program);
  const count = src.readUInt16BE(4);
  const tables = new Map();
  for (let i = 0; i < count; i++) {
    const at = 12 + i * 16;
    const tag = src.toString('latin1', at, at + 4);
    const offset = src.readUInt32BE(at + 8);
    tables.set(tag, src.subarray(offset, offset + src.readUInt32BE(at + 12)));
  }
  tables.set('cmap', cmapTable());
  const tags = [...tables.keys()].sort();
  const n = tags.length;
  const pow = 2 ** Math.floor(Math.log2(n));
  const header = Buffer.alloc(12 + n * 16);
  header.writeUInt32BE(src.readUInt32BE(0), 0);
  header.writeUInt16BE(n, 4);
  header.writeUInt16BE(pow * 16, 6);
  header.writeUInt16BE(Math.log2(pow), 8);
  header.writeUInt16BE(n * 16 - pow * 16, 10);
  const bodies = [];
  let offset = header.length;
  tags.forEach((tag, i) => {
    const data = tables.get(tag);
    const padded = Buffer.alloc((data.length + 3) & ~3);
    data.copy(padded);
    let sum = 0;
    for (let j = 0; j < padded.length; j += 4) sum = (sum + padded.readUInt32BE(j)) >>> 0;
    const at = 12 + i * 16;
    header.write(tag, at, 'latin1');
    header.writeUInt32BE(sum, at + 4);
    header.writeUInt32BE(offset, at + 8);
    header.writeUInt32BE(data.length, at + 12);
    bodies.push(padded);
    offset += padded.length;
  });
  return Buffer.concat([header, ...bodies]);
}

function toUnicode() {
  const entries = codeChars.map((ch, i) =>
    `<${(i + 1).toString(16).padStart(2, '0').toUpperCase()}> <${ch.charCodeAt(0).toString(16).padStart(4, '0').toUpperCase()}>`);
  return [
    '/CIDInit /ProcSet findresource begin', '12 dict begin', 'begincmap',
    '/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def',
    '/CMapName /Adobe-Identity-UCS def', '/CMapType 2 def',
    '1 begincodespacerange', '<00> <FF>', 'endcodespacerange',
    `${entries.length} beginbfchar`, ...entries, 'endbfchar',
    'endcmap', 'CMapName currentdict /CMap defineresource pop', 'end', 'end', '',
  ].join('\n');
}

function build() {
  const doc = new mupdf.PDFDocument();
  const program = doc.addStream(withCmap(subsetProgram()), doc.newDictionary());
  const descriptor = doc.addObject(doc.newDictionary());
  descriptor.put('Type', doc.newName('FontDescriptor'));
  descriptor.put('FontName', doc.newName('AAAAAA+NotoSansHebrew'));
  descriptor.put('Flags', 4);
  const box = doc.newArray();
  for (const value of [-500, -300, 1200, 1000]) box.push(value);
  descriptor.put('FontBBox', box);
  descriptor.put('ItalicAngle', 0);
  descriptor.put('Ascent', 1000);
  descriptor.put('Descent', -300);
  descriptor.put('CapHeight', 714);
  descriptor.put('StemV', 80);
  descriptor.put('FontFile2', program);
  const widths = doc.newArray();
  for (const gid of gids) widths.push(Math.round(handle.advanceGlyph(gid) * 1000));
  const font = doc.addObject(doc.newDictionary());
  font.put('Type', doc.newName('Font'));
  font.put('Subtype', doc.newName('TrueType'));
  font.put('BaseFont', doc.newName('AAAAAA+NotoSansHebrew'));
  font.put('FirstChar', 1);
  font.put('LastChar', codeChars.length);
  font.put('Widths', widths);
  font.put('FontDescriptor', descriptor);
  font.put('ToUnicode', doc.addStream(Buffer.from(toUnicode(), 'latin1'), doc.newDictionary()));
  const fonts = doc.newDictionary();
  fonts.put('F1', doc.addSimpleFont(new mupdf.Font('Helvetica'), mupdf.Font.SIMPLE_ENCODING_LATIN));
  fonts.put('F2', font);
  const resources = doc.newDictionary();
  resources.put('Font', fonts);
  const hex = (codes) => `<${codes.map((code) => code.toString(16).padStart(2, '0')).join('')}>`;
  const ops = [
    'BT /F1 12 Tf 72 740 Td (PDF4All) Tj ET',
    ...LINES.map(([, y], i) => `BT /F2 28 Tf 72 ${y} Td ${hex(lineCodes[i])} Tj ET`),
  ].join('\n');
  doc.insertPage(-1, doc.addPage([0, 0, 612, 792], 0, resources, Buffer.from(`${ops}\n`, 'latin1')));
  return doc.saveToBuffer({ compress: true, garbage: true }).asUint8Array();
}

const data = build();
const out = path.join(here, 'edit-coded.pdf');
writeFileSync(out, data);
const digest = createHash('sha256').update(data).digest('hex');
writeFileSync(path.join(here, 'edit-coded.provenance.json'), `${JSON.stringify({
  id: 'edit-coded',
  file: 'fixtures/p0b/edit-coded.pdf',
  bytes: data.length,
  sha256: digest,
  pages: 1,
  page_size_pt: [612, 792],
  notes: [
    '/F1 Helvetica WinAnsi control run (PDF4All) at 72 740',
    '/F2 symbolic simple TrueType AAAAAA+NotoSansHebrew: no /Encoding, /Flags 4, /FirstChar 1, /Widths, 1-byte /ToUnicode, /FontFile2 subset with a code->gid cmap',
    `/F2 28 pt runs (hex codes in first-use order): ${LINES.map(([text, y], i) => `${text} at 72 ${y} = ${lineCodes[i].join(',')}`).join('; ')}`,
    'the two x of line 3 are codes 13 and 14, both mapping to x: x is ambiguous and never in the repertoire',
  ],
  generator: {
    path: 'fixtures/p0b/make-edit-coded.mjs',
    helpers: ['fixtures/fetch-font.mjs'],
    sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
    reproduce: 'node fixtures/p0b/make-edit-coded.mjs',
    deterministic: true,
    timestamps_in_output: false,
  },
  provenance: {
    authorship: 'text written by the project team for the coded-fonts unit; font is upstream OSS',
    third_party_content: `one OSS font (${FONT.name}), subset, cmap replaced`,
    fonts: [{ name: FONT.name, licence: FONT.license, licence_text: FONT.licenceText, source_url: FONT.url, source_sha256: FONT.sha256, subset: true }],
    network_access: 'first run only, to fetch the pinned font',
  },
  license: "fixture content follows the app's final license choice; the font stays OFL-1.1",
  generated_on: '2026-09-25',
}, null, 2)}\n`);
console.log(`wrote ${out} (${data.length} bytes) sha256=${digest}`);
