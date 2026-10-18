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

## App engine view dependency decision (hayro, October 14, 2026)

Page display and structured text for the app engine come from hayro
(`src/view/`, see `APP-ENGINE-DESIGN.md`). The core crate pins `hayro =0.8.0`,
`hayro-interpret =0.8.0` and `hayro-syntax =0.8.0` with their default features:
`embed-fonts` (substitute base-14 font programs compiled in, so standard-font
text renders with real glyphs on WASM, which has no system fonts), `embed-cmaps`
(the predefined CJK CMaps) and `simd`. hayro needs Rust 1.92, so `rust-version`
is 1.92 in the core and bridge crates.

hayro's own crates and most of its codec and geometry crates forbid unsafe code.
Several dependencies contain internal unsafe code: SIMD kernels (`fearless_simd`,
`pic-scale`, `moxcms`, `pxfm`, `zune-jpeg`), compression (`zlib-rs`, `flate2`,
`brotli`), containers and casts (`hashbrown`, `smallvec`, `arrayvec`, `bytemuck`,
`yoke`). `hayro-syntax` has one `transmute` for its cached page tree, and
`hayro-interpret` turns on its `unsafe` feature, which adds `flate2`/`zlib-rs`,
`memchr` and SIMD image decoding. As with RustCrypto, the engine's
own `unsafe_code = "forbid"` rule is unchanged and does not extend to dependencies.
The proc-macro crates run at build time only.

Every crate this added to `cargo tree -e normal` (unsafe = any `unsafe` block,
function or impl in the packaged source):

| Crate | Version | License | Unsafe code | Pulled in by |
| --- | --- | --- | --- | --- |
| alloc-no-stdlib | 2.0.4 | BSD-3-Clause | yes | alloc-stdlib, brotli, brotli-decompressor |
| alloc-stdlib | 0.2.4 | BSD-3-Clause | yes | brotli, brotli-decompressor |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 | yes | kurbo, polycool |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | yes | hayro-interpret, png |
| brotli | 8.0.4 | BSD-3-Clause AND MIT | yes | hayro-cmap |
| brotli-decompressor | 5.0.3 | BSD-3-Clause/MIT | yes | brotli |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT | yes | color, font-types, hayro, image, peniko, read-fonts, skrifa, vello_common, vello_cpu |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT | yes | bytemuck (build-time proc-macro) |
| byteorder-lite | 0.1.0 | Unlicense OR MIT | no (`forbid(unsafe_code)`) | image |
| color | 0.3.3 | Apache-2.0 OR MIT | yes | peniko |
| crc32fast | 1.5.2 | MIT OR Apache-2.0 | yes | flate2, png |
| euclid | 0.22.14 | MIT OR Apache-2.0 | yes | guillotiere, kurbo |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | phf_generator |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 | no (`forbid(unsafe_code)`) | png |
| fearless_simd | 0.7.0 | Apache-2.0 OR MIT | yes | vello_common |
| fearless_simd | 1.1.0 | Apache-2.0 OR MIT | yes | hayro, hayro-jbig2, hayro-jpeg2000 |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | yes | hayro-syntax, png |
| foldhash | 0.2.0 | Zlib | yes | hashbrown |
| font-types | 0.12.6 | MIT OR Apache-2.0 | yes | read-fonts |
| guillotiere | 0.7.0 | MIT/Apache-2.0 | no | vello_common |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | yes | vello_cpu |
| hayro | 0.8.0 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | tandem-pdf-engine |
| hayro-ccitt | 0.4.0 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | hayro-jbig2, hayro-syntax |
| hayro-cmap | 0.1.0 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | hayro-interpret |
| hayro-interpret | 0.8.0 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | hayro |
| hayro-jbig2 | 0.3.1 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | hayro-syntax |
| hayro-jpeg2000 | 0.4.1 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | hayro-syntax |
| hayro-postscript | 0.1.0 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | hayro-cmap |
| hayro-syntax | 0.8.0 | Apache-2.0 OR MIT | yes (one `transmute` for the cached page tree; the `unsafe` feature hayro-interpret enables adds flate2, memchr and SIMD decoding) | hayro-interpret |
| image | 0.25.10 | MIT OR Apache-2.0 | yes | hayro |
| kurbo | 0.13.1 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | hayro, hayro-interpret, peniko |
| linebender_resource_handle | 0.1.1 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | peniko |
| log | 0.4.34 | MIT OR Apache-2.0 | yes | vello_common |
| memchr | 2.8.3 | Unlicense OR MIT | yes | hayro-syntax |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 | no (`forbid(unsafe_code)`) | flate2 |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 | yes | hayro-interpret, image |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | yes | euclid, image, moxcms, pic-scale |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | yes | read-fonts |
| peniko | 0.6.1 | Apache-2.0 OR MIT | yes | vello_common |
| phf | 0.13.1 | MIT | no | hayro-interpret |
| phf_generator | 0.13.1 | MIT | no | phf_macros |
| phf_macros | 0.13.1 | MIT | no | phf (build-time proc-macro) |
| phf_shared | 0.13.1 | MIT | no | phf, phf_generator, phf_macros |
| pic-scale | 0.7.12 | BSD-3-Clause OR Apache-2.0 | yes | hayro |
| png | 0.18.1 | MIT OR Apache-2.0 | no (`forbid(unsafe_code)`) | image, vello_common, vello_cpu |
| polycool | 0.4.0 | MIT OR Apache-2.0 | no | kurbo |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | yes | bytemuck_derive, phf_macros, quote, syn, synstructure, thiserror-impl, yoke-derive, zerofrom-derive |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 | yes | moxcms, pic-scale |
| quote | 1.0.47 | MIT OR Apache-2.0 | no | bytemuck_derive, phf_macros, syn, synstructure, thiserror-impl, yoke-derive, zerofrom-derive |
| read-fonts | 0.43.3 | MIT OR Apache-2.0 | no (`forbid(unsafe_code)`) | skrifa |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT | no | hayro, hayro-interpret, hayro-syntax |
| simd-adler32 | 0.3.10 | MIT | yes | fdeflate, miniz_oxide |
| siphasher | 1.0.4 | MIT OR Apache-2.0 | yes | hayro-interpret, phf_shared |
| skrifa | 0.46.2 | MIT OR Apache-2.0 | no (`forbid(unsafe_code)`) | hayro-interpret |
| smallvec | 1.16.3 | MIT OR Apache-2.0 | yes | hayro-interpret, hayro-syntax, kurbo, peniko, vello_common |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | yes | yoke |
| syn | 2.0.119 | MIT OR Apache-2.0 | yes | phf_macros |
| syn | 3.0.9 | MIT OR Apache-2.0 | yes | bytemuck_derive, synstructure, thiserror-impl, yoke-derive, zerofrom-derive |
| synstructure | 0.14.0 | MIT | no | yoke-derive, zerofrom-derive |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | no | vello_common |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | no | thiserror (build-time proc-macro) |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | yes | proc-macro2, syn |
| vello_common | 0.3.0 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | vello_cpu |
| vello_cpu | 0.3.0 | Apache-2.0 OR MIT | no (`forbid(unsafe_code)`) | hayro |
| yoke | 0.8.3 | Unicode-3.0 | yes | hayro-interpret |
| yoke-derive | 0.8.4 | Unicode-3.0 | yes | yoke (build-time proc-macro) |
| zerofrom | 0.1.8 | Unicode-3.0 | no | yoke |
| zerofrom-derive | 0.1.8 | Unicode-3.0 | no | zerofrom (build-time proc-macro) |
| zlib-rs | 0.6.8 | Zlib | yes | flate2 |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib | no | zune-jpeg |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib | yes | hayro-syntax |

Each package's license files are kept verbatim under
`reference/licenses/<crate>-<version>/`; `alloc-stdlib` ships none (see its
README there). SHA-256 hashes of those files are in
`reference/app-view-20261014/license-files.json`. Source checksums are pinned in
`Cargo.lock`. The rust-version raise also enables Clippy lints for let chains,
`is_multiple_of` and `as_chunks`; they are allowed in the core manifest rather than
rewriting existing code.

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

## Image document dependency decision (png, October 14, 2026)

`pdf-from-images` must decode PNG files (MuPDF stores their samples, not the
PNG stream). The core crate now names `png =0.18.1` directly, the version and
default features hayro's tree (`image`, `vello_cpu`, `vello_common`) already
compiles, so no crate is added to the tree and `Cargo.lock` gains only the
direct edge. png forbids unsafe code; its locked dependencies (`bitflags`,
`crc32fast`, `fdeflate`, `flate2`, `miniz_oxide`) are listed in the table
above. The decoder runs with `png::Limits` of 256 MiB. JPEG files are not
decoded: their headers are read in `src/app/ops/pages/images.rs` and the data
is embedded as DCTDecode, as MuPDF embeds it. The license files are under
`reference/licenses/png-0.18.1/` and acknowledged in `NOTICE`.

## Patched hayro crates (October 15, 2026)

The corpus differential (`reference/corpus-20261015/`) and its review found
panics, which abort the WASM engine, and unbounded allocation inside hayro
0.8.0 and hayro-interpret 0.8.0 while they paint: tint transform outputs that
do not fit their alternate space, tiling step pixmaps cast to 16 bits, a
lattice mesh read without end, images unpacked into their declared size, size
products that wrap a 32-bit `usize` on wasm32, image decoders that allocate
from declared sizes before decoding (DCT through zune-jpeg, CCITT and JBIG2
through hayro-syntax, JPX through hayro-jpeg2000; those crates are unchanged,
their inputs are checked first), and pixmaps, fills and strokes kept until the
page is composed. They depend on values hayro derives while painting (a tint
transform's output count can depend on its input; step pixmaps on the CTM;
the size an image decodes at on the size it is drawn), so no check in the
engine before hayro runs can prevent them without refusing pages MuPDF
draws. The engine therefore uses patched copies:
`vendor/hayro-0.8.0/` and `vendor/hayro-interpret-0.8.0/`, the published
crates (same `.crate` checksums as the lock pinned) with the smallest patches
found, each marked `Tandem patch` in the source and listed, with the full diff
against the published crate, in the crate's `PATCH.md`. `[patch.crates-io]`
in `Cargo.toml` and `bridge/Cargo.toml` points both lockfiles at them (their
entries lose the registry source and checksum). No crate is added to the tree,
the licences (Apache-2.0 OR MIT) and bundled assets are unchanged, and the
patches add no unsafe code (both crates keep `forbid(unsafe_code)`). Reasons,
budgets and measurements: `design/app-corpus-robust.md`. Upgrading hayro means
re-applying or dropping each patch against the new release.

October 16, 2026 (integration acceptance review): `vendor/hayro-syntax-0.8.0/`
joins them, the published crate (same `.crate` checksum) with a patch that
holds Flate, LZW, run-length and ASCII85 output to a decoding limit and makes
predictor rows of no bytes decode to nothing (a division by zero panicked);
hayro and hayro-interpret gain bounds on nesting (PostScript procedures,
colour spaces, stitching functions), sampled function tables, nested and
cyclic functions (built once per render), and the bytes one path may cost
vello. Same rules as above: `forbid(unsafe_code)` kept, no crate added, the
full diff in each `PATCH.md`; `tools/workflow-sources.sh` and
`scripts/build.mjs` now count `vendor/` among the WASM package's sources.
Reasons and measurements: `design/app-accept-integration.md`.

