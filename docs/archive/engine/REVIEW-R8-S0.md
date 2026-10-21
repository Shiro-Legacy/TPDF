# Independent review: explicit simple-font encodings (S0)

Reviewed `src/simple_encoding.rs`, generated `src/simple_encoding_data.rs`, its generator and `src/cli_encoding.rs`. No blocking production issue found within the declared standalone S0 scope.

`tools/differential-simple-encoding.py` independently reads pinned `encodings.h` and `glyphlist.h`, verifies all recorded source hashes, and checks 1,024 native-oracle rows before comparing Rust output. It uses no Python codec substitutions. `reference/simple-encoding-differential-s0.json` records 210 passing inputs and 53,760 complete slot comparisons: the four base tables, all 378 registry names assigned through Differences, 200 deterministic generated override sequences, and four dictionary/null cases. Both executable hashes remained unchanged throughout the run. All 31 malformed CLI cases refused with nonzero status and empty stdout.

The comparisons preserve glyph bytes, exact table-specific Unicode mappings, undefined base slots, explicit `.notdef`, and unknown names. Generated cases include repeated/nonmonotonic/consecutive resets, byte 255, high bytes, escaped delimiters, line feeds, 127-byte names, and unknown Adobe-style aliases. Adapter cases cover missing/null/indirect BaseEncoding, wrong Type, unknown meaningful keys, non-array/indirect Differences, invalid integer/type entries, empty/overlong/NUL PDF names and the default item cap. Explicit `.notdef` retains its glyph name while returning UndefinedCode. Manual NUL glyph names remain opaque through the library; PDF name parsing keeps its existing stricter NUL refusal.

Four additional tests in `tests/review_simple_encoding.rs` pass, as does targeted Clippy:

- All 256 slots own opaque NUL/high-byte identifiers after source buffers are dropped; follow-up updates and zero-budget empty updates preserve untouched owned names.
- An independent 256-slot override model agrees with both batched and successive updates across 32 seeds and 9,600 assignments.
- Repeatedly overwritten names pass at exactly 262,144 processed name bytes and fail on the next byte at the exact Difference index.
- An invalid overwritten entry cannot be hidden by a later reset; errors preserve the original encoding, report the offending index, and reject nested Object values without traversing/cloning the nested payload.

Implementation review confirms that bounds are checked before retaining replacements or allocating names; every incoming name is charged even if overwritten. Final retained state is fixed at 256 slots. Borrowed static table names are reused and owned names are copied with fallible reservations. A cursor reaching 256 is rejected before indexing and can be reset explicitly. Object adaptation checks signed integers before conversion and rejects references instead of resolving them. Result fields are private with immutable accessors. Unicode lookup is restricted to the sorted 378-name registry and uses valid Rust characters.

The native table oracle alone does not exercise PDF Differences parsing. Generated Differences expectations therefore come from an independent explicit reset/assignment model over the source/native-confirmed base tables. The report does not claim native font-loader, fallback, text extraction, metrics, rendering, or general Adobe Glyph List compatibility. Existing owned names do not consume a later call's incoming-name budget, as documented; callers choosing enlarged limits explicitly permit enlarged owned inputs.
