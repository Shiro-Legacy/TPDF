# Embedded simple TrueType glyph selection (R17 G0)

This capability maps a byte from a PDF simple TrueType font to a validated glyph
ID and then to the existing O0 unhinted simple outline. It is separate from text
extraction, page interpretation and rasterization. It never reads ToUnicode,
loads a system font, substitutes a font program or calls native code.

## Three boundaries

1. `truetype_cmap::TrueTypeCmapSession` borrows the immutable O0 font directory.
   The caller selects an original record index. Format 4, language zero is the
   only accepted subtable. The reader retains record identity and absolute table
   and subtable ranges; it allocates no mapping table. It validates every covered
   code and mapped GID before publication. Missing GID0 is a typed result.
2. `embedded_glyph::GlyphFontLoader` loads a requested indirect font dictionary,
   its descriptor and an indirect FontFile2 stream through an existing Document.
   Owned results retain the requested font ID, resolved program ID, flags,
   encoding and decoded program bytes. Descriptor/encoding reference identities
   and intermediate font aliases are not retained. Opening the program and
   selecting its cmap happen separately from PDF metadata/stream loading.
3. `EmbeddedOutlines` owns the validated borrowed font and its glyph map together.
   Its session accepts source bytes, never an externally supplied glyph ID or map.
   A nonzero glyph with an empty outline succeeds. Missing and unsupported codes
   fail distinctly, and both consume the wrapper's code-attempt allowance.

The prior Document parser's work is a separate prerequisite. No API here claims
to execute page text, apply a text matrix, choose advances, hint or rasterize.

## Strict cmap acceptance

Directory version, record offsets, selected length, even segment count, reserved
padding and ordered disjoint ranges are checked. The final segment must be the
canonical FFFF-to-zero sentinel. Search accelerator fields are ignored. A glyph
array offset must be even and point from its own offset word into the actual
array, with the whole segment in bounds. Array value zero bypasses idDelta;
other values and direct deltas wrap at 16 bits. Every nonzero result must be
below maxp.numGlyphs. Unselected formats are retained as records, not interpreted.
Duplicates and aliases are preserved by the reader; the PDF policy rejects an
ambiguous selector instead of silently choosing a duplicate.

A failed selection clears the previous selection. Record, selection, segment,
covered-code validation, query and work limits are cumulative per session.
Failed work remains charged; over-limit admission does not corrupt counters.
The whole16-bit character-code domain is queried in the differential gate, including missing entries.

## PDF policy and explicit refusals

The policy is a strict subset of pinned MuPDF 1.28.1 `pdf-font.c` and `font.c`:

- Flags32 overrides Flags4. Symbolic fonts require absent/null Encoding and an
  unambiguous platform3/encoding0 record. Non-symbolic fonts require an explicit
  supported S0 encoding and select platform3/encoding1 before platform1/encoding0.
- Each slot starts with the raw-byte lookup, then retries F000+code when missing.
  A supported encoding glyph name may override this seed when its lookup succeeds.
  Unicode selection uses S0's exact known name registry. MacRoman selection uses
  the native correction table followed by the first matching MacRoman name;
  therefore source byte202 /space can resolve through lookup code32.
- Native's special22EF-to-2026 retry is preserved. Format4 cannot map values
  outside the16-bit character-code domain. Unknown Unicode glyph names are explicit per-code refusals.
- When a known-name override fails and a post table may expose glyph names,
  the code refuses the unimplemented native name/alias/fuzzy fallback. It does
  not guess that an existing raw seed is correct. No post-name implementation
  is claimed. No post table, or format3, permits the known absent-name branch.
- Undefined encoding entries and .notdef preserve the native raw seed. GID0 stays
  missing; it is not rendered automatically as a replacement glyph.

Wrong present dictionary Types, alternate FontFile/FontFile3 programs, implicit
non-symbolic encodings, symbolic explicit encodings, ambiguous or unsupported
cmap selection, CFF/Type0 fonts and unsupported outline forms refuse explicitly.
Metadata field aliases resolve under bounded cycle/depth checks. Differences
array entries must themselves be direct S0 integers/names; indirect entries
refuse. Composite outlines, shaping, hinting and font repair remain unsupported.

## Resource accounting

Loading separately caps attempts, reference hops, metadata items, processed
Differences name bytes, encoded bytes and decoder work. Stream calls reserve
remaining capped allowances before execution; success refunds to measured usage,
while failure retains the full reservation. Encoding's own item, per-name and
processed-name caps apply before it allocates final slots. A successful font load
owns its bytes independently of the loader and Document.

Every mapping call has a fresh explicit cmap session and fixed256-slot output.
It performs at most256 MacRoman reverse searches over256 fixed entries and16
corrections. Those fixed policy/name comparisons and the fixed output arrays are
separate from the reported cmap work ledger. Names are bounded by S0; the cmap
ledger measures validation and queries, not universal CPU time or allocation.
The outline wrapper caps all code attempts and exposes that count separately
from O0's actual glyph decoding/point/contour/command/allocation/work accounting.
These are logical partitions, not process-RSS or recoverable process-OOM claims.

## Inspection and reproduction

`pdf-inspect --tt-cmap=all FONT.ttf` emits all original records with selection
provenance and65536 dense GIDs per record; `--tt-cmap=N` selects one record index.
It refuses any unsupported selected record. `--font-glyphs=N:G FILE.pdf` emits
all256 source slots with mapped/missing/unsupported status, lookup route and
lookup code, plus O0 geometry for each nonzero mapped slot. Output is buffered
under the existing64MiB cap; a later failure publishes no partial stdout.
CLI input remains capped at16MiB. It is an inspection interface, not a renderer.

See `REVIEW-R17-G0.md`, `GLYPH-SELECTION-NATIVE-NOTES.md`, and the two final reports:
`reference/truetype-cmap-differential-g0.json` and
`reference/glyph-selection-differential-g1.json`. The latter's G1 suffix labels
the PDF-policy comparison inside R17 G0; it is not a separate accepted milestone.
The native tools are test-only and preserve MuPDF and FreeType attribution.
Build with `tools/build-glyph-selection-reference.sh` using the pinned source.
Run `tools/differential-truetype-cmap.py --rust PATH` and
`tools/differential-glyph-selection.py --rust PATH` after extracting the existing
inventory's five fonts as documented in the native notes. Reports hash inputs,
binaries and output evidence. Strict refusal and budget behavior has independent
Rust tests rather than treating permissive native recovery as an oracle.

Specification reference: https://learn.microsoft.com/en-us/typography/opentype/spec/cmap
Pinned implementation hashes and license files are retained in the native notes.
