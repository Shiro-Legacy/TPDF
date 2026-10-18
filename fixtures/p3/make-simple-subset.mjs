#!/usr/bin/env node
/**
 * P3 U3 §2.4 fixture: subset *simple* TrueType (NotoSansHebrew, WinAnsi) drawing `ALPHA`.
 * `Z` is not in the subset: mupdf encodeCharacter('Z') on the stext font handle returns 0
 * (probe: .quad/shared/p3/p3-unit3-probe.md item 4). DroidSansFallback has no Latin glyphs.
 * Run: node fixtures/p3/make-simple-subset.mjs
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import mupdf from 'mupdf';
import { fontBytes } from '../fetch-font.mjs';

const DIR = import.meta.dirname;
const FONT = {
  name: 'NotoSansHebrew[wdth,wght].ttf',
  url: 'https://github.com/google/fonts/raw/main/ofl/notosanshebrew/NotoSansHebrew%5Bwdth,wght%5D.ttf',
  sha256: '7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88',
  license: 'OFL-1.1',
  licenceText: 'docs/third-party/notosanshebrew-OFL-1.1.txt',
};
const TEXT = 'ALPHA';

const doc = new mupdf.PDFDocument();
const fontDict = doc.newDictionary();
fontDict.put('F1', doc.addSimpleFont(new mupdf.Font(FONT.name, await fontBytes(FONT)), mupdf.Font.SIMPLE_ENCODING_LATIN));
const resources = doc.newDictionary();
resources.put('Font', fontDict);
const ops = `BT /F1 40 Tf 72 600 Td (${TEXT}) Tj ET\n`;
doc.insertPage(-1, doc.addPage([0, 0, 612, 792], 0, resources, Buffer.from(ops, 'latin1')));
doc.subsetFonts();
const data = doc.saveToBuffer({ compress: true, garbage: true }).asUint8Array();
const file = path.join(DIR, 'simple-subset.pdf');
writeFileSync(file, data);

const provenance = {
  id: 'simple-subset',
  file: 'fixtures/p3/simple-subset.pdf',
  bytes: data.length,
  sha256: createHash('sha256').update(data).digest('hex'),
  pages: 1,
  page_size_pt: [612, 792],
  notes: [
    `/F1 subset TrueType, WinAnsiEncoding, 40 pt, ${TEXT} Tj at 72 600; subset keeps A L P H only`,
    "encodeCharacter('Z') on the subset handle = 0; forcing (ALPHZ) draws a .notdef box",
  ],
  generator: {
    path: 'fixtures/p3/make-simple-subset.mjs',
    helpers: ['fixtures/fetch-font.mjs'],
    sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
    reproduce: 'node fixtures/p3/make-simple-subset.mjs',
    deterministic: true,
    timestamps_in_output: false,
  },
  provenance: {
    authorship: 'text written by the project team for P3 U3; font is upstream OSS',
    third_party_content: `one OSS font (${FONT.name}), subset`,
    fonts: [{ name: FONT.name, licence: FONT.license, licence_text: FONT.licenceText, source_url: FONT.url, source_sha256: FONT.sha256, subset: true }],
    network_access: 'first run only, to fetch the pinned font',
  },
  license: "fixture content is MIT (LICENSE at the repo root); the font stays OFL-1.1",
  generated_on: '2026-09-17',
};
writeFileSync(path.join(DIR, 'simple-subset.provenance.json'), `${JSON.stringify(provenance, null, 2)}\n`);
console.log(`wrote ${file} (${data.length} bytes) sha256=${provenance.sha256}`);
