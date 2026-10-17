Current functional preview (updated October 14, 2026, M1 in progress): see [WORKFLOW-STATUS.md](WORKFLOW-STATUS.md) for the implemented text-edit, undo/redo, save and reopen workflow, its limits, tests and launcher. The historical milestone descriptions below remain reference material.

# Tandem PDF Rust engine experiment

A safe Rust redesign of the MuPDF functionality used by Tandem PDF. The app engine (`src/app`, `src/view`) and the WASM bridge's `RustApp` export are what the application ships; WORKFLOW-STATUS.md and ../docs describe them. The sections below are the historical milestone record for the lower layers that remain.

The user authorized this experiment on October 2, 2026. Engine source is AGPL-3.0-or-later; see COPYING, NOTICE and reference/PROVENANCE.md. The parent application's existing license is unchanged. Native MuPDF is used only by test oracles.

The legacy standalone page pipeline (the `RustEngine` WASM export and its `session` module, page programs, paths, strokes and rasterization, ToUnicode maps and text decoding, font resources, TrueType outlines and embedded glyphs, page geometry, image resources and sampling, optional content and generated appearances) was removed on October 17, 2026; the app never used it. Its milestones R6b, R7, R8b, R9, R11, R13 to R19 and R21 to R24 survive only in git history, with their design and review documents.

## Implemented

- Byte-preserving PDF lexer with exact source spans, strict malformed-input errors and token/count limits.
- Owned direct objects, dictionaries, arrays and checked indirect references; bounded depth, object count and decoded payload. Duplicate keys reject by default; KeepLast is explicit.
- Read-only classic and stream xrefs, incremental revisions, and compressed object streams. Exact wrappers, offsets, generation numbers, newest-entry/free precedence, stream lengths and borrowed stream ranges. Global syntax/object/payload/metadata budgets, container and member limits. Bootstrap and object-stream metadata have a deliberately narrow direct identity/Flate subset.
- Bounded Flate, ASCIIHex, ASCII85, RunLength and LZW decoding and filter chains. LZW supports EarlyChange 0/1; Flate/LZW support TIFF2 and PNG10–15 predictors with 1/2/4/8/16-bit components. Termination and cumulative expansion are checked. Strict malformed input is deliberately less tolerant than MuPDF recovery.
- Reference-free content operands and incremental operation parsing, with source spans, sticky errors and cumulative limits. Inline images stop explicitly at BI before binary payloads are read.
- Explicit WinAnsi, MacRoman, Standard and MacExpert byte encodings with an exact 378-name Unicode registry, transactional Differences, and separate undefined/unknown outcomes. This is declared encoding data, not font-program glyph selection.
- Deterministic bounded object serialization, preserving binary values and finite f64 reals in decimal syntax.
- Experimental full-document rewrites with retained effective live objects, original header version, inherited Info/ID metadata, exact raw stream bytes, and fresh sparse classic xrefs. Stream Length becomes a direct integer; dormant original xref/object streams remain retained objects. Input bytes remain unchanged; output is returned as a new buffer.

Unsupported features fail explicitly. The reader excludes hybrid xrefs, encryption, external-file streams and repair; it requires the PDF header at byte zero. Compressed containers require direct bootstrap metadata and may not themselves be compressed. The writer additionally refuses signature-bearing inputs (including unsigned signature fields), linearization, unknown meaningful trailer entries and dangling references. It does not preserve signatures or incremental revision history. Optional null trailer entries are treated as absent, including inherited Root/Info/ID; this is an explicit reader policy, not a claim that all such incomplete trailers are standards-conforming.

Parsing or rewriting success is not proof of PDF conformance or Rust rendering capability. The reference renderer's identical output is used to test rewrite preservation.

## Build and use

Rust 1.85.1 or newer, with rustfmt and clippy, is required. The core suite has also been run on 1.85.1; normal development uses 1.98.1. Cargo.lock pins dependencies. No Node/Electron is needed to run the Rust engine.

```sh
cd engine
cargo build --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
printf '<< /Type /Page /Parent 2 0 R >>' | cargo run --quiet -- --canonical
printf '00ff>' | cargo run --quiet -- --decode=asciihex
printf 'q 1 0 0 1 10 20 cm Q' | cargo run --quiet -- --content
cargo run --quiet -- --stream=4:0 /path/to/input.pdf
cargo run --quiet -- --rewrite /path/to/input.pdf > /path/to/new-output.pdf
```

Use a **different output path** with shell redirection: shells truncate the redirected path before the program opens its input. The library has no file-overwrite API.

`pdf-inspect --tokens` emits token TSV, `--object` prints an owned object, `--canonical` emits canonical direct-object syntax, `--decode=FILTERS` emits decoded bytes, and `--stream=N:G` decodes a document stream. Raw LZW CLI names are `lzw` (EarlyChange1) and `lzw0`; PDF DecodeParms resolve parameters through the document API. `--content` emits hex-encoded operator names, top-level operand counts and canonical operands as TSV. `--rewrite` emits a complete new PDF after successful serialization. Input defaults to stdin, or pass a file path; CLI input is capped at 16 MiB, library limits are configurable. Documents borrow original stream bytes and own parsed objects, including decoded compressed members.

## Reference tests

The official MuPDF 1.28.1 archive must match SHA256 `dc94c60b2537e2ac9a2d379dd3801545f84a3a302d15c9da358362a1270707c3`. Download location is recorded in reference/PROVENANCE.md. Large sources and binaries are ignored, not vendored.

```sh
sh tools/build-reference.sh /path/to/mupdf-1.28.1-source.tar.gz
sh tools/build-filter-reference.sh reference/build
sh tools/build-document-reference.sh reference/build
python3 tools/differential.py --reference reference/build/mupdf-lex-oracle --cases 2000
python3 tools/differential-filters.py --reference reference/build/mupdf-filter-oracle --cases 1000
python3 tools/compare-fixtures.py --minimum-supported 49
node tools/compare-rewrites.mjs /path/to/mupdf/dist/mupdf.js ../fixtures target/debug/pdf-inspect reference/build/mupdf-document-oracle reference/rewrite-comparison.json
```

Additional predictor/LZW/object-stream/serializer differential tools are under tools/. Decoder comparisons check exact output against independent Python encoders and MuPDF. Fixture hashes are checked before geometry and rewrite comparisons. Golden inventory uses pinned MuPDF 1.28.1 WASM; passwords and private keys are not needed or recorded. Native document comparisons query repair state after page loading.

Current evidence and stage acceptance are in STATUS.md, REVIEW-*.md and reference/*-r*.json. Deterministic mutation tests are not coverage-guided fuzzing or a complete security audit. Logical limits do not guarantee process allocation failures become recoverable errors. Gates on this machine do not establish Windows/Linux runtime validation. Rust 1.85.1 core tests and an all-feature wasm32 library compilation have passed.

## Encodings

`--encoding` reads a PDF encoding name or dictionary with an explicit BaseEncoding
and optional Differences. It emits the 256 slot names and Unicode/undefined/unknown
outcomes. Missing bases require font context rather than an invented default.
See `REVIEW-R8-S0.md`: all 1,024 base slots and 378 known names pass independent
native/source comparisons, plus generated overrides and 31 strict CLI refusals.

## Builtin advances

Two exact Type1 faces, Helvetica and Times-Roman, have 654 verified glyph-name
advances each. Tables use the intersection of the pinned AFM and bundled CFF,
with the font data's OFL notices retained. Width selection is independent of
Unicode decoding. Undefined bytes, `.notdef` slots and missing metrics remain
explicit outcomes. `--builtin=FACE` inspects an explicit encoding. Aliases, subset-name stripping and all other faces are unsupported.
Independent implementation and fixture review passed: all 1,308 table entries,
402 excluded AFM-only names, 49,152 standalone slot outcomes, and all 48 fixture
fonts / 12,288 slots match the recorded expectations. Thirty-seven strict CLI
refusals emit no output. See `FONT-METRICS-DESIGN.md` and `REVIEW-R12-M1.md`.

## Pinned standard-font outlines (R20)

```sh
cargo run --offline --locked --bin pdf-inspect -- --builtin-outlines=Helvetica
```

Pinned Helvetica and Times-Roman outlines are licensed canonical font assets with
safe Rust lookup; the view uses them for unembedded standard fonts. General
CFF/Type1 decoding is a separate gap. See BASE14-OUTLINE-DESIGN.md,
BUILTIN-OUTLINES-NATIVE-NOTES.md and STATUS.md.
