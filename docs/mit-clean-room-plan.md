# Relicensing the engine MIT: clean-room plan

Status: proposal, 2026-10-17. Nothing has been rewritten yet.

## Goal

Ship the whole product (desktop app, mobile app, Rust engine and WASM) under
MIT. Today the engine is AGPL-3.0-or-later because parts of it were written
from MuPDF 1.28.1 source knowledge. Those parts are rewritten from the PDF
specification by implementers who never see the MuPDF-derived code. MuPDF stays
a black-box oracle: we compare outputs, never code.

Caveat: an AI implementer has probably seen MuPDF's source in training. A
lawyer should judge how much protection this clean room gives before the MIT
release.

## Audit result (four read-only audits, 2026-10-17)

### Already fine
- The default Rust build ships no MuPDF code. The npm `mupdf` package only
  serves the optional `--engine=mupdf` build and the tests.
- Every Cargo dependency is permissive. Fonts are OFL-1.1 (URW) or Apache-2.0
  (Droid Sans Fallback). JBIG2 is hayro-jbig2, not jbig2dec.
- Fixtures are self-authored.
- Most of the app layer (redaction, signing, page ops, editing, navigation,
  journal, page tree) is the app's own design. MuPDF appears there only as
  parity notes, so those comments get reworded.

### Dead but shipped (class B): delete, no rewrite
The legacy `RustEngine` WASM export (`engine/bridge/src/lib.rs`) wraps
`session::Session`. The app never calls it, but it compiles about 25k lines
into the shipped WASM: path, program, page_program, text_position,
font_metrics, font_resources, embedded_glyph, cmap, page_geometry,
page_content, image_resources, image_sampling, generated_appearance, plus
original-but-dead raster, stroke, truetype and session modules. Deleting the
export, `bridge/src/main.rs`, `session*`, the `cli_*` users and these modules
removes them from the licence question.

### MuPDF-derived and shipped (class A): clean-room rewrite
| # | Workstream | Files | Shipped lines | Rebuilt from |
|---|---|---|---|---|
| 1 | Syntax | lexer.rs; parser.rs lenient path; object.rs dict order and text strings; serialize.rs compact style | ~1,200 | ISO 32000-2 §7.2–7.3, §7.9.2, Annex D |
| 2 | Filters | stream.rs: ASCIIHex, ASCII85, RunLength, LZW, predictors | ~600 | §7.4; TIFF 6.0 and PNG predictor specs |
| 3 | Repair | repair.rs | ~2,000–2,800 | §7.5 (the spec defines no repair, so this is a new design) |
| 4 | Text extraction | view/stext.rs; view/mod.rs PageFrame; render.rs rounding | ~2,600 | §9.4, §9.10, §14.7, §14.9.4, §7.7.3.3, §14.11.2, UAX #9 |
| 5 | Glyph to Unicode | view/text_unicode.rs; simple_encoding.rs | ~1,250 | §9.6.5–9.6.6, §9.7.5, §9.10.2, Annex D, AGL spec, OpenType cmap/post |
| 6 | Annotation appearances | app/annots/appearance.rs; annots/view.rs synthesis | ~1,200 | §12.5.2, 12.5.4–12.5.6, §8.4–8.5; new icon and stamp artwork |
| 7 | Widget appearances | app/forms/appearance.rs, text.rs, scripts.rs | ~2,000 | §12.7, §12.5.6.19, §9.6–9.7, Adobe CID collections |
| 8 | JPEG resolution | app/ops/pages/images.rs exif/photoshop | ~120 | EXIF 2.3, TIFF 6.0, Photoshop image resources |

Total is about 11,000–12,000 lines.

### Data derived from MuPDF files (class D): regenerate from the upstream source
- simple_encoding_data.rs: regenerate from Annex D and the Adobe Glyph List.
- view/text_unicode_data.rs: AGL, Annex D, Adobe symbol/zapfdingbats tables,
  the Apple `post` glyph order, UnicodeData.txt.
- annots/fonts.rs widths: regenerate from the URW AFMs (OFL).
- Icon paths, stamp labels, appearance constants and CJK descriptors inside 6
  and 7 are dropped, not copied.

### Borderline (class C, but reword and re-derive one rule)
- security.rs `revision()`: the guess for a missing /R. Re-derive from the spec.
- Comments naming `pdf_*`/`fz_*` internals across C files: reword as observed
  behaviour.
- Vendored hayro PATCH.md: reword to "observed behaviour".

## Process

### Phase 0: shrink and fence (mechanical, no clean room needed)
1. Remove the `RustEngine` export and the class B modules. Gates must stay green.
2. Move the native oracles (`engine/tools/mupdf-*-oracle.c`, `build-*-reference.sh`)
   and the AGPL Python models into `engine/oracle/` with their own COPYING and a
   README saying they are AGPL and outside the MIT grant.
3. Move npm `mupdf` to devDependencies. Remove the `--engine=mupdf` build and
   the TypeScript engine (`src/renderer/engine.ts` and friends) after moving the
   protocol types into their own file.
4. Delete stale `mobile/ios|android/.../public/engine/vendor/mupdf*` copies.

#### Phase 0 result (2026-10-18)
- mit-p0-dead removed the `RustEngine` export, 29 dead modules, 12 `cli_*`
  modules, 63 test files and 54 design docs (about 46k lines). The WASM went
  from 13.6 MB to 12.7 MB.
- mit-p0-fence moved the MuPDF oracles, MuPDF-using scripts, `bench/` and the
  old TypeScript MuPDF engine into `engine/oracle/` (AGPL, test-only, built by
  `npm run build:oracle` into the ignored `build-oracle/`). The app builds Rust
  only, `mupdf` is a devDependency, and 132 tools that served only deleted
  modules were deleted.
- `font_metrics.rs` survives because `builtin_metrics` uses its `FontAdvance`
  type. It is MuPDF-informed, so trim it to that type in workstream 5.
- Ten app tests that inject MuPDF faults are skipped until the Rust engine has
  a fault hook (p1-worker 2, p3-journal 5, p2-pages 1, p2-edit 1, p6-paragraph 1).
- Still to clean up later: engine/*.md docs and old reference JSON cite
  pre-move tool paths.

### Phase 1: specs (the "dirty" side)
One spec writer per workstream. The writer may read the current code and run
MuPDF as an oracle. The writer produces:
- a plain-language functional spec: inputs, outputs, limits, error behaviour,
  ISO section references, and behaviour observed from MuPDF/pdf.js/Acrobat
  output. No code, no MuPDF function or constant names, no copied constants.
  Numbers come from the spec or from fresh black-box measurement.
- a black-box acceptance suite: input PDFs plus expected behaviour. Where the
  current tests assert byte-equality with MuPDF streams (for example
  `synthesis_matches_mupdfs_recorded_streams`, `stamp_labels_are_mupdfs`, the
  app-forms/app-annots equality gates), replace them with spec checks and
  pixel-tolerance or text-equivalence checks.
A reviewer checks each spec for leaked implementation detail before hand-off.

### Phase 1.5: hand-off prep (dirty side, 2026-10-18)
- Scrub the MuPDF-citing comments in surviving code. Re-derive the
  `startxref` search distance (decision R7). Trim `font_metrics.rs`.
- Acceptance tests are written reviewer-side from the specs. Tests for new
  behaviour that the old code fails are marked
  `#[ignore = "clean-room: ..."]`; the implementer un-ignores them.
- `engine/tools/cleanroom/make-handoff.sh` builds a hand-off tree. It has no
  git history and keeps only the engine plus the specs. Every class A file
  or part not yet rewritten is replaced by an interface stub (public
  signatures from the spec, `todo!()` bodies), so the tree compiles while
  showing no derived code.

### Phase 2 waves (dependency order)
| Wave | Workstreams | Why this order |
|---|---|---|
| 1 | 01 syntax, 02 filters, 08 JPEG | Base layers; 08 is independent |
| 2 | 03 repair, 05 glyph→Unicode, 07 widgets | Need wave 1's lexer and parser |
| 3 | 04 text extraction, 06 annotations | 04 needs 05; 06 needs 07's layout API |

Workstream 05 changed the interface that structured text uses, as spec 05
defines it. The old structured-text code no longer builds against it, so
05 waits on branch `mit-ws05`. Workstream 04's tree is made from that
branch, and the two merge together.

Each workstream gets its own hand-off tree, made from the repo after the
previous waves have merged. The implementer's files are copied back onto
a repo branch (`mit-ws<NN>`), where the full gates run. An independent
reviewer, who may see everything, checks the result against the spec and
for resemblance to the old code, and then it is merged.

Some acceptance tests need code from another workstream (for example,
widget tests read text back through extraction), and that code is a stub
in the implementer's tree. The reviewer runs the full suite on the repo
branch, where that code exists, and relays only the test names and failure
messages to the implementer. Implementer
transcripts are kept as clean-room evidence.

### Phase 2: clean implementation
- The implementer works in a fresh directory exported without git history and
  with the class A files deleted. It contains the spec, the acceptance suite,
  and the rest of the engine (class C code is allowed).
- The implementer may not read MuPDF source, the old files, old commits, or the
  NOTICE/PROVENANCE sections about them. Each brief says so, and each session
  records what it read.
- Output goes back to the main repo as a single commit per workstream.

### Phase 3: acceptance and relicense
- Independent review, existing gates, the full corpus run, and the acceptance
  ledger for every op touched.
- When all eight workstreams land: rewrite NOTICE (permissive third-party
  notices only), replace COPYING with MIT, change Cargo.toml and bridge licence
  fields, SPDX headers in engine/src, tests and examples, the About panel and
  `check-macos-package.mjs`, package.mjs / mobile build NOTICE files and
  `Engine-AGPL-3.0.txt`, the mobile footer link, READMEs and release.yml
  comments, `reference/licenses/*` notices, fixture provenance.
- Add the four missing licence files (glifo, autocfg, version_check, serde).
- Lawyer review before the first MIT release.

## Expected behaviour changes
Rewritten areas will not be byte-identical to MuPDF:
- New annotation icons and stamp artwork.
- Slightly different text-extraction spacing and paragraph breaks.
- Different repair results on some broken files.
Each difference is judged on what is best for users, not on MuPDF parity.

## Resources
At most 4 subagents, at most 2 building or testing at once (8 GB machine,
`tpdf-wt/tools/slot`). Nothing is pushed without the owner's go-ahead.
