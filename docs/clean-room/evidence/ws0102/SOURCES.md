# Sources used (workstreams 01 and 02)

Everything below was used from the implementer's own knowledge of these
public documents. No web page was fetched, and no file outside this
directory was opened. No MuPDF code, documentation or output was consulted.

| Source | Where from | Used for |
|---|---|---|
| ISO 32000-2:2020 (PDF 2.0) §7.2.2–7.2.4 | ISO standard | White space, delimiters, regular characters, comments (`lexer.rs`, `serialize.rs`, `stream.rs`) |
| ISO 32000-2:2020 §7.3.2–7.3.10 | ISO standard | Booleans, numbers, literal and hex strings and their escapes, names and `#hh` escapes, arrays, dictionaries, null, indirect references (`lexer.rs`, `parser.rs`, `serialize.rs`) |
| ISO 32000-2:2020 §7.4.1–7.4.5, Tables 6–12 | ISO standard | Filter chains, ASCIIHexDecode, ASCII85Decode, LZWDecode and its EarlyChange parameter, FlateDecode, the predictor parameters (Predictor, Colors, BitsPerComponent, Columns) and their defaults, RunLengthDecode (`stream.rs`) |
| ISO 32000-2:2020 §7.9.2.2 | ISO standard | Text string encodings: UTF-16BE with a byte order mark, UTF-8 with a mark, PDFDocEncoding (`object.rs`) |
| ISO 32000-2:2020 Annex D, Table D.2 | ISO standard | The PDFDocEncoding codes that differ from Latin-1 (0x18–0x1F, 0x80–0x9E, 0xA0) and the undefined codes (0x7F, 0x9F, 0xAD); `PDF_DOC_SPECIAL` in `object.rs` was written from the table |
| RFC 1950 (zlib) | IETF | Header fields (CM, FCHECK, the FDICT preset-dictionary bit), Adler-32 trailer (`stream.rs`) |
| RFC 1951 (DEFLATE) | IETF | The 32 KiB maximum back-reference distance, which sizes the inflate window (`stream.rs`) |
| TIFF Revision 6.0, Section 14 (Differencing Predictor) | Adobe, public | Predictor 2: horizontal differencing per component, per row (`stream.rs`) |
| PNG specification (W3C / ISO/IEC 15948), §9 Filtering | W3C | Filter types 0–4, the byte distance per pixel, the Average and Paeth predictors and Paeth's tie order (`stream.rs`) |
| IEEE 754-2019 binary64 | IEEE | Shortest round-trip reals, integral range checks (`serialize.rs`, `lexer.rs`) |
| The Unicode Standard, §3.9 (UTF-8, UTF-16) | Unicode Consortium | Well-formed UTF-8 (no overlong forms, no surrogates, nothing above U+10FFFF), surrogate pairs, U+FFFD substitution of maximal ill-formed subparts (`object.rs`) |
| Rust standard library documentation (Rust 1.92) | doc.rust-lang.org (from knowledge) | `f64` `Display` (shortest digits that round-trip, positional, no exponent) and `str::parse::<f64>` (correctly rounded), integer `FromStr`, `str::from_utf8` and `Utf8Error::{valid_up_to, error_len}`, `char::decode_utf16`, `Vec::try_reserve*`, `Arc::make_mut` / `Arc::ptr_eq` |
| `miniz_oxide` 0.8.9 public API (already a dependency) | docs.rs (from knowledge) | `inflate::core::decompress`, `DecompressorOxide`, `inflate_flags::{TINFL_FLAG_PARSE_ZLIB_HEADER, TINFL_FLAG_COMPUTE_ADLER32, TINFL_FLAG_HAS_MORE_INPUT}`, `inflate::TINFLStatus`, and the wrapping-window convention of `decompress`; the crate's source was not opened |

Specs and tests read (all inside this directory): `HANDOFF.md`,
`specs/01-syntax.md`, `specs/02-filters.md`, `specs/OWNER-DECISIONS.md`,
`specs/SPEC-RULES.md`, the acceptance tests in `engine/tests/`, and the
engine's own callers of these modules in `engine/src/` (for the interface).
