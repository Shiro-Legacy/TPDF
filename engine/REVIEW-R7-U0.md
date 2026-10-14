# Independent review: strict standalone ToUnicode maps

Reviewed typed character-code construction, code-space intervals, prefix decoding, strict CMap grammar, UTF-16 conversion, range expansion, and cumulative resource accounting. No blocking implementation finding was identified in the documented U0 subset.

## Independent evidence

`tools/differential-cmap.py` compares standard-library Python expectations, the Rust query API, and the pinned native CMap oracle. The accepted report is `reference/cmap-differential-u0.json`:

- 160 generated maps plus all four strict-valid fixture maps match: 3,680 mapping entries and 8,402 queries. The actual fixture maps contribute 517 entries, all enumerated.
- Coverage includes source widths 1–4, leading zeros, unsigned high-bit codes, bfchar, sequential bfrange, array destinations, literal and hexadecimal source/destination strings, zero/one/100-entry sections, and interleaved forms.
- Unicode cases include NUL, BOM as an ordinary scalar, noncharacters, supplementary characters, combining sequences, ligatures, and supplementary characters before/after BMP characters.
- All 28 generated malformed programs refuse with empty stdout.
- All six malformed actual maps refuse at an independently identified offending destination offset. The native oracle's truncation recovery is separately checked against independently computed truncated UTF-16 expectations and recorded, never treated as the intended Unicode mapping.
- A separate width-collision map maps `80` to A and `0080` to B. Rust preserves both. Native integer lookup returns the later B for both numeric codes; the report records this native API limitation without claiming parity.
- Invalid/incomplete code segmentation returns distinct Rust errors. Native fallback to consumed-one/value-zero is recorded separately.

Six independent Rust tests pass. They include 131,072 short-prefix classification checks against an explicitly enumerated finite code set, typed-width constructor boundaries, exact mixed-form record/entry/token/scalar budgets, precise odd-byte destination offsets, cross-width interval projection in both insertion orders, preflight expansion refusal, and the 64/65 UTF-16-unit limit. Targeted Clippy with warnings denied passes.

## Invariants and limits

Code width and Unicode scalars are private, validated fields; immutable getters cannot mutate them. Expanded entries and their diagnostic offsets are separately retained in bounded maps. Source ranges reserve their full entry work before expansion. Every produced Unicode scalar is validated and charged; metadata names/strings also consume the owned-byte budget. Code-space overlap and prefix ambiguity are rejected independently of which codes happen to have mappings. Missing mappings stay absent rather than becoming spaces or numeric Unicode identity.

U0 deliberately rejects unknown executable syntax, base CMaps, CID/notdef sections, unsupported wrappers, malformed Unicode, duplicate/overlapping mappings, source numeric bfranges above 65535, UTF-16 carry, and ranges crossing invalid surrogate boundaries. These strict refusals differ from native recovery. No font loading, document resource resolution, fallback, glyph mapping, extraction, text positioning, or rendering follows from this standalone parser.
