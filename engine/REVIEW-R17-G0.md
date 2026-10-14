# R17 G0 independent review

2026-10-03. Reviewed standalone TrueType format4 cmap selection and the separate PDF simple-TrueType source-byte policy/outline bridge. No production source was edited by the reviewer.

Disposition: no remaining implementation blocker found in the reviewed source. All 19 independent Rust tests pass: 11 in `tests/review_truetype_cmap.rs` and 8 in `tests/review_embedded_glyph.rs`. The final Rust-inclusive native comparison evidence is recorded below. Whole-crate compiler/format/lint gates remain recorded by the coordinating agent in milestone status.

## Design boundary and evidence

The standalone reader uses an explicitly selected directory record and a 16-bit code in that record's own encoding. It does not interpret Symbol or Macintosh codes as Unicode and does not choose a Unicode subtable automatically. The reviewed structural and arithmetic rules follow the [OpenType cmap specification](https://learn.microsoft.com/en-us/typography/opentype/spec/cmap): range offsets are relative to their own words, a zero glyph-array entry stays missing, nonzero entries use wrapping delta arithmetic, and search hints need not be trusted. The implementation adopts a stricter subset with language zero, disjoint ordered segments and an FFFF sentinel mapping to zero. It bounds the selected table and validates every covered result against the same font's glyph count before publishing a selection.

Pinned MuPDF source `source/pdf/pdf-font.c` and `source/fitz/font.c` were inspected separately for PDF selection, raw-byte seed, private-use retries, encoding-name overrides and MacRoman reverse lookup. ToUnicode is a different capability and is deliberately never read by this bridge. The regression supplies an invalid ToUnicode reference and malformed Widths while confirming glyph selection succeeds independently.

Read-only inspection of the five hash-pinned fixture fonts found nine actual cmap subtables, all with language zero and the FFFF/FFFF/delta1/rangeOffset0 sentinel. Eight are delta-only. The simple-subset font uses a glyph array with holes and four nonzero mappings (41→1, 48→2, 4C→3, 50→4). Thus those strict standalone rules exclude none of the measured subtables.

## Findings fixed before review completion

- `max_segments` initially capped a single selection while usage reported cumulative segments. The owner made admission cumulative and documented it. Exact-edge and repeated-selection tests verify the cap cannot reset.
- Missing or unsupported PDF source bytes initially returned before reaching the underlying O0 outline attempt counter. The root added a separate code-attempt counter, admitted before any slot outcome. Two missing attempts now exhaust a two-attempt wrapper even if a later code has a valid glyph.
- MacRoman reverse lookup initially constructed a complete encoding table for every code. The root now constructs it once per map.
- Present contradictory font and FontDescriptor Type names initially passed the new loader. The root now validates optional Type declarations; both wrong-type regressions refuse before font decoding.

## Independent tests and model

Executed against the integrated crate:

```
CARGO_TARGET_DIR=/private/tmp/tpdf-page-review-target /Users/owner/.cargo/bin/cargo test --test review_truetype_cmap --test review_embedded_glyph --offline
```

Result: 19 passed, 0 failed.

Standalone cases cover raw-zero versus delta wrapping, different word-relative offsets in one table, final-code out-of-range GIDs, pointers into structural words, odd pointers, pointers crossing the declared subtable, overlapping/unsorted segments, malformed sentinels/padding/language, ignored search hints, directory aliases and unselected unsupported formats, failed selection invalidation, all truncated table prefixes, cumulative selection/query/segment/validation/work limits, and the full 65,536-code domain with maximum valid GID 65,534.

PDF cases cover raw-byte priority over private-use fallback, Flags32 overriding Flags4, explicit encoding overrides, the MacRoman duplicate-space alias at byte202, ambiguity refusal, unknown-name fallback refusal, native raw-seed preservation where known safe, contradictory metadata types, outputs surviving their input Document, repeated load accounting, conservative failed stream reservations, and missing versus successful empty nonzero outlines.

`tools/truetype_cmap_model.py` independently expands segments into a dense mapping instead of sharing the Rust binary-search algorithm. Its generator constructs expected mappings independently, then checks its encoder with that expanded model. Synthetic cases include both delta and array mappings, holes, raw zero, nonzero values wrapping to zero, positive/negative deltas and final GID256. The accepted O0 model supplies unrelated sfnt scaffolding.

The oracle agent reports the preliminary native/model gate passed all nine actual records plus 220 generated records: 15,007,744 exact code queries, saved in `reference/truetype-cmap-native-preliminary-g0.json`. The separate actual-PDF native policy inventory covers all 1,280 byte slots of the five fonts, including 71 nonzero selections. Final Rust comparison is recorded separately.

## Source audit and limitations

The cmap reader adds no heap allocation. Its table bytes borrow the accepted immutable O0 font; opening that font remains a separately bounded prerequisite. Record order, duplicate selectors and exact aliases are preserved for the caller. The PDF policy rejects ambiguous preferred selectors rather than picking one. Failed selections clear the active selection and retain admitted work; invalid query attempts still count.

The outline bridge constructs and retains its validated font and map together. External callers cannot pass an unrelated font/map pair into its decode path; source-code decode is private to that binding. GID0 is missing and is never automatically rendered as `.notdef`. A nonzero glyph with an empty loca range remains a valid empty outline. Wrapper code attempts and underlying O0 decode usage are exposed separately.

PDF loading, O0 opening, cmap validation/queries and outline execution are separate explicit budget partitions. Successful font loads refund only measured unused stream reservations; failed loads retain their reservation. Encoding metadata and processed Difference names charge cumulative loader work. Indirect items within Differences remain a deliberate S0 refusal. The PDF policy also performs fixed-table name work outside CmapUsage: at most 256 source slots each scanning at most 256 MacRoman entries, plus the fixed 16-name correction table and constant-count selector passes. Name sizes remain subject to S0/loader limits, and the MacRoman table is constructed once per map. CmapUsage therefore does not claim to count every PDF-policy name comparison. These partitions are not a universal allocation or process-RSS measurement.

The supported PDF policy requires absent Encoding for symbolic fonts and explicit supported named-base encodings for non-symbolic fonts. It does not implement post glyph-name tables or fuzzy name fallback; affected source codes are explicitly unsupported, even where native recovery would produce zero. Maps retain per-code source, selected lookup code, route and record provenance. The owning loaded object/CLI retains the font/program identity context; it does not claim a complete history of every metadata reference.

No CFF, Type0/CID mapping, composite expansion, shaping, page-program glyph integration, hinting or raster/pixel parity follows from this milestone. The strict parser intentionally rejects malformed maps that the pinned FreeType reference repairs; malformed comparisons must record refusal differences rather than adopt recovery silently.


## Final coordinated evidence

The final reports were inspected after generation:

- `reference/truetype-cmap-differential-g0.json`: all five real fonts/nine actual subtables and 220 generated fonts pass Rust/native/independent-model comparison. There are 229 records and 15,007,744 exact 16-bit character-code queries, with zero failures and no numeric tolerance.
- `reference/glyph-selection-differential-g1.json`: all 1,280 slots are classified. Rust resolves 71 nonzero slots, confirms 989 native-zero slots as missing, and explicitly refuses 220 slots requiring unsupported name fallback. Every native nonzero slot is verified; all 71 resolved outlines match the accepted O0 native geometry, including 1,544 exact commands.
- `reference/glyph-comparator-validation-g1.json`: two valid baselines pass and all 13 injected semantic corruptions are rejected, including record/source-code identity and order, missing queries/slots, changed GIDs, altered outline points/commands, and route/lookup-code corruption.

The coordinating agent reports 571 whole-crate tests passing on both the normal and minimum supported Rust toolchains, including all 19 independent tests above. The final helper-only array cleanup was rechecked by rerunning the 14 bridge/loader tests on both toolchains. Formatting, strict all-target Clippy and minimum-toolchain wasm compilation pass. No remaining production changes are pending.
