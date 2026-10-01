#!/usr/bin/env node
/** P3 U3 subset-font fixture. Run: node fixtures/p3/make-edit3.mjs */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import mupdf from 'mupdf';
import { fontBytes } from '../fetch-font.mjs';

const DIR = import.meta.dirname;
const FONT = {
  name: 'DroidSansFallback.ttf',
  url: 'https://github.com/aosp-mirror/platform_frameworks_base/raw/master/data/fonts/DroidSansFallback.ttf',
  sha256: '21b96a0377f067833a93af3082eb28d4ffab7a8cd46bfd513286f1d64b7b0949',
  license: 'Apache-2.0',
};
const bytes = await fontBytes(FONT);
const doc = new mupdf.PDFDocument();
const f2 = new mupdf.Font(FONT.name, bytes);
const f3 = new mupdf.Font(FONT.name, bytes);
const f3doc = new mupdf.PDFDocument();
const fontDict = doc.newDictionary();
fontDict.put('F1', doc.addSimpleFont(new mupdf.Font('Helvetica'), mupdf.Font.SIMPLE_ENCODING_LATIN));
fontDict.put('F2', doc.addFont(f2));
fontDict.put('F3', doc.graftObject(f3doc.addFont(f3)));
const resources = doc.newDictionary();
resources.put('Font', fontDict);

const hexCodes = (font, text) => [...text].map((ch) => {
  const code = font.encodeCharacter(ch);
  if (!code) throw new Error(`no glyph U+${ch.codePointAt(0).toString(16)}`);
  return code.toString(16).padStart(4, '0');
});
const hex = (font, text) => `<${hexCodes(font, text).join('')}>`;
const lines = [
  { font: '/F2', text: '日本語テスト', codes: hexCodes(f2, '日本語テスト') },
  { font: '/F2', text: 'テスト', codes: hexCodes(f2, 'テスト') },
  { font: '/F2', text: '日本 語', codes: [...hexCodes(f2, '日本'), '-500', ...hexCodes(f2, '語')] },
  { font: '/F3', text: '試験', codes: hexCodes(f3, '試験') },
  { font: '/F1', text: 'NEIGHBOR', codes: [...Buffer.from('NEIGHBOR')].map((n) => n.toString(16).padStart(2, '0')) },
];
const ops = [
  `BT /F2 28 Tf 72 700 Td ${hex(f2, '日本語テスト')} Tj ET`,
  `BT /F2 28 Tf 72 640 Td ${hex(f2, 'テスト')} Tj ET`,
  `BT /F2 28 Tf 72 580 Td [${hex(f2, '日本')} -500 ${hex(f2, '語')}] TJ ET`,
  `BT /F3 28 Tf 72 520 Td ${hex(f3, '試験')} Tj ET`,
  'BT /F1 12 Tf 72 460 Td (NEIGHBOR) Tj ET',
].join('\n');
const page = doc.addPage([0, 0, 612, 792], 0, resources, Buffer.from(`${ops}\n`, 'latin1'));
doc.insertPage(-1, page);
doc.subsetFonts();
const data = doc.saveToBuffer({ compress: true, garbage: true }).asUint8Array();
const file = path.join(DIR, 'edit-subset2.pdf');
writeFileSync(file, data);

const provenance = {
  id: 'edit-subset2',
  file: 'fixtures/p3/edit-subset2.pdf',
  bytes: data.length,
  sha256: createHash('sha256').update(data).digest('hex'),
  pages: 1,
  page_size_pt: [612, 792],
  notes: lines.map(({ font, text, codes }, i) => `line ${i + 1} ${font} ${text}: ${codes.join(' ')}`),
  generator: {
    path: 'fixtures/p3/make-edit3.mjs',
    helpers: ['fixtures/fetch-font.mjs'],
    sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
    reproduce: 'node fixtures/p3/make-edit3.mjs',
    deterministic: true,
    timestamps_in_output: false,
  },
  provenance: {
    authorship: 'text written by the project team for P3 U3; font is upstream OSS',
    third_party_content: `one OSS font (${FONT.name}), embedded twice as distinct font objects`,
    fonts: [{ name: FONT.name, licence: FONT.license, source_sha256: FONT.sha256, subset: true }],
    network_access: 'first run only, to fetch the pinned font',
  },
  license: "fixture content follows the app's final license choice; the font stays Apache-2.0",
  generated_on: '2026-09-17',
};
writeFileSync(path.join(DIR, 'edit-subset2.provenance.json'), `${JSON.stringify(provenance, null, 2)}\n`);
console.log(`wrote fixtures/p3/edit-subset2.pdf (${data.length} bytes)`);
