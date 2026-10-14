# Independent review: bounded mapped-string decoding

Reviewed `text_decode.rs` and the CLI output path. No blocking finding was identified. The decoder borrows original input, owns validated UTF-8 text and per-code spans, and does not retain the map. Private fields and immutable getters preserve span/text consistency. Checked cumulative limits run before output growth, and geometric allocation avoids repeated prefix copies. Every repeated code charges its scalar and UTF-8 expansion again.

Five independent tests pass:

- Borrowing an interior input subslice preserves pointer identity and source offsets relative to that slice. Returned span copies cannot mutate the stored result.
- Mixed one/two/three/four-byte source codes produce exact complete-mapping UTF-8 spans, including NUL, FEFF, ligatures, combining marks, supplementary scalars, and noncharacters.
- All four limits succeed at exact boundaries and fail at the start of the code that exceeds them.
- Explicit expected UTF-8 byte sequences cover the one/two/three/four-byte encoding boundaries through U+10FFFF.
- Repeated multi-scalar mappings charge every occurrence; missing, invalid, and incomplete codes remain distinct after a valid multibyte prefix and leave input unchanged.

`tools/differential-text-decode.py` passes 106 cases with 6,332 decoded codes and 39,116 UTF-8 bytes. It checks complete UTF-8 bytes and every source/output span against independent Python expectations, then checks code boundaries and Unicode values through the native CMap oracle. All entries from the four valid actual maps are exercised twice. Six malformed text inputs refuse with no stdout. A separate width-collision case verifies Rust's typed mapping while documenting native numeric lookup's limitation. Binary identities and fixture hashes are recorded in `reference/text-decode-differential-r7b.json`.

Targeted tests, Clippy with warnings denied, and formatting checks pass. This API decodes only through an explicitly supplied validated ToUnicode map. It performs no normalization, replacement policy, font/resource lookup, fallback, glyph selection, document text extraction, positioning, or reading-order inference. Spans cover one complete source-code mapping, including any ligature expansion; they are not grapheme or per-scalar provenance.
