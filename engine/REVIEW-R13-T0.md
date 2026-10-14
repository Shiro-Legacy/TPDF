# T0 independent positioning review

Reviewed `text_font.rs`, `text_position.rs`, and the direct standalone configuration/trace adapter in `cli_text.rs`. The accepted scope is one-byte horizontal source-code positioning under explicitly injected mapping and advance policies. It does not load a page's fonts, select glyph outlines, draw text, infer reading order, or execute mixed page content. No remaining blocker was found after the two review fixes below.

## Independent and native arithmetic gate

`tools/text_position_model.py` implements a separate Python Fraction affine/state model. It does not use Rust output, parser, matrix code or floating-point arithmetic to derive expected state. Hand-calculated checks separately exercised noncommuting affine composition, a ligature receiving one advance, source-byte word spacing, text rise and a trailing numeric adjustment.

`tools/differential-text-position.py` compares the complete Rust JSON trace, including every source code, Unicode byte range, decoded-string/source provenance, font policy, width origin, scalar state, before/after matrices, normalized-em transform, page origin, separate glyph/word displacements, adjustment record, text-object end matrix and event order. All 246 programs / 2,906 source-code placements match exactly. The 240 generated arithmetic programs include 80 cases with negative/fractional/zero widths and signed or zero size/scale; all rational inputs are chosen so division by 1000 and 100 produces dyadic steps. Four additional programs cover both builtin faces, Encoding Differences, and authoritative ToUnicode that differs from glyph-name mapping. Two cover declared versus default missing-width provenance. Builtin expectations come from the independently verified M1 native table evidence.

The generated state sequences exercise every accepted text operator, several BT/ET objects, font changes, noncommuting CTM/Tm, reflections/rotations/skews, Td after showing, TD/T*, quote spacing persistence, q/Q around objects, empty strings/arrays, and adjacent/trailing numeric adjustments. Mapping cases include source0x20 mapped away from space, another byte mapped to Unicode space, fi as one source code, NBSP, supplementary Unicode and a combining sequence.

For the 160 supported native integer-width cases, the tolerance was declared before either comparison: absolute component error <=0.002 + 0.00002*max(1,abs(expected)), with at most100 source codes and expected coordinates bounded by10000. Native float32 is not compared bitwise to Rust f64. Observed maximum absolute component error was 0.000007600000003549212. Primary native items and Unicode continuation entries are grouped by callback without inventing extra advances; glyph IDs and glyph bounds are not parity claims. Native font substitution/recovery and fractional-width quantization are outside this gate.

The final report is `reference/text-position-differential-t0.json`. It records deterministic seed2026100313, native output hashes, all binary/harness/evidence hashes and zero failures. The final Rust binary SHA256 is `4f638c49f9b4a3cb53f79b3b0b653ccd96a97104d05180f7637c611dad787969`; hashes remained unchanged during the run. All48 malformed/configuration/unsupported/missing-metric CLI cases refuse with diagnostics and empty stdout, including invalid suffixes after a valid prefix.

## Independent implementation regressions

All six tests in `tests/review_text_position.rs` pass, with targeted strict Clippy:

- Literal octal/line-continuation and hex whitespace provenance refers to decoded strings plus enclosing operation spans. Output and font descriptors remain valid after the source/registry/mappings are dropped.
- Operation exhaustion reports the attempted operation index and remains sticky across further push and finish calls.
- Directly constructed, validated TJ operations still preflight cumulative element/payload limits before decoding. Bypassing ContentParser does not bypass interpreter bounds.
- Empty shows and numeric-only arrays consume bounded work and preserve trailing cursor state.
- Repeated multiscalar mappings charge actual scalar/UTF8 work and preserve operation, operand, array-element and decoded-string error context.
- A zero horizontal scale cannot hide an overflowing width/size intermediate.

## Findings resolved and resource review

The independent review found that a failed operation-count increment left the counter unchanged while error construction subtracted one, reporting the prior operation. `push` now captures the attempted index before execution; the regression verifies both current span/index and sticky behavior.

Root review found that retained arena growth charged only capacity deltas while promising cumulative requested capacity. Both vector and string reserves now charge their full requested target on each growth. The owner's exact-boundary regression and the source audit cover that fix. Single-byte decoding avoids a whole-string temporary; its conservative four-times temporary code/UTF8 reservation is cumulative, while persistent arenas, font names and the graphics stack are independently charged. Limits bound operations, empty/string shows, TJ elements, source/code/scalar/UTF8 totals, text objects, selected names, allocation requests and numerical magnitudes. Requested allocation is not a claim about allocator bookkeeping or process RSS.

TextFont construction rejects wider or vertical ToUnicode sources once. Builtin width selection uses its own exact Encoding, while an authoritative ToUnicode changes Unicode alone. Missing mappings and missing/undefined builtin metrics remain errors. No callback or mutable resource loader runs during interpretation. q/Q save CTM and text state outside BT; q/Q/cm inside BT remain explicitly unsupported, preserving the stated native/spec policy boundary. Unsupported paths, colors, marked content and other suffixes cannot publish a partial trace.

Owner full-feature/MSRV/wasm gates are maintained by root separately. This review does not broaden T0 into PDF-page text extraction or font rendering.
