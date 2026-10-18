# Spec 01: PDF syntax (lexer, lenient parser, dictionary order, text strings, compact serialization)

Workstream 1 of `docs/mit-clean-room-plan.md`. Written under
`docs/clean-room/SPEC-RULES.md`. Standards: ISO 32000-2:2020 §7.2 (lexical
conventions), §7.3 (objects), §7.9.2 (string types), Annex D (PDFDocEncoding);
IEEE 754-2019 (binary32/binary64).

Black-box observations of MuPDF 1.28.1 come from the npm `mupdf` package
(mupdf.js) driven by scripts the spec writer wrote; they are listed in
Appendix A with their inputs and outputs. They are evidence of what another
viewer does, not requirements, unless this spec says so.

## 0. Owner decisions taken in this spec

Two choices decide whether saved files stay byte-identical with the current
engine's output. Byte-identity with the old output is **not** required by any
user-facing behaviour: the app's own test suites (`tests/*.test.mjs`) look
for operators inside content streams, which other code writes, but never for
serialized object syntax; the view does not depend on serializer output; and the
only exact-byte consumers are the engine tests listed in §6, most of which are
being rewritten anyway. The defaults below are therefore the spec-clean
choices. If the owner wants the old bytes instead, Appendix B gives the exact
observable rule for each, and §6 lists the test changes either way.

- **D1 (dictionary order).** Default: a dictionary keeps insertion order at
  every size. Alternative (Appendix B.1): sort by key once a dictionary
  already holding 101 or more entries receives an insertion.
- **D2 (real numbers in the compact style).** Default: write the shortest
  decimal that reads back as the same binary64 value (no precision loss on a
  load/save cycle). Alternative (Appendix B.2): round to binary32 first and
  write a specific shortest form of that.

Everything else in this spec is the same under either choice.

## 1. Scope

The implementer writes, from this spec only:

| File | Part rebuilt |
|---|---|
| `engine/src/lexer.rs` | whole file |
| `engine/src/parser.rs` | whole file (the strict path is described here so the file can be rebuilt; the lenient path is the part this workstream exists for) |
| `engine/src/object.rs` | `Dictionary` and its iterators; the text-string functions (`encode_text_string`, `decode_text_string`, `decode_text_prefix`, `pdf_doc_byte`). `Object` and `ObjectId` stay as provided. |
| `engine/src/serialize.rs` | whole file (both styles; the canonical style is the engine's own and is specified exactly; the compact style and real formatting are the core of this workstream) |

The interface below is the post-Phase-0 tree (`RustEngine` export, `session*`
and the page/raster modules removed; the content parser no longer suspends and
resumes the lexer). Surviving callers:

- lexer: `content.rs` (content-stream operations and inline images),
  `parser.rs`, `app/view_fixes/{cmap,color,inherit}.rs`, `app/document.rs`
  (limit error mapping), `repair.rs` (one error kind), `main.rs` (`pdf-inspect
  --tokens`).
- parser: `document.rs` (`parse_one` for every indirect object), `repair.rs`
  (lenient parser with `KeepLast`), `app/document.rs`, `app/info.rs`,
  `main.rs`, `cli_encoding.rs`.
- `Dictionary`: everywhere. Ordering affects `writer.rs` (app save),
  `incremental.rs`, `app/overlay.rs`, `repair.rs` output.
  `sorted_entries` is used by `stream_object.rs`, `writer.rs`, `document.rs`.
- text strings: `app/*` (titles, outlines, fields, labels, annotations,
  signatures, links), `security.rs` and `app/password.rs`
  (`pdf_doc_byte`, `decode_text_string` for password conversion),
  `view/stext.rs`.
- serializer: `writer.rs`, `incremental.rs`, `app/overlay.rs`, `repair.rs`
  (the crate-internal `Writer`), `app/view_cost.rs` and `main.rs`
  (`serialize_object`), `app/annots/{appearance,view}.rs` (`compact_real` for
  content-stream numbers; see §3.7.4 note).

No `unsafe`. No panics on any input (indexing must be checked or provably in
range). New dependencies only with a permissive licence and owner approval.

## 2. Interface

Derives listed are required (callers or tests use them).

### 2.1 `lexer`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexLimits { pub max_token_bytes: usize, pub max_tokens: usize }
impl Default for LexLimits  // max_token_bytes = 1_048_576, max_tokens = 1_000_000

#[derive(Debug, Clone, PartialEq)]
pub struct Token { pub kind: TokenKind, pub span: std::ops::Range<usize> }

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Integer(i64), Real(f64), Name(Vec<u8>), String(Vec<u8>),
    ArrayStart, ArrayEnd, DictStart, DictEnd, Keyword(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError { pub offset: usize, pub kind: LexErrorKind }   // + Display, std::error::Error

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LexErrorKind {
    TokenTooLong { limit: usize }, TooManyTokens { limit: usize },
    UnexpectedByte(u8), InvalidNameEscape, NullInName,
    UnterminatedString, UnterminatedHexString, InvalidHexDigit(u8),
    InvalidNumber, NumberOutOfRange, AllocationFailed,
}

#[derive(Debug)]
pub struct Lexer<'a> { /* private */ }
impl<'a> Lexer<'a> {
    pub fn new(input: &'a [u8], limits: LexLimits) -> Self;
    pub fn with_lenient(self) -> Self;
    pub fn position(&self) -> usize;
    pub fn next_token(&mut self) -> Result<Option<Token>, LexError>;
    pub(crate) fn input(&self) -> &'a [u8];
    pub(crate) fn seek(&mut self, position: usize);
}
```

Semantics:

- `next_token` skips white space and comments, then returns the next token,
  `Ok(None)` at end of input (repeatedly), or an error. `Token::span` is the
  exact byte range of the token in `input`, delimiters included (`<<` is two
  bytes, `(a)` three, `/A` two). Trailing white space is **not** consumed:
  after a token, `position()` equals `span.end`. After `Ok(None)`,
  `position()` equals `input.len()`.
- Errors are sticky: once `next_token` returns an error, every later call
  returns an equal error and `position()` no longer changes.
- `with_lenient` switches on the tolerant reading of §4.1. It is meant to be
  called before the first token.
- `input` returns the whole input slice. `seek(p)` moves the read position to
  `min(p, input.len())`; the content parser uses it to resume after inline
  image data it consumed itself. Seeking does not reset the token count or a
  stored failure.
- `LexError` `Display`: `PDF lexical error at byte {offset}: {message}` with
  messages: `token exceeds {limit} encoded bytes`, `token count exceeds
  {limit}`, `unexpected byte 0x{b:02x}`, `name escape requires two hex
  digits`, `a PDF name cannot contain a null byte`, `unterminated literal
  string`, `unterminated hexadecimal string`, `invalid hex digit 0x{b:02x}`,
  `invalid PDF number`, `number is outside the supported range`, `token
  allocation failed`.

### 2.2 `parser`

```rust
#[derive(Clone, Copy, Debug)]
pub struct ParseLimits { pub max_depth: usize, pub max_objects: usize,
                         pub max_decoded_bytes: usize, pub lex: LexLimits }
impl Default for ParseLimits  // max_depth 64, max_objects 100_000, max_decoded_bytes 16 MiB, lex default

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DuplicateKeyPolicy { #[default] Reject, KeepLast }

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedObject { pub object: Object, pub span: std::ops::Range<usize> }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseLimitKind { Depth, Objects, DecodedBytes }

#[derive(Debug)]
pub enum ParseErrorKind {
    Lex(LexError), EmptyInput,
    UnexpectedEnd { expected: &'static str },
    UnexpectedToken { expected: &'static str, found: &'static str },
    UnknownKeyword(Vec<u8>), Unsupported(&'static str),
    InvalidReference { number: i64, generation: i64 },
    DuplicateKey(Vec<u8>), LimitExceeded(ParseLimitKind), TrailingData,
}
#[derive(Debug)]
pub struct ParseError { pub offset: usize, pub kind: ParseErrorKind }
// + Display, std::error::Error (source() is the LexError for Lex)

pub struct Parser<'a> { /* private */ }
impl<'a> Parser<'a> {
    pub fn new(input: &'a [u8], limits: ParseLimits) -> Self;
    pub fn with_duplicate_key_policy(self, policy: DuplicateKeyPolicy) -> Self;
    pub fn with_lenient(self) -> Self;
    pub fn parse_next(&mut self) -> Result<Option<ParsedObject>, ParseError>;
    pub fn parse_complete(self) -> Result<Object, ParseError>;
}
pub fn parse_one(input: &[u8], limits: ParseLimits) -> Result<ParsedObject, ParseError>;
pub fn parse_complete(input: &[u8], limits: ParseLimits) -> Result<Object, ParseError>;
```

Semantics:

- `parse_next` returns the next direct object, `Ok(None)` at end of input,
  or an error. `span` runs from the first byte of the object's first token to
  the last byte of its last token; it excludes trailing white space and any
  token read ahead (to detect `N G R`) that turned out not to belong to it.
  `parse_one` is `parse_next` on a fresh parser, with `EmptyInput` (offset
  `input.len()`) instead of `None`. `parse_complete` additionally requires
  that nothing but white space and comments follows (`TrailingData` at the
  first extra token's offset; `EmptyInput` for blank input).
- A parser is terminal after any error: further `parse_next` calls fail.
- A lexical error after a complete object is deferred: `parse_one(b"1
  (unterminated")` succeeds with `Integer(1)`; the error surfaces on the next
  call (`ParseErrorKind::Lex`).
- Limits are cumulative over the parser's life. Every value, container and
  dictionary key counts one toward `max_objects`. Names, strings and keys
  count their decoded length toward `max_decoded_bytes`, including values
  later overwritten under `KeepLast`. `max_depth` is the nesting depth of
  arrays/dictionaries (a scalar at the top needs depth 0, `[]` needs 1); it
  is capped at 256 whatever the caller asks.
- `ParseError` `Display`: `PDF object error at byte {offset}: ` then: the
  lexical error's text; `expected an object`; `unexpected end; expected
  {expected}`; `expected {expected}, found {found}`; `unknown object keyword
  {bytes:?}`; `unsupported {feature}`; `invalid indirect reference {n} {g} R`;
  `duplicate dictionary key {bytes:?}`; `{kind:?} limit exceeded`; `trailing
  data after object`. `found` labels: `integer`, `real`, `name`, `string`,
  `[`, `]`, `<<`, `>>`, `keyword`.

### 2.3 `object`

`Object` (`Null, Boolean, Integer(i64), Real(f64), Name(Vec<u8>),
String(Vec<u8>), Array(Vec<Object>), Dictionary(Dictionary),
Reference(ObjectId)`) and `ObjectId` are unchanged. `Object::text(&str)`
returns `Object::String(encode_text_string(text))`. On 64-bit targets
`size_of::<Object>()` must stay 32 bytes (a test asserts it), so `Dictionary`
must fit in 24 bytes.

```rust
#[derive(Clone)]
pub struct Dictionary { /* private */ }
impl Dictionary {
    pub const fn new() -> Self;
    pub fn with_capacity(capacity: usize) -> Self;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn clear(&mut self);
    pub fn contains_key<Q>(&self, key: &Q) -> bool where Vec<u8>: Borrow<Q>, Q: Ord + ?Sized;
    pub fn get<Q>(&self, key: &Q) -> Option<&Object> where /* same bounds */;
    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&Vec<u8>, &Object)> where /* … */;
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut Object> where /* … */;
    pub fn insert(&mut self, key: Vec<u8>, value: Object) -> Option<Object>;
    pub fn remove<Q>(&mut self, key: &Q) -> Option<Object> where /* … */;
    pub fn remove_entry<Q>(&mut self, key: &Q) -> Option<(Vec<u8>, Object)> where /* … */;
    pub fn retain<F: FnMut(&Vec<u8>, &mut Object) -> bool>(&mut self, keep: F);
    pub fn iter(&self) -> Iter<'_>;          pub fn iter_mut(&mut self) -> IterMut<'_>;
    pub fn keys(&self) -> Keys<'_>;          pub fn values(&self) -> Values<'_>;
    pub fn values_mut(&mut self) -> ValuesMut<'_>;
    pub fn into_keys(self) -> IntoKeys;      pub fn into_values(self) -> IntoValues;
    pub fn sorted_entries(&self) -> Vec<(&Vec<u8>, &Object)>;
    pub fn shares_entries(&self, other: &Self) -> bool;
    // D1 default: `is_sorted` is removed. Under the Appendix B.1 alternative it stays:
    // pub fn is_sorted(&self) -> bool;
}
impl Default, PartialEq, Debug, Extend<(Vec<u8>, Object)>, FromIterator<(Vec<u8>, Object)>,
     From<[(Vec<u8>, Object); N]>, From<BTreeMap<Vec<u8>, Object>>,
     IntoIterator (for Dictionary, &Dictionary, &mut Dictionary),
     std::ops::Index<&Q> (Output = Object)
```

Iterator types `Iter<'a>` (item `(&'a Vec<u8>, &'a Object)`, `Clone`),
`IterMut<'a>` (`(&'a Vec<u8>, &'a mut Object)`), `Keys<'a>` (`&'a Vec<u8>`,
`Clone`), `Values<'a>` (`&'a Object`, `Clone`), `ValuesMut<'a>`, `IntoIter`
(`(Vec<u8>, Object)`), `IntoKeys`, `IntoValues`: each is `pub`, `Debug`,
`Iterator`, `DoubleEndedIterator`, `ExactSizeIterator`, `FusedIterator`, and
yields entries in the dictionary's order (§3.5).

Semantics:

- `insert` returns the replaced value. A key already present keeps its
  position; a new key goes last. `remove`, `remove_entry` and `retain` keep
  the remaining entries in order. `clear` empties. `get_mut`, `iter_mut` and
  `values_mut` change values in place, never order.
- `Index` returns `&Object::Null` for an absent key (never panics).
- Equality ignores order: equal when both hold the same key set with equal
  values.
- `Debug` prints as a map in order, keys as byte lists:
  `{[65]: Name([65]), [77]: Name([77])}`.
- `FromIterator`, `Extend`, `From<[_; N]>` insert in iteration order (a
  repeated key keeps its first position and its last value). `From<BTreeMap>`
  gives key order.
- `sorted_entries` returns the entries in ascending byte order of keys
  (lexicographic, shorter prefix first).
- Cloning is O(1) and shares storage until either side is written (copy on
  write); a write copies only the written dictionary, shallowly, so nested
  dictionaries an edit does not reach stay shared. `shares_entries(other)`
  is true when `other` is an unwritten clone of `self` (or vice versa) and
  the dictionary is non-empty; it may return false whenever sharing cannot
  be proven, but must return true in that clone case and false after either
  side has been written through `insert`, `remove*`, `retain`, `clear`,
  `get_mut` of a present key, `iter_mut` or `values_mut`. `get_mut` of an
  absent key does not count as a write.
- Performance: lookup, insertion, and removal must keep building, parsing,
  comparing, printing and pruning a 200,000-key dictionary well inside the
  existing test's 60-second bound (O(n log n) overall, never quadratic).

Text strings (§3.6):

```rust
pub fn encode_text_string(text: &str) -> Vec<u8>;
pub fn decode_text_string(bytes: &[u8]) -> String;
pub fn decode_text_prefix(bytes: &[u8], max: usize) -> (String, bool);
pub(crate) fn pdf_doc_byte(c: char) -> Option<u8>;
```

### 2.4 `serialize`

```rust
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SerializeStyle { #[default] Canonical, Compact }

#[derive(Clone, Copy, Debug)]
pub struct WriteLimits { pub max_output_bytes: usize, pub max_depth: usize, pub max_objects: usize }
impl Default for WriteLimits  // 16 MiB, 64, 100_000

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SerializeError { OutputLimit, DepthLimit, ObjectLimit, NullInName, NonfiniteReal, AllocationFailed }
// + Display, std::error::Error

pub fn serialize_object(value: &Object, limits: WriteLimits) -> Result<Vec<u8>, SerializeError>; // Canonical
pub fn serialize_object_with_style(value: &Object, limits: WriteLimits, style: SerializeStyle)
    -> Result<Vec<u8>, SerializeError>;
pub(crate) fn compact_real(value: f64) -> String;

pub(crate) struct Writer { /* private */ }
impl Writer {
    pub(crate) fn with_style(limits: WriteLimits, style: SerializeStyle) -> Self;
    pub(crate) fn keep_order(&mut self);                       // canonical dictionaries in their own order
    pub(crate) fn null_references(&mut self, ids: BTreeSet<ObjectId>); // print these references as null
    pub(crate) fn len(&self) -> usize;                         // bytes written so far
    pub(crate) fn finish(self) -> Vec<u8>;
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<(), SerializeError>; // raw bytes, counted
    pub(crate) fn value(&mut self, value: &Object, depth: usize) -> Result<(), SerializeError>;
    pub(crate) fn dictionary_entries<'a>(&mut self,
        entries: impl IntoIterator<Item = (&'a [u8], &'a Object)>) -> Result<(), SerializeError>;
    pub(crate) fn stream_dictionary(&mut self, values: &Dictionary, length: usize)
        -> Result<(), SerializeError>;
}
```

`SerializeError` `Display` texts: `PDF serialization output limit exceeded`,
`… depth limit exceeded`, `… object limit exceeded`, `PDF names cannot contain
null bytes`, `PDF real numbers must be finite`, `PDF serialization allocation
failed`.

## 3. Required behaviour

### 3.1 Character classes (ISO 32000-2 §7.2.3, Tables 1 and 2)

- White space: NUL (0x00), HT (0x09), LF (0x0A), FF (0x0C), CR (0x0D), SP
  (0x20). Nothing else; VT (0x0B) is a regular character.
- Delimiters: `( ) < > [ ] { } / %`.
- Regular: every other byte.
- A comment starts at `%` outside a string and runs to, not including, the
  next CR or LF (or end of input). Comments and white space separate tokens
  and are otherwise ignored.

### 3.2 Tokens, strict mode

A token starts at the first byte that is not white space or comment.

| First byte(s) | Token | Rule |
|---|---|---|
| `/` | Name | Following regular bytes up to a delimiter or white space. `#hh` (two hex digits, either case) stands for byte `hh` (§7.3.5). A `#` not followed by two hex digits is `InvalidNameEscape` at the `#`. `#00` is `NullInName` at the `#`. Any other byte (including 0x80–0xFF and controls) is kept as is. `/` alone is the empty name. |
| `(` | String | §7.3.4.2. Balanced unescaped parentheses are kept. Escapes: `\n \r \t \b \f \( \) \\`; `\ddd` one to three octal digits, value taken modulo 256 (`\777` is 0xFF, `\400` is 0x00, `\378` is 0x1F then `8`); backslash before any other byte is dropped (`\q` is `q`); backslash + EOL (CR, LF or CR LF) is a line continuation and produces nothing. An unescaped CR, LF or CR LF inside the string reads as one LF. End of input before the closing `)` (including directly after a `\`) is `UnterminatedString` at end of input. |
| `<<` | DictStart | |
| `<` | String (hex) | §7.3.4.3. Hex digits of either case; white space ignored; an odd final digit is followed by an implied 0. Any other byte is `InvalidHexDigit(b)` at that byte. No `>` before end of input is `UnterminatedHexString`. |
| `>>` | DictEnd | |
| `[` `]` | ArrayStart / ArrayEnd | |
| `)` `>` `{` `}` | — | `UnexpectedByte(b)` at that byte. |
| other | number or keyword | The maximal run of regular bytes. If it starts with `+ - .` or a digit it must be a number (below); otherwise it is a Keyword, every byte of which must be in 0x21–0x7E (else `UnexpectedByte(b)` at the first offending byte). |

Numbers (§7.3.3): optional sign, then digits with at most one `.`, with at
least one digit somewhere (`1.`, `.5`, `-.5`, `+.5`, `-0.` are valid). Any
other shape (`+`, `-`, `.`, `--1`, `1.2.3`, `1e3`, `1-2`, `123abc`) is
`InvalidNumber`. Without a point the value is an `Integer` when it fits i64;
otherwise it is the `Real` nearest to the decimal value if that is finite
(`9223372036854775808` reads as Real 9223372036854775808.0, never wrapped),
else `NumberOutOfRange`. With a point the value is the correctly rounded
binary64 `Real` (`16777217.0` stays 16777217.0; `-0.` keeps its negative
sign); an infinite result is `NumberOutOfRange`.

Keywords are returned as bytes; the lexer gives no meaning to `true`,
`false`, `null`, `R`, `obj` etc.

### 3.3 Lexer limits and error offsets

- `max_token_bytes` bounds the **encoded** length of one token, delimiters
  and escapes included, whether or not the bytes produce output (`<    >`
  and `(\<LF>\<LF>)` count every byte). Reading the byte that would make the
  token longer than the limit fails with `TokenTooLong { limit }` at that
  byte's offset; a token of exactly `limit` bytes succeeds.
- `max_tokens` bounds the number of tokens returned. Requesting one more
  fails with `TooManyTokens { limit }` at the start of the token that would
  exceed it; trailing white space, comments and end of input never fail (a
  zero limit still returns `Ok(None)` for blank input).
- Allocation failure while growing token storage is `AllocationFailed`
  (use fallible reservation; never abort).
- Offsets: the `#` for name escape errors; the offending byte for
  `UnexpectedByte`/`InvalidHexDigit`/`InvalidNumber` (or the token start
  when the number has no digit); end of input for unterminated strings.
- Nesting of literal-string parentheses must not use recursion (a string of
  100,000 nested parentheses must work).

### 3.4 Parser, strict mode

- `null`, `true`, `false` keywords are values. `N G R` (two Integers and the
  keyword `R`) is a `Reference` when `1 ≤ N ≤ 4294967295` and `0 ≤ G ≤
  65535`; otherwise `InvalidReference` at the first integer. Lookahead for
  `R` never loses ordinary numbers: `[1 2 3 R 4 5 true 6]` is
  `[1, 2 3 R, 4, 5, true, 6]`.
- `stream`, `endstream` → `Unsupported("stream syntax")`; `obj`, `endobj` →
  `Unsupported("indirect-object wrapper syntax")`; `xref`, `trailer`,
  `startxref` → `Unsupported("document syntax")`; any other keyword where a
  value is expected → `UnknownKeyword`.
- Arrays `[ … ]`; dictionaries `<< /Key value … >>` (§7.3.6–7.3.7). A key
  that is not a Name, a `]`/`>>` where a value is expected, or a missing
  value is `UnexpectedToken` (at that token) or `UnexpectedEnd` (at end of
  input). Duplicate keys (compared after `#` decoding) are `DuplicateKey` at
  the second key under `Reject`; under `KeepLast` the later value replaces
  the earlier one in the earlier position.
- Recursion depth is bounded by `max_depth` (≤ 256), so stack use is bounded.

### 3.5 Dictionary order (D1)

A dictionary's order is the order in which its keys were first inserted:
source order for a parsed dictionary (a duplicate under `KeepLast` keeps the
first position, e.g. `<</B 1/A 2/B 3/C 4/A 5>>` holds B 3, A 5, C 4 in that
order; MuPDF was observed to do the same, E-PARSE), call order for a built
one. Replacing a value keeps the position; removing a key keeps the order of
the rest; a removed key inserted again goes last. This holds at every size;
there is no size at which the order changes.

Observed difference, kept deliberately: MuPDF moves the last entry into the
hole left by a deletion (deleting `/AP` from
`…/Rect[0 0 10 10]/AP<<>>/F 4/P 7` then putting it back printed
`…/Rect[0 0 10 10]/P 7/F 4/AP<<>>`, E-DICT). This engine prints
`…/Rect[0 0 10 10]/F 4/P 7/AP<<>>`: the user's other keys do not move.

### 3.6 Text strings (§7.9.2.2, Annex D)

`encode_text_string(text)`:
- If every character is ASCII (U+0000–U+007F) and none is U+0018–U+001F,
  the result is the UTF-8 bytes of `text` (identical to its ASCII bytes).
- Otherwise the result is FE FF followed by the UTF-16BE code units of
  `text` (surrogate pairs for characters above U+FFFF). U+0018–U+001F force
  this form because PDFDocEncoding reads those codes as accents.
- So `decode_text_string(&encode_text_string(t)) == t` for every `t`.
  (mupdf.js gave the same bytes for `café` → `<FEFF00630061006600E9>`,
  `abc` → `(abc)`, `中` → `<FEFF4E2D>`, E-SER; for `\u{18}x` it wrote raw
  `<1878>`, which reads back as `˘x`; this engine deliberately differs.)

`decode_text_string(bytes)`:
1. Starts with FE FF: UTF-16BE of the rest. An odd trailing byte is
   ignored. Unpaired surrogates become U+FFFD (one per unpaired unit).
2. Starts with FF FE: UTF-16LE of the rest, same rules. (Not in the
   standard; MuPDF reads it the same way, E-TEXT. Accepting it costs nothing.)
3. Starts with EF BB BF: UTF-8 of the rest (§7.9.2.2, PDF 2.0). Invalid
   sequences become U+FFFD (standard lossy UTF-8 decoding: each maximal
   invalid subpart is one U+FFFD).
4. No mark, and the bytes are valid UTF-8 (strict: no overlong forms, no
   encoded surrogates, nothing above U+10FFFF), contain at least one byte ≥
   0x80, and contain no byte in 0x18–0x1F: UTF-8. (Tolerance for producers
   that write UTF-8 without a mark; MuPDF does the same for such strings,
   E-TEXT `unmarked-utf8`, and falls back to PDFDocEncoding when a byte
   0x18–0x1F is present, E-TEXT `unmarked-utf8-with-18`.)
5. Otherwise PDFDocEncoding, one character per byte, per Annex D Table D.2.
   Codes the table leaves undefined (0x7F, 0x9F, 0xAD) and the control codes
   0x00–0x17 decode to the Unicode code point with the same number (their
   Latin-1 value). The table must be regenerated from Annex D, not copied
   from any program.

`decode_text_prefix(bytes, max)`: decode at most the first `max` bytes,
returning `(text, bytes.len() > max)`. The encoding is chosen from the
**whole** string (rules 1–5 applied to all of `bytes`), and then only the
prefix is decoded. When the cut splits a character (half a UTF-16 unit, a
high surrogate without its low half, an incomplete UTF-8 sequence), that
partial character is left out rather than turned into U+FFFD. A complete but
invalid sequence before the cut still becomes U+FFFD. PDFDocEncoding cuts
are always whole characters.

`pdf_doc_byte(c)`: the PDFDocEncoding code of `c` per Annex D Table D.2, or
`None` when the table has no code for it. Concretely: the 40 characters the
table maps to codes 0x18–0x1F, 0x80–0x9E and 0xA0 give those codes;
U+0000–U+0017, U+0020–U+007E and U+00A1–U+00FF except U+00AD give their own
value; everything else (U+0018–U+001F, U+007F–U+00A0, U+00AD, and other
characters the table lacks) gives `None`. (The existing measured file
`reference/app-encrypt-20261014/docenc-passwords.json` agrees and stays as
the acceptance data, §6.)

### 3.7 Serialization

Both styles share: names are written `/` then each byte as itself if it is
in 0x21–0x7E and is not one of `( ) < > [ ] { } / % #`, else as `#` and two
**uppercase** hex digits (§7.3.5); a name containing byte 0 fails with
`NullInName`. References are `N G R`, or `null` when the writer's
`null_references` set holds them. Integers are plain decimal. `true`,
`false`, `null`. A non-finite real fails with `NonfiniteReal`. No output is
returned on any failure.

Limits (both styles, all entry points): every value, every container and
every dictionary key (including a `/Length` the writer adds) counts one
toward `max_objects`; the total bytes written by the `Writer` (including
`append`) may not exceed `max_output_bytes` (`OutputLimit`; an exact fit
succeeds; a zero limit fails even for `null`). Depth: `value(v, d)` fails
with `DepthLimit` when it must open an array or dictionary at depth `d ≥
min(max_depth, 256)`; children are written at `d + 1`. `serialize_object*`
start at depth 0. `dictionary_entries` and `stream_dictionary` fail when
`max_depth == 0` and write values at depth 1. A chain of 256 nested arrays
writes with `max_depth = usize::MAX`; 257 fails.

#### 3.7.1 Canonical style (the engine's own format; exact)

- Arrays: `[` items separated by one space `]` (`[]` when empty).
- Dictionaries: `<<`, then for each entry a SP, the key, a SP and the value,
  then ` >>` (so the empty dictionary is `<< >>`). Entries in
  `sorted_entries` order, or in the dictionary's own order after
  `keep_order()`.
- Strings: always hex, uppercase, `<00FF>`.
- Reals: the shortest decimal digit string that reads back as the same
  binary64, in positional notation (no exponent; leading `0` before the
  point for |v| < 1; `-` for negative values including −0), with `.0`
  appended if the result has no point. Examples: `-0.0`, `2.0`, `0.1`,
  `1000…0.0` for 1e100.
- `stream_dictionary(values, length)`: entries in key order with a
  `/Length length` entry in its key-order place, replacing any `/Length`
  present; after `keep_order()`, as for the compact style below.

Example: `<< /A#20#2F#23 [-4 true 2 3 R] /Z <00FF> >>`.

#### 3.7.2 Compact style: token separation

The compact style writes no white space except one SP where two adjacent
tokens would otherwise merge. A SP is written before a token that begins
with a regular character (a number, `true`, `false`, `null`, `N G R`) when
the previous byte written is a regular character or ends an empty name
(`/` as the last byte). Names, strings, arrays and dictionaries begin with a
delimiter and never take a SP. Apart from the empty-name case, the first
token written by a top-level `value` or `dictionary_entries` call takes no SP
(the caller writes its own separators, e.g. `1 0 obj\n`).

Examples (exact): `[/FT/Sig/T(Signature1)/Rect[0 0 0 0]]`;
`[[1/X](q)(q)<</A null/B<<>>>>1 0 R 1 true false null[]]`;
`<</ 5>>` (empty key); `[// 3]`.

MuPDF's printer was observed to write `<</5>>` and `[//3]` for these
(E-SER), which read back as different objects; the SP after an empty name is
required here.

#### 3.7.3 Compact style: dictionaries

- `<<`, entries, `>>`, keys immediately followed by their values.
- Order: the dictionary's order (§3.5), except that if the dictionary has a
  `/Type` entry whose value is not null, that entry is written first, and if
  it also has a `/Subtype` entry whose value is not null, that entry is
  written second. A reference in the writer's `null_references` set counts
  as null for this test. The other entries keep their relative order.
  Observed in MuPDF saves (E-DICT, all ten cases match this rule, including
  `<</X 1/Subtype null/Type/T>>` → `<</Type/T/X 1/Subtype null>>`,
  `<</Z 1/Subtype 1/A 1>>` unchanged, `<</X 1/Subtype/S/Type null>>`
  unchanged, `<</X 1/Type 5/Subtype/S>>` → `<</Type 5/Subtype/S/X 1>>`).
  It is kept because it makes saved objects easier to read and keeps most
  existing expectations; it has no semantic effect.
- `dictionary_entries(entries)` writes the given entries in the given order
  (compact: with the /Type,/Subtype rule; canonical: exactly as given).
- `stream_dictionary(values, length)` (compact style, and canonical after
  `keep_order()`): the dictionary's entries, with the value of an existing
  `/Length` replaced in place by `length`, or `/Length length` added last
  when absent. MuPDF's save does the same for both cases (E-DICT-2).

#### 3.7.4 Compact style: strings

The literal form is `(`, each byte as `\n \r \t \b \f \( \) \\` for those
bytes, `\ddd` (three octal digits) for other bytes below 0x20 or at/above
0x7F, itself otherwise, `)`. The hex form is uppercase `<…>`, two digits per
byte. Write the hex form if the string starts with FE FF or FF FE (a
Unicode text string stays recognisable), or if the literal form would be
strictly longer than the hex form; otherwise write the literal form (ties
go to the literal).

Observed to match MuPDF's save output on 26 strings (E-SER-SAVE), for
example `(line\nbreak\r\t\b\f)`, `<00016162>`, `(ab\000)`, `(abcd\377)`,
`(a\177b)`, `<FFFE4142434445464748>`, `(\357\273\277ABCDEFGHI)`,
`(ab\001)`, `<6101>`, `<FF>`, `(\(\(\))`.

#### 3.7.5 Compact style: reals (D2) and `compact_real`

`compact_real(v)` and the compact real token are the same string. For finite
`v`:
1. If `v` is integral and −2^63 ≤ v < 2^63: the integer's decimal digits
   (−0.0 → `0`). This reads back as an Integer (§7.3.3 lets an integer stand
   where a real is expected).
2. Else let D be the shortest decimal digit string that reads back as `v`
   under correctly rounded binary64 conversion (when two shortest strings
   qualify, the one nearer `v`). Write it in positional notation, no
   exponent, no trailing zeros after the point, and **no** `0` before the
   point when |v| < 1 (`.5`, `-.5`, `.0001`). If `v` is integral (only
   possible at |v| ≥ 2^63) write its digits followed by `.` so it reads back
   as a real.
3. `compact_real` of NaN returns `0`; of ±infinity, the string for
   ±(largest finite binary64). Serialization of non-finite values still
   fails with `NonfiniteReal`.

Result: compact output reads back to exactly the value written; a no-change
load and save never alters a real. Examples: `.5`, `-.5`, `.1`, `612`, `0`
(for −0.0), `.0000001`, `123456.789`, `72.12345678`, `15000000000`,
`.3333333333333333`, `3.141592653589793`.

Note for workstreams 6 and 7: appearance-stream code that formats computed
coordinates with `compact_real` will see up to 17 significant digits for
values such as 0.1 + 0.2. Those writers should round to the precision they
need before formatting; this spec does not round.

## 4. Tolerant behaviour (lenient mode)

The repair path (`repair.rs`, a separate workstream) reads damaged objects
with `Parser::new(..).with_duplicate_key_policy(KeepLast).with_lenient()`.
The goal is to keep as much of a damaged dictionary as the reference viewer
keeps, so a repaired file shows the same pages, annotations and forms.
Every rule below was re-observed with mupdf.js on inputs built for this spec
(Appendix A, E-LEX and E-PARSE: each input was placed as object 4 of a
well-formed file and read back). Where this engine differs, the reason is
given.

### 4.1 Lenient lexer

1. Names: a `#` not followed by two hex digits stands for itself:
   `/A#G0` → bytes `A#G0`, `/A#0` → `A#0`. `#00` also stands for itself:
   `/A#00B` → `A#00B` (observed; failing the whole object instead would
   lose more). Other escapes decode as in strict mode.
2. Hex strings: a byte that is neither a hex digit nor white space counts as
   the digit 0: `<ABQD>` → AB 0D, `<A B Q>` → AB 00, `<AB\xA5>` → AB 00,
   `<4 1\x0B>` → 41 00. An unterminated hex string is still an error.
3. Keywords may contain any regular byte (`\xA5` alone is a one-byte
   keyword). A stray `)`, `>`, `{` or `}` is a one-byte keyword.
4. Numbers. A token that starts with `+`, `-`, `.` or a digit and is valid in
   strict mode reads as in strict mode. Otherwise:
   - It is a **keyword** (which the lenient parser turns into null inside a
     container) if, ignoring the first byte and, when the first byte is `-`,
     any run of `-` directly after it, it contains a byte other than a
     digit, `.` or `-`, or contains `.` twice (counting a leading `.`).
     Observed: `1e5`, `1.0E-4`, `1.2.3`, `1..5`, `.5.`, `-+5`, `12abc`,
     `0x10`, `7g2`, `1,5`, `0_612`, `3.40282347e38`, `5+`, `1.5.`, `++5`,
     `-5.5.5` all read as null.
   - Otherwise it is a number read as follows: a leading run of `-` makes it
     negative; then `+` signs are skipped; then the digits up to the first
     non-digit give the integer part (none → 0). If the token contains a `.`
     it is a Real: if the integer part is directly followed by `.`, the
     digits directly after that are the fraction; anything after is
     ignored. Without a `.` it is an Integer. Observed: `-.5`→−0.5, `+.5`→0.5,
     `5.`→5.0, `.`→0.0, `-.`→0.0, `0.-5`→0.0, `1.5-3`→1.5, `1-.5`→1.0 (Real);
     `.-5`→0.0, `-.-5`→0.0 (Real); `-`→0, `+`→0, `--`→0, `--5`→−5, `+-5`→0,
     `5-`→5, `-5-`→−5, `-1-2-3`→−1 (Integer).
     A negative zero real reads as +0.0.
   - The two rules above are the spec writer's summary of the observed
     cases. What is normative is that every observed case listed here and
     in Appendix A reads as stated, plus the magnitude rules below. For
     shapes not listed, any reading consistent with those cases is
     acceptable.
   - Magnitudes (this engine differs from MuPDF on purpose): MuPDF was
     observed to wrap integers past the i64 range (`18446744073709551617-` →
     1, `--99999999999999999999` → −7766279631452241919, and even the valid
     `1000000000000000000000000000000` → 5076944270305263616) and to drop
     the integer part of a real past 2^32 (`4294967296.5` → .5). Here an
     integer whose digits exceed i64 is the Real those digits name (negative
     when signed so), `-9223372036854775808` stays the Integer i64::MIN, a
     real's integer part is read whole, and values beyond binary64 become
     ±(largest finite binary64). Wrapping would silently save a different
     number. Fraction digits beyond binary64 precision may be ignored; a
     token of 5,000 fraction digits must not overflow or take quadratic time.

### 4.2 Lenient parser

Inside an array or a dictionary value position (depth ≥ 1):

1. Any keyword other than `true`, `false`, `null` (an unknown word, a
   damaged number, `R`, `obj`, `endobj`, `stream`, `xref`, a stray `)` `{`
   `}` `>`) is null. Observed: `[1 bogus 3 true1 nul]` → `[1 null 3 null
   null]`; `[1 { 2 } ) 3]` → `[1 null 2 null null 3]`; `[1 0 obj 2]` → `[1 0
   null 2]`; `<</A endobj /B stream /C xref /D R /E nul /F true1>>` → all
   six null.
2. A stray `]` where a dictionary value belongs, or `>>` inside an array, is
   null: `<</MediaB792] /A [1 >> 2] /B <</X ]>> /C 1 ]>>` →
   `<</MediaB792 null/A[1 null 2]/B<</X null>>/C null>>`.
3. A reference to an impossible object (number 0, negative, beyond u32;
   generation beyond u16) is null: `/D 0 0 R` and `[0 0 R]` → null. (MuPDF
   keeps a reference to object 0, which resolves to nothing; this engine's
   `ObjectId` cannot hold 0, and null is what a reader gets either way.)
4. A dictionary value that begins with an Integer N reads as:
   - N, when the next token is a Name, `>>`, or there is none;
   - the reference `N G R` when the next tokens are an Integer G and `R`;
   - otherwise null, and the token after N is consumed with it; if that
     token was an Integer, the token after it is consumed too.
   Observed: `<</A 1 bogus /B 3>>` → A null, B 3; `<</A 1 2 bogus /B 3>>` →
   A null, B 3; `<</A 1 2 3 /B 4>>` → A null, B 4; `<</A 1 (s) /B 4>>` → A
   null, B 4; `<</A 5 0 R /B 1 2.5 /C 3>>` → A ref, B null, C 3;
   `<</A 1 0 R1 /B 2 /C 1 } /D 2>>` → A null, B 2, C null, D 2;
   `<</X 4 0\xA5R …>>` → X null. Because of the consumption rule some inputs
   then fail as a whole: `<</A 1 [2] /B 4>>` and `<</A 1 2>>` fail (MuPDF
   also failed both).
   In an array, integers are read as in strict mode: `[1 4 0\xA5R 3]` →
   `[1 4 null 3]`.
5. Still errors (the whole value fails), as observed in MuPDF: a dictionary
   key that is not a name (`<</A 1 2 /B 3>>`, `<</A (x) (y) /B 2>>`, `<</A
   {1} /B 2>>`, `<</A ]] /B 2>>`), a top-level value that is an unknown
   keyword or damaged number (`bogus`, `1e5`), an unterminated hex or
   literal string.
6. More lenient than MuPDF on purpose: `[R]` and `[1 R 2]` read as
   `[null]` and `[1 null 2]`; MuPDF failed both objects entirely (E-LEX),
   losing every other entry of the containing object.

Lenient mode never changes how valid input reads.

## 5. Limits and safety

- All limits in §2–§3 are enforced before allocation grows past them; every
  allocation that depends on input size uses fallible reservation and maps
  failure to the `AllocationFailed` kind of that module.
- No recursion proportional to input except container nesting, which is
  capped at 256 in the parser and serializer.
- The lexer scans each input byte a bounded number of times (white space and
  comments once).
- No `unsafe`, no panics (fuzzed by `review.rs`, `lexer.rs`
  `every_byte_and_deterministic_hostile_inputs…`, `m10_fuzz.rs`).

## 6. Acceptance

Counts are `#[test]` functions. "Scrub" means the test stays but its name or
comments cite MuPDF internals or design notes and must be reworded before the
implementer sees it (they cite function names such as the dictionary put and
delete routines, the float printer, or `engine/design/*.md`).

### 6.1 Keep (black-box, unchanged under the defaults)

| File | Tests | Notes |
|---|---|---|
| `tests/lexer.rs` | 22 of 22 | Scrub `lenient_numbers_read_as_mupdf_reads_them` (rename, drop the design-note reference and "MuPDF wraps" comments). Add the `#00` case of §4.1.1 (new). |
| `tests/parser.rs` | 16 of 16 | Scrub `lenient_values_read_as_mupdf_reads_them` (rename; drop the design-note path). |
| `tests/review.rs` | 8 of 8 | parser/lexer review cases. |
| `tests/number_overflow.rs` | 3 of 3 | Scrub the module comment's MuPDF reference (behaviour only, fine to keep as "another viewer wraps"). |
| `tests/serialize.rs` | 8 of 8 | canonical style. |
| `tests/review_serialize.rs` | 6 of 6 | canonical style and `pdf-inspect --canonical`. |
| `tests/serialize_roundtrip.rs` | 2 of 3 | `an_empty_name_keeps_a_delimiter_before_a_regular_token`, `documents_with_empty_names_reopen_identically_in_every_writer_style`. |
| `tests/dictionary.rs` | 9 of 14 | `entries_keep_insertion_order`, `equality_ignores_order`, `keep_last_duplicates_keep_the_first_position` (scrub the put-routine name), `canonical_sorts_and_compact_keeps_source_order`, `app_save_keeps_source_order_and_drops_null_objects`, `stream_length_and_repair_keep_positions`, `removal_keeps_the_order_of_the_others` (scrub the delete-routine description), `app_save_drops_what_resolves_to_null`, `app_save_keeps_the_chains_the_engine_resolves`. |
| `tests/document_app.rs` | 5 relevant | `text_strings_are_encoded_as_mupdf_encodes_them` (rename), `text_string_decoding_follows_byte_order_marks_and_annex_d`, `text_strings_without_a_mark_read_as_utf8_when_they_are` (scrub "MuPDF passes those on"), `a_text_prefix_keeps_whole_characters`, `app_save_of_hello_matches_mupdf_object_syntax` (rename; integers only). Also scrub the module comment (names the reference program's save options) and the "as MuPDF writes" / "MuPDF 1.28.1 output" comments in the compact-syntax test. |
| `tests/serialize_roundtrip.rs` | (comments) | Scrub the comments that describe the reference program's real printer (they belong to the replaced model, 6.2). |
| `tests/app_journal.rs` | copy-on-write tests | keep except one assertion (6.2). |
| `tests/app_corpus_open.rs` | 6 lenient-token tests | `a_damaged_token_in_a_page_dictionary_keeps_the_page`, `a_damaged_token_keeps_annotations_forms_and_the_catalog`, `a_damaged_matrix_entry_keeps_the_form`, `a_catalog_with_a_damaged_type_opens`, `a_non_hex_character_in_the_encryption_dictionary_reads_as_zero`, `a_stray_closing_bracket_in_a_page_dictionary_keeps_the_page`: end-to-end checks of §4 through repair; owned with workstream 3's inventory. |
| `src/security.rs` unit test | `passwords_convert_to_pdfdoc_as_mupdf_does` | Measured data file; scrub the comment naming MuPDF's conversion routine (class C file, already on the plan's reword list). |
| `src/app/password.rs` unit test | `pdf_doc_bytes_match_mupdf_table` | rename. |

### 6.2 Replace

| Test | Why | Replacement check |
|---|---|---|
| `dictionary.rs` `random_operations_match_a_list_model` | model sorts past 100 (D1) | same random operations against a plain insertion-order list model; no sorted state. |
| `dictionary.rs` `large_dictionaries_sort_as_mupdf_sorts_them` | asserts the sort | `large_dictionaries_keep_insertion_order`: 100, 101, 102 and 10,000 descending keys print in source order in both `Dictionary::iter` and compact output; inserting `Z1` then `A0` appends them. |
| `dictionary.rs` `huge_dictionaries_stay_fast` | asserts key order past 100 | same sizes and 60 s bound; assert source order (`K0 K1 K2 …` for the ascending build, `K199999 …` for the reversed build) and canonical output still in byte order. |
| `dictionary.rs` `equality_agrees_across_storage` | `is_sorted` assertions | keep the equality matrix (same keys in any order, one value changed, one key replaced); drop `is_sorted`. |
| `dictionary.rs` `large_stream_dictionaries_sort_at_the_length_put` | asserts sort at /Length | for 100, 101 and 102 entries, an existing `/Length` keeps its place and a missing one is added last, exactly once. |
| `app_journal.rs` `a_cleared_or_written_clone_never_changes_the_original` | `cleared.is_sorted()` | assert `cleared.is_empty()` only. |
| `serialize_roundtrip.rs` `every_object_reads_back_identically_in_both_styles` | compact model rounds reals to binary32 (D2) | compact round trip equals the input exactly, except that an integral real with \|v\| < 2^63 reads back as the equal Integer. |
| `document_app.rs` `compact_style_matches_mupdf_golden_syntax` | reals table is binary32 output (D2) | keep the token, name, string and text cases unchanged (§3.7.2–3.7.4 match them); replace the 25-entry reals table with §3.7.5 examples plus: every compact real parses back to the same binary64; no exponent; no leading `0.`; integral values print as integers. Rename (drop "mupdf golden"). |
| `document_app.rs` `huge_reals_survive_a_save_and_reopen` | binary32 reals (D2) | expect the S2 forms: `/Z` is 1 followed by 300 zeros and `.`, `/V 15000000000`; reopened `/Z` is `Real(1e300)` and `/V` is `Integer(15000000000)`. |
| `src/serialize.rs` unit test `fast_real_digits_agree_with_the_boundary_search` | internal to the old real printer | dropped with the file. |
| `tests/app_annots.rs` appearance-stream byte assertions using printed reals (`.99994519 .0104717849 …`, `177.59575`) | follow D2 through `compact_real` | owned by workstream 6, which replaces these byte checks anyway. |
| `engine/oracle/compare-app-save.mjs` | object-by-object byte equality with MuPDF's save | reviewer-side report only (not handed to the implementer, not a gate): reopen both saves with mupdf.js and compare each object structurally, keys as sets, reals within binary32 rounding, strings by bytes. |
| `engine/oracle/differential.py`, `differential-serialize.py` | run the native AGPL oracle | reviewer-side differential after implementation; moved to `engine/oracle/` in Phase 0; not handed to the implementer. |
| `engine/oracle/fuzz/p22-lexfuzz.mjs` | compares against the removed TypeScript MuPDF engine | rewrite on mupdf.js (as Appendix A's harness does) or retire; reviewer-side. |

Under the Appendix B alternatives instead, the five `dictionary.rs` /
`app_journal.rs` tests (B.1) or the two real-format tests (B.2) stay as they
are (after scrubbing), and the replacements above are not needed for them.

### 6.3 New cases

1. Lenient `/A#00B` reads as the name `A#00B` (§4.1.1).
2. Compact output of `<</ 5>>`, `[// 3]`, `[/ 1 0 R/ 2.5/]` reads back
   identically (exists in part; extend with references and reals).
3. Compact reals: for 100,000 random binary64 values (all exponent bands,
   subnormals, ±0) the compact token parses back to the same bits (or the
   equal Integer for integral values), contains only `-`, digits and at most
   one `.`, and never starts with `0.` or `-0.`.
4. `/Type` `/Subtype` placement: all cases of §3.7.3, plus a reference in
   the `null_references` set as /Type value keeps the original order.
5. Text strings: `encode_text_string("\u{18}x")` is UTF-16BE with a mark and
   round-trips; `decode_text_string(FE FF D8 00 00 41)` is `"\u{FFFD}A"`;
   `decode_text_string(EF BB BF C3 41)` is `"\u{FFFD}A"`.
6. Dictionary: `size_of::<Object>() == 32`; `shares_entries` true for an
   unwritten clone of a non-empty dictionary and false after `insert`,
   `remove`, `retain`, `clear`, `get_mut` of a present key, `iter_mut`,
   `values_mut` on either side; still true after `get_mut` of an absent key.
7. Lexer `seek`: after an inline image, `seek(p)` past the data resumes
   tokenizing at `p`; `seek` beyond the end gives `Ok(None)`.

## Appendix A. Experiment log

Reference: npm `mupdf` 1.28.1 (`node_modules/mupdf/dist/mupdf.js`), run on
macOS arm64 through `slot node <script>`. Scripts and raw
outputs: `reviewer archive spec-01-02/` (`h.mjs` builds a five-object
classic-xref file whose object 4 is the input and whose catalog references
it; `wasRepaired()` was false for every case below, so these are object-level
readings, not file repair).

- **E-LEX** (`t1.mjs`, `e-lex.txt`): each input is object 4; each array item
  printed with its type. Selected rows (input → items):
  `[/A#20B /A#G0 /A#0 /A#00B /#41#42 /a#2f /]` → names `A B`, `A#G0`,
  `A#0`, `A#00B`, `AB`, `a/`, empty.
  ``[(\n\r\t\b\f\(\)\\\q\0\12\123\777\400\378\8)]`` → bytes
  `0a0d09080c28295c71000a53ff001f3838`.
  `[(a\rb\r\nc\n\rd)]` → `610a620a630a0a64`; `[(a\<CR>b\<LF>c\<CR><LF>d)]` →
  `61626364`.
  `[<ABQD> <A B Q> <41 4> <>]` → `ab0d`, `ab00`, `4140`, empty.
  `[1e5 1.0E-4 1.2.3 1..5 .5. -+5 12abc 0x10 7g2]` → nine nulls.
  `[-.5 +.5 5. . -. 0.-5 1.5-3 1-.5]` → reals −0.5 0.5 5 0 0 0 1.5 1.
  `[- + -- --5 +-5 5- -5-]` → integers 0 0 0 −5 0 5 −5.
  `[5+ 1.5. -1-2-3 ++5 .-5 -.-5 -5.5.5 1,5 0_612 3.40282347e38 +5 007
  -007.50]` → `null null -1 null 0.0 0.0 null null null null 5 7 -7.5`.
  `[1 { 2 } ) 3]` → `1 null 2 null null 3`. `[1 R 2]`, `[R]` → object
  fails ("cannot parse indirect reference in array").
  `[0 0 R 5]` → reference to object 0, then 5.
- **E-PARSE** (`t2.mjs`, `e-parse.txt`): tight print of object 4.
  `[4294967296.5- 4294967296.5 18446744073709551617- 9223372036854775808
  -9223372036854775809 --99999999999999999999 -2147483649
  1000000000000000000000000000000]` → `[.5 .5 1 -9223372036854775808
  9223372036854775807 -7766279631452241919 -2147483649
  5076944270305263616]`. Dictionary rows as quoted in §4.2; failures print
  "invalid key in dict" or "syntax error in object". `<</B 1/A 2/B 3/C 4/A
  5>>` → `<</B 3/A 5/C 4>>`.
- **E-DICT** (`t3.mjs`, `e-dict.txt`): `saveToBuffer('incremental=no,garbage=yes')`.
  Dictionaries of n descending keys `/K{n-1} … /K0`: n = 99, 100, 101 saved
  in source order; n = 102 saved in key order (`<</K0 0/K1 1/K10 10/K100
  …`). A put of `A0` into a parsed dictionary of 99 or 100 entries appended
  it; into 101 entries it produced key order. Delete-then-put and
  /Type,/Subtype rows as quoted in §3.5 and §3.7.3; `<</X 1/Subtype 5 0
  R/Type/T>>` with object 5 missing → `<</Type/T/X 1/Subtype null>>`.
- **E-DICT-2** (`t9.mjs`, `e-dict2.txt`): 101 descending keys plus a
  duplicate `/K3 -3` → saved in key order; 100 plus the duplicate → source
  order. Stream dictionaries with an existing `/Length` and 100, 101, 102
  entries: 100 kept source order; 101 and 102 were saved in key order with
  one `/Length`. `<</B 1/Length 2/A 2>>` → unchanged; `<</Z 1/A 2>>` +
  3-byte data → `<</Z 1/A 2/Length 3>>`.
- **E-SER** (`t4.mjs`, `e-ser.txt`): `toString(true)` of constructed
  values; names `A B`→`/A#20B`, `a#b`→`/a#23b`, `x/y`→`/x#2Fy`, `é`→`/#C3#A9`,
  `a%b`→`/a#25b`, `a(b)`→`/a#28b#29`, `a{b}`→`/a#7Bb#7D`, `~!`→`/~!`,
  `a\x7Fb`→`/a#7Fb`, `a\x01`→`/a#01`; text strings as in §3.6; separators
  `[[1/X](q)(q)<</A null/B<<>>>>1 0 R 1 true false null[]//3/.5 -1.25]`,
  `<</5>>`. (`toString` prints some strings as hex where the save prints a
  literal; the save output, E-SER-SAVE, is the one this spec follows.)
- **E-SER-SAVE** (`t5.mjs`, `e-ser-save.txt`): strings and reals written by
  `saveToBuffer('incremental=no,garbage=yes')`; the string rows are quoted in
  §3.7.4. Reals (input → MuPDF): 0.5→`.5`, −0.5→`-.5`, 1/3→`.33333335`,
  2/3→`.6666667`, 612→`612`, 1e-7→`.0000001`, 123456.789→`123456.79`, −0→`0`,
  π→`3.1415928`, 72.12345678→`72.12346`, 1.5e10→`15000001000`,
  4600095744→`4600095900`, 1e-40→`.0000000000000000000000000000000000000001`,
  0.30000001192092896→`.3`, 1e20→`100000000000000000000`,
  −3e19→`-30000000000000000000`, 3.4e38→`340000000000000000000000000000000000000`,
  1e300→`340282350000000000000000000000000000000`, 2147483647.5→`2147483700`,
  −2147483648→`-2147483648`, 16777217→`16777216`, 1e-45→`.000…002` (45
  decimals), 5e-324→`0`, 0.333333333→`.33333335`, 8388608.5→`8388608`,
  123456789012→`123456790000`, 1234.5678→`1234.5678`, 2.675→`2.675`.
- **E-TEXT** (`t6.mjs`, `e-text.txt`): `newByteString(bytes).asString()`.
  `FEFF00414E2D`→`A中`; `FEFF00414E`→`A`; `FFFE41002D4E`→`A中`;
  `EFBBBFC3A941`→`éA`; `EFBBBFC341`→U+FFFD `A`; `636166C3A9`→`café`;
  `18C3A9`→U+02D8 U+00C3 U+00A9; `63C341`→`cÃA`; `80818A929E`→U+2022 U+2020
  U+2212 U+2122 U+017E; `A0`→U+20AC; `01090A17`→same code points; `E9`→`é`.
  Outputs for undefined codes and malformed UTF-16/UTF-8 were not reliable
  through the binding (C-string conversion) and are not used.

## Appendix B. Byte-compatible alternatives (only if the owner overrides D1 or D2)

### B.1 Dictionary order with size-triggered sorting

Insertion order as in §3.5, except: when `insert` (a new key or a
replacement) is applied to a dictionary that already holds 101 or more
entries, the dictionary is first put into ascending key byte order and stays
in key order from then on (new keys go to their key-order place; removal
keeps key order; `clear` keeps the dictionary in key-order mode). Parsing
inserts every key, so a parsed dictionary of 102 or more entries is in key
order and one of 101 is not. `stream_dictionary` treats its `/Length` as an
insertion: a dictionary of 101 or more entries (counting an added `/Length`)
is written in key order with `/Length` in its key-order place.
`is_sorted()` reports key-order mode. Evidence: E-DICT, E-DICT-2.

### B.2 Reals rounded to binary32

Let r be `v` rounded to the nearest binary32 (ties to even), with values
beyond the binary32 range taken as ±(largest finite binary32).
1. If r is integral and −2^31 ≤ r < 2^31: the integer's digits (−0 → `0`).
2. Otherwise let lo and hi be the midpoints between r and the adjacent
   binary32 values below and above |r|. Among decimal numbers strictly
   between lo and hi, take those with the fewest significant digits, and of
   those the **largest**. Write it positionally without exponent, leading
   `0` or trailing zeros; with the sign of r. An integral result is written
   without a point, except that when its digits do not fit in i64 a trailing
   `.` is added so it reads back as a number (MuPDF omits the point; this
   engine keeps it).

Every E-SER-SAVE real above follows this rule (e.g. 1/3: the 8-digit
candidates inside the interval are .33333333, .33333334, .33333335 and the
largest is written; 1.5e10: lo is exactly 15000000000, which is excluded,
so `15000001000`; 4600095744: hi is exactly 4600096000, excluded, so
`4600095900`).

## Appendix C. Leak-check change log (2026-10-18)

Independent leak check against SPEC-RULES.md. Edits made:

1. §3.6: "MuPDF's `newString`" reworded to "mupdf.js" (a binding call, not
   an internal name).
2. §3.7.4: the hex/literal choice was stated as per-byte escape weights and
   a cost comparison, a shape that mirrors one program's internal test.
   Restated as the observable criterion (literal unless it would be longer
   than hex; ties to literal). Same outputs on every E-SER-SAVE row.
3. §4.1.4: added that the lenient-number rule is a summary of observed
   cases; the cases and magnitude rules are normative, not the rule's
   shape.
4. Appendix B.1: removed an unrecorded statement about how MuPDF's removal
   works internally.
5. §6.1: added scrub items for `document_app.rs` (module comment,
   compact-syntax comments) and `serialize_roundtrip.rs` comments.
6. §6.2: tool paths updated to `engine/oracle/` (moved in Phase 0).

Questions for the owner (gaps found, not filled):

- Does the `/Type`, `/Subtype` ordering of §3.7.3 also apply to
  `stream_dictionary` in the compact style (the text says "the dictionary's
  entries")? E-DICT-2 rows have no `/Type`.
- D1 and D2 defaults need the owner's confirmation before hand-off.
