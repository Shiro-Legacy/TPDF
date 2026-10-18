#!/usr/bin/env node
/**
 * Deterministic generator for fixtures/rtl.pdf.
 *
 * Hebrew and Arabic lines with embedded, subset OSS fonts (Noto Sans Hebrew /
 * Noto Sans Arabic, both OFL-1.1), plus a Base-14 Latin marker line. Run:
 *   node fixtures/make-rtl.mjs
 * Needs the network only on the first run, to fetch the pinned fonts.
 *
 * Bidi/shaping are NOT implemented by this generator - see the comments on
 * HEBREW and ARABIC below and progress.md §5 Fixtures for what the fixture
 * actually contains and what extraction returns.
 */
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fontBytes } from './fetch-font.mjs';
import { makeTextPdf } from './textpdf.mjs';

const here = import.meta.dirname;
const OUT = path.join(here, 'rtl.pdf');

const HEBREW_FONT = {
  name: 'NotoSansHebrew[wdth,wght].ttf',
  url: 'https://github.com/google/fonts/raw/main/ofl/notosanshebrew/NotoSansHebrew%5Bwdth,wght%5D.ttf',
  sha256: '7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88',
  license: 'OFL-1.1',
  licenceText: 'docs/third-party/notosanshebrew-OFL-1.1.txt',
};
const ARABIC_FONT = {
  name: 'NotoSansArabic[wdth,wght].ttf',
  url: 'https://github.com/google/fonts/raw/main/ofl/notosansarabic/NotoSansArabic%5Bwdth,wght%5D.ttf',
  sha256: '63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e',
  license: 'OFL-1.1',
  licenceText: 'docs/third-party/notosansarabic-OFL-1.1.txt',
};

const HEBREW = 'שלום';
const ARABIC = 'مرحبا';

// The fixture stores glyphs in visual order, right to left, because that is
// what a PDF content stream does: the generator has no bidi engine, so it
// reverses the logical string itself.
const HEBREW_VISUAL = [...HEBREW].reverse().join('');

// Minimal hand-shaping: "مرحبا" written with the contextual (presentation)
// forms so the letters join, in logical order m-r-h-b-a:
//   م U+FEE3 initial, ر U+FEAE final, ح U+FEA3 initial,
//   ب U+FE92 medial,  ا U+FE8E final
// then reversed into visual order. No harfbuzz involved; the forms are hard
// coded for this one word.
const ARABIC_VISUAL = [...'\uFEE3\uFEAE\uFEA3\uFE92\uFE8E'].reverse().join('');

const [hebrew, arabic] = await Promise.all([fontBytes(HEBREW_FONT), fontBytes(ARABIC_FONT)]);
const bytes = makeTextPdf({
  runs: [
    { font: { resource: 'F1', name: 'Helvetica' }, text: 'PDF4All', size: 20, x: 72, y: 740 },
    { font: { resource: 'F2', name: 'NotoSansHebrew', bytes: hebrew }, text: HEBREW_VISUAL, size: 28, x: 72, y: 660 },
    { font: { resource: 'F2', name: 'NotoSansHebrew', bytes: hebrew }, text: ' PDF4All', size: 28, x: 160, y: 660 },
    { font: { resource: 'F3', name: 'NotoSansArabic', bytes: arabic }, text: ARABIC_VISUAL, size: 28, x: 72, y: 590 },
    { font: { resource: 'F2', name: 'NotoSansHebrew', bytes: hebrew }, text: ' PDF4All', size: 28, x: 170, y: 590 },
  ],
});
writeFileSync(OUT, bytes);

const digest = createHash('sha256').update(bytes).digest('hex');
writeFileSync(
  path.join(here, 'rtl.provenance.json'),
  `${
    JSON.stringify(
      {
        id: 'rtl',
        file: 'fixtures/rtl.pdf',
        bytes: bytes.length,
        sha256: digest,
        pages: 1,
        page_size_pt: [612, 792],
        strings: { hebrew: HEBREW, arabic: ARABIC, marker: 'PDF4All' },
        glyph_order: {
          hebrew: `${HEBREW_VISUAL} - visual (right-to-left) order, unshaped base letters; MuPDF structured text restores logical order on extraction`,
          arabic: `U+FEE3 U+FEAE U+FEA3 U+FE92 U+FE8E reversed - hard-coded contextual forms in visual order, no shaping engine involved`,
        },
        fonts: [
          {
            role: 'Hebrew + Latin marker (F2)',
            name: HEBREW_FONT.name,
            licence: HEBREW_FONT.license,
            licence_text: HEBREW_FONT.licenceText,
            source_url: HEBREW_FONT.url,
            source_sha256: HEBREW_FONT.sha256,
            note: 'variable font (wdth,wght); MuPDF uses the default instance (400/100)',
            subset: 'embedded by MuPDF PDFDocument.subsetFonts() at generation time',
          },
          {
            role: 'Arabic (F3)',
            name: ARABIC_FONT.name,
            licence: ARABIC_FONT.license,
            licence_text: ARABIC_FONT.licenceText,
            source_url: ARABIC_FONT.url,
            source_sha256: ARABIC_FONT.sha256,
            note: 'variable font (wdth,wght); MuPDF uses the default instance (400/100)',
            subset: 'embedded by MuPDF PDFDocument.subsetFonts() at generation time',
          },
          {
            role: 'Latin marker header (F1)',
            name: 'Base-14 Helvetica',
            licence: 'not embedded; reference only',
          },
        ],
        generator: {
          path: 'fixtures/make-rtl.mjs',
          helpers: ['fixtures/textpdf.mjs', 'fixtures/fetch-font.mjs'],
          sha256: createHash('sha256').update(readFileSync(new URL(import.meta.url))).digest('hex'),
          dependencies: ['mupdf 1.28.1 (engine + font subsetting)', 'Node >= 20 global fetch (generation only)'],
          reproduce: 'node fixtures/make-rtl.mjs',
          network: 'first run only, to fetch the pinned fonts; recorded sha256 is verified',
          deterministic: true,
          timestamps_in_output: false,
        },
        provenance: {
          authorship: 'text and hand-shaping done by the project team for this spike (ds2); fonts are upstream OSS',
          third_party_content: `two OSS font subsets (${HEBREW_FONT.name}, ${ARABIC_FONT.name})`,
          network_access: 'none at test time; generation fetches the pinned fonts once',
        },
        extraction_contract: {
          'mupdf_1.28.1': `asText() contains "${HEBREW} PDF4All" and "${ARABIC} PDF4All"; search() returns one hit each`,
          known_limitation: 'Arabic is shaped by hand (5 hard-coded presentation forms), not by a shaper; this fixture proves glyph embedding + extraction, not Arabic line layout',
        },
        license: "fixture content is MIT (LICENSE at the repo root); the embedded font subsets stay OFL-1.1",
      },
      null,
      2,
    )
  }\n`,
);
console.log(`wrote ${OUT} (${bytes.length} bytes) sha256=${digest}`);
