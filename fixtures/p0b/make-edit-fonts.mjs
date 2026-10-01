#!/usr/bin/env node
/**
 * P0-b probe-1 font fixtures: edit-embedded.pdf (full TTF) and edit-subset.pdf
 * (same strings, subsetFonts). Uses fixtures/.fonts/DroidSansFallback.ttf via
 * fixtures/fetch-font.mjs + mupdf 1.28.1. Never shipped.
 *
 * Run: node fixtures/p0b/make-edit-fonts.mjs
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import mupdf from 'mupdf';
import { fontBytes } from '../fetch-font.mjs';

const here = import.meta.dirname;
const OLD = '日本語テスト';
const NEW = '日本語試験'; // 試/験 are not in OLD, so a subset of OLD cannot draw NEW
const NEI = '隣の行';
const FONT = {
  name: 'DroidSansFallback.ttf',
  url: 'https://github.com/aosp-mirror/platform_frameworks_base/raw/master/data/fonts/DroidSansFallback.ttf',
  sha256: '21b96a0377f067833a93af3082eb28d4ffab7a8cd46bfd513286f1d64b7b0949',
  license: 'Apache-2.0',
};

function build({ bytes, subset }) {
  const doc = new mupdf.PDFDocument();
  const handle = new mupdf.Font(FONT.name, bytes);
  const fontDict = doc.newDictionary();
  fontDict.put('F1', doc.addSimpleFont(new mupdf.Font('Helvetica'), mupdf.Font.SIMPLE_ENCODING_LATIN));
  fontDict.put('F2', doc.addFont(handle));
  const resources = doc.newDictionary();
  resources.put('Font', fontDict);
  const codes = (text) => {
    const hex = [...text].map((ch) => {
      const code = handle.encodeCharacter(ch);
      if (!code) throw new Error(`no glyph U+${ch.codePointAt(0).toString(16)}`);
      return code.toString(16).padStart(4, '0');
    });
    return `<${hex.join('')}>`;
  };
  const ops = [
    `BT /F1 12 Tf 72 740 Td (PDF4All) Tj ET`,
    `BT /F2 28 Tf 72 660 Td ${codes(OLD)} Tj ET`,
    `BT /F2 28 Tf 72 600 Td ${codes(NEI)} Tj ET`,
  ].join('\n');
  const page = doc.addPage([0, 0, 612, 792], 0, resources, Buffer.from(`${ops}\n`, 'latin1'));
  doc.insertPage(-1, page);
  if (subset) doc.subsetFonts();
  return doc.saveToBuffer({ compress: true, garbage: true }).asUint8Array();
}

function write(name, data, extra) {
  const out = path.join(here, name);
  writeFileSync(out, data);
  const digest = createHash('sha256').update(data).digest('hex');
  writeFileSync(
    out.replace(/\.pdf$/, '.provenance.json'),
    `${JSON.stringify({
      id: name.replace('.pdf', ''),
      file: `fixtures/p0b/${name}`,
      bytes: data.length,
      sha256: digest,
      pages: 1,
      page_size_pt: [612, 792],
      old: OLD,
      new: NEW,
      neighbor: NEI,
      ...extra,
      generator: {
        path: 'fixtures/p0b/make-edit-fonts.mjs',
        helpers: ['fixtures/fetch-font.mjs'],
        sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
        reproduce: 'node fixtures/p0b/make-edit-fonts.mjs',
        deterministic: true,
        timestamps_in_output: false,
      },
      provenance: {
        authorship: 'text written by the project team for this spike (grok); font is upstream OSS',
        third_party_content: `one OSS font (${FONT.name})`,
        fonts: [{ name: FONT.name, licence: FONT.license, source_sha256: FONT.sha256, subset: extra.subset }],
        network_access: 'first run only, to fetch the pinned font',
      },
      license: "fixture content follows the app's final license choice; the font stays Apache-2.0",
      generated_on: '2026-09-16',
    }, null, 2)}\n`,
  );
  console.log(`wrote ${out} (${data.length} bytes) sha256=${digest}`);
}

const ttf = await fontBytes(FONT);
write('edit-embedded.pdf', build({ bytes: ttf, subset: false }), {
  subset: false,
  note: 'full embedded DroidSansFallback; NEW glyphs 試/験 should exist',
});
write('edit-subset.pdf', build({ bytes: ttf, subset: true }), {
  subset: true,
  note: 'subsetFonts of OLD+NEI only; NEW glyphs 試/験 must be reported missing',
});
