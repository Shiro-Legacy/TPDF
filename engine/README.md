What the engine does today is in [STATUS.md](STATUS.md) (the dated progress log) and [WORKFLOW-STATUS.md](WORKFLOW-STATUS.md) (the text-edit, undo/redo, save and reopen workflow, its limits, tests and launcher). The milestone descriptions below are the historical record of the lower layers; they predate encryption reading (`src/encrypt.rs`) and repair (`src/repair.rs`), which the status log covers.

# Tandem PDF Rust engine

A safe Rust PDF engine for Tandem PDF, which replaced the MuPDF engine the app used before. The app engine (`src/app`, `src/view`) and the WASM bridge's `RustApp` export are what the application ships; WORKFLOW-STATUS.md and ../docs describe them. The sections below are the historical milestone record for the lower layers that remain.

The user authorized this experiment on October 2, 2026. The engine is MIT, like the rest of the repository (`../LICENSE`); NOTICE lists its third-party notices (URW font data, Adobe and Unicode data, hayro and the other Rust crates). Parts of earlier versions were written with knowledge of MuPDF's source and were AGPL-3.0-or-later; in October 2026 they were deleted or rewritten in a clean room from the PDF specification (`../docs/mit-clean-room-plan.md`, `../docs/clean-room/`, reference/PROVENANCE.md). MuPDF is now only a black-box test oracle in `oracle/` (AGPL, not shipped).

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

At the time of this record, unsupported features failed explicitly. The reader excluded hybrid xrefs, encryption, external-file streams and repair (encryption reading and repair were added later: `src/encrypt.rs`, `src/repair.rs`, STATUS.md); it requires the PDF header at byte zero. Compressed containers require direct bootstrap metadata and may not themselves be compressed. The writer additionally refuses signature-bearing inputs (including unsigned signature fields), linearization, unknown meaningful trailer entries and dangling references. It does not preserve signatures or incremental revision history. Optional null trailer entries are treated as absent, including inherited Root/Info/ID; this is an explicit reader policy, not a claim that all such incomplete trailers are standards-conforming.

Parsing or rewriting success is not proof of PDF conformance or Rust rendering capability. The reference renderer's identical output is used to test rewrite preservation.

## Build and use

Rust 1.92 or newer (the workspace's `rust-version`), with rustfmt and clippy; CI builds with 1.98.1. The engine is a member of the repository's one Cargo workspace (`crates/Cargo.toml`): `crates/Cargo.lock` pins its dependencies and its builds land in `crates/target`. No Node is needed to run the Rust engine.

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

The MuPDF differential harness that compared these layers with MuPDF 1.28.1 was removed on October 21, 2026; its scripts are in the git history before that date, `oracle/README.md` says what remains under `oracle/` (the test-only MuPDF reader of the application suite and the generators of the pinned font tables), and `reference/PROVENANCE.md` describes the evidence the harness produced. The generators still build their oracle against the official MuPDF 1.28.1 archive, which must match SHA256 `dc94c60b2537e2ac9a2d379dd3801545f84a3a302d15c9da358362a1270707c3`; the download location is recorded in `reference/PROVENANCE.md`, and large sources and binaries are ignored, not vendored.

The engine's own suite is `cargo test` here. The application suite (`tests/`, `npm test`) drives the built app and inspects its output through the MuPDF reader in `oracle/app-engine/`. Deterministic mutation tests are not coverage-guided fuzzing or a complete security audit, and logical limits do not guarantee that process allocation failures become recoverable errors.

## Encodings

`--encoding` reads a PDF encoding name or dictionary with an explicit BaseEncoding
and optional Differences. It emits the 256 slot names and Unicode/undefined/unknown
outcomes. Missing bases require font context rather than an invented default.
See `docs/archive/engine/REVIEW-R8-S0.md`: all 1,024 base slots and 378 known names pass independent
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
refusals emit no output. See `FONT-METRICS-DESIGN.md` and `docs/archive/engine/REVIEW-R12-M1.md`.

## Pinned standard-font outlines (R20)

```sh
cargo run --offline --locked --bin pdf-inspect -- --builtin-outlines=Helvetica
```

Pinned Helvetica and Times-Roman outlines are licensed canonical font assets with
safe Rust lookup; the view uses them for unembedded standard fonts. The outline data
is packed in `src/builtin_outlines/*.bin` (format, provenance and the OFL notice in
the README there) and decoded once per face. General
CFF/Type1 decoding is a separate gap. See BASE14-OUTLINE-DESIGN.md,
docs/archive/engine/BUILTIN-OUTLINES-NATIVE-NOTES.md and STATUS.md.
