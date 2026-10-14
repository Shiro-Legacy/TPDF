# R18 page glyph placement reference

The final gate is recorded in `reference/page-glyph-differential-g1.json` and
`reference/page-glyph-reference-validation-g1.json`. This is an unhinted geometry
and source identity gate, not a raster, hinted-rendering, or pixel parity claim.
The production engine remains Rust-only; the native executables are test tools.

## Drawing origin and reference independence

Pinned MuPDF 1.28.1 `source/fitz/font.c`, `fz_outline_ft_glyph`, loads a glyph at
1024 ppem with `FT_LOAD_IGNORE_TRANSFORM`; hinting is enabled unless loading
fails. It decomposes the loaded outline, then applies a scale of 1/65536 and the
supplied glyph placement matrix. Synthetic italic/bold and substitute-font width
adjustments are outside this reference subset.

The primary comparison therefore performs an explicit separate
`FT_LOAD_NO_SCALE | FT_LOAD_NO_HINTING | FT_LOAD_NO_AUTOHINT | FT_LOAD_NO_BITMAP |
FT_LOAD_IGNORE_TRANSFORM` load. FreeType has already translated its unscaled
outline by the negative first phantom point, `xMin - lsb`. The helper records
these actual drawing coordinates and applies the native text placement matrix.
It also records raw glyf coordinates by restoring that offset, solely as an O0
cross-check. Restored raw coordinates are never used unadjusted as the native
page drawing reference.

The Rust bridge equivalently evaluates
`((x2/2 + lsb - xMin)/unitsPerEm, (y2/2)/unitsPerEm)` and applies the existing
source-code transform once. Declared PDF widths control the advance between
codes. Neither hmtx advance width nor ToUnicode controls glyph shape or choice.
All five original embedded fonts have zero origin shift, so the generated
nonzero-shift cases are essential. A valid empty nonzero glyph produces no
commands; its metadata uses the engine's zero-bounds convention, so the origin
shift is lsb. It remains distinct from missing GID0, which refuses the page.

The native run processor supplies source byte/CID, GID, Unicode, text placement,
advance, rendering mode, and interleaved path/clip/color callbacks. The helper
binds the native font pointer back to the loaded PDF font and FontFile2 object
IDs. Ambiguous bindings refuse rather than guessing a resource alias. Selected
cmap record, hhea/hmtx byte ranges, side bearing and units are also compared.
Native batch boundaries remain in raw evidence; semantic comparison emits one
record per primary source code and preserves all Unicode continuations.

## Final acceptance evidence

Every readable fixture page is attempted without candidate filtering:

- 572 pages: 2 accepted and matched; 570 explicitly refused by both implementations.
- `p3/simple-subset.pdf` supplies 5 source-code placements and 87 commands.
- `p3/field-defs.pdf` supplies the second accepted page, with no shown glyphs.
- The separate 500-page stress document is entirely refused for unsupported shown
  fonts; it is not counted as 500 successful glyph pages.
- Native refusal categories: 541 shown fonts outside indirect embedded TrueType,
  19 unsupported operators, 5 unsupported font subtypes, 4 lexical-subset refusals,
  and 1 unclosed text/graphics state. No native document error or skipped page.

This narrow glyph-stage result does not reduce the earlier PP1 parser/trace gate,
which still verifies 543 of the same 572 pages. An accepted page trace is not a
claim that the engine can draw every font on that page.

The generated real-PDF gate has 237 accepted/matched pages, 2,362 source-code
placements, and 32,434 page commands:

- 220 synthetic-font PDFs use the independent O0 contour encoder and cyclic
  contour model, 1024 units per em, varied positive/negative bearings, and exact
  rational text/page transforms. All 2,200 placements have nonzero origin shifts.
  Page crop, all four right-angle rotations, UserUnit, cm/Tm, TJ, rise, horizontal
  scaling, spacing, nested clips, paths, colors, and Tr0/Tr3 are exercised.
- One explicit case uses raw xMin=100, lsb=-40, hmtx advance=900, PDF width=250,
  unitsPerEm=1000, and two source A codes mapped by ToUnicode to Omega. Its first
  drawing x is -0.4, its second 2.1. This establishes the separate roles of source
  glyph choice, drawing origin, outline units, Unicode, and declared advance.
- 16 PDFs embed the immutable actual simple-subset font, with deliberately
  unrelated ToUnicode and declared widths, page transforms, and mixed state.
  Their source-code/GID expectations and rational mixed-state trace are
  independent; actual raw font outlines are compared against the native loader,
  not represented as independently encoded synthetic outlines.

Primary native page-command tolerance is the preexisting T0 rule
`abs(actual-expected) <= .002 + 2e-5*max(1,abs(expected))`. The maximum observed
absolute difference is 0.0000090000000057 in both the fixture and generated
populations. Existing path/color/page geometry retains the P0 rule
`max(1e-5, 2e-6*abs(expected))`. No tolerance was widened to pass this gate.
The 220 dyadic rational cases require exact float equality and observed zero
difference. Only the explicitly labeled 1000-unit bearing case permits one ULP
against the rational coordinate. Identity, order, command kinds/counts, glyph
ranges, provenance, clip ancestry and Unicode are exact.

Direct `fz_outline_glyph` output is retained separately. It has 3 topology
mismatches among the 5 fixture glyphs and 1,524 among generated glyphs. This
includes native path compression/implicit contour closure as well as the
separate hinted/scaled load. For glyphs with matching command topology only,
32 fixture coordinate fields differ (maximum 0.000305) and 1,782 generated fields
differ (maximum 0.0001735). Those maxima are not bounds on glyphs with differing
topology. None of these observations is silently normalized into primary parity.

The validation gate rejects 29 deliberately altered outputs, including source
code, Unicode, font/program/GID, cmap selection, units, origin/metrics, command
ranges, ordering, clips, and coordinates. Three positive baselines include an
accepted empty nonzero glyph and a deliberately different secondary hinted path.
Ten native wrapper checks cover normal/empty/bearing cases, input/page bounds,
malformed input, and atomic per-page refusal during glyph callbacks.

## Bounds, cleanup, licensing and reproduction

The helper retains PP1 limits: 16 MiB input, 128 MiB native heap, 4 MiB decoded
content per page/128 MiB per document, 64 MiB atomic output, bounded tokens/events/
commands/fonts/clip depth, CPU 30 seconds and wall 45 seconds. Font tables and
outline points have additional checked bounds. System-font callbacks cannot read
files. Original repair/warning diagnostics remain failures. On a callback refusal,
the device is disabled/closed before processor cleanup, preventing a spurious
"dropping unclosed PDF processor" warning from replacing the genuine per-page
refusal. No partial page is published.

`tools/build-page-glyph-reference.sh` checks pinned source hashes, including the
existing PP1 interpreter/page/path set and FreeType loading/decomposition files.
Key source SHA-256 values:

- `source/fitz/font.c`: `7fe9aa150fd9ac515e769f2213570b9e696a5650bdb7cd2a62a2bef35a48b6e6`
- `thirdparty/freetype/src/truetype/ttgload.c`: `627e20092871f554c7eaf719e56dac72497d2d9a1a91b5919ac78bcf74d84deb`
- `thirdparty/freetype/src/base/ftoutln.c`: `e9a31e1a88398615df585a009632c5eebbc456da0e09b6f6ad6ac2965c1bc3b3`

MuPDF source/reference wrappers remain AGPL-3.0-or-later; the pinned FreeType
reference dependency retains its FreeType license. Existing reference archive
and license provenance are unchanged. No native implementation is copied into
production Rust. Full source/executable/model/input/output hashes are in reports;
the production binary is SHA-256
`fcd48042be619fa1200e48db98ea5844ff0109b98f045b2e33c57f871a78174f`.

From the engine repository root:

```sh
sh engine/tools/build-page-glyph-reference.sh /private/tmp/tandem-mupdf-reference
python3 engine/tools/test-page-glyph-reference.py --rust /private/tmp/pdf-inspect-r18-oracle-final --report engine/reference/page-glyph-reference-validation-g1.json
python3 engine/tools/differential-page-glyph.py --rust /private/tmp/pdf-inspect-r18-oracle-final --report engine/reference/page-glyph-differential-g1.json --evidence-dir /private/tmp/tandem-r18-page-glyph-evidence
```

The actual-font input is `/private/tmp/outline-fonts/font1.ttf`, hash-checked
against the immutable embedded-font inventory before use; `--actual-font` can
supply an equivalent extraction. The evidence directory holds 572 compressed
raw JSON outputs (98 corpus document outputs and 474 generated outputs).
