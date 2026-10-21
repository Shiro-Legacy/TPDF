# Independent review: explicit metrics and unified declared text decoding

Reviewed `src/font_metrics.rs`, the `load_metrics` adapter and counters in `src/font_resources.rs`, the refactored shared accumulator in `src/text_decode.rs`, `src/font_text.rs`, and the encoding-text CLI boundary. No blocking production issue found in the declared M0/D0 scope.

Six tests in `tests/review_metrics_text.rs` pass, with targeted strict Clippy and rustfmt:

- All 256 width slots preserve their finite signed/fractional values and explicit-entry provenance, including negative zero, without substituting MissingWidth for explicit zero.
- Two fonts sharing one Widths array retain different MissingWidth values/provenance; cached aliases avoid repeated width work while selection attempts still count.
- Exhausting metadata work at the second width reports exact index 1 and publishes no partial metrics or retained-width charge.
- Failed metrics loading does not poison a previously cached Unicode mapping.
- Every defined slot in all four simple encodings yields one source byte, a complete contiguous UTF8 span, and the original code value.
- Unknown opaque glyph names (including manual NUL bytes) remain distinct from `.notdef` after a valid multibyte-output prefix, with source-byte error offsets and no partial result.

`tools/differential-encoding-text.py` uses independently parsed pinned source tables already verified against the native encoding oracle; it uses no Python codec substitution. `reference/encoding-text-differential-d0.json` records 110 passing valid inputs, 50,674 codes and 98,146 UTF8 bytes, covering every one of 746 defined base slots, all 378 registry names through overrides, empty inputs and generated strings. All 278 undefined base slots, six unknown/undefined overrides and two input/work limits refuse with empty stdout: 286 strict refusals in total.

The shared accumulator also passes the existing independent/native ToUnicode string corpus again: 106 cases, 6,332 codes, 39,116 UTF8 bytes and six strict refusals, saved separately in `reference/text-decode-differential-d0-regression.json`. This preserves the earlier R7b evidence while testing the new implementation. The Rust executable hash remained `2c584a0065ca37e09cdb7e2dc7f05778923273097c7967c4ceb65db76bb85165` throughout both comparisons.

Code review confirms exact first/last range and width-count validation before fixed temporary storage is indexed; all width numbers pass finite/exact-integer validation; missing declarations require builtin/font-program context instead of guessing. Width attempts, processed entries, retained metrics/entries, metadata and references have separate bounds. Metrics identity remains font identity rather than shared array identity. Explicit metrics can load independently of rejected ToUnicode and Encoding declarations.

The private text accumulator bounds input, codes, scalar work, UTF8 bytes and geometric allocations; verifies positive bounded code consumption; borrows source bytes while owning text/spans; and publishes only a complete result. Mapping dispatch retains authoritative ToUnicode and distinct undefined/unknown encoding outcomes. It does not combine widths with text, shape glyphs, infer text positions, normalize Unicode, perform fallback or claim extraction correctness. Root owns the separate five-font fixture metrics comparison.
