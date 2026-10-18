#!/usr/bin/env node
/**
 * Print-to-PDF fixture (contract-print-edit.md D8): quartz.pdf, shaped like a macOS Quartz
 * "Save as PDF" page. Every glyph is its own block
 *   q <clip> re W n /Cs1 cs 0 0 0 sc q S 0 0 S X Y cm BT 1 0 0 1 0 0 Tm /TTn 1 Tf (c) Tj ET Q Q
 * with symbolic simple TrueType subsets (no /Encoding, /Flags 4, /FirstChar 33, codes in
 * first-use order, 1-byte /ToUnicode, /Widths) and NO space glyph: a word gap is only a
 * larger x step (0.278 em). The font programs are mupdf subsets of NotoSansHebrew (Latin
 * glyphs, ids kept) whose cmap is replaced by (1,0) and (3,0) format-4 subtables mapping
 * code (and 0xF000 + code) to the glyph id (technique of fixtures/p0b/make-edit-coded.mjs).
 * /TT3 (D11) is the other Quartz shape: a non-symbolic subset with /Encoding /MacRomanEncoding,
 * /Flags 32, /FirstChar 32, codes = ASCII, /Widths, no /ToUnicode, same cmap technique; its
 * spaces are explicit 0x20 blocks and every other block's cm d is a - 0.00001 (D12).
 *
 * Run: node fixtures/p6/make-quartz.mjs
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
const GAP = 0.278;
const BODY = [
  'Applicants must read these notes before filling',
  'in the form. Every section applies to new owners',
  'and to renewals of an existing business licence.',
  'Print clearly in black ink and sign the last page.',
  'Attach a copy of the lease or the property deed.',
  'Home based operators also need written consent',
  'from the owner of the building where they work.',
  'Fees are payable by cheque or by card at the desk.',
  'A licence is valid for one year from the date shown.',
  'Renewal notices are mailed thirty days in advance.',
  'Change of address must be reported within ten days.',
  'Keep the licence on display at the place of business.',
  'Inspectors may visit during normal opening hours.',
  'Send any other questions to the licensing office.',
];
// [font, size, text, x, y]; x null = continue from the previous run's pen, y offset from its baseline.
const RUNS = [
  ['TT1', 16, 'Business Licence Application Form', 72, 720],
  ['TT1', 10, 'Unit', 72, 680],
  ['TT1', 10, 'Postal code', 320, 680],
  ['TT1', 10, 'Suite and Street address', 72, 660],
  ['TT1', 10, 'Business', 72, 640],
  ['TT2', 10, 't', null, 0],
  ['TT1', 10, 'rade name', null, 0],
  ['TT3', 10, 'Office use only', 320, 640],
  ['TT1', 10, 'Passed all assessments', 72, 620],
  ['TT1', 10, 'Floor 2', 72, 600],
  ['TT1', 6, 'nd', null, 4],
  ...BODY.map((text, i) => ['TT1', 9, text, 72, 560 - 14 * i]),
];

const ttf = await fontBytes(FONT);
const handle = new mupdf.Font(FONT.name, ttf);
const round4 = (value) => Math.round(value * 10000) / 10000;
const num = (value) => String(round4(value));

// Codes per font from 33, in first-use order; no space glyph anywhere.
const fonts = new Map([['TT1', { chars: [], prefix: 'AAAAAA' }], ['TT2', { chars: [], prefix: 'BAAAAA' }], ['TT3', { chars: [], prefix: 'CAAAAA', ascii: true }]]);
for (const [name, , text] of RUNS) {
  const font = fonts.get(name);
  for (const ch of text) if ((ch !== ' ' || font.ascii) && !font.chars.includes(ch)) font.chars.push(ch);
}
for (const font of fonts.values()) {
  font.gids = font.chars.map((ch) => {
    const gid = handle.encodeCharacter(ch);
    if (!gid) throw new Error(`no glyph for ${ch}`);
    return gid;
  });
  font.widths = font.gids.map((gid) => Math.round(handle.advanceGlyph(gid) * 1000));
  font.codes = font.chars.map((ch, i) => (font.ascii ? ch.charCodeAt(0) : 33 + i));
}

// 1. A mupdf subset of the font (glyph ids kept): draw the glyphs through Identity-H and subset.
function subsetProgram(gids) {
  const doc = new mupdf.PDFDocument();
  const dict = doc.newDictionary();
  dict.put('F1', doc.addFont(handle));
  const resources = doc.newDictionary();
  resources.put('Font', dict);
  const hex = gids.map((gid) => gid.toString(16).padStart(4, '0')).join('');
  doc.insertPage(-1, doc.addPage([0, 0, 612, 792], 0, resources, Buffer.from(`BT /F1 12 Tf 72 700 Td <${hex}> Tj ET\n`, 'latin1')));
  doc.subsetFonts();
  const descriptor = doc.loadPage(0).getObject().get('Resources').get('Font').get('F1')
    .get('DescendantFonts').get(0).get('FontDescriptor');
  return Uint8Array.from(descriptor.get('FontFile2').readStream().asUint8Array());
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
    out.writeUInt16BE(code, 14 + i * 2);
    out.writeUInt16BE(code, 16 + n * 2 + i * 2);
    out.writeUInt16BE(code === 0xffff ? 1 : (gid - code + 0x10000) & 0xffff, 16 + n * 4 + i * 2);
    out.writeUInt16BE(0, 16 + n * 6 + i * 2);
  });
  return out;
}

function cmapTable(gids, codes) {
  const sorted = gids.map((gid, i) => [codes[i], gid]).sort((a, b) => a[0] - b[0]);
  const mac = format4(sorted);
  const win = format4(sorted.map(([code, gid]) => [0xf000 + code, gid]));
  const head = Buffer.alloc(4 + 2 * 8);
  head.writeUInt16BE(0, 0);
  head.writeUInt16BE(2, 2);
  head.writeUInt16BE(1, 4); head.writeUInt16BE(0, 6); head.writeUInt32BE(head.length, 8);
  head.writeUInt16BE(3, 12); head.writeUInt16BE(0, 14); head.writeUInt32BE(head.length + mac.length, 16);
  return Buffer.concat([head, mac, win]);
}

// 3. Rewrite the sfnt directory with the new cmap (tag order, 4-byte aligned, checksums recomputed).
function withCmap(program, gids, codes) {
  const src = Buffer.from(program);
  const count = src.readUInt16BE(4);
  const tables = new Map();
  for (let i = 0; i < count; i++) {
    const at = 12 + i * 16;
    const tag = src.toString('latin1', at, at + 4);
    const offset = src.readUInt32BE(at + 8);
    tables.set(tag, src.subarray(offset, offset + src.readUInt32BE(at + 12)));
  }
  tables.set('cmap', cmapTable(gids, codes));
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

function toUnicode(chars) {
  const entries = chars.map((ch, i) =>
    `<${(33 + i).toString(16).padStart(2, '0').toUpperCase()}> <${ch.charCodeAt(0).toString(16).padStart(4, '0').toUpperCase()}>`);
  return [
    '/CIDInit /ProcSet findresource begin', '12 dict begin', 'begincmap',
    '/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def',
    '/CMapName /Adobe-Identity-UCS def', '/CMapType 2 def',
    '1 begincodespacerange', '<00> <FF>', 'endcodespacerange',
    `${entries.length} beginbfchar`, ...entries, 'endbfchar',
    'endcmap', 'CMapName currentdict /CMap defineresource pop', 'end', 'end', '',
  ].join('\n');
}

function fontObject(doc, font) {
  const program = doc.addStream(withCmap(subsetProgram(font.gids), font.gids, font.codes), doc.newDictionary());
  const descriptor = doc.addObject(doc.newDictionary());
  descriptor.put('Type', doc.newName('FontDescriptor'));
  descriptor.put('FontName', doc.newName(`${font.prefix}+NotoSansHebrew`));
  descriptor.put('Flags', font.ascii ? 32 : 4);
  const box = doc.newArray();
  for (const value of [-500, -300, 1200, 1000]) box.push(value);
  descriptor.put('FontBBox', box);
  descriptor.put('ItalicAngle', 0);
  descriptor.put('Ascent', 1000);
  descriptor.put('Descent', -300);
  descriptor.put('CapHeight', 714);
  descriptor.put('StemV', 80);
  descriptor.put('FontFile2', program);
  const firstChar = font.ascii ? 32 : 33;
  const lastChar = Math.max(...font.codes);
  const widths = doc.newArray();
  for (let code = firstChar; code <= lastChar; code++) {
    const index = font.codes.indexOf(code);
    widths.push(index < 0 ? 0 : font.widths[index]);
  }
  const dict = doc.addObject(doc.newDictionary());
  dict.put('Type', doc.newName('Font'));
  dict.put('Subtype', doc.newName('TrueType'));
  dict.put('BaseFont', doc.newName(`${font.prefix}+NotoSansHebrew`));
  dict.put('FirstChar', firstChar);
  dict.put('LastChar', lastChar);
  dict.put('Widths', widths);
  dict.put('FontDescriptor', descriptor);
  if (font.ascii) dict.put('Encoding', doc.newName('MacRomanEncoding'));
  else dict.put('ToUnicode', doc.addStream(Buffer.from(toUnicode(font.chars), 'latin1'), doc.newDictionary()));
  return dict;
}

/** The glyph blocks, one per non-space character, with the clip and colour group Quartz writes. */
function blocks() {
  const out = [];
  let penX = 0;
  let penY = 0;
  for (const [name, size, text, x, y] of RUNS) {
    const font = fonts.get(name);
    let pen = x === null ? penX : x;
    const baseline = x === null ? penY + y : y;
    let space = false;
    let n = 0;
    for (const ch of text) {
      if (ch === ' ' && !font.ascii) { space = true; continue; }
      if (space) pen = round4(pen + GAP * size);
      space = false;
      const index = font.chars.indexOf(ch);
      const code = font.codes[index];
      const adv = (font.widths[index] / 1000) * size;
      const x0 = Math.floor(pen), y0 = Math.floor(baseline - 0.25 * size);
      const x1 = Math.ceil(pen + adv), y1 = Math.ceil(baseline + 1.1 * size);
      const byte = String.fromCharCode(code);
      const literal = byte === '(' || byte === ')' || byte === '\\' ? `\\${byte}` : byte;
      const d = font.ascii && n++ % 2 ? (size * 100000 - 1) / 100000 : size;
      out.push(`q ${x0} ${y0} ${x1 - x0} ${y1 - y0} re W n /Cs1 cs 0 0 0 sc q ${size} 0 0 ${d} ${num(pen)} ${num(baseline)} cm BT 1 0 0 1 0 0 Tm /${name} 1 Tf (${literal}) Tj ET Q Q`);
      pen = round4(pen + adv);
    }
    penX = pen;
    penY = baseline;
  }
  return out;
}

function build() {
  const doc = new mupdf.PDFDocument();
  const dict = doc.newDictionary();
  dict.put('F1', doc.addSimpleFont(new mupdf.Font('Helvetica'), mupdf.Font.SIMPLE_ENCODING_LATIN));
  for (const [name, font] of fonts) dict.put(name, fontObject(doc, font));
  const spaces = doc.newDictionary();
  spaces.put('Cs1', doc.newName('DeviceRGB'));
  const resources = doc.newDictionary();
  resources.put('Font', dict);
  resources.put('ColorSpace', spaces);
  const glyphs = blocks();
  const ops = ['BT /F1 12 Tf 72 750 Td (PDF4All control) Tj ET', ...glyphs].join('\n');
  doc.insertPage(-1, doc.addPage([0, 0, 612, 792], 0, resources, Buffer.from(`${ops}\n`, 'latin1')));
  return { data: doc.saveToBuffer({ compress: true, garbage: true }).asUint8Array(), count: glyphs.length };
}

const { data, count } = build();
const out = path.join(here, 'quartz.pdf');
writeFileSync(out, data);
const digest = createHash('sha256').update(data).digest('hex');
writeFileSync(path.join(here, 'quartz.provenance.json'), `${JSON.stringify({
  id: 'quartz',
  file: 'fixtures/p6/quartz.pdf',
  bytes: data.length,
  sha256: digest,
  pages: 1,
  page_size_pt: [612, 792],
  notes: [
    '/F1 Helvetica WinAnsi control run (PDF4All control) at 72 750 in a plain BT ... ET',
    `${count} Quartz-style glyph blocks: q <floor/ceil clip> re W n /Cs1 cs 0 0 0 sc q S 0 0 S X Y cm BT 1 0 0 1 0 0 Tm /TTn 1 Tf (c) Tj ET Q Q`,
    '/TT1 and /TT2: symbolic simple TrueType subsets of NotoSansHebrew, no /Encoding, /Flags 4, /FirstChar 33, codes in first-use order, /Widths, 1-byte /ToUnicode, /FontFile2 with a code->gid cmap; no space glyph (word gap 0.278 em)',
    `/TT1 chars in code order from 33: ${fonts.get('TT1').chars.join('')}; /TT2: ${fonts.get('TT2').chars.join('')}`,
    '/TT3 (D11): non-symbolic simple TrueType subset of NotoSansHebrew, /Encoding /MacRomanEncoding, /Flags 32, /FirstChar 32, codes = ASCII, /Widths (0 for undrawn codes), no /ToUnicode, /FontFile2 with a code->gid cmap',
    '/TT3 line (D12): its two spaces are explicit 0x20 glyph blocks (pen advances by the space width), and every other block (2nd, 4th, ...) writes cm d = S - 0.00001',
    `runs [font, size, text, x, y] (x null = continues the pen, y = rise): ${JSON.stringify(RUNS)}`,
  ],
  generator: {
    path: 'fixtures/p6/make-quartz.mjs',
    helpers: ['fixtures/fetch-font.mjs'],
    sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
    reproduce: 'node fixtures/p6/make-quartz.mjs',
    deterministic: true,
    timestamps_in_output: false,
  },
  provenance: {
    authorship: 'text written by the project team for the print-edit unit; font is upstream OSS',
    third_party_content: `one OSS font (${FONT.name}), subset twice, cmap replaced`,
    fonts: [{ name: FONT.name, licence: FONT.license, licence_text: FONT.licenceText, source_url: FONT.url, source_sha256: FONT.sha256, subset: true }],
    network_access: 'first run only, to fetch the pinned font',
  },
  license: "fixture content is MIT (LICENSE at the repo root); the font stays OFL-1.1",
  generated_on: '2026-09-30',
}, null, 2)}\n`);
console.log(`wrote ${out} (${data.length} bytes) sha256=${digest} glyph blocks=${count}`);
