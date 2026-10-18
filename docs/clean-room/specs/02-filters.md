# Spec 02: Stream filters and predictors

Workstream 2 of `docs/mit-clean-room-plan.md`. Written under
`docs/clean-room/SPEC-RULES.md`. Standards: ISO 32000-2:2020 §7.4.1–7.4.5
(filters, Tables 6–12), RFC 1950 (zlib) and RFC 1951 (DEFLATE), TIFF 6.0
§14 (differencing predictor), PNG (ISO/IEC 15948 / W3C PNG, §9 "Filtering",
filter types 0–4).

Black-box observations of MuPDF 1.28.1 come from the npm `mupdf` package
(mupdf.js): a raw stream with the given `/Filter` and `/DecodeParms` is added
with `addRawStream` and read with `readStream()`. Inputs were produced by
encoders the spec writer wrote from §7.4 (Appendix A). Where MuPDF recovers
from damage and this engine refuses, the refusal is the engine's chosen
policy (§4) and the existing tests check it.

## 1. Scope

The implementer writes `engine/src/stream.rs` from this spec: the
ASCIIHexDecode, ASCII85Decode, RunLengthDecode and LZWDecode decoders, the
TIFF and PNG predictors, the filter-chain driver with its budgets and
errors, and the FlateDecode stage (a thin layer over the existing
`miniz_oxide` dependency, specified here so the whole file can be rebuilt).

Dictionary interpretation (`/Filter` names, `/DecodeParms` entries,
abbreviations, indirect references) is **not** in this module: callers
build typed `FilterSpec`s. Surviving callers (post-Phase-0 tree):

- `stream_object.rs`: strict and lenient document stream decoding (maps
  `/Filter` names and `/DecodeParms` integers to `FilterSpec`; `EarlyChange`
  0/1 to `Filter::Lzw`).
- `repair.rs`: lenient Flate for object and xref streams, and limit-error
  detection (`DecodedOutputLimit` / `TotalOutputLimit`).
- `app/redact/images.rs`: inline-image data (`decode_filter_specs`).
- `document.rs`: structural streams (Flate only).
- `app/{forms,redact,ops,view_fixes}`: `StreamLimits` values, and matching
  on the two limit kinds to tell "too big" from "damaged".
- `main.rs`: `pdf-inspect --decode=<filters>`.

No `unsafe`. No panics on any input. No recursion. Memory use is bounded by
the limits plus fixed decoder state.

## 2. Interface

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter { Flate, AsciiHex, Ascii85, RunLength, Lzw { early_change: bool } }
// early_change: true is EarlyChange 1 (the PDF default), false is EarlyChange 0.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredictorParams { pub predictor: u8, pub colors: usize,
                             pub bits_per_component: u8, pub columns: usize }
impl Default for PredictorParams  // predictor 1, colors 1, bits_per_component 8, columns 1 (ISO 32000-2 Table 10 defaults)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterSpec { pub filter: Filter, pub predictor: PredictorParams }
impl From<Filter> for FilterSpec   // default predictor parameters

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamLimits { pub max_encoded_bytes: usize, pub max_decoded_bytes: usize,
                          pub max_filters: usize, pub max_total_decoded_bytes: usize }
impl Default for StreamLimits  // 64 MiB, 64 MiB, 8, 128 MiB (the engine's own defaults; callers rely on them)

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamError { pub offset: usize, pub filter_index: Option<usize>, pub kind: StreamErrorKind }
// + Display, std::error::Error

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamErrorKind {
    EncodedInputLimit { limit: usize }, DecodedOutputLimit { limit: usize },
    TotalOutputLimit { limit: usize }, FilterCountLimit { limit: usize },
    MissingEndMarker, InvalidByte(u8), InvalidEndMarker, TrailingData,
    InvalidAscii85Group, Ascii85Overflow, TruncatedRun,
    InvalidFlate, TruncatedFlate, FlateChecksumMismatch, AllocationFailed,
    InvalidPredictor(&'static str), UnsupportedPredictor(u8),
    UnsupportedBitsPerComponent(u8), TruncatedPredictorRow, InvalidPngFilter(u8),
    InvalidLzwCode(u16), TruncatedLzw, MissingLzwClear, LzwDictionaryFull, NonZeroLzwPadding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeUsage { pub encoded_bytes: usize, pub total_decoded_bytes: usize }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedStream { pub bytes: Vec<u8>, pub usage: DecodeUsage }

pub fn decode_filters(data: &[u8], filters: &[Filter], limits: StreamLimits) -> Result<Vec<u8>, StreamError>;
pub fn decode_filters_with_usage(data: &[u8], filters: &[Filter], limits: StreamLimits) -> Result<DecodedStream, StreamError>;
pub fn decode_filter_specs(data: &[u8], filters: &[FilterSpec], limits: StreamLimits) -> Result<Vec<u8>, StreamError>;
pub fn decode_filter_specs_with_usage(data: &[u8], filters: &[FilterSpec], limits: StreamLimits) -> Result<DecodedStream, StreamError>;
pub fn decode_filter_specs_lenient(data: &[u8], filters: &[FilterSpec], limits: StreamLimits) -> Result<DecodedStream, StreamError>;
```

- `filters` are in `/Filter` order: the first one decodes `data`
  (§7.4.1). `decode_filters*` are the `*_specs*` functions with default
  predictor parameters.
- `decode_filter_specs_lenient` differs only in FlateDecode stages (§3.6).
  Every other filter and the predictors stay strict.
- On error no partial output is returned.
- `StreamError::offset` is a byte offset into the failing stage's input
  (for a predictor, into the inflated bytes it was given), never into the
  PDF file. `filter_index` is the zero-based index of the failing
  `FilterSpec` (a predictor failure carries its filter's index), or `None`
  for the input-size and filter-count checks and for the no-filter copy.
- `Display`: `PDF stream filter {index} at byte {offset}: {message}`, or
  `PDF stream at byte {offset}: {message}` when `filter_index` is `None`.
  Messages: `encoded input exceeds {limit} bytes`, `decoded stage exceeds
  {limit} bytes`, `total decoded output exceeds {limit} bytes`, `filter chain
  exceeds {limit} stages`, `missing end-of-data marker`, `invalid encoded byte
  0x{b:02x}`, `invalid end-of-data marker`, `unexpected trailing data`,
  `invalid ASCII85 group`, `ASCII85 group exceeds 32 bits`, `truncated
  run-length packet`, `invalid zlib stream`, `truncated zlib stream`, `zlib
  checksum mismatch`, `stream allocation failed`, `invalid predictor
  parameters: {reason}`, `unsupported predictor {p}`, `unsupported predictor
  bit depth {b}`, `truncated predictor row`, `invalid PNG row filter {p}`,
  `invalid LZW code {code}`, `truncated LZW stream or missing EOD code`, `LZW
  stream must begin with a clear code`, `full LZW dictionary requires a clear
  or EOD code`, `nonzero LZW padding bits after EOD`. (`pdf-inspect`'s error
  text is checked for `unsupported predictor 3`.) `reason` is a short
  English phrase; its wording is free.

## 3. Required behaviour

### 3.1 Chain driver and budgets

In this order:

1. `data.len() > max_encoded_bytes` → `EncodedInputLimit { limit }`
   (offset 0, no index).
2. `filters.len() > max_filters` → `FilterCountLimit { limit }` (offset 0,
   no index). With no filters the limit is irrelevant (an empty chain is
   always allowed, even with `max_filters = 0`).
3. No filters: the result is a copy of `data`, charged as one stage (step
   6); usage is `{ encoded_bytes: len, total_decoded_bytes: len }`.
4. Validate every spec before decoding anything (so an invalid later stage
   fails before an expensive earlier one runs), in index order:
   a. A non-default `predictor` on a filter other than Flate or LZW →
      `InvalidPredictor`.
   b. Predictor parameters (§3.7.1), checked even when `predictor` is 1.
   c. When `predictor` is not 1: one decoded row (§3.7.1) larger than the
      stage cap of step 6 computed with the full total budget → that limit
      error, even for empty data.
5. Decode stage by stage. After a filter stage, if its `predictor` is not
   1, apply the predictor as a further stage. Predictor 1 adds no stage, no
   copy and no charge.
6. Budget: each stage (filter output, predictor output, or the no-filter
   copy) may produce at most `cap = min(max_decoded_bytes, remaining)`
   bytes, where `remaining` is `max_total_decoded_bytes` minus everything
   the earlier stages produced. Exceeding it fails at the point of
   exceeding (before the bytes are materialized) with
   `TotalOutputLimit { limit: max_total_decoded_bytes }` when
   `remaining < max_decoded_bytes`, else `DecodedOutputLimit { limit:
   max_decoded_bytes }`. An exact fit succeeds; a zero cap still allows
   empty output.
7. `DecodeUsage.total_decoded_bytes` is the sum of every stage's output,
   intermediate ones included (`AsciiHex,RunLength` decoding
   `0261626380>` reports 8: 5 then 3). `encoded_bytes` is `data.len()`.
8. Memory: at any time the chain holds at most the current stage input and
   output plus fixed-size decoder state; output buffers grow geometrically
   but never beyond the cap; allocation failure is `AllocationFailed`.

### 3.2 PDF white space

For the ASCII filters: NUL, HT, LF, FF, CR, SP (ISO 32000-2 §7.2.3 Table 1).
VT (0x0B) is not white space.

### 3.3 ASCIIHexDecode (§7.4.2)

- Hex digits of either case give 4 bits each; white space is ignored; `>`
  is EOD. An odd final digit is followed by an implied 0 (`A>` → A0).
- Any other byte before EOD → `InvalidByte(b)` at that byte.
- No `>` → `MissingEndMarker` at `data.len()`.
- After `>`, only white space may follow; otherwise `TrailingData` at the
  first other byte (`00> 00`, `00>>`).

### 3.4 ASCII85Decode (§7.4.3)

- Bytes `!`..`u` are base-85 digits (value = byte − 0x21); groups of 5
  digits give 4 bytes, most significant first. White space is ignored
  anywhere, including inside a group.
- `z` as the first character of a group gives four zero bytes; `z` inside a
  group → `InvalidAscii85Group` at the `z`.
- `~` must be immediately followed by `>` (EOD) → otherwise
  `InvalidEndMarker` at the `~` (`~`, `~ `, `~x`, `~\n>` all fail).
- A final partial group of n = 2, 3 or 4 digits is padded with `u` (84) to
  five digits and gives n − 1 bytes. A final group of one digit →
  `InvalidAscii85Group` (at the `~`).
- A group (full, or partial after padding) whose value exceeds 2^32 − 1 →
  `Ascii85Overflow` (`s8W-"`, `uuuuu`, `uu~>`).
- Any byte other than digits, `z`, `~`, white space → `InvalidByte(b)`
  (this includes `v`–`y`, `{`–`}`, controls, ≥ 0x80, and the `<~` prefix
  some PostScript producers write).
- No `~>` → `MissingEndMarker` at `data.len()`. After `~>` only white space.
  Known vectors: `87cURD_*#TDfTZ)+T~>` → `Hello, world!`;
  `9jqo^BlbD-BleB1DJ+*+F(f,q~>` → `Man is distinguished`; `rr~>` → FF;
  `s8N~>` → FF FF; `s8W*~>` → FF FF FF; `s8W-!~>` → FF FF FF FF.

### 3.5 RunLengthDecode (§7.4.5)

- Length byte L: 0–127 → copy the next L + 1 bytes; 129–255 → repeat the
  next byte 257 − L times; 128 → EOD.
- A packet whose data runs past the end → `TruncatedRun` at the length
  byte (`[0]`, `[2,'a','b']`, `[255]`).
- No EOD → `MissingEndMarker` at `data.len()` (`[]`, `[0,'a']`,
  `[255,'a']`). After EOD only white space (`[128, 13, 10, 0]` is fine,
  `[128, 1]` is `TrailingData`).
- Repeat runs are checked against the cap before they are written.

### 3.6 FlateDecode (§7.4.4, RFC 1950/1951)

Strict (the default entry points):
- `data` must be exactly one complete zlib stream: valid header, DEFLATE
  data, Adler-32 trailer that matches. Bytes after the zlib stream →
  `TrailingData` at the first such byte. Input that ends before the stream
  is complete → `TruncatedFlate`. Adler-32 mismatch →
  `FlateChecksumMismatch`. Anything else → `InvalidFlate`. Offsets are input
  offsets where decoding stopped.
- The output cap applies while inflating: inflation stops as soon as the
  cap would be exceeded (a zlib bomb never materializes beyond the cap).
  The cap must be exact at every output length (`tests/stream.rs` checks
  1, 63, 64, 65, 127, 128, 129, 4095, 4096, 4097, 8192 and 16385 bytes).

Lenient (`decode_filter_specs_lenient`), for damaged files:
- Bytes after the end of the zlib stream are ignored.
- A truncated or corrupt stream, or a checksum mismatch, is not an error:
  the bytes produced before the problem are the stage's output (possibly
  empty).
- Limits and allocation failures are still errors.
- MuPDF behaves this way on such input (recorded earlier with the native
  oracle, not re-run for this spec: a stream without its Adler trailer
  decoded to `abc` with a warning).

### 3.7 Predictors (§7.4.4.4, Table 10)

#### 3.7.1 Parameters and row geometry

Validation, in this order (all with offset 0 and the filter's index):
1. `predictor` must be 1, 2 or 10–15 → else `UnsupportedPredictor(p)`
   (e.g. 0, 3, 9, 16, 255).
2. `bits_per_component` must be 1, 2, 4, 8 or 16 → else
   `UnsupportedBitsPerComponent(b)` (0, 3, 7, 32, 255).
3. `colors` must be 1..=32 → else `InvalidPredictor`. (32 is the largest
   number of DeviceN colourants in the architectural limits of ISO
   32000-1:2008 Annex C; MuPDF was also observed to refuse 33, 64 and 65 and
   accept 32, E-PRED.)
4. `columns` must be ≥ 1 → else `InvalidPredictor`.
5. Any arithmetic overflow computing the sizes below → `InvalidPredictor`.

Row geometry: a row has `columns` pixels of `colors` samples of
`bits_per_component` bits; decoded row bytes = ceil(columns × colors × bpc
/ 8); bytes per pixel for PNG = ceil(colors × bpc / 8) (at least 1).
Samples are packed most-significant bit first; 16-bit samples are
big-endian.

The predictor's input must be a whole number of encoded rows (decoded row
bytes, plus one tag byte per row for PNG); otherwise
`TruncatedPredictorRow` at `input.len()`. No partial row is ever returned.
The output is rows × decoded row bytes and counts as a stage (§3.1).

#### 3.7.2 TIFF Predictor 2 (TIFF 6.0 §14)

Each sample is the encoded sample plus the decoded sample `colors`
positions earlier in the same row, modulo 2^bpc; the first pixel of each
row is unchanged; history never crosses rows. For bpc < 8 the unused bits
at the end of each decoded row are zero.
Observed and required (E-PRED): bpc 8, 1 colour, 4 columns: rows `01010101
02020202` → `01020304 02040608`; bpc 8, 3 colours: `0a141e 010101 050505
ff0000` (2 rows of 2 pixels) → `0a141e0b151f 050505040505`; bpc 16, 1 colour, 2 columns (4-byte rows):
`00ff0002 ffff0001` → `00ff0101 ffff0000`; bpc 1, 5 columns: `a8 ff` → `c8 a8`; bpc 2,
3 columns: `57 ff` → `6c e4`; bpc 4, 3 columns: `111f 222f` → `1230 2460`
(padding nibble cleared); bpc 4, 2 colours, 1 column: unchanged.

#### 3.7.3 PNG Predictors 10–15

Each encoded row starts with a tag byte selecting the PNG filter type for
that row: 0 None, 1 Sub, 2 Up, 3 Average, 4 Paeth, computed bytewise
exactly as in the PNG specification, with "left" being the byte
bytes-per-pixel positions earlier in the decoded row (0 before the row
start), "above" the same byte of the previous decoded row (0 for the first
row), and Paeth's tie order as in PNG (left, then above, then upper-left).
The predictor number 10–15 only says "PNG"; the row's own tag is always
used, whatever the number (§7.4.4.4; MuPDF does the same, E-PRED: the
same five-row input decodes identically under 10, 11, 12, 13, 14 and 15).
A tag above 4 → `InvalidPngFilter(tag)` at the row's first byte.
Observed and required (E-PRED, 1 colour, bpc 8, 3 columns unless noted):
rows tagged 0,1,2,3,4 `00 010203 | 01 010101 | 02 010101 | 03 010101 | 04
010101` → `010203 010203 020304 020304 030405`; Paeth second row `000a141e
04010203` → `0a141e 0b1621`; Average second row `000a141e 03010203` →
`0a141e 060f19`; 3 colours Sub `01 0a141e 010101` → `0a141e0b151f`; bpc 4,
4 columns Sub `01 1211` → `1223`; bpc 16, 2 columns Sub `01 0102 0101` →
`0102 0203`.

### 3.8 LZWDecode (§7.4.4.2, Table 10 EarlyChange)

- Codes are read most-significant bit first, starting at 9 bits.
- Code 256 is clear-table, 257 is EOD, 258 onward are table entries; codes
  0–255 are single bytes. The table never exceeds 4096 entries (codes are
  at most 12 bits).
- Code width: the standard states the encoder's rule (the first 10-bit code
  follows the creation of table entry 511, and so on for 11 and 12 bits,
  "one code early" for EarlyChange 1; postponed as long as possible for
  EarlyChange 0). A decoder adds each entry one code later than the
  encoder, so it switches to 10, 11, 12 bits as soon as its next free entry
  is 511, 1023, 2047 (EarlyChange 1) or 512, 1024, 2048 (EarlyChange 0).
  Test vectors: a stream of n literal codes after a clear uses 9-bit codes
  for code positions 0–253 (EarlyChange 1) or 0–254 (EarlyChange 0), and so
  on (the helper `literal_lzw_width` in `tests/stream.rs` encodes this).
- After a clear (and at the start), the table holds only the 258 fixed codes
  and the width is 9. The next code must be a literal (< 256), clear, or
  EOD; 258 or more → `InvalidLzwCode(code)`.
- Otherwise each code c with a previous code p: if c is an existing entry,
  output its string and add the entry p's string + first byte of c's
  string; if c is the next free entry (the KwKwK case), its string is p's
  string + first byte of p's string, output it and add it; if c is beyond
  the next free entry → `InvalidLzwCode(c)`.
- When the table is full (4096 entries), the next code must be clear or
  EOD → else `LzwDictionaryFull`.
- Strictness (engine policy, §4): the stream must start with a clear code
  (`MissingLzwClear` otherwise); input that ends before EOD →
  `TruncatedLzw`; the bits after EOD in its last byte must be zero
  (`NonZeroLzwPadding`); no byte may follow (`TrailingData`).
- Each output string is checked against the cap before it is copied. String
  reconstruction must not recurse and must be bounded (a chain of 3,838
  successive KwKwK codes decodes to 7,370,880 bytes of `A` when the cap
  allows it, and fails cleanly with `DecodedOutputLimit` when it does not).
- `EarlyChange` values other than 0 and 1 are refused by `stream_object.rs`
  before this module is called.
Vectors (both EarlyChange values): `256,257` → empty; `256,65,66,258,260,257`
→ `ABABABA`; `256,65,258,259,260,257` → `AAAAAAAAAA`;
`256,65,258,256,256,66,258,257` → `AAABBB` (clear replaces old entries).

## 4. Tolerant behaviour: recorded differences

This module's strict entry points serve editing and redaction, where a
silent partial decode could destroy content when the stream is rewritten;
they refuse damaged data. The view renders through a separate decoder and is
not affected. Only Flate is relaxed, and only in the lenient entry point
(§3.6), because repair needs object and xref streams back.

| Input (filter) | MuPDF 1.28.1 observed (E-FILT, E-PRED) | This engine |
|---|---|---|
| `414` (AHx, no `>`) | `41`, no error | `MissingEndMarker` |
| `41>ZZ` (AHx) | `41`, trailing ignored | `TrailingData` |
| `4G>` (AHx) | error | `InvalidByte(G)` |
| `41\x0b42>` (AHx, VT) | error | `InvalidByte(0x0b)` |
| `…Ebo80` (A85, no `~>`) | decoded, no warning | `MissingEndMarker` |
| `…Ebo80~` (A85) | decoded, warning | `InvalidEndMarker` |
| `…Ebo80~>xyz` (A85) | decoded, trailing ignored | `TrailingData` |
| `!~>` (A85, 1-digit group) | empty, warning | `InvalidAscii85Group` |
| `s8W-"~>`, `uuuuu~>` (A85 > 2^32−1) | wraps (`00000000`, `08780ec4`) | `Ascii85Overflow` |
| `<~87cURD…~>` (A85 with `<~`) | error | `InvalidByte('<')` |
| `02616263` (RL, no EOD) | `616263` | `MissingEndMarker` |
| `0041800042` (RL, data after EOD) | `41` | `TrailingData` |
| `0261` (RL, short packet) | error | `TruncatedRun` |
| LZW without leading clear | decoded | `MissingLzwClear` |
| LZW without EOD | decoded plus a spurious final byte | `TruncatedLzw` |
| LZW `256,258,257` | empty, no error | `InvalidLzwCode(258)` |
| LZW bytes after EOD | ignored | `TrailingData` |
| LZW nonzero padding after EOD | ignored | `NonZeroLzwPadding` |
| LZW code beyond next entry | error | `InvalidLzwCode` |
| PNG tag 5 or 255 | row treated as None, warning | `InvalidPngFilter` |
| PNG/TIFF partial last row | partial row output | `TruncatedPredictorRow` |
| Predictor 3, 9, 16 | treated as 1, warning | `UnsupportedPredictor` |
| BitsPerComponent 3 | error | `UnsupportedBitsPerComponent` |
| Columns 0 | garbage output | `InvalidPredictor` |
| Colors 33 | error | `InvalidPredictor` |

Open question for the owner (not required for acceptance): whether the
lenient entry point should also accept a missing EOD for ASCIIHex, ASCII85,
RunLength and LZW and a missing leading LZW clear, as MuPDF does. The
current tests do not cover lenient non-Flate input, so either answer is
compatible with them.

## 5. Limits and safety

- All size arithmetic is checked; overflow is an error, never a wrap.
- Caps are enforced before writing; no stage ever holds more than its cap.
- No recursion; LZW string reconstruction uses bounded scratch space.
- Every input terminates: each loop consumes input or fails; the Flate
  output buffer grows strictly on every "more output" round so it cannot
  spin.
- Fallible allocation (`try_reserve*`) everywhere sizes depend on input;
  failure is `AllocationFailed`.

## 6. Acceptance

Counts are `#[test]` functions.

### 6.1 Keep (black-box)

| File | Tests | Notes |
|---|---|---|
| `tests/stream.rs` | 29 of 29 | all filters, limits, predictors, LZW widths/clears/KwKwK, usage. |
| `tests/review_lzw.rs` | 4 of 4 | Scrub the module comment ("native-confirmed"), the test name `native_vectors_cover_kwkwk_and_clear_replacing_old_dictionary_entries` (drop "native") and the line "Native lzw1 decoder gives …" to "independently confirmed"; they refer to the AGPL oracle, which the implementer must not run. |
| `tests/review_predictor.rs` | 5 of 5 | |
| `tests/review_r3.rs` | 9 of 9 | 3 exercise this module (`filter_chain_bounds_intermediate_expansion_and_total_work`, `flate_expansion_exact_limit_and_zero_output`, `deterministic_adversarial_filters_stay_within_limits`); 6 are document-level and unaffected. |
| `tests/stream_object.rs` | 17 of 17 | dictionary layer over this module. |
| `tests/review_stream_object.rs` | 7 of 7 | includes the `pdf-inspect` message check `unsupported predictor 3`. |

Apart from the `review_lzw.rs` items above, none of these cite MuPDF or its oracle.

### 6.2 Replace

| Item | Why | Replacement |
|---|---|---|
| `engine/reference/filter-vectors.json` (12 vectors with `mupdf_*` fields) | records the native oracle's output and diagnostics; not handed to the implementer (move to `engine/oracle/`) | Written by the acceptance-suite author before hand-off: new Rust test `tests/filter_vectors.rs` with the same 12 inputs and this engine's expected results: the 4 valid vectors decode to the recorded bytes; the 4 malformed ones fail with `InvalidFlate`, `InvalidByte(b'G')`, `InvalidByte(b'v')`, `TruncatedRun`; the 4 missing-EOD ones fail strictly (`TruncatedFlate`, `MissingEndMarker` ×3) and the Flate one gives `abc` through the lenient entry point. |
| `engine/oracle/differential-filters.py`, `differential-lzw.py`, `differential-predictors.py` (moved in Phase 0) and their reports (`differential-filters-r3.json`, `differential-lzw.json`, `differential-predictors-r3b.json`, `stream-comparison-r3*.json`) | drive the native AGPL oracle | Reviewer-side differential after implementation (moved with the oracle in Phase 0); not handed to the implementer. |

### 6.3 New cases

1. The rows of the §4 table, each asserting the engine's result (several
   already exist in `tests/stream.rs`; add the missing ones: AHx with VT;
   A85 `<~` prefix exists; RL data after EOD; LZW `256,258,257` exists;
   predictor 9 and 16 exist).
2. All §3.7.2 and §3.7.3 vectors as exact-output tests (bpc 1, 2, 4, 16
   TIFF; PNG tags 0–4 under every predictor number 10–15; bpc 4 and 16 Sub).
3. LZW round trips with an encoder written from §7.4.4.2 for both
   EarlyChange values on 20,000 bytes with clears when the table fills,
   plus a stream whose table fills exactly and continues after a clear.
4. Lenient Flate: trailing garbage ignored; truncated stream keeps its
   prefix; bad Adler keeps the full output; garbage input gives empty
   output; limits still fail.
5. `filter_index` and `offset` for an error in each stage of a three-stage
   chain (`AsciiHex, Flate+PNG`, error in the predictor reports index 1 and
   an offset into the inflated bytes).

## Appendix A. Experiment log

Reference: npm `mupdf` 1.28.1 via `tpdf-wt/tools/slot node <script>` on macOS
arm64. Scripts and outputs in `tpdf-wt/scratch/spec-01-02/`: `fenc.mjs`
(the spec writer's own LZW, run-length and ASCII85 encoders, written from
§7.4), `t7.mjs` → `e-filt.txt`, `t8.mjs` → `e-pred.txt`. Each case:
`addRawStream(bytes, <</Filter … /DecodeParms …>>)`, then `readStream()`;
the hex output and any warning lines are recorded.

- **E-FILT** ASCIIHex: `41 42>`→`4142`; `4>`→`40`; `414`→`41`; `41>ZZ`→`41`;
  `4G>`→ error, empty; `>`, empty input → empty; `41\x0042>`→`4142`;
  `41\x0b42>`→ error; `a b c d>`→`abcd`; `41>42>`→`41`.
  ASCII85: `z!<~>`→`0000000001`; `87cURD]i,"Ebo80`→ decoded (12 bytes);
  `…~`→ decoded + warning; `…~>xyz`→ decoded; `z~>`→`00000000`;
  `a z~>`→ error; `!~>`→ empty + warning; `!!~>`→`00`;
  `!!!~>`→`0000`; `!!!!~>`→`000000`; `s8W-!~>`→`ffffffff`;
  `s8W-"~>`→`00000000`; `uuuuu~>`→`08780ec4`; `v~>`→ error;
  `87cU RD\n]i~>`→`48656c6c6f21`; `zz~>`→ 8 zero bytes; `a\x00b~>`→`ca`;
  `<~…~>`→ empty + warnings.
  RunLength: `0261626380`→`616263`; `02616263ff7880`→`6162637878`;
  `fe4180`→`414141`; `814180`→ 128 × `41`; `0261`→ error;
  `02616263`→`616263`; `ff`→ error; `8041`, `80`, empty → empty;
  `0041800042`→`41`.
  LZW (`fenc.mjs` encoder): `-----A---B` with EarlyChange 1 and 0 decoded
  exactly; data encoded with EarlyChange 1 read with EarlyChange 0 → error; 20,000 bytes round-trip exactly under both values; no leading
  clear → decoded; no EOD → decoded plus a trailing `ff` and a
  warning; `256,257`→ empty; `256,65,66,258,260,257`→`ABABABA`;
  `256,65,300,257`→ error; `256,258,257`→ empty, no error; bytes after EOD
  and nonzero padding → ignored; `/EarlyChange 2` → read as 1.
- **E-PRED** (FlateDecode over the given rows, compressed with node zlib):
  every row quoted in §3.7.2 and §3.7.3; TIFF partial row `010101010202`
  (4 columns) → `010203040204`; PNG partial row → `01020301`; PNG tags 5
  and 255 → row read as None, warning; Predictor 1 →
  identity; 3, 9, 16 → warning, identity; bpc 3 → error; Colors 0 → output as if 1; Colors 32 →
  decoded; 33, 64, 65 → error; Columns 0 → `0104`
  (meaningless); no DecodeParms Columns → default 1; LZW then PNG
  Predictor 12 → `010101020202`.

## Appendix B. Leak-check change log (2026-10-18)

Independent leak check against SPEC-RULES.md. Edits made:

1. §4 table and Appendix A: removed MuPDF's own diagnostic message texts
   (expressive content not needed to implement); outcomes now read
   "error" or "warning".
2. §3.6: the Flate missing-trailer fact is marked as recorded earlier with
   the native oracle and not re-run for this spec.
3. §6.1: `review_lzw.rs` scrub extended to the test name containing
   "native"; the "none cite MuPDF" claim corrected.
4. §6.2: `filter-vectors.json` marked as not handed to the implementer (the
   acceptance-suite author writes `tests/filter_vectors.rs` first); tool
   paths updated to `engine/oracle/`.

Questions for the owner (gaps found, not filled):

- §3.1 step 4a: is "a non-default `predictor`" on a non-Flate, non-LZW
  filter only `predictor != 1`, or any parameter differing from Table 10's
  defaults (for example `Colors 3` with predictor 1 on ASCIIHex)?
- §3.6: a zlib header with a preset dictionary (FDICT set): `InvalidFlate`
  in both strict and lenient modes?
- §3.6: are the reported error offsets for Flate required to be exact, or
  only "within the input"? (They depend on the inflater's buffering.)
- §4: the open question on lenient non-Flate decoding.
