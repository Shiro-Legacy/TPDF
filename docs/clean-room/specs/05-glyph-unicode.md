# Spec 05: character codes to Unicode, and simple encodings

Workstream 5 of `docs/mit-clean-room-plan.md`. Status: draft for review,
2026-10-18. Rules: `docs/clean-room/SPEC-RULES.md`.

This spec replaces:

- `engine/src/view/text_unicode.rs`: the Unicode text of the character
  codes a page shows, for structured text (spec 04);
- `engine/src/simple_encoding.rs`: the four named simple-font encodings,
  `/Differences`, and their glyph names' Unicode values, used by the app's
  form, stamp and edit code;

and it says how to regenerate, **from public sources only**, the data in

- `engine/src/view/text_unicode_data.rs`;
- `engine/src/simple_encoding_data.rs`.

The current data files and their generators were derived from MuPDF
(its source tables, or measurements of it). They are deleted, not edited.

The implementer may read: this spec, spec 04, ISO 32000-2 (Annex D in
particular), the Adobe Glyph List and its specification, the Unicode
Character Database, the OpenType specification, Apple's TrueType Reference
Manual, the public mapping files named in section 5, the acceptance tests
of section 8, the experiment scripts of section 7, and the surviving
engine code. The implementer may not read the current `text_unicode.rs`,
`simple_encoding.rs`, their data files, the old data generators and
reference evidence (8.5; kept out of the hand-off tree), their git
history, the `engine/design/*` notes about them, or MuPDF source.

Section numbers like §9.10.2 refer to ISO 32000-2:2020.

---

## 1. Scope

### 1.1 What the component does

1. For one font dictionary, it splits the bytes of a shown string into
   character codes and gives each code its Unicode text (§9.10.2), or
   says it has no information so that the interpreter's own reading is
   used.
2. It resolves glyph names to Unicode (Adobe Glyph List specification).
3. It normalises extracted characters (ligatures and presentation forms)
   and gives the bidirectional class and mirror of a character, for
   spec 04.
4. It provides the four named simple encodings of Annex D and their
   `/Differences` overlays as a small public API.

### 1.2 Callers (surviving code)

| Caller | Uses |
|---|---|
| `view::stext` (spec 04) | everything in 2.3 |
| `app::forms::text` | `SimpleEncoding::from_named(WinAnsi)`, `lookup_unicode`, `glyph_name`: the WinAnsi code of each character it writes into a field appearance, and the glyph names of codes 0x80–0xFF. |
| `app::ops::edit::font` | `NamedEncoding`, `SimpleEncoding::from_named`, `lookup_unicode`: reads and writes text in fonts with a named encoding. |
| `app::ops::stamp` | `SimpleEncoding::from_named(WinAnsi)` for stamp and watermark text. |
| `builtin_metrics` (`BuiltinMetrics::from_encoding`, `encoding`, `glyph_name`) | owns a `SimpleEncoding` to look up glyph widths by code. |
| `text_font`, `text_decode`, `font_resources`, `embedded_glyph`, `generated_appearance`, `cli_*` | Class B (phase 0 deletes them). If any survives, it uses `with_differences`, `with_object_differences`, `EncodingLimits`, `EncodingError`; the interface keeps them. |
| Tests | section 8 |

### 1.3 Surviving code the component builds on

- `hayro_interpret::hayro_cmap` (vendored, Apache-2.0 OR MIT): `CMap`
  parsing, `CMapName`, `CidFamily`, `load_embedded` (Adobe's predefined
  CMaps and the `Adobe-*-UCS2` CMaps, BSD-3-Clause from Adobe's
  `cmap-resources`), `BfString`, `WritingMode`.
- `hayro_syntax` objects (`Dict`, `Name`, `Array`, `Stream`).
- `crate::object::Object` (for `with_object_differences`).

### 1.4 Non-goals

- Font programs' outlines and metrics.
- PDFDocEncoding and text strings (workstream 1).
- Writing ToUnicode CMaps (the app's own code).

---

## 2. Interface

### 2.1 `crate::simple_encoding` (public; exact)

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamedEncoding { WinAnsi, MacRoman, Standard, MacExpert }
impl NamedEncoding {
    pub fn from_pdf_name(name: &[u8]) -> Option<Self>;
    pub fn pdf_name(self) -> &'static [u8];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodingLimits {
    pub max_items: usize,
    pub max_name_bytes: usize,
    /// All incoming name bytes, overwritten assignments included. Base
    /// slots do not count.
    pub max_processed_name_bytes: usize,
}
impl Default for EncodingLimits; // 4096 items, 127 name bytes, 256 KiB

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Difference<'a> { Code(u8), Name(&'a [u8]) }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlyphUnicode<'a> {
    /// An undefined base slot, or an explicit `.notdef`.
    UndefinedCode,
    UnknownGlyphName(&'a [u8]),
    Unicode(&'a [char]),
}

#[derive(Debug, Eq, PartialEq)]
pub struct EncodingError { pub index: Option<usize>, pub kind: EncodingErrorKind }
#[derive(Debug, Eq, PartialEq)]
pub enum EncodingErrorKind {
    ExpectedArray, ExpectedCodeOrName, CodeOutOfRange, NameBeforeCode,
    CodeOverflow, EmptyName, LimitExceeded(&'static str),
    AllocationFailed { code: u8 },
}
impl fmt::Display for EncodingError; impl std::error::Error for EncodingError {}

#[derive(Debug, Eq, PartialEq)]
pub struct SimpleEncoding { /* private: 256 optional glyph names */ }
impl SimpleEncoding {
    pub fn from_named(base: NamedEncoding) -> Self;
    pub fn glyph_name(&self, code: u8) -> Option<&[u8]>;
    pub fn lookup_unicode(&self, code: u8) -> GlyphUnicode<'_>;
    pub fn with_differences(&self, entries: &[Difference<'_>], limits: EncodingLimits)
        -> Result<Self, EncodingError>;
    pub fn with_object_differences(&self, value: &Object, limits: EncodingLimits)
        -> Result<Self, EncodingError>;
}
```

The limits' default values are the project's own interface (the 127-byte
name length is Annex C's name limit); tests pin them.

Semantics:

- `from_pdf_name`: exactly `WinAnsiEncoding`, `MacRomanEncoding`,
  `StandardEncoding`, `MacExpertEncoding` (case-sensitive); `pdf_name`
  is the inverse.
- `from_named`: the Annex D table (5.1). `glyph_name(code)`: the slot's
  name bytes, or `None` for an undefined slot.
- `lookup_unicode(code)`: `UndefinedCode` for an undefined slot or the
  name `.notdef`; `Unicode(chars)` when the name is one of the **registry
  names**, the union of the names in the four Annex D tables (378 names),
  with its single Adobe Glyph List value (5.1); otherwise
  `UnknownGlyphName(name)`. This lookup is deliberately not a general
  glyph-name resolver: `uni0041` in a slot is `UnknownGlyphName`.
- `with_differences(entries, limits)` returns a new encoding; `self` is
  never changed, also on error. It is transactional: every entry is
  checked and charged before anything is allocated. In order:
  1. `entries.len() > max_items` → `LimitExceeded("Differences items")`,
     `index: None`.
  2. For each entry *i* in order:
     - `Code(c)`: the cursor becomes *c*.
     - `Name(n)`, checked in this order, each error with `index: Some(i)`:
       no cursor yet → `NameBeforeCode`; cursor past 255 →
       `CodeOverflow`; empty `n` → `EmptyName`; `n.len() > max_name_bytes`
       → `LimitExceeded("glyph name bytes")`; running total of name bytes
       past `max_processed_name_bytes` →
       `LimitExceeded("processed name bytes")`. Otherwise the slot at the
       cursor gets `n` (a later assignment to the same slot wins) and the
       cursor advances by one.
  3. Names are opaque bytes (no UTF-8 or C-string conversion: NUL and
     0xFF bytes are kept). An allocation failure for a slot →
     `AllocationFailed { code }` with the index of the entry that set it
     (`None` for a base slot).
- `with_object_differences(value, limits)`: `value` must be
  `Object::Array` (else `ExpectedArray`, `index: None`); each item an
  `Object::Integer` in 0..=255 (`Code`; out of range → `CodeOutOfRange`)
  or an `Object::Name` (`Name`); anything else, references included →
  `ExpectedCodeOrName`, with the item's index. Then as above.
- `Display`: `PDF simple encoding`, then ` Differences[{i}]` when `index`
  is set, then one of `: expected Differences array`,
  `: expected integer code or byte name`, `: code must be in 0..=255`,
  `: name needs a preceding code`,
  `: name exceeds slot 255 without a code reset`, `: empty glyph name`,
  `: {resource} limit exceeded`, `: allocation failed for slot {code}`.

### 2.2 `crate::view::text_unicode` data

The module keeps its data in a generated file (`text_unicode_data.rs`,
section 5). Its content is described by source, not copied.

### 2.3 `crate::view::text_unicode` (visible to `view`; the contract with spec 04)

```rust
/// Predefined CMaps parsed so far, by name, shared by the fonts of one
/// TextReader (a CJK collection's CMaps are large).
pub(super) type CMaps = HashMap<Vec<u8>, Option<Rc<CMap>>>;

/// One character code of shown text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Code {
    pub code: u32,
    /// The CID for a composite font; the code itself for a simple font.
    pub cid: u32,
    /// Bytes of the shown string the code took (1 to 4).
    pub bytes: u8,
}

pub(super) struct FontText { /* private */ }
impl FontText {
    /// `font` is a font dictionary (Type0 with its descendant, Type1,
    /// MMType1, TrueType or Type3); `program` the decoded embedded font
    /// program of the font (the descendant's for Type0), if any.
    pub(super) fn new(font: &Dict<'_>, program: Option<&[u8]>, cmaps: &mut CMaps) -> Self;
    /// Appends the codes of `shown`, split as the interpreter splits them
    /// (3.2).
    pub(super) fn codes(&self, shown: &[u8], out: &mut Vec<Code>);
    /// The Unicode text of one code (3.1), or None: no information here,
    /// use the interpreter's reading of the glyph.
    pub(super) fn text(&self, code: Code) -> Option<Vec<char>>;
    /// The font writes vertically (WMode 1 of its CMap, §9.7.4.3).
    pub(super) fn vertical(&self) -> bool;
}

/// The glyph name of each code of a simple font's /Encoding (3.4):
/// base encoding, then /Differences. For Type3 fonts too.
pub(super) fn encoding_names(font: &Dict<'_>) -> Vec<Option<Vec<u8>>>; // 256 slots

/// A glyph name's Unicode text by the AGL specification (4.1), or None.
/// `dingbats`: the font is ZapfDingbats (use the ITC Zapf Dingbats list).
pub(super) fn glyph_name(name: &[u8], dingbats: bool) -> Option<Vec<char>>;

/// Appends the characters structured text gives `c` (4.3): ligatures and
/// presentation forms as their components, U+FEFF as nothing, every
/// other character as itself.
pub(super) fn normalise_extracted(c: char, out: &mut Vec<char>);

/// Bidi_Class of `c` (UCD DerivedBidiClass.txt, defaults included).
pub(super) fn bidi_class(c: char) -> BidiClass;
/// Bidi_Mirroring_Glyph of `c` (BidiMirroring.txt), if any.
pub(super) fn bidi_mirror(c: char) -> Option<char>;
```

`BidiClass` is an enum of the 23 UAX #9 classes (it may come from the
`unicode-bidi` crate if the owner approves the dependency, spec 04 Q6).
Every function is pure apart from the `CMaps` cache, never panics, and is
`O(input)` or `O(log table)` per call.

---

## 3. Required behaviour: codes and text

### 3.1 Order of sources for one code (§9.10.2)

For each code, the first of these that gives text wins:

1. **ToUnicode** (§9.10.3): the font's `/ToUnicode` stream maps the code
   (3.3). A ToUnicode given as a name: for a Type0 font, `/Identity-H` or
   `/Identity-V` reads each two-byte code as a UTF-16 code unit (a
   surrogate code reads as nothing); any other name is ignored. For a
   simple font a name is ignored (G3).
2. **Composite fonts** (Type0) without a mapping from step 1:
   - a CIDFont whose `/CIDSystemInfo` names one of the Adobe character
     collections that have a UCS2 CMap (Adobe-Japan1, Adobe-GB1,
     Adobe-CNS1, Adobe-Korea1, and Adobe-KR if the CMap data has it): the
     CID through that collection's `Adobe-<Ordering>-UCS2` CMap
     (§9.10.2 step 2). A CID the CMap does not map → U+FFFD;
   - a `CIDFontType2` (TrueType) font with no embedded program whose
     encoding is Identity-H or Identity-V: the CID is a glyph index of the
     original TrueType font. When the CID is below 258, it is read as the
     glyph name at that index of the standard Macintosh glyph order
     (Apple TrueType Reference Manual, `post` table), resolved by 4.1;
     else U+FFFD. Rationale: the common system TrueType fonts start with
     that order (G4: CIDs 0x24 and 0x44 read `A` and `a`). The same rule is
     *not* applied to `CIDFontType0` (CFF) fonts, whose CIDs have no such
     meaning (MuPDF applies it; G4 shows `?` for CID 0x22);
   - an embedded `CIDFontType2` program: `None` here, so the interpreter's
     reading applies; the implementation should read the glyph's Unicode
     from the program's Unicode cmap (3,1 or 3,10) by inverting it for the
     glyph the CID selects (`/CIDToGIDMap`), when that cmap maps the glyph
     from exactly one character;
   - otherwise U+FFFD.
3. **Simple fonts** (Type1, MMType1, TrueType, Type3) without a mapping
   from step 1:
   - the glyph name of the code from the font's encoding (3.4) resolved
     by 4.1;
   - else, by the font's built-in encoding: a non-embedded font that is
     Symbol or ZapfDingbats by name (3.6) reads through that font's
     built-in table (5.3, 5.4); any other non-embedded font reads through
     StandardEncoding; an embedded TrueType program reads through its
     cmap and glyph names (3.5); an embedded Type1 or CFF program →
     `None` (the interpreter reads the program's own encoding and glyph
     names);
   - Type3 fonts: the glyph name resolved by 4.1, else the Type3 rule of
     4.2.
   - A code that none of these settle → U+FFFD.

Every character obtained goes through the control rule of 3.7.

### 3.2 Splitting shown bytes into codes

- Simple fonts: one byte, one code (`cid` = code).
- Composite fonts: by the encoding CMap's codespace ranges (§9.7.6.2):
  at each position, the byte sequence of 1 to 4 bytes that falls in a
  codespace range of its own length. A position no range matches takes
  the shortest length for which the interpreter does the same; the split
  must equal the interpreter's (hayro's), because spec 04 pairs codes
  with drawn glyphs. `cid` is the CMap's CID for the code (0 when
  unmapped, §9.7.6.3 notdef rules).
- The encoding is a predefined CMap by name (§9.7.5.2, Table 116; data
  from `hayro_cmap::load_embedded`), `Identity-H`/`Identity-V`, or an
  embedded CMap stream (with `/UseCMap`, §9.7.5.3).

### 3.3 ToUnicode streams

A ToUnicode stream is a CMap (§9.10.3, Adobe Technical Note #5411)
whose `bfchar` and `bfrange` blocks map codes to UTF-16BE strings.

Required:

- `beginbfchar`: code → destination string, UTF-16BE, surrogate pairs
  joined (G3: `<D83DDE00>` → U+1F600). A destination of several units is a
  sequence (`<00660069>` → `fi`).
- `beginbfrange` with a string destination: the destination of the n-th
  code of the range is the destination with its last UTF-16 unit
  increased by n (G3: `<00660069>` over three codes → fi, fj, fk). An
  increase past 0xFF in the last byte carries into the unit (G3: 00FF,
  0100, 0101), as the evident intent of the writer.
- `beginbfrange` with an array destination: one string per code.
- Codes are looked up by numeric value, whatever the byte length of the
  codespace (G3: a two-byte codespace still maps the one-byte code 0x41
  of a simple font).
- A later mapping of the same code replaces an earlier one (G3).
- `begincidchar`/`begincidrange` in a ToUnicode stream: the CID is read as
  the Unicode scalar (G3: 72 → `H`). Harmless and seen in practice.
- `usecmap` inside a ToUnicode stream is followed for predefined names.

Tolerant (each observed in G3; chosen as best for users):

| Input | MuPDF 1.28.1 | This spec |
|---|---|---|
| no codespace range | maps (warning) | maps |
| destination `<0000>`, or a control other than tab/LF/VT/FF/CR | falls back to the encoding | same: the code is unmapped here (3.7) |
| destination `<0009>` (whitespace control) | space | space |
| destination an unpaired surrogate | three U+FFFD | the code is unmapped here (fallback) |
| empty destination `<>` | falls back | falls back |
| one-byte destination `<5A>` | falls back (`A`) | **`Z`**: an odd-length destination's bytes are read as Latin-1 when it is one byte; longer odd lengths are padded with a zero byte at the end, then read as UTF-16BE |
| a malformed entry (`<41> /H`) | the whole CMap is ignored | **only that entry is skipped**; the other entries map |
| a stream that is not a CMap at all | ignored | ignored |
| a stray `>` or junk tokens outside the mapping blocks | not recorded in 7.4 | skipped |
| a code the stream does not map | the encoding's reading | the next source of 3.1 |

The current engine makes a stream with a malformed block give U+FFFD for
every code with no fallback; this spec keeps every good entry and falls
back for the rest, which loses no text.

### 3.4 Simple-font encodings (§9.6.5, §9.6.6)

The glyph name of each code:

1. `/Encoding` a name: one of the four of Annex D (5.1) → that table.
   Any other name → as if absent.
2. `/Encoding` a dictionary: start from `/BaseEncoding` (one of the four;
   another value → as if absent); without one, from the font's default
   (below). Then apply `/Differences` (§9.6.5.1): a number sets the code,
   each following name fills it and advances; out-of-range codes and
   non-name, non-number items are skipped. Type3 fonts have no default
   base: only `/Differences` apply.
3. `/Encoding` absent: the font's default.

The default base is StandardEncoding for a non-embedded font that is not
Symbol or ZapfDingbats (3.6) and is not symbolic. For an embedded program,
or a symbolic font, there is no default (the built-in encoding of 3.1
applies to codes without a name). The symbolic flag is `/Flags` bit 3 set
with bit 6 clear (§9.8.2).

G2 shows these readings for non-embedded fonts: no encoding → Standard
(0x60 → U+2018); WinAnsi; MacRoman; `/Differences` without a base on a
standard font → Standard with the overlay; an unknown base name →
Standard.

### 3.5 Embedded TrueType programs (§9.6.5.4, OpenType `cmap`, `post`)

For a code with no glyph name from 3.4, in an embedded TrueType program
(`glyf`-based; a CFF-based OpenType program is treated as Type1/CFF):

1. Choose a cmap subtable: for a symbolic font, (3, 0) if present; then
   (3, 1); then (1, 0); then the first subtable of a supported format.
   Supported formats: 0, 4, 6, 12, 13 (OpenType spec); others are
   skipped.
2. Look up the glyph: with (3, 0), the code c, else 0xF000 + c (and the
   0xF100, 0xF200 ranges §9.6.5.4 allows); with (1, 0) and others, c.
   Format 4: the first segment (in table order) whose endCode ≥ c decides
   (OpenType). Formats 12 and 13: groups are sorted by startCharCode; look
   up by bisection (a 500,000-group table must cost microseconds per
   code). A table that claims more entries than its bytes hold reads what
   is there.
3. Name the glyph from the `post` table: format 1 (the 258 standard
   Macintosh names, 5.5), format 2 (indexes into the standard names, then
   Pascal strings), format 2.5 optional; format 3 has no names. Resolve
   the name by 4.1.
4. A glyph with no name, or glyph 0: the implementer may read the code as
   WinAnsi when the font is symbolic and the code is printable ASCII
   (writers often mark subset fonts symbolic while using ASCII codes), if
   the corpus gate (8.3) shows this gives the shown text; otherwise
   U+FFFD.

### 3.6 Standard-font identity for built-in encodings

The font is **Symbol** when its `/BaseFont`, without a six-letter subset
prefix (`ABCDEF+`) and without a style suffix after a comma, is `Symbol`
or `SymbolMT`; **ZapfDingbats** when it is `ZapfDingbats` or `Dingbats`.
A font descriptor is not required. Such a font without a descriptor is
symbolic.

### 3.7 Controls and invalid values

Applies to every character from every source:

- U+0009–U+000D (tab, LF, VT, FF, CR) become U+0020.
- Other C0 controls, U+007F and C1 controls (U+0080–U+009F), U+0000,
  surrogates and non-characters U+FFFE/U+FFFF: the source gives nothing
  for that code (the next source of 3.1 is tried). In a multi-character
  destination, such characters are dropped and the rest kept.
- Values above U+10FFFF → nothing.

### 3.8 Vertical writing

`vertical()` is true when the encoding CMap's writing mode is 1: a
predefined `-V` CMap, `Identity-V`, or an embedded CMap whose `/WMode`
is 1 (§9.7.4.3, §9.7.5.3). Where the stream dictionary's `/WMode` and the
CMap program's `/WMode` disagree, the dictionary wins (§9.7.5.3 says the
dictionary's value shall be used); the interpreter must draw it the same
way, or this is reported as a difference.

---

## 4. Required behaviour: names, normalisation, bidi

### 4.1 Glyph name to Unicode (AGL specification, section 2)

1. Drop everything from the first `.` on (`A.sc` → `A`). An empty result
   (`.notdef`) → nothing.
2. Split at `_` into components; the result is the concatenation of each
   component's characters (`f_i` → `fi`, `T_h` → `Th`,
   `uni0041_uni0042` → `AB`). A component that maps to nothing makes the
   whole name map to nothing.
3. A component maps by the first rule that applies:
   - it is in the Adobe Glyph List (`glyphlist.txt`), or, for a
     ZapfDingbats font, in the ITC Zapf Dingbats list
     (`zapfdingbats.txt`): its value, which may be several characters
     (`dalethatafpatah` → U+05D3 U+05B2);
   - `uni` followed by a multiple of four hex digits, each group a BMP
     value outside the surrogate range: those characters
     (`uni00410042` → `AB`);
   - `u` followed by four to six hex digits giving a scalar value outside
     the surrogate range and ≤ U+10FFFF: that character (`u1F600` →
     U+1F600);
   - otherwise nothing.
4. Tolerance beyond the AGL spec: lowercase hex digits are accepted in
   `uni` and `u` forms (`uni00e9` → é; G1 shows MuPDF accepts them too).
   No other leniency (no signs, no `0x` prefixes, no spaces).

Departures from MuPDF, all toward the AGL specification (G1): MuPDF reads
`uni00410042` as U+FFFD, `u1F600` as U+F600, `T_h` as `T`,
`uni0041_uni0042` as `A`, and `dalethatafpatah` as U+FFFD.

Names in the AGL with a ligature value (`fi` → U+FB01) resolve to the
ligature; spec 04 then normalises it (4.3) so extracted text reads `fi`.

### 4.2 Type3 fonts: names that are codes

Type3 fonts (§9.6.4) often come from generators that name glyphs by
their code. A Type3 code whose glyph name does not resolve by 4.1 reads
as the character U+00NN of its own code NN (Latin-1) when the name is
`a` followed by the code's own decimal number (`/a65` at code 65 → `A`).

Re-observed need (7.4): in 1,144 corpus files scanned (pdf.js,
govdocs1, veraPDF), 3 files have Type3 fonts with 3,503 names of the form
`a<digits>`, and in **every** one the number equals the code. Names of the
form `g<digits>` (1,281) matched their code in 8 cases decimal and 37 hex,
so they get no rule. A name `a<digits>` that is not its own code
(`/a67` at code 162) reads U+FFFD; MuPDF reads it as `C`.

### 4.3 `normalise_extracted`

- Every character in U+FB00–U+FEFF that has a decomposition in
  `UnicodeData.txt` (canonical or compatibility, any tag) is replaced by
  its full decomposition, applied recursively (the NFKD of the character):
  Latin, Armenian and Hebrew ligatures and presentation forms, Arabic
  presentation forms A and B, vertical, small and CJK compatibility forms.
  Unicode 16.0 has 854 such characters.
- U+FEFF (byte order mark / zero-width no-break space) becomes nothing.
- Every other character is itself (no general normalisation: fullwidth
  forms, compatibility ideographs and the like stay).

### 4.4 Bidi data

`bidi_class` from `DerivedBidiClass.txt` (which assigns default classes
to unassigned code points in right-to-left blocks), `bidi_mirror` from
`BidiMirroring.txt`, both of the Unicode version the generator pins
(16.0 today).

---

## 5. Data to regenerate (class D)

A generator, `engine/tools/generate-text-data.py` (new, MIT, self-written),
reads the source files below from a directory given on its command line,
checks each file's SHA-256 against values pinned in the script, and
writes `src/simple_encoding_data.rs` and `src/view/text_unicode_data.rs`
with a header naming each source, its version, licence and SHA-256. The
source files are not committed unless their licence allows it; the script
says where to fetch them. Tables are sorted for binary search.

| Table | Source | Licence | Notes |
|---|---|---|---|
| 5.1 WinAnsi, MacRoman, Standard, MacExpert glyph names | ISO 32000-2 Annex D.2 (Latin character set and encodings) and D.4 (Expert set) | the standard's facts (glyph names per code) | Typed from the standard into a checked-in text table with the generator; Annex D's notes apply: WinAnsi has `bullet` at the codes above 0x20 that its table leaves unused (0x7F, 0x81, 0x8D, 0x8F, 0x90, 0x9D), `space` also at 0xA0 and `hyphen` also at 0xAD; MacRoman has `space` also at 0xCA. Cross-check WinAnsi against Microsoft's `CP1252.TXT`, MacRoman against Apple's `ROMAN.TXT` and Standard against Adobe's `stdenc.txt` (unicode.org `Public/MAPPINGS/VENDORS/`); the known PDF differences (currency at MacRoman 0xDB, no Apple logo) are listed in the generator. Expected counts: undefined codes 32 (WinAnsi), 48 (MacRoman), 107 (Standard), 91 (MacExpert); 378 distinct names. |
| 5.1 registry Unicode | Adobe Glyph List 2.0 `glyphlist.txt` (github.com/adobe-type-tools/agl-aglfn) | BSD-3-Clause | One value per registry name (all 378 have a single value). Spot checks: `Omega` U+2126, `Delta` U+2206, `mu` U+00B5, `hyphen` U+002D, `Euro` U+20AC, `fraction` U+2044. |
| 4.1 glyph list | AGL `glyphlist.txt` and `zapfdingbats.txt` (ITC Zapf Dingbats list), same repository | BSD-3-Clause | Two tables: the general list and the dingbats list. hayro-interpret embeds the general list too (`font/generated/glyph_names.rs`); the generator must take the Adobe files, not MuPDF's list. |
| 5.3 Symbol built-in encoding | Adobe `symbol.txt` (unicode.org `Public/MAPPINGS/VENDORS/ADOBE/symbol.txt`) | Adobe's mapping-file terms (Q4) | Code → Unicode, as given (it uses private-use values U+F8E5–U+F8FE for parts like `radicalex`; G2 shows MuPDF gives the same, e.g. 0x60 → U+F8E5, 0xA0 → U+20AC). |
| 5.4 ZapfDingbats built-in encoding | Adobe `zdingbat.txt` (same directory), or the AGL `zapfdingbats.txt` with the font's built-in code order from its AFM (URW D050000L AFM, OFL) | as above / BSD-3 + OFL | Code → U+27xx dingbats. G2 shows MuPDF gives control characters and Latin letters here; this is a bug, not behaviour to keep. |
| 5.5 standard Macintosh glyph order | Apple TrueType Reference Manual, `post` table, the 258 standard glyph names | facts | Used by `post` formats 1 and 2 (3.5) and by the CIDFontType2 rule of 3.1. Resolve each name by 4.1 at run time; no separate CID → Unicode table. |
| 4.3 presentation forms | `UnicodeData.txt` field 5, Unicode 16.0 | Unicode License v3 | 854 entries; values fully decomposed. |
| 4.4 bidi classes and mirrors | `DerivedBidiClass.txt`, `BidiMirroring.txt`, Unicode 16.0 | Unicode License v3 | Range table and pair table. |

NOTICE gains the AGL BSD-3 notice and the Unicode License v3 notice.

The old generators and their evidence are retired (8.5).

---

## 6. Limits and safety

- No panics, no `unsafe`. Every table read from a font program or a CMap
  is bounds-checked; lengths and counts in font tables are clamped to the
  bytes present.
- CMaps: the predefined ones are parsed once per name per reader
  (`CMaps`). `usecmap` chains are followed to a small fixed depth, no
  cycles.
- ToUnicode: at most one mapping per possible code of the codespace;
  ranges are not expanded eagerly when large (a range of 65,536 codes is
  one entry). Parsing is linear in the stream.
- `with_differences`: as 2.1 (limits, transactional, allocation checked).
- Performance tests: 8 fonts sharing a TrueType program with a
  500,000-group format 12 subtable, read in under 1.5 s (test
  `truetype_cmaps_with_many_groups_are_read_by_bisection`); a page of
  200 non-embedded Japan1 fonts under 300 MB peak (spec 04, 6.3).

---

## 7. Recorded black-box experiments

### 7.1 Method

Scripts (self-authored) in
`reviewer archive spec-04-05/`: `lib.mjs`,
`exp05.mjs [names|simple|tounicode|cid]`, `probe-type3b.mjs` (corpus,
structure only). Oracle: MuPDF 1.28.1 (npm `mupdf`),
`toStructuredText("preserve-whitespace").asText()`. Raw output:
`exp05.txt`. Run through `slot`.

### 7.2 G1 glyph names (Type3 font, one code per name)

| Name | MuPDF | This spec |
|---|---|---|
| `A`, `Euro`, `Acircumflex`, `afii10017` | U+0041, U+20AC, U+00C2, U+0410 | = |
| `uni0041`, `uni00e9`, `u00E9`, `u0041` | A, é, é, A | = |
| `uni00410042` | U+FFFD | `AB` |
| `uniD800` | three U+FFFD | U+FFFD |
| `uni004` (3 digits), `uFFFFFF`, `foo`, `g65`, `G41`, `cid65`, `a1` (not a dingbats font) | U+FFFD | = |
| `u1F600` | U+F600 | U+1F600 |
| `A.sc` | A | = |
| `f_i`, `f_f_i` | `fi`, `ffi` | = |
| `T_h` | `T` | `Th` |
| `uni0041_uni0042` | `A` | `AB` |
| `a67` (at code 0xA2) | `C` | U+FFFD (4.2) |
| `a5` | U+FFFD | = |
| `space`, `nbspace`, `sfthyphen` | U+0020, U+00A0, U+00AD | = |
| `dalethatafpatah` | U+FFFD | U+05D3 U+05B2 |
| `Omega`, `Delta`, `mu` | U+2126, U+2206, U+00B5 | = |
| `fi`, `ff` | `fi`, `ff` (normalised) | = |
| `uni0000` | U+FFFD | U+FFFD |
| `uni0009`, `u0007` | U+0009, U+0007 | U+0020, U+FFFD (3.7) |

### 7.3 G2 non-embedded simple fonts, codes 0x20–0xFF

Code: MuPDF's character.

| Font | 0x41 | 0x60 | 0x7F | 0x80 | 0xA0 | 0xA4 | 0xAD | 0xC6 |
|---|---|---|---|---|---|---|---|---|
| Helvetica, no `/Encoding` | A | U+2018 | FFFD | FFFD | FFFD | U+2044 | U+203A | U+02DC |
| Helvetica WinAnsi | A | ` | U+2022 | U+20AC | space | ¤ | - | Æ |
| Helvetica MacRoman | A | ` | FFFD | Ä | U+2020 | § | U+203A | U+02D8 |
| Symbol, no `/Encoding` | U+0391 | U+F8E5 | FFFD | U+F8FF | U+20AC | U+2044 | U+2191 | U+2205 |
| ZapfDingbats, no `/Encoding` | U+001D | U+003C | U+0059 | U+005A | U+0065 | U+006A | U+007A | U+0093 |
| Arial TrueType, Flags 32 or 4, no `/Encoding` | as Helvetica without encoding |
| Helvetica `/Differences [65 /Omega]`, no base | U+2126 | as Standard |
| Helvetica `/FooEncoding` | as Standard |

Note (review, 2026-10-18): the engine reads Standard 0xC6 as U+02D8 (breve), as Annex D and Adobe's `stdenc.txt` give it; the U+02DC in the first row is the reference viewer's reading, not a requirement.

This spec: the same, except ZapfDingbats (5.4: real dingbats, for
example 0x41 → U+2721).

### 7.4 G3 ToUnicode (simple font, WinAnsi, shows `ABCD`)

As tabled in 3.3. Additional rows: bfrange `<41> <43> <0061>` → `abc` then
`D`; array destination `[<0078> <00790079> <007A>]` → `x`, `yy`, `z`;
partial map (only B) → `A`, `X`, `C`, `D`; `/ToUnicode /Identity-H` on a
simple font → ignored.

### 7.5 G4 composite fonts without ToUnicode, no program, Identity-H

Shown CIDs 0x0022, 0x0410, 0x3042:

| Ordering | MuPDF |
|---|---|
| Adobe-Japan1 | U+0041 U+03B6 U+308D |
| Adobe-GB1 | U+0041 U+62B1 U+5DFC |
| Adobe-CNS1 | U+0041 U+8840 U+9A04 |
| Adobe-Korea1 | U+0041 U+0423 U+BB23 |
| Adobe-Identity, Adobe-UCS (CIDFontType0) | U+003F U+FFFD U+FFFD (Mac order) |

`CIDFontType2` Arial, Identity, CIDs 0x24, 0x44 → `A`, `a`.
`90ms-RKSJ-H` (Japan1) bytes 82A0 41 42 42 → U+3042 A B B.

This spec: the same, except the CIDFontType0 Identity row (U+FFFD each,
3.1).

### 7.6 Corpus probe (structure only)

`probe-type3b.mjs` over the pdf.js, govdocs1 and veraPDF corpus files
(4,102 listed; 1,144 opened before batches stopped on files the oracle
traps on): 29 files with Type3 fonts. `/Differences` names: 1,038
AGL-like, 87 `uniXXXX`, 3,503 `a<digits>` in 3 files (all equal to their
code), 1,281 `g<digits>`, 109 `c<digits>`/`cid<digits>`, 71 other. 6 files
have a non-ASCII `/BaseFont`. No document text was recorded.

---

## 8. Acceptance

### 8.1 Existing tests

**`engine/tests/simple_encoding.rs` (12): keep all.** They pin the
interface of 2.1 and Annex D facts (undefined counts 32/48/107/91, 378
names, WinAnsi bullets, MacRoman 0xCA and 0xDB). Scrub nothing.

**`engine/tests/review_simple_encoding.rs` (4): keep all.**

**`engine/tests/builtin_metrics.rs`, `review_builtin_metrics.rs`,
`builtin_metrics_resources.rs`:** keep (they use `SimpleEncoding` only
through the API).

**App tests using WinAnsi through the API** (`app_forms.rs`,
`app_misc.rs` stamps and watermarks, `app_edit.rs`,
`app_accept_edit.rs`): keep. Their MuPDF-stream equality assertions
belong to workstreams 6 and 7, not here.

**Class B tests** (`text_decode.rs`, `review_text_decode.rs`,
`text_font.rs`, `encoded_text.rs`, `font_resources.rs`,
`review_font_resources.rs`, `text_position.rs`,
`review_text_position.rs`, `program.rs`, `review_program.rs`,
`review_metrics_text.rs`): deleted with their modules in phase 0; any
that survive keep their `simple_encoding` use unchanged.

**From `engine/tests/view_text.rs`:**

| Test | Class | New check |
|---|---|---|
| `to_unicode_cmaps_read_as_leniently_as_mupdf` | modify, rename `to_unicode_cmaps_read_leniently` | Stray `>`: `Hell`. CID mapping: `HIJJ`. Malformed block `<0001> /H`: that entry skipped; with no other entry, the codes fall back to 3.1 step 2 (Identity ordering, CIDFontType2, no program: Mac order, CIDs 1, 2, 3, 3 name `.null`, `nonmarkingreturn`, `space`, `space`, so the text is U+FFFD, U+FFFD, space, space). |
| `truetype_cmaps_with_many_groups_are_read_by_bisection` | keep | |
| `s22pdf_chinese_fonts_read_as_gbk_with_full_em_advances` | delete (Q2) | |

**From `engine/tests/view.rs`:** `type3_text_comes_from_encoding_names_and_widths`
modify as spec 04 8.1 says (names per 4.1 and 4.2).

**Unit tests in `text_unicode.rs`** (`format_4_segments_decide_codes_as_a_lookup_by_code_would`,
`format_12_groups_are_found_by_bisection`): deleted with the file;
replaced by N4 and N5, which test the same through a font on a page.

**Reference evidence:** `reference/simple-encoding-differential-s0.json`,
`reference/encoding-table-inventory.json`,
`reference/encoding-text-differential-d0.json`,
`reference/text-decode-differential-*.json` and
`reference/text-unicode-20261015/mupdf-unicode.json` are MuPDF
measurements or MuPDF-derived. Move them to `engine/oracle/` (AGPL) or
delete; none is a gate for the new code.

### 8.2 New cases

Each builds its page or font in the test.

| Case | Expect |
|---|---|
| N1 | Every row of 7.2 with this spec's value (Type3 font, one code each). |
| N2 | ToUnicode: every row of 3.3's tolerant table and 7.4. |
| N3 | Non-embedded Symbol and ZapfDingbats, codes 0x20–0xFE: each code's character equals the source table (5.3, 5.4); ZapfDingbats 0x34 → U+2714 (✔). |
| N4 | An embedded TrueType program built in the test with a format 4 subtable whose segment ends are out of order and overlapping: every code reads the glyph the first segment with endCode ≥ code gives, named through `post` format 2. |
| N5 | Format 12: groups looked up by bisection; a group count larger than the table reads what is present; a symbolic font with a (3,0) subtable at 0xF020–0xF07E reads ASCII codes. |
| N6 | Composite fonts: G4 rows (Japan1, GB1, CNS1, Korea1 through the UCS2 CMaps; CIDFontType2 Identity without program through the Mac order; CIDFontType0 Identity → U+FFFD). |
| N7 | `/ToUnicode /Identity-H` on a Type0 font: codes read as UTF-16 units. |
| N8 | `normalise_extracted`: U+FB01 → `fi`, U+FB03 → `ffi`, U+FB2C → U+05E9 U+05BC U+05C1, U+FE8D → U+0627, U+FE10 → `,`, U+FEFF → nothing, U+FF21 → itself; exactly 854 characters change in U+FB00–U+FEFF. |
| N9 | Controls (3.7): ToUnicode to U+0007 falls back; to U+000A reads a space; a multi-character destination with a NUL drops only the NUL. |
| N10 | Type3 4.2: `/a65` at 65 → `A`; `/a200` at 200 → U+00C8; `/a67` at 162 → U+FFFD; `/g65` at 65 → U+FFFD (amended by 12.2: `A`). |
| N11 | Data integrity (generator output): registry 378 names; WinAnsi/MacRoman/Standard/MacExpert undefined counts; AGL entry count equal to the pinned file's (recorded by the generator); every table sorted; `bidi_class` of U+05D0 R, U+0627 AL, U+0661 AN, U+0031 EN, U+10C80 R (beyond the BMP); `bidi_mirror('(') == Some(')')`. |
| N12 | A page whose Hebrew font has a ToUnicode and is drawn in visual order: spec 04 N6 passes with these bidi tables. |

### 8.3 Corpus acceptance

Shared with spec 04 (8.3 there), plus:

- The runner records, per file, the number of U+FFFD characters in Rust's
  and MuPDF's `text-all` (a count, no text). No file reads more U+FFFD
  than the baseline engine run, except files an owner decision names
  (T9). Owner decision T14 (2026-10-18) made this the gate; the number
  of files with more U+FFFD than the reference viewer is still reported,
  with U5's counting, but is not a target.
- The 3 files with `a<digits>` Type3 names (7.6) read with no U+FFFD from
  those fonts.
- pdf.js `SimFang-variant.pdf` and `XiaoBiaoSong.pdf` (Q2) are listed with
  their before/after class; a change there is accepted only with the
  owner's answer to Q2.
- Amended by 12.6 (U5): controls and noncharacters in the reference's
  text count as its U+FFFD; the `a<digits>` clause covers codes from 0x20.

### 8.4 Gates

`cargo test` (engine, WASM build) through `slot`; the
generator re-run gives byte-identical data files; the corpus run;
independent review that the implementation and the data were made without
the old files.

### 8.5 Comments, files and names to scrub or retire

- The old generators and oracles (`generate-simple-encoding.py`, which
  reads MuPDF source, `generate-text-unicode.py`, `probe-text-unicode.mjs`,
  `mupdf-encoding-oracle.c`, `build-encoding-reference.sh`,
  `differential-simple-encoding.py`, `differential-encoding-text.py`,
  `encoding-table-inventory.py`) moved to `engine/oracle/` in Phase 0
  (`differential-encoding-text.py` is in neither `tools/` nor `oracle/`
  any more); keep `engine/oracle/` out of the hand-off tree.
- `src/view/fonts_unicode.rs` and `tools/generate-view-font-unicode.py`
  (not in this workstream) take AGL values from hayro's embedded copy:
  fine as is; they could use the new AGL table instead.
- `tests/view_text.rs` comments on the Chinese-font case.
- `src/app/ops/edit/font.rs`: the comment about naming fonts "as MuPDF
  does" (spec 04).

---

## 9. Behaviour changes users would notice

1. **ZapfDingbats text** (check marks, arrows, bullets in forms) copies
   and searches as the dingbat characters (✔, ➔) instead of stray Latin
   letters and control characters.
2. **Glyph names written per the AGL spec** read fully: `T_h` → "Th",
   `uni00410042` → "AB", astral `uXXXXX` names, multi-character list
   entries.
3. **ToUnicode maps with one bad entry** keep their other entries, and
   one-byte destinations read as written: more text, fewer U+FFFD.
4. **Non-embedded CFF CID fonts with Identity ordering** read U+FFFD
   instead of guessed ASCII.
5. **Old Chinese fonts declared WinAnsi with GBK face names** (2 known corpus files) read as their
   declared WinAnsi encoding (Latin-1 letters), not as GBK Chinese, unless
   the owner answers Q2 otherwise.
6. **Type3 `a<number>` names** not equal to their code read U+FFFD.
7. **Vertical and small presentation forms** (U+FE10–U+FE6B) read as their
   ordinary punctuation, so search finds them.

---

## 10. Open questions for the owner

- **Q1.** Add the dependency `unicode-bidi` (shared with spec 04 Q6), or
  have the generator emit the bidi tables (this spec's default)?
- **Q2.** Old Chinese PDFs that declare a WinAnsi TrueType font with a
  GBK-encoded face name but show GBK double-byte text (pdf.js
  `SimFang-variant.pdf`, `XiaoBiaoSong.pdf`): the current engine reads
  them as Chinese; this spec drops that because the rule was not
  independently re-derived from observation. A clean
  rule could be designed from the observed need (a non-embedded simple
  font whose `/BaseFont` decodes as GBK Chinese and whose shown strings
  are valid GBK), with a separate experiment. Wanted?
- **Q3.** TeX glyph names that the AGL lacks (e.g. `integraldisplay`):
  some viewers read some of them (not measured here). The public `texglyphlist.txt` (lcdf-typetools) has
  them, but its licence must be checked. Add it?
- **Q4.** The licence of Adobe's `symbol.txt` and `zdingbat.txt` on
  unicode.org ("may be freely copied and redistributed" with the
  copyright notice). Acceptable, or derive the Symbol and ZapfDingbats
  built-in encodings from the URW AFMs (OFL) and the AGL instead?
- **Q5.** 3.5 rule 4 (WinAnsi reading of unnamed codes in symbolic
  TrueType fonts): keep as an implementer option tuned on the corpus, or
  require U+FFFD?
- **Q6.** 3.1: invert the embedded TrueType cmap for CID fonts without
  ToUnicode ("should"), or leave it to the interpreter?

---

## 11. Leak-check change log (2026-10-18)

Independent leak check against SPEC-RULES.md. Data tables are described
by public source, not copied; rules are tied to ISO 32000-2, the AGL
specification, OpenType and the Apple `post` order, or to section 7.
Edits made:

1. Header: replaced the list of old generator and evidence paths (stale
   since Phase 0, and a pointer) with a reference to 8.5.
2. 3.3 tolerant table: the "stray `>`" row had no G3 record in
   `exp05.txt`; the MuPDF cell now says "not recorded".
3. Q2: removed the statement about MuPDF's internal special case and
   "source reading"; it now describes the current behaviour only.
4. Q3: removed an unrecorded claim about what MuPDF knows.
5. 8.5: updated to the Phase 0 locations (`engine/oracle/`).
6. 8.5: "MuPDF's Chinese-font special case" reworded.

Hand-off prerequisites found (not spec text): the reference files named
in 8.1 are still in `engine/reference/` and must leave the hand-off tree;
`tests/view_text.rs` comments (the Chinese-font case) are scrubbed
first.

Residual note: the 3.1 rule "CIDFontType2 without a program, Identity:
CID below 258 is a Macintosh standard glyph index" matches a MuPDF
fallback (its G4 warning names such a mapping). It is kept because it is
grounded in the Apple `post` order and G4 records the outcome, but a
reviewer should confirm it is wanted (see also Q6).

Questions for the owner (gaps found, not filled):

- 3.2: "a position no range matches takes the shortest length for which
  the interpreter does the same" depends on hayro's behaviour, which the
  implementer must read from the vendored code; acceptable?
- 3.6: are `SymbolMT` and `Dingbats` the only aliases, or should other
  common names (for example `Wingdings`, which is not Symbol) be listed
  explicitly as not matching?
- 3.5 rule 4 (Q5) and 3.1 (Q6) decide observable output on the corpus;
  they need an answer before the corpus gate is meaningful.

---

## 12. Integration addenda (corpus run)

Added 2026-10-18 after the corpus run of the integrated rewrite. The run
counted U+FFFD per file (8.3) and found 72 files where the new code reads
more U+FFFD than the previous engine. Each addendum below states a rule
that is part of this spec from now on; where it changes an earlier
section, it says so and wins. Tests: `engine/tests/cleanroom_unicode_corpus.rs`.

### 12.1 Experiments

Method as 7.1. Scripts (self-authored) in the reviewer archive
`spec-05-addenda/`: `exp-t3.mjs`, `exp-t3b.mjs` (Type3 fonts built in the
script), `mu-text.mjs` (runs the reference on the PDFs the tests of this
section build, saved in `test-pdfs/`). Reference: MuPDF 1.28.1 (npm),
`toStructuredText("preserve-whitespace").asText()`, read as code points.
Raw output: `exp-t3.txt`, `exp-t3b.txt`, `exp-tests-ref.txt`. Run
through `slot`.

**E1.** Input: a Type3 font, one code shown per page; the code is named
in `/Differences` by the name given, or has no entry; every glyph a
filled box. Command: `node exp-t3.mjs`, `node exp-t3b.mjs`. Observed:

| Name | Codes 0x20, 0x21, 0x27, 0x2E, 0x41, 0x60, 0x61, 0x72, 0x7E | Codes 0x0B, 0x1F, 0x7F, 0x80, 0x95, 0xA1, 0xE9 |
|---|---|---|
| none, `foo`, `leaderdot`, `thindash`, `C72`, `D8`, `g65`, `striangle` | the code's ASCII character (U+0020, U+0021, U+0027, U+002E, …; not the StandardEncoding quotes at 0x27 and 0x60) | U+FFFD |
| `square`, `triangle` | U+25A1, U+25B3 at every code | the same |
| `rect` | not measured | U+FFFD (at 0xA1) |
| `circlecopyrt`, `dblarrowup` | not measured | U+20DD, U+21C8 (at 0xA1) |

**E2.** Input: the PDFs of the tests named below. Command:
`node mu-text.mjs test-pdfs/*.pdf`. Observed:

| Test PDF | Reference |
|---|---|
| `unnamed_glyphs_of_a_unicode_cmap_read_as_the_code` (3 fonts) | `ABC` each |
| `symbolic_truetype_unnamed_codes_read_as_win_ansi` | U+2019 U+2013 U+0041 U+00B0 |
| `an_inverted_cmap_glyph_of_several_characters_reads_the_lowest` | U+FFFD for all five glyphs (the reference does not read the program's cmap here) |
| `type3_a_number_names_at_control_codes_read_as_unknown` | U+0010, U+001C, U+0041 |
| `to_unicode_noncharacters_and_controls_are_not_text` | U+FFFF, U+FFFE, U+2126 |
| `type3_codes_without_a_usable_name_read_as_ascii` | as the test expects, except `/a65` at 0x42: `A` (this spec: `B`, 4.2) |

### 12.2 U1: Type3 codes without a usable name read as ASCII

Rule. In a Type3 font, a code whose glyph name is absent (no
`/Differences` entry), or does not resolve by 4.1 and is not settled by
4.2, reads as its own character when the code is printable ASCII
(0x20 to 0x7E: U+0020 to U+007E). Any other such code reads U+FFFD.
This replaces the last line of 3.1 step 3 for Type3 fonts ("a code that
none of these settle") for those codes; 4.1 and 4.2 still come first.

Basis. ISO 32000-2 §9.10.2 ends by letting a processor choose a reading
when its methods give no Unicode value. Programs that make Type3 fonts
(bitmap fonts of TeX and PostScript drivers, converters) keep the ASCII
codes of ASCII characters while naming glyphs freely (`leaderdot`, `C72`,
`D8`, `G065`). E1 shows the reference does the same, ASCII and not
StandardEncoding.

Consequences.

- 8.2 N10 and 4.2: `/g65` at 65 and `/a65` at 66 now read `A` and `B` (by
  the code, not the name). `/a67` at 162 still reads U+FFFD. The test
  `type3_names_of_another_code_read_as_unknown` now uses codes above 0x7E.
- Names outside the AGL that a fuller name list would resolve read as the
  code too (`square` at 0x61 reads `a`; E1: the reference reads
  U+25A1). T9 keeps that list out; if a list is added
  later, its names resolve by 4.1 and win.

Corpus: govdocs1 `000818.pdf` (2,159 leader dots), pdf.js
`issue14999_reduced.pdf`, `issue9655_reduced.pdf`, `Type3WordSpacing.pdf`
(unnamed 0x20), `SimFang-variant.pdf`, `issue5039.pdf`, `bug1011159.pdf`,
and 44 veraPDF files and 5 pdf.js files whose shape fonts name `square`,
`triangle`, `striangle`, `rect`.

Tests: `type3_codes_without_a_usable_name_read_as_ascii` (ignored until
implemented), `type3_glyph_list_names_win_over_the_code`.

### 12.3 U2: an unnamed glyph of a Unicode cmap subtable reads as the code

Rule. Addition to 3.5 steps 3 and 4. When the subtable chosen in step 1
is a Unicode subtable (platform 3 encoding 1 or 10, or platform 0) and
the code finds a glyph other than glyph 0 that has no usable name (no
`post` table, `post` format 3, or a name that does not resolve by 4.1),
the code reads as the Unicode character it was looked up as (U+00NN for
code NN). 3.7 applies to it (codes 0x80 to 0x9F give no text by this rule).
An `/Encoding` that is neither a name nor a dictionary (for example a
stream) counts as absent (3.4 rule 3).

Basis. OpenType `cmap`: these subtables map Unicode code points to
glyphs, so the glyph found for U+00NN is the font's glyph of that
character. E2: the reference reads `ABC` in all three test fonts.

Corpus: veraPDF `isartor-6-3-7-t01-fail-a.pdf`, pdf.js `issue6894.pdf`,
`issue14821.pdf` (non-symbolic subset TrueType fonts with a (3,1) subtable
and no glyph names).

Test: `unnamed_glyphs_of_a_unicode_cmap_read_as_the_code` (ignored until
implemented).

### 12.4 U3: a symbolic font's unnamed glyph reads as WinAnsi (owner, T11)

Rule. Change to 3.5 step 4. For a symbolic font, a code that finds a glyph
other than glyph 0 that has no usable name reads as the character
WinAnsiEncoding gives the code (Annex D, with its notes; 5.1), for every
code WinAnsiEncoding names, not only printable ASCII. A code WinAnsi
leaves undefined reads U+FFFD. Glyph 0 stays as step 4 says (the ASCII
reading optional).

Basis. The observation behind step 4: writers mark subset fonts symbolic
while showing WinAnsi codes, above 0x7E too (quotes, dashes, degree
signs). E2: the reference reads U+2019 U+2013 A U+00B0 for codes 0x92
0x96 0x41 0xB0.

Owner. T11 left step 4 to the implementer. With the printable-ASCII range
only, the corpus gate (8.3) fails on govdocs1 `000016.pdf`, `000025.pdf`,
`000028.pdf` and pdf.js `issue2017r.pdf` (apostrophes, dashes, degree
signs that the previous engine read): taking the option over the WinAnsi
range is needed to pass. This turns T11's option into a requirement; the
owner confirms.

Test: `symbolic_truetype_unnamed_codes_read_as_win_ansi` (ignored until
implemented and confirmed).

### 12.5 U4: a glyph that several characters map to (T12)

Rule. Change to 3.1 step 2, embedded `CIDFontType2` program. When the
program's Unicode cmap maps the glyph from more than one character, the
glyph reads as the lowest-valued of those characters outside the Private
Use Areas (U+E000 to U+F8FF, planes 15 and 16); when all are private use,
the lowest of them. A glyph mapped from exactly one character reads as
before.

Basis. Fonts give one glyph to an ordinary character and to its
compatibility or presentation variants: U+0020 and U+00A0, U+002D and
U+00AD, U+03A9 and U+2126, Arabic letters and their presentation forms.
UnicodeData.txt decomposes most such variants to the ordinary character
(U+00A0, U+2126, the presentation forms), and in every such pair the
ordinary character has the lower value; private-use values have no
meaning outside the font. E2: the reference does not read the program here
(U+FFFD for every glyph), so the rule is this spec's choice, best for
search and copy.

Corpus: pdf.js `ThuluthFeatures.pdf` (an embedded Times New Roman whose
space glyph is mapped from U+0020 and U+00A0).

Test: `an_inverted_cmap_glyph_of_several_characters_reads_the_lowest`
(ignored until implemented).

### 12.6 U5: the corpus gate and controls (8.3)

Rule. Two changes to 8.3.

1. When the runner compares U+FFFD counts, a character of the reference's
   text that 3.7 forbids as text counts as a U+FFFD of the reference: a C0
   control other than U+0009 to U+000D, U+007F, a C1 control (U+0080 to
   U+009F), U+FFFE and U+FFFF. Where the reference shows such a character
   and this spec reads U+FFFD, the file is not "more U+FFFD".
2. The clause "the 3 files with `a<digits>` Type3 names read with no
   U+FFFD from those fonts" applies to codes 0x20 and above. Below 0x20,
   4.2 gives a control, which 3.7 drops; the code reads U+FFFD (U+0009 to
   U+000D read a space).

Basis. 3.7 (controls and noncharacters are not text). E2: the reference
reads U+0010 and U+001C for `/a16` and `/a28`, U+FFFF and U+FFFE for
ToUnicode destinations `<FFFF>` and `<FFFE>`.

Corpus: pdf.js `issue918.pdf` (`/a16`, `/a17`, `/a28`, `/a30`),
`arial_unicode_ab_cidfont.pdf` (four codes mapped to U+FFFF); veraPDF
`6-2-11-7-2-t02-fail-b`, `6-2-10-7-t01-fail-b`, `7.21.7-t02-fail-b`,
`8.4.5.8-t02-fail-b` (a code mapped to U+FFFE). These keep reading U+FFFD.

Tests: `type3_a_number_names_at_control_codes_read_as_unknown`,
`to_unicode_noncharacters_and_controls_are_not_text` (pass now; they pin
the rule).

### 12.7 Corpus findings that need no addendum

- **Count of characters, not their reading** (spec 04): pdf.js
  `complex_ttf_font.pdf` (programs with neither `cmap` nor `post`: every
  glyph U+FFFD in every engine), `ThuluthFeatures.pdf` (Arabic fonts with
  only symbol cmaps, all U+FFFD in every engine), `issue5874.pdf` (codes
  the ToUnicode leaves unmapped, no `cmap`), `TAMReview.pdf` (two more
  than before). The difference is how many characters the layout emits for
  those glyphs; it belongs to spec 04's review.
- **Names outside the AGL in embedded Type1 programs** (T9): pdf.js
  `tracemonkey*.pdf` read code 0x0D of a TeX symbol font, named
  `circlecopyrt`, as U+00A9 (the interpreter's reading, 3.1 step 3); the
  reference reads U+20DD (E1). No U+FFFD; not worse for
  users. pdf.js `bug1068432.pdf`: names `square`, `dblarrowup`,
  `dblarrowdwn` at control codes read U+FFFD (E1: the reference reads
  `square` and `dblarrowup` as U+25A1 and U+21C8). Both depend on T9.

### 12.8 U6: whitespace controls through a Unicode cmap (second corpus run)

Rule. Change to U2. Through U2, a code 0x09 to 0x0D reads U+0020 (3.7)
only when the glyph it finds draws nothing: in `glyf`, a glyph with no
outline data (its two `loca` offsets are equal) or with no contours. When
that glyph draws something, the code gives no text by U2, like the other
controls, and the next source of 3.5 step 4 (and U3) decides; with none it
reads U+FFFD. The other control codes (0x00 to 0x08, 0x0E to 0x1F, 0x7F
to 0x9F) stay as U2 says.

Basis. OpenType fonts draw the whitespace controls with empty glyphs.
A program whose Unicode subtable sends U+000B to a glyph with an outline
is a subset that reuses low codes for its own glyphs, so the code's
character says nothing about the glyph. Reading it as a space hides an
unknown glyph and splits a word in search and copy; U+FFFD marks it. The
reference reads U+FFFD for every control code here, the empty-glyph ones
too (E3); this spec keeps the space for an empty glyph, which is what the
page shows.

Corpus: pdf.js `issue5501.pdf` (a symbolic subset TrueType font with a
platform 0 subtable, `post` format 3: code 0x0B reads a space where the
reference reads U+FFFD).

Test: `unicode_cmap_whitespace_controls_read_a_space_only_for_blank_glyphs`
(`cleanroom_unicode_corpus.rs`; ignored until implemented).

**E3.** Method as 7.1. Input: the PDFs the test builds (the TrueType
program of the test: glyphs 1 and 3 a filled triangle, glyph 2 empty; a
(3,1) format 4 subtable mapping 0x04 and 0x41 to glyph 3, 0x09 and 0x0A to
glyph 2, 0x0B and 0x0D to glyph 1; `post` format 3; one code per line;
descriptor `/Flags` 32 and 4), saved by the reviewer from the test.
Command: `slot node mu-text.mjs test-pdfs/u6_flags32.pdf
test-pdfs/u6_flags4.pdf`. Script, PDFs and raw output (`exp-u6.txt`) in
`reviewer archive corpus-addenda-2026-10-18-r2/`. Observed, both flags:

| Code | 0x0B | 0x0D | 0x09 | 0x0A | 0x04 | 0x41 |
|---|---|---|---|---|---|---|
| Glyph | outline | outline | empty | empty | outline | outline |
| Reference | U+FFFD | U+FFFD | U+FFFD | U+FFFD | U+FFFD | U+0041 |
| This spec | U+FFFD | U+FFFD | U+0020 | U+0020 | U+FFFD | U+0041 |

Correction (2026-10-18, review of the U6 implementation): the PDFs E3 ran
were built with the test's first fixture, whose single segment for 0x09
and 0x0A sent 0x0A to glyph 3 (an outline), not to the empty glyph 2. The
0x0A column above therefore describes the intended fixture, not the run.
The reference read U+FFFD for every code, outlined or empty, so the
conclusion is unchanged; the test now maps 0x0A to glyph 2 by its own
segment.

### 12.9 Second corpus run: findings that need no addendum

- pdf.js `issue11922_reduced.pdf` (page text now empty, the reference
  shows ten U+FFFD): an embedded `CIDFontType2` program whose ToUnicode
  maps every code to an empty destination (3.3: falls back) shows one CID
  ten times; its glyph is the one the program's Unicode cmap maps from
  U+0020 and U+00A0, so U4 reads U+0020, and the page is ten spaces.
  Expected by U4 and right: that glyph is the font's space.
- pdf.js `issue15910.pdf`: two characters U+00E4 now read where the
  previous build read U+FFFD (U2 on a symbolic subset TrueType font). They
  are Latin-1 letters, not C1 controls: the corpus probe's label
  "U+0080-block" covers U+0080 to U+00FF. Improvement. The line-order
  change on the same page is spec 04 C5 rule 1.
- pdf.js `issue918.pdf` (11 U+FFFD): each sits where the reference shows
  U+0010, U+0011, U+001C or U+001E, from Type3 names `a16`, `a17`, `a28`,
  `a30`. Expected by U5 rule 2 (and U5 rule 1 for the gate); not a 4.2
  fault.
- pdf.js `arial_unicode_ab_cidfont.pdf`: four U+FFFD where the reference
  shows U+FFFF. Expected by U5.

### 12.10 U7: Private Use codes of a Unicode encoding CMap (third corpus run)

Rule. Made precise for 3.1 step 2. A composite font whose encoding is a
predefined Unicode CMap (the `Uni…-UCS2-…` and `Uni…-UTF16-…` CMaps of
§9.7.5.2, Table 116) and whose CIDFont names an Adobe character
collection with a UCS2 CMap may read each code as the UTF-16 text it is
(a shortcut that agrees with 3.1 step 2 for nearly every code), with one
exception: a code that is a Private Use code point (U+E000 to U+F8FF,
planes 15 and 16) reads through its CID and the collection's
`Adobe-<Ordering>-UCS2` CMap, as 3.1 step 2 says, whenever that gives a
character outside the Private Use Area. Otherwise the code itself.

Basis. Writers of Chinese text encode the Hong Kong supplementary
characters by their legacy Private Use code points; the Adobe CMaps map
those codes to collection CIDs whose Unicode is the standard code point.
A Private Use character is neither searchable nor portable; the CID route
gives the character the page shows. Adobe's public CMap files:
`UniCNS-UTF16-H` and `UniCNS-UCS2-H` map `<E687>` to CID 14975, and
`Adobe-CNS1-UCS2` maps CID 14975 to U+2555B (`<D855DD5B>`).

Observed (E-U7, black box, MuPDF 1.28.1 npm): the PDFs the test builds
(non-embedded `MingLiU`, CIDFontType2, Adobe-CNS1-4, shows
`<4E2DE687>`), command `slot node mu-text.mjs test-pdfs/u7_*.pdf`, read
U+4E2D U+2555B with either CMap (raw output `exp-u7.txt` in
`reviewer archive corpus-addenda-2026-10-18-r3/`). This tree reads U+4E2D
U+E687. Corpus: pdf.js `issue19182.pdf` (one character).

Test: `u7_a_private_use_code_of_a_unicode_cmap_reads_through_its_cid`
(`cleanroom_unicode_corpus.rs`).

### 12.11 Third corpus run: findings that need no addendum

- **The "euro" cluster is `circlecopyrt`.** The 16 files counted as one
  wrong character each (pdf.js `tracemonkey.pdf` and its five variants,
  `comments.pdf`, `highlights.pdf`, `bug1885505.pdf`, `bug1992868.pdf`,
  `issue7014.pdf`, `issue12337.pdf`, `issue14438.pdf`, `issue15012.pdf`,
  `issue18911.pdf`, `issue19239.pdf`) all show one copyright notice set in
  a TeX symbol font: code 0x0D named `circlecopyrt`, with a `c` drawn
  inside. The reference reads U+20DD (combining enclosing circle), this
  tree U+00A9 (the interpreter's reading, 3.1 step 3). No WinAnsi or
  U+20AC path is involved. Already in 12.7 (T9); `©` followed by `c` is
  at least as useful to a reader.
- **ZapfDingbats bullets** (5.4, section 9 item 1): govdocs1 `000859.pdf`
  (41 characters on 12 pages), `000282.pdf`, `000444.pdf`, `000886.pdf`.
  A non-embedded `ZapfDingbats` without `/Encoding` shows bullets and
  boxes (code 0x6C, for example); this tree reads U+25CF and U+2751 (the
  shapes the page shows), the reference Latin letters (G2).
- **ActualText of structure elements** (spec 04 3.7): pdf.js
  `bug1997343.pdf` reads `LaTeX` and `TeX` from the `/ActualText` of the
  structure elements the MCIDs name; the reference reads the glyphs
  (`L A T E X`).
- **Names outside the AGL** (T9): qpdf `issue-202.pdf` page 9, two glyphs
  of TeX math fonts the reference reads as U+0338 and U+22C6; this tree
  reads U+FFFD.
- **Presentation forms** (4.3): pdf.js `issue4061.pdf`, U+FE30 becomes
  `..` by its compatibility decomposition.
- **Improvement over the reference**: pdf.js `90ms_rksj_h_sample.pdf`
  reads its Japanese sample text; the reference reads Latin letters and
  U+FFFD there.
- **Outside the rewritten code** (vendored interpreter):
  - TWG `A025-pdfa2-fail-a.pdf`: an embedded encoding CMap whose
    `/UseCMap` is another embedded CMap stream (§9.7.5.3; PDF/A forbids
    it, ISO 32000 allows it). The vendored interpreter does not load
    CMaps named by `usecmap` from the file, so it splits the two-byte
    codes into single bytes and draws them in a fallback face; the text
    follows its split (3.2) and reads U+FFFD and Latin letters where the
    reference reads `Hello world`. A patch to the vendored interpreter.
  - pdf.js `issue12823.pdf`: see spec 04 §12 third-run item 4.
