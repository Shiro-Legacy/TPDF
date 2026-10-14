# Engine boundary and dependency map

This is an isolated engine experiment. Tandem continues using its existing
MuPDF WASM engine; no production engine has been replaced. The Rust engine starts
with owned byte-oriented lexical/object structures. R3 adds the pure Rust
`miniz_oxide =0.8.9` dependency (and locked `adler2 2.0.1`) for bounded Flate
decoding; there is no native library runtime dependency. C and MuPDF remain
development-only reference tooling. The crate is licensed AGPL-3.0-or-later; see `NOTICE` and `COPYING`.

## Existing application responsibilities

| Responsibility | Existing evidence | Replacement implications |
| --- | --- | --- |
| Document opening, password authentication, PDF objects | `src/renderer/engine.ts`, `createEngine`, `openDocument`, `authenticatePassword` | Needs parser, xref/object resolution, streams, encryption and repair policy |
| Rendering and thumbnails | `engine.ts`, `page.toPixmap` | Needs content interpreter, fonts, image codecs, clipping, blending and color management |
| Search and selection | `engine.ts`, `page.search`; structured-text operations | Needs text interpretation, encodings, glyph positioning and extraction |
| Editing, forms, annotations and page grafting | `engine.ts`, PDF object methods and graft maps | Needs resource ownership, document graph operations and change tracking |
| Undo/redo | `engine.ts`, `enableJournal` | Needs explicit operation history and failure atomicity |
| Save/reopen validation | `engine.ts`, `saveToBuffer` | Needs serialization, reachability, stream encoding and document round-trip validation |
| Destructive redaction | `src/renderer/redact.ts`, `applyRedactions`, pixel verification | Needs content-aware removal and independent verification; not satisfied by painting over content |
| Application-specific editing policy | `src/renderer/content-edit.ts` and `engine.ts` | Existing application logic remains useful; isolate it from backend mechanics |

A small JavaScript API surface reaches a large implementation. R1 lexical work
and R2 owned object parsing are foundations, not a replacement for these entire
responsibilities. The Rust engine must expose explicit unsupported cases until
each capability has defined limits and verification.

## Native libraries under the reference

MuPDF's source release bundles third-party code and assets. The existing WASM
inventory in `.quad/shared/p0/p0-dependencies.md` records actual compiled symbols;
that inventory must not be assumed identical to this native static build. The
reference build uses bundled libraries rather than adding Rust dependencies.
Selected major upstream dependencies and licensing entry points are:

| Component | Role | Upstream license entry point |
| --- | --- | --- |
| MuPDF core | PDF interpretation, objects, drawing and document operations | `COPYING`, AGPL-3.0-or-later; commercial alternatives are separate |
| FreeType | Font parsing/rasterization | `thirdparty/freetype/LICENSE.TXT`, FreeType License/GPL alternatives and component notices |
| HarfBuzz | Text shaping | `thirdparty/harfbuzz/COPYING`, Old MIT; component exceptions |
| IJG JPEG | JPEG decoding | `thirdparty/libjpeg/README`, LEGAL ISSUES section |
| OpenJPEG | JPEG 2000 | `thirdparty/openjpeg/LICENSE`, BSD-2-Clause |
| jbig2dec | JBIG2 decoding | `thirdparty/jbig2dec/LICENSE` and `COPYING`, AGPL-3.0-or-later with marked exceptions |
| Little CMS | ICC color conversion | `thirdparty/lcms2/LICENSE`, MIT; threaded plugin has separate notices |
| zlib | Deflate streams | `thirdparty/zlib/LICENSE`, zlib license |
| Gumbo and cmark-gfm | HTML/Markdown functionality in broad MuPDF builds | `thirdparty/gumbo-parser/COPYING`, Apache-2.0; `thirdparty/cmark-gfm/COPYING`, BSD terms |
| Embedded fonts and ICC profiles | Fallback font and color resources | `docs/third-party/mupdf-wasm-assets.md` and upstream per-asset notices |

All paths in the last table refer to the pinned upstream source tree, except
`docs/third-party/...`, which refers to this repository. This table is a design
map, not a complete redistribution notice bundle. The verified source archive
retains the actual notices. Any future shipped native build needs its own exact
compiled-component/asset inventory. The Rust redesign must separately justify
and record dependencies as codecs or rendering are introduced; retaining C
libraries would retain their native-code safety boundary.

## R3 Rust dependency decision

Flate-compressed streams occur in the existing fixture corpus. Reimplementing
DEFLATE from scratch would expand the milestone without improving its PDF object
boundary, so the approved R3 contract uses exact `miniz_oxide =0.8.9` with its
locked `adler2 2.0.1` checksum implementation. `Cargo.lock` records source checksums.
Their crate roots forbid unsafe code. The production crate retains its own
`unsafe_code = "forbid"` rule, which does not itself apply to dependency crates.

The source package license expressions are `MIT OR Zlib OR Apache-2.0` for
miniz_oxide and `0BSD OR MIT OR Apache-2.0` for adler2. All packaged license files
are copied verbatim under `reference/licenses/` and acknowledged in `NOTICE`.
The strict stream layer remains responsible for input/output/cumulative limits,
EOD validation, filter-chain limits and explicit unsupported predictor behavior.

## M1 encryption-reading dependency decision

Opening the application's encrypted fixtures needs MD5, SHA-256/384/512 and
AES-128/256. The completion plan requires reviewed implementations rather
than new cryptography, so the engine pins RustCrypto `aes =0.8.4`,
`md-5 =0.10.6` and `sha2 =0.10.9`. Their locked support crates are listed in
`NOTICE`, with license files under `reference/licenses/`. `aes` and
`cpufeatures` contain internal unsafe code for hardware acceleration; the
engine's own `unsafe_code = "forbid"` rule is unchanged. RC4 and the CBC
chaining around the AES block cipher are the standard algorithms implemented
in `src/security.rs`, checked against RFC 6229 and NIST SP 800-38A vectors.
This covers reading only. Writing encrypted output belongs to M8.

## M9 application dispatch dependency decision

The application engine (`src/app`) takes the worker's requests and replies as
JSON, so dispatch can run and be tested natively as well as behind the WASM
bridge. The engine pins `serde_json =1.0.151`, which the bridge already used,
with its `float_roundtrip` feature so that a number reads back as the exact
double JavaScript sent. Its locked compiled dependencies are `serde_core
1.0.229`, `itoa 1.0.18`, `memchr 2.8.3` and `zmij 1.0.23`. The license
expressions are `MIT OR Apache-2.0` for serde_json, serde_core and itoa,
`Unlicense OR MIT` for memchr and `MIT` for zmij. Their packaged license files
are copied verbatim under `reference/licenses/` and acknowledged in `NOTICE`.
serde_json, itoa, memchr and zmij contain internal unsafe code for fast
parsing and formatting; the engine's own `unsafe_code = "forbid"` rule is
unchanged.
