# R20 pinned Base14 outline reference

> Licence note (October 18, 2026): the Rust engine and the app are now MIT, and no MuPDF code ships. Where this document says engine code is AGPL or MuPDF-derived, or that a build owes AGPL obligations, it records the state before the clean-room relicense (see the Licence section of [`PROVENANCE.md`](reference/PROVENANCE.md)).

This gate covers the engine's explicit pinned Helvetica and Times-Roman policy. It does not implement a production CFF parser or claim that these substitutes reproduce arbitrary installed fonts. All native code remains in test tooling. The production artifact is a fixed Rust table of canonical integer contours.

## Provenance and reproducible generation

`tools/build-builtin-outlines-reference.sh` verifies the pinned MuPDF 1.28.1 font loader, font table, FreeType CFF loader, outline implementation, and both input CFF assets before linking the test oracle. `tools/generate-builtin-outlines.py` queries all 855 names in each pinned AFM. Exactly 654 names per face resolve to matching native glyph names; their advances equal the AFM entries. The generated tables cover native GIDs 0–653 exactly once per face, including `.notdef` only as an integrity record. PDF selection of `.notdef` remains unsupported.

| Face | Pinned CFF SHA256 | Canonical commands |
| --- | --- | ---: |
| Helvetica / NimbusSans-Regular | `366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706` | 12,788 |
| Times-Roman / NimbusRoman-Regular | `eb609a306b81357853ba91c2a3c47f3bba04f3a8cfdeedc733b1aa4af5d03d4c` | 17,838 |

The source licenses and notices are retained in `reference/licenses/urw-outlines/`. Derived outline data has an OFL-1.1 header, while the generator and surrounding implementation remain AGPL-3.0-or-later. `reference/builtin-outlines-manifest.json` contains per-name GID, advance, command range and digest, input hashes, CFF matrix evidence, and output digest. Two independent generation runs produced the same 2,190,965-byte Rust file, SHA256 `d811729c399645d18c013015871b98f9ed22a3f7e2902e84d2c55fa9602ef4dd`; rustfmt leaves it unchanged. Maximum glyph-name length is 20 bytes. Both faces have three empty records: `.notdef`, `space`, and `uni00A0`.

The compiled target reports 908,842 logical bytes for both outline tables, including record layout, name bytes, and command arrays. This is a target-specific accounting value, not a portable ABI-size guarantee.

## Units and actual drawing origin

A bounded test-only parser reads the original CFF INDEX and DICT structures independently of the native outline decomposer. Both assets are non-CID CFF1 with Type2 charstrings and an omitted Top DICT FontMatrix. The CFF default is a 1/1000 identity matrix with zero translation. Pinned FreeType `cffload.c` initializes identity/zero-offset matrix state; `cffobjs.c` assigns 1000 units for a pure CFF without an explicit matrix. Native faces independently report 1000 units per em.

The canonical outline is obtained through `FT_Load_Glyph` with `NO_SCALE | NO_HINTING | NO_AUTOHINT | NO_BITMAP | IGNORE_TRANSFORM`. Decomposition emits integer moves, lines, cubics and explicit contour closes at the actual CFF drawing origin. No TrueType `xMin - lsb` correction is applied to these CFF records. All FreeType calls are made under MuPDF's FreeType lock, which also installs the native allocator context. CFF and embedded TrueType page provenance remain separate.

A second oracle loads the original PDF page and records the native font object, original source code, Unicode, selected GID/name, actual pinned font-buffer digest, text matrix, paths, colors, clips, and order. It independently decomposes each shown glyph and applies native page placement. This establishes normalization and drawing-origin parity on real pages rather than assuming them from the asset extraction alone. Glyph advance comes from the PDF text machinery; the generated PDFs deliberately declare advances different from the canonical AFM/native glyph advance.

## Comparison policies fixed before results

- Canonical asset membership, names, GIDs, units, advances, asset identity, command topology and integer coordinates are exact. Native envelope byte size and full query membership are also checked, including rejected provenance mutations.
- Page identities, source bytes, Unicode, font-object provenance, glyph names/GIDs, source kind, command ranges, ordering, clip topology and rendering mode are exact.
- Native page glyph coordinates retain the previously approved T0 threshold `0.002 + 2e-5 * max(1, abs(expected))`. Native path coordinates retain P0's `max(1e-5, 2e-6 * abs(expected))`. Neither threshold was adjusted using R20 results.
- Generated mixed-state matrices and text positions are independently rational. Canonical CFF normalization by 1000 is usually not exactly representable in binary64, so each output coordinate is checked against an independently constructed rational interval with one endpoint ULP for each normalized division, multiply and sum. The bound is derived per coordinate; it is not an empirical global epsilon. A bounded 65,536-entry cache stores immutable intervals without changing arithmetic or checks.
- Native `fz_outline_glyph` hinted/normalized output is retained as a separate diagnostic. Topology and coordinate differences are reported without claiming hinted or pixel parity.
- Raster checks preserve R19's independently certified Bernstein curve model and its predeclared sample uncertainty bands. Native rasterization consumes independently extracted unhinted geometry at 4× AA0 and separately AA8. The native diagnostic boundary band remains one device pixel with a 1/4096-pixel curve enclosure. Controlled exact orthogonal parity remains the separate, unchanged R19 gate.

## Bounded, fail-closed wrappers

The asset oracle accepts at most 4096 printable ASCII names of at most 127 bytes, with exact-name round-trip checks and no Unicode/name fallback. It bounds heap to 128 MiB, output to 64 MiB, points to 32,768 and contours to 4096 per glyph, commands to 4096 per glyph and one million per process, and CPU/wall time to 30/45 seconds. Missing names return an explicit `found: false` row.

The page oracle bounds PDF input to 16 MiB, native heap to 128 MiB, decoded content to 4 MiB per page/128 MiB per document, output to 64 MiB, and page/document work counters. Repairs and native warnings reject the document. System-font loaders cannot read files. Unsupported features use explicit refusal envelopes; expected atomic document errors use exit 2 with the oracle diagnostic prefix and zero output. Unexpected exits, signals, partial output and malformed envelopes fail the comparison even when Rust independently refuses the page. Corpus pages run in separate bounded sessions; cumulative session behavior is covered by the Rust integration tests.

## Evidence

Final reports are `reference/builtin-outlines-differential-b0.json`, `reference/page-outlines-differential-b0.json`, `reference/page-outlines-reference-validation-b0.json`, and `reference/page-outlines-raster-b0.json`. Raw compressed outputs remain in `/private/tmp/tandem-r20-builtin-evidence`, `/private/tmp/tandem-r20-page-evidence`, and `/private/tmp/tandem-r20-raster-evidence`; their digests appear in the reports. The final Rust CLI SHA256 is `358f47bc3b6c56d036a8c925ed1d9814f4ea9632a5e29a87f17a73d886ea51c1`.

All 572 readable pages were attempted. The outline gate matches 543 pages, including all 500 stress pages, and records 29 explicit refusals in both Rust and native code. The matching pages contain 223,657 glyph placements (223,634 pinned and 23 embedded) and 3,006,160 commands. Native maximum coordinate difference is 0.000193 under the unchanged tolerance. The 10,603,730 corpus rational coordinate checks have maximum absolute error 5.64e-14 and maximum derived enclosure radius 1.25e-13. The 220 independently generated mixed-state pages all match, covering 2,200 glyphs and 40,480 commands; maximum native difference is 9e-6 and rational difference 1.55e-14. The generated coordinate-check count includes both the canonical reapplication check and independent model-state check.

Raster coverage is 43 accepted pages and 529 atomic refusals. All 500 stress pages still refuse rasterization because stroke support remains outside R20. The accepted pages contain 20,746,368 pixels. Native unhinted 4× AA0 differs on 22,852 pixels and AA8 on 33,948; no differences lie outside the fixed diagnostic boundary band. These are explicitly bounded diagnostics, not exact native pixel parity. All 43 accepted canvas sizes are separately verified from native page bounds, preventing a wrong Rust canvas from shrinking the reference target.

The 16 generated CFF raster cases contain 1,024 pixels. Eleven match the independent sample model byte for byte; five differ on seven pixels within the 118 conservatively uncertain boundary pixels. There are zero model differences outside the uncertainty band, and zero native differences outside the separate diagnostic band. Both pinned faces, negative-bearing shapes, empty space, straight and cubic contours are exercised.

Final validation passes six positive checks, 47 rejected comparator mutations, and 17 native wrapper checks. Two fresh generator invocations exactly reproduce both checked-in data and manifest. The actual driver is tested with a Rust refusal paired with a native signal and rejects the run. Asset metadata, query membership, source/name/GID/Unicode, font identity, command order/range/coordinates, clip, synthetic provenance, and canvas-size mutations are rejected. Both previously accepted embedded-outline pages and both earlier raster pages remain accepted.

Reproduction from the repository root, after building the pinned references:

```sh
sh engine/tools/build-builtin-outlines-reference.sh /private/tmp/tandem-mupdf-reference
sh engine/tools/build-page-outlines-reference.sh /private/tmp/tandem-mupdf-reference
python3 engine/tools/differential-builtin-outlines.py --rust /private/tmp/pdf-inspect-r20-oracle-final --report engine/reference/builtin-outlines-differential-b0.json --evidence-dir /private/tmp/tandem-r20-builtin-evidence
python3 engine/tools/differential-page-outlines.py --rust /private/tmp/pdf-inspect-r20-oracle-final --report engine/reference/page-outlines-differential-b0.json --evidence-dir /private/tmp/tandem-r20-page-evidence
python3 engine/tools/differential-builtin-raster.py --rust /private/tmp/pdf-inspect-r20-oracle-final --report engine/reference/page-outlines-raster-b0.json --evidence-dir /private/tmp/tandem-r20-raster-evidence
python3 engine/tools/test-builtin-outline-reference.py --rust /private/tmp/pdf-inspect-r20-oracle-final --report engine/reference/page-outlines-reference-validation-b0.json --raster-report engine/reference/page-outlines-raster-b0.json
```

The raster command also uses the unchanged R19 `mupdf-page-raster-oracle`, built by `tools/build-page-raster-reference.sh`. Data regeneration takes positional arguments: pinned source directory, native asset oracle, output Rust file, and output manifest. Use temporary output files for reproduction; the validation command does this twice automatically.
