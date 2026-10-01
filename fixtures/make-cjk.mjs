#!/usr/bin/env node
/**
 * Deterministic generator for fixtures/cjk.pdf.
 *
 * Japanese + Simplified Chinese strings with an embedded, subset OSS font
 * (Droid Sans Fallback, Apache-2.0) plus a Base-14 Latin marker line, because
 * Droid Sans Fallback carries no Latin glyphs. Run:
 *   node fixtures/make-cjk.mjs
 * Needs the network only on the first run, to fetch the pinned font.
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fontBytes } from './fetch-font.mjs';
import { makeTextPdf } from './textpdf.mjs';

const here = import.meta.dirname;
const OUT = path.join(here, 'cjk.pdf');

const FONT = {
  name: 'DroidSansFallback.ttf',
  url: 'https://github.com/aosp-mirror/platform_frameworks_base/raw/master/data/fonts/DroidSansFallback.ttf',
  sha256: '21b96a0377f067833a93af3082eb28d4ffab7a8cd46bfd513286f1d64b7b0949',
  license: 'Apache-2.0 (Copyright 2005-2008 The Android Open Source Project)',
  notice: 'docs/third-party/droid-sans-fallback-NOTICE.txt',
};

const JAPANESE = '日本語テスト';
const CHINESE = '中文测试';

const cjk = await fontBytes(FONT);
const bytes = makeTextPdf({
  runs: [
    { font: { resource: 'F1', name: 'Helvetica' }, text: 'PDF4All', size: 20, x: 72, y: 740 },
    { font: { resource: 'F2', name: 'DroidSansFallback', bytes: cjk }, text: JAPANESE, size: 28, x: 72, y: 660 },
    { font: { resource: 'F2', name: 'DroidSansFallback', bytes: cjk }, text: CHINESE, size: 28, x: 72, y: 600 },
  ],
});
writeFileSync(OUT, bytes);

const digest = createHash('sha256').update(bytes).digest('hex');
writeFileSync(
  path.join(here, 'cjk.provenance.json'),
  `${
    JSON.stringify(
      {
        id: 'cjk',
        file: 'fixtures/cjk.pdf',
        bytes: bytes.length,
        sha256: digest,
        pages: 1,
        page_size_pt: [612, 792],
        strings: { japanese: JAPANESE, chinese: CHINESE, marker: 'PDF4All' },
        fonts: [
          {
            role: 'CJK text (F2)',
            name: FONT.name,
            licence: FONT.license,
            licence_text: FONT.notice,
            source_url: FONT.url,
            source_sha256: FONT.sha256,
            subset: 'embedded by MuPDF PDFDocument.subsetFonts() at generation time; only the glyphs used are in the file',
          },
          {
            role: 'Latin marker (F1)',
            name: 'Base-14 Helvetica',
            licence: 'not embedded; reference only (MuPDF renders it with the URW Nimbus Sans built into the wasm)',
          },
        ],
        generator: {
          path: 'fixtures/make-cjk.mjs',
          helpers: ['fixtures/textpdf.mjs', 'fixtures/fetch-font.mjs'],
          sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
          dependencies: ['mupdf 1.28.1 (engine + font subsetting)', 'Node >= 20 global fetch (generation only)'],
          reproduce: 'node fixtures/make-cjk.mjs',
          network: 'first run only, to fetch the pinned font; recorded sha256 is verified',
          deterministic: true,
          timestamps_in_output: false,
        },
        provenance: {
          authorship: 'text written by the project team for this spike (ds2); font is upstream open source',
          third_party_content: `one OSS font subset (${FONT.name})`,
          network_access: 'none at test time; generation fetches the pinned font once',
        },
        extraction_contract: {
          'mupdf_1.28.1': `asText() contains "${JAPANESE}" and "${CHINESE}"; search() returns one hit each`,
          note: 'Droid Sans Fallback has no Latin glyphs, hence the separate Helvetica marker line',
        },
        license: "fixture content follows the app's final license choice; the embedded font subset stays Apache-2.0",
      },
      null,
      2,
    )
  }\n`,
);
console.log(`wrote ${OUT} (${bytes.length} bytes) sha256=${digest}`);
