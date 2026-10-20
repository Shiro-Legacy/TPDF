# Spec 03: tolerant open and repair

Workstream 3 of `docs/mit-clean-room-plan.md`. Status: draft for review,
2026-10-17. Rules: `docs/clean-room/SPEC-RULES.md`.

This spec replaces `engine/src/repair.rs`. ISO 32000-2 §7.5 (file
structure) defines only well-formed files. It says nothing about repair, so
everything in sections 4 and 5 is a new design. That design is driven by
what users should see and by black-box experiments recorded in section 7.
It is not a description of any existing implementation.

The implementer may read: this spec, ISO 32000-2, the acceptance tests
listed in section 8, the experiment inputs in section 7, and the surviving
engine code (everything except the class A files of the plan). The
implementer may not read the current `repair.rs`, its git history, the
`engine/design/*` notes about it, or MuPDF source.

---

## 1. Scope

### 1.1 What the component does

Given the bytes of a file that the strict reader (`crate::document::Document`)
refuses, the component produces the bytes of **one well-formed revision**
that the strict reader accepts. It also produces a report saying what it
did, and whether the input was *damaged* (section 5.2).

The strict reader does not change. It accepts only classic tables, xref
streams, incremental updates and bounded object streams, laid out exactly
as §7.5 describes. It refuses:

1. **Damaged files.** The xref is missing or wrong, `startxref` is wrong,
   a body or trailer is truncated, stream lengths are wrong, objects are
   cut short, there is junk inside the structure, and so on.
2. **Valid or near-valid layouts it does not implement.** These include
   linearized files (§7.5.6 / Annex F), hybrid-reference files (§7.5.8.4),
   offsets that point at the line end before an object, text after
   `%%EOF`, a missing `%%EOF`, junk between objects, duplicate dictionary
   keys, references naming the wrong generation, and lexical damage that
   the lenient parser reads through.

The component handles both kinds. For the second kind, the result is the
file's own content, read through its own cross-reference data. It is *not*
reported as damaged.

### 1.2 Callers (surviving code)

| Caller | Uses | Needs |
|---|---|---|
| `app::document::AppDocument::load_with` (`engine/src/app/document.rs`) | `rebuild_with_passwords`, `RepairLimits::for_document`, `RepairError`, report fields `damaged`, `linearized`, `xref_sections`, `password_dependent` | Opens strictly first and calls the repair only after a refusal that is not a size limit. `damaged` becomes the app's `repaired` flag (`DocumentInfo.repaired`, shown in the UI). When `damaged` is false, the app counts `versions` as `xref_sections` minus one if `linearized`, and at least 1. When `damaged` is true, `versions` is 1. `password_dependent` decides whether a wrong password can be answered without a reload. |
| `app::ops::sign` (`extendable`, `rebuilt`) | `rebuild`, `RepairedPdf`, report `damaged`, `xref_size` | sign-prepare appends an incremental update *after the original bytes* of a signed file. This works only when the rebuild was not `damaged`, because then the original file's own xref is valid and can be extended. `xref_size` is the first object number the update may use. |
| `app::info::is_node` | `untyped_kid_is_node` | One rule for "is this untyped page-tree kid a node?", shared by the repair and the app's page walk, so that a file reads the same whether or not it was repaired. |
| `examples/app_save_probe.rs` | `open_tolerant` | Strict open, or a rebuild if that fails. |
| Tests (section 8) | All of the above, plus `rebuild`'s report fields | |

### 1.3 Surviving crate APIs the component builds on

- `document::{Document, DocumentError, DocumentErrorKind, DocumentLimits, OpenOptions}`.
  These are the strict reader that the output must satisfy, and the
  caller's limits.
- `parser::{Parser, ParseLimits, ...}`, in its *lenient* mode. Workstream 1
  specifies that mode: damaged numbers and keywords read as null, a non-hex
  digit in a hex string reads as 0, `#` without two hex digits in a name
  stands for itself, and the last of duplicate keys wins. The repair parses
  every value it reads with that mode, and values it reads that way are not
  damage.
- `stream` filters (workstream 2), to decode object streams and xref
  streams and to check stream extents (section 4.6).
- `security::{Security, resolve_encryption}` and
  `encrypt::{OutputCipher, Drbg}`, to authenticate passwords and to decrypt
  and re-encrypt object-stream members (section 4.12).
- `serialize::Writer` and related items (workstream 1), to write the output.
- `incremental::startxref`. This is the single rule for locating the last
  `startxref`. The repair must use it, not a locator of its own (section 4.3).

### 1.4 Non-goals

- Recovering content from inside streams. Damaged content streams, fonts
  and images are the view's problem.
- Writing a linearized, compressed or incremental result. The output is
  internal: the app's save always rewrites the document.
- Any change to the strict reader.

---

## 2. Interface

The module path stays `crate::repair` (`engine/src/repair.rs`). It may
become a directory module. Every item below is `pub` unless noted, with
exactly these names and types. Private items are the implementer's choice.

### 2.1 `RepairLimits`

```rust
#[derive(Clone, Copy, Debug)]
pub struct RepairLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    /// Object definitions kept while reading, superseded ones and
    /// object-stream members included.
    pub max_objects: usize,
    /// Parsed values across all definitions and trailer dictionaries,
    /// dictionary keys included.
    pub max_values: usize,
    pub max_object_streams: usize,
    /// Bytes decoded from object streams, xref streams and stream-extent
    /// checks, across the whole input.
    pub max_decoded_bytes: usize,
    /// NEW. Cross-reference sections followed in the file's own chain
    /// (section 4.3). A longer chain is not followed; the file is
    /// reconstructed instead.
    pub max_revisions: usize,
    /// Limits for each parsed value.
    pub parse: ParseLimits,
}
impl Default for RepairLimits { /* see below */ }
impl RepairLimits {
    pub fn for_document(limits: DocumentLimits) -> Self;
}
```

- `for_document(l)` returns limits no looser than the ones the rebuilt file
  must later open under:
  - `max_input_bytes` = `max_output_bytes` = `l.max_input_bytes`
  - `max_objects` = `l.max_xref_entries + l.max_compressed_members` (saturating)
  - `max_values` = `l.max_objects`
  - `max_object_streams` = `l.max_object_streams`
  - `max_decoded_bytes` = `l.max_metadata_decoded_bytes`
  - `max_revisions` = `l.max_revisions`
  - `parse` = `l.parse`
- `Default` is `for_document(crate::app::document::limits())`. These are
  the limits the app opens files with, so a test that uses the defaults
  exercises what users get.
- `max_revisions` is new. Every existing literal in the tests uses
  `..RepairLimits::default()`, so adding the field breaks nothing.

### 2.2 `RepairReport`

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepairReport { /* fields below, all pub */ }
```

| Field | Type | Meaning (exact) |
|---|---|---|
| `objects_recovered` | `usize` | Indirect objects written to the output. Object 0, synthesized objects (section 4.8 and 4.10), dropped xref streams, object-stream containers and linearization objects are not counted. |
| `objects_overridden` | `usize` | Readable definitions in the input that are not in the output because another definition of the same number won (section 4.4), or, when `xref_intact`, because the chain does not select them (superseded, freed, or appended without an xref). Each definition counts once. An object-stream member counts like a top-level definition. |
| `objects_dropped` | `usize` | Object headers whose value could not be read, object-stream members that could not be read or that share a byte offset with an earlier member of the same container, and containers that could not be decoded. Each counts once. |
| `object_streams_expanded` | `usize` | Object-stream containers whose members were written as top-level objects. |
| `streams_length_recovered` | `usize` | Streams written whose data extent was not given by a usable `/Length` (section 4.6). |
| `references_repaired` | `usize` | References inside written objects that were changed: a dangling reference replaced by null or removed from a list, or a reference with another generation pointed at the surviving object (section 4.9). References in the trailer are not counted. |
| `catalog_from_scan` | `bool` | No trailer `/Root` named a usable catalog. The catalog was chosen by type or synthesized (section 4.8). |
| `page_tree_repaired` | `bool` | The output's page tree differs from the input's in any way: kids removed, `/Count`, `/Parent` or `/Type` corrected, `/MediaBox` supplied, blank pages kept for unreadable pages, or a tree built (section 4.10). This is a diagnostic. See 5.2 for which of these count as damage. |
| `linearized` | `bool` | A linearization parameter dictionary was dropped, with its hint streams (section 4.11). |
| `xref_intact` | `bool` | The output's objects are the ones the file's own chain selects (section 4.3). |
| `objects_unreadable` | `usize` | Objects the chain locates at a correct header whose value could not be read. They are left out, except listed pages, which become blank pages (4.10). |
| `xref_sections` | `usize` | Sections in the chain that was read. A hybrid table and its `/XRefStm` stream count as one section. A linearized file's first-page section counts. 0 when no chain was read. |
| `xref_size` | `u64` | One past the largest object number the chain accounts for: the largest `/Size` in it, or the largest number it lists, whether in use or free, whichever is larger. 0 when no chain was read. |
| `damaged` | `bool` | Section 5.2. |
| `strict_error` | `Option<String>` | `None` from `rebuild*`. `open_tolerant` sets it to the strict reader's refusal, rendered with `Display`. |
| `locked` | `bool` | The input is encrypted, has object streams, and no supplied password authenticates. The output holds the structure only (section 4.12). |
| `password_dependent` | `bool` | The input is encrypted and has at least one stream that can hold objects (one with `/Type /ObjStm`, or with integer `/N` and `/First`). When false, rebuilding with any password list gives identical bytes. |

### 2.3 `RepairedPdf`, `RepairError`

```rust
#[derive(Debug)]
pub struct RepairedPdf { pub bytes: Vec<u8>, pub report: RepairReport }

#[derive(Debug)]
pub enum RepairError {
    Invalid(&'static str),
    Unsupported(&'static str),
    LimitExceeded(&'static str),
    Serialize(SerializeError),
    /// The rebuilt bytes did not open strictly, or `open_tolerant` could not
    /// open the input (a strict limit) or the repaired file under the
    /// caller's limits.
    Document(DocumentError),
}
impl fmt::Display for RepairError { .. }
impl std::error::Error for RepairError { .. }   // source(): Serialize, Document
impl From<SerializeError> for RepairError { .. }
```

`Display` strings are part of the interface, because the app shows them
inside `open: the file could not be read (…)`:

- `Invalid(r)` displays `PDF repair: invalid {r}`
- `Unsupported(r)` displays `PDF repair: unsupported {r}`
- `LimitExceeded(r)` displays `PDF repair: {r} limit exceeded`
- `Serialize(e)` displays `PDF repair: {e}`
- `Document(e)` displays `e`'s own text

Reason strings that tests match exactly, plus the ones the app shows:

| Reason | Variant | When |
|---|---|---|
| `"document catalog"` | `Invalid` | No catalog and no page object could be found (section 4.8). |
| `"encryption without the file identifier"` | `Invalid` | The input is encrypted with a handler whose key derivation uses the first `/ID` element (§7.6.4.3, revisions 2–4), and no `/ID` survives (section 4.12). |
| `"object-stream member strings"` | `Invalid` | A member of an encrypted object stream cannot be re-encrypted for output. |
| `"input bytes"` | `LimitExceeded` | `input.len() > max_input_bytes`. Checked before any work. |
| `"objects"` | `LimitExceeded` | More than `max_objects` definitions. |
| `"values"` | `LimitExceeded` | More than `max_values` parsed values. |
| `"object streams"` | `LimitExceeded` | More than `max_object_streams` containers to decode. |
| `"object-stream bytes"` | `LimitExceeded` | Decoded bytes over `max_decoded_bytes`. |
| `"object streams in an encrypted document"` | `Unsupported` | Only from `open_tolerant`, when the rebuild is `locked`. |

If the output would exceed `max_output_bytes`, the result is a `Serialize`
error, as the writer reports it.

### 2.4 `TolerantPdf` and `open_tolerant`

```rust
#[derive(Debug)]
pub struct TolerantPdf {
    pub bytes: Vec<u8>,
    pub repair: Option<RepairReport>,
    pub strict_error: Option<DocumentError>,
    pub original: Option<Vec<u8>>,
    pub needs_password: bool,
}
pub fn open_tolerant(input: Vec<u8>, limits: DocumentLimits, password: &[u8])
    -> Result<TolerantPdf, RepairError>;
```

1. If `input.len() > limits.max_input_bytes`, return `Err(LimitExceeded("input bytes"))`.
2. Open strictly with `Document::open_with_password(&input, limits, password)`.
   - `Ok`: return the input unchanged, with `repair: None`,
     `strict_error: None`, `original: None` and `needs_password: false`.
   - `PasswordRequired`: the same, with `needs_password: true`.
   - `LimitExceeded(_)`: return `Err(Document(error))`. A strict limit is
     the caller's decision. It is not damage, and the file is never
     flattened behind the caller's back.
   - Any other error: go to step 3.
3. Run `rebuild_with_passwords(&input, RepairLimits::for_document(limits), &[password])`.
   If the report is `locked`, return
   `Err(Unsupported("object streams in an encrypted document"))`.
   Otherwise set `report.strict_error = Some(strict.to_string())`.
4. Open the rebuilt bytes strictly with `limits` and `password`.
   `PasswordRequired` gives `needs_password: true`. Any other error gives
   `Err(Document(e))`.
5. Return `bytes` = the rebuilt bytes, `repair: Some(report)`,
   `strict_error: Some(strict)` and `original: Some(input)`.

A password that does not authenticate is **not** a reason to rebuild.

### 2.5 `rebuild`, `rebuild_with_passwords`

```rust
pub fn rebuild(input: &[u8], limits: RepairLimits) -> Result<RepairedPdf, RepairError>;
pub fn rebuild_with_passwords(input: &[u8], limits: RepairLimits, passwords: &[&[u8]])
    -> Result<RepairedPdf, RepairError>;
```

- `rebuild(i, l)` is the same as `rebuild_with_passwords(i, l, &[])`.
- An empty `passwords` list means `[b""]`. The passwords are tried in
  order, and the first one that authenticates is used to read encrypted
  object streams.
- Either function accepts **any** input, well-formed ones included.
  `rebuild` on a valid file returns that file's content as one revision,
  with `xref_intact` true and `damaged` false.
- Postcondition: `Document::open(&bytes, …)` succeeds, or fails only with
  `PasswordRequired`, under structural limits no tighter than the
  `RepairLimits` given. The function must check this before it returns. An
  output that would not open is returned as `Err(Document(e))`, never as
  `Ok`.
- The function is deterministic (section 5.1).

### 2.6 `untyped_kid_is_node`

```rust
pub(crate) fn untyped_kid_is_node(kids: Option<usize>, contents: bool, media: bool) -> bool;
```

This decides whether a page-tree kid with no `/Type`, or a null `/Type`, is
an intermediate node.

- `kids` is `None` when `/Kids` is missing or null. Otherwise it is the
  number of entries, and 0 when `/Kids` is not an array.
- `contents` is true when `/Contents` is a reference or an array.
- `media` is true when `/MediaBox` is present.

Result: `false` when `kids` is `None`. `!(contents || media)` when `kids`
is `Some(0)`. `true` otherwise.

So a kid with entries in `/Kids` is a node. A kid with an empty `/Kids` is
a node unless it looks like a page. A kid without `/Kids` is a page.

`app::info::is_node` calls this function, and the repair must classify
kids with it too (section 4.10).

---

## 3. When repair runs, and the two kinds of result

### 3.1 Trigger

Callers always try the strict reader first. The repair runs only after a
refusal that is not a password and not a size limit. `open_tolerant` makes
that decision itself (2.4). `AppDocument::load_with` makes it in surviving
code. That code also retries the strict reader after skipping a short
prefix of junk before `%PDF-`, and treats a reference to another
generation as a refusal.

### 3.2 Result kinds

| Kind | When | `xref_intact` | `damaged` |
|---|---|---|---|
| **Read through its own xref** | The file's own cross-reference chain is usable (4.3) | true | false, unless something in 5.2 applies (for example, an object the chain locates is unreadable) |
| **Reconstructed** | No usable chain | false | true |

Both kinds produce one strict revision with the same output contract
(section 5).

---

## 4. Required behaviour

### 4.1 Principles

Every rule below follows from these, in order of priority.

- **P1. Never lose an intact object because of damage elsewhere.** An
  object whose own bytes are well-formed must survive in the output,
  whatever damage comes before or after it. This is the main purpose of
  repair.
- **P2. Follow the file's own word when it is usable.** When the file's
  cross-reference data describes its objects, show exactly the revision it
  describes (ISO 32000-2 §7.5.4–7.5.8). Objects outside that revision stay
  out, even if they are present in the bytes. For a signed file, this is
  the signed revision.
- **P3. Newest wins.** When the cross-reference data is unusable and two
  definitions of an object compete, the one written later in the file
  wins. Incremental updates append (§7.5.6), so later bytes are newer.
- **P4. Same document, same reading.** A file reads the same (same pages,
  page order, page objects and text) whether its xref is intact or broken,
  as long as the objects themselves are intact. Page-tree kids are
  classified the same way on both paths.
- **P5. Never show ciphertext as text.** An encrypted file is either
  decrypted correctly or refused. It is never read as plaintext.
- **P6. Bounded.** Time and memory grow linearly with the input and stay
  within `RepairLimits` (section 6).
- **P7. Say when content was guessed or lost.** That is the `damaged` flag
  (5.2). Layouts that are merely unusual are not damage.

### 4.2 Header, version and bytes before the header

- The header is the first `%PDF-` that starts the input or follows only
  junk bytes. The search must reach at least as far into the file as the
  app's own leading-junk tolerance (`app::document` skips a short prefix
  before retrying the strict reader). How much further it reaches is the implementer's
  choice. Experiment 09 (2,000 junk bytes) must still recover all pages.
- Bytes before the header are not part of the document. Offsets in the
  file's xref are taken relative to the header (experiment 08; app test
  `junk_before_the_header_is_skipped_without_repair`). Offsets that count
  the junk do not describe the file, so the file is reconstructed.
- The output's header keeps the input's version when it is one ISO 32000
  defines (1.0–1.7, 2.0). Otherwise the output says 1.7: the body may use
  any PDF 1.x feature, and saying 2.0 would claim conformance the input
  never declared.
- A header not at byte 0 makes the result `damaged` (5.2). If no header is
  found, the input is read from byte 0. It opens if objects and a catalog
  or pages are found (experiment 93).

### 4.3 Following the file's own cross-reference chain

**Locate.** The newest section is the one named by the last `startxref`,
as `crate::incremental::startxref` finds it. Using that function is
required. It means the reader and the incremental writer (sign-prepare)
agree on which revision is last: a file whose last `startxref` the writer
cannot find is never read through its xref here either.

**Follow.** Read sections from that one back through `/Prev`, as
§7.5.6–7.5.8 describe:

- Classic tables, xref streams, and hybrid files. In a hybrid file, the
  table's in-use entries come first, and the `/XRefStm` stream supplies
  entries for numbers the table omits or marks free (§7.5.8.4).
- Linearized files: the first-page section's `/Prev` may point forward
  (Annex F).
- Within the newest-first view, an entry in a newer section hides any
  entry for that number in older sections. A free entry means that object
  is absent (§7.5.4).
- Follow at most `max_revisions` sections. A `/Prev` loop ends the chain.
  A loop is not damage if every section read so far is usable.

**Usable.** The chain is usable when every section parses and every in-use
entry in the newest-first view locates a definition of that object number:

- a classic entry's offset lands on its `N G obj` header, or on whitespace
  that runs straight up to that header (experiments 06 and a0: an offset
  one or two bytes early, at the EOL before the header, is accepted, not
  damage);
- a compressed entry names a container that is itself located, decodes,
  and holds the member at that index.

The same whitespace slack applies to `startxref` itself (experiment a1).
An offset that lands *inside* a header or another object is unusable
(experiments 07, a2, a4).

A different generation in the header than in the entry is tolerated and
handled as in 4.9.

**Containers the chain names** are read whatever their `/Type`. Their
filter may be written `/Fl` (the abbreviation of §8.9.7). Bytes after the
end of the compressed data are ignored (experiments i0–i2: MuPDF reads all
three).

**Result.** When usable, the output holds exactly the objects the chain
selects:

- Objects in the body that the chain does not select are not written.
  This covers definitions appended after the last revision with no xref
  section of their own, superseded definitions, and objects a section
  frees. Experiments 22, 24 and 26: both MuPDF and today's engine show the
  last complete revision, unrepaired. App tests
  `objects_appended_without_an_xref_leave_the_last_complete_revision` and
  `a_signed_file_with_an_appended_object_shows_the_signed_revision`.
- Trailer entries come from the chain's trailers, newest first (4.7).
  Dictionaries elsewhere in the file are ignored, whether or not they look
  like trailers (app test `loose_dictionaries_do_not_override_an_intact_xref`;
  experiments j0 and j1).
- Each selected object is read from its own location. Its value never
  extends into another object the chain locates (P1; app test
  `a_header_glued_to_a_damaged_object_keeps_an_intact_xref`).

**Not usable.** If any of the above fails, reconstruct (4.4). Reading the
chain still fills `xref_sections` and `xref_size` when the chain itself
could be parsed.

### 4.4 Reconstruction: finding objects and choosing among them

When there is no usable chain, the document is rebuilt from the object
definitions found in the bytes. How the input is scanned is the
implementer's choice, within section 6. The scan must meet these criteria:

1. **What is a definition.** An `N G obj` header begins a definition when
   it occurs at a token boundary outside any string, comment or stream
   data of another object. A header inside a string literal is never a
   definition (experiment 60; test `keywords_inside_strings_do_not_cut_objects`).
   The same goes for the words `trailer`, `endobj`, `stream` and
   `endstream` inside strings.

2. **A header inside a complete object is part of that object.** If an
   enclosing object's value still parses completely to its end (`endobj`,
   or the next definition) when the header is read as damaged tokens, the
   header is not a definition. The enclosing object keeps all of its
   entries (experiments 61 and 62; tests
   `an_object_header_inside_a_damaged_object_does_not_replace_the_object`
   and m10 `a_header_read_as_a_damaged_value_does_not_replace_an_object`).

3. **A cut-short object ends where a complete definition begins.** A value
   that is unterminated (lost `>>`, `]` or `endobj`) ends before the next
   header whose own definition is complete. That later object is kept
   whole (experiments 63, 64 and 94; tests
   `a_dictionary_missing_its_end_keeps_the_next_object` and
   `fixtures_with_a_dictionary_missing_its_end_keep_their_objects`). The
   cut-short object may be kept with the entries it completed, or dropped.
   Either way, it must not take the next object with it.

4. **Ranking.** Among definitions of the same object number, a complete
   definition outranks a fragment: one whose value only parses by reading
   through damage. Among definitions of equal standing, the one later in
   file order wins (P3), *whatever its generation*:
   - experiment 30: two generation-0 definitions, the later one wins;
   - experiment 31: a later generation-1 definition wins over an earlier
     generation-0 one;
   - experiment 32: a later generation-0 definition wins over an earlier
     generation-3 one.

   MuPDF and today's engine agree on all three. The winner keeps its own
   generation in the output.

5. **Object streams.** Every container found is expanded (4.12 covers
   encrypted ones), and its members compete *at the container's position
   in the file*: a member beats top-level definitions before the
   container, and loses to those after it (experiments 84 and 85, both
   engines agree; tests `object_stream_members_compete_at_the_container_position`
   and m10 `an_object_stream_member_outranks_a_read_through_header_not_an_update`).
   - A member never outranks a complete top-level definition that comes
     later.
   - A fragment never outranks a member.
   - A container inside another container is not expanded (§7.5.7
     forbids it).
   - Members that share one byte offset are read once, and only the first
     is kept. Members that cannot be read are dropped.
   - Containers and xref streams are never written to the output.

6. **Unparsable definitions.** A header whose value cannot be read, even
   leniently, is dropped. The number stays free for other definitions.

7. **Xref streams as trailers.** Each xref-stream dictionary found counts
   as a trailer for section 4.7, as does each dictionary that follows a
   `trailer` keyword, or follows a damaged keyword in that place outside
   any object (experiment 40 and the encrypted `g-*-trailer-keyword-damaged`
   cases).

   One more case: a dictionary outside every object (between objects or
   after `%%EOF`) that holds `/Root`, `/Info`, `/ID` or `/Encrypt` also
   counts, but *only* during reconstruction. That keeps the identifier of
   an encrypted file whose `trailer` keyword is damaged (app test
   `a_damaged_trailer_keyword_keeps_the_file_identifier`).

### 4.5 Values

- Every value is parsed in the lenient mode (1.3). What that mode reads is
  not damage. For example, a damaged token in a page dictionary leaves the
  page and its text, and an intact xref stays unrepaired (app tests
  `a_damaged_token_*`, `a_catalog_with_a_damaged_type_opens`,
  `a_name_escape_without_two_hex_digits_stands_for_itself_in_repair`).
- For duplicate keys, the last value wins. Experiments b0 and b1: MuPDF
  agrees on intact and broken files. ISO 32000-2 §7.3.7 does not allow
  duplicate keys and says nothing about which one wins.
- Dictionary key order is kept exactly as read (test
  `stream_length_and_repair_keep_positions`).
- Nesting deeper than `parse.max_depth`: the object is unreadable, and is
  dropped, or becomes a blank page if listed (app unit test
  `a_value_nested_past_the_parse_depth_is_still_repaired_away`).
- A parse that hits a size or count limit (values, decoded bytes, tokens)
  is handled by the app before the repair runs. Inside the repair, that
  object is dropped.

### 4.6 Stream extents

For each stream, the data begins after the `stream` keyword and its EOL.
ISO says the EOL is CRLF or LF (§7.3.8.1). A lone CR, a space, or no EOL
at all is also accepted (experiments `c-stream-eol-cr`, `-space`, `-none`).
For the `none` case, MuPDF draws the page blank and today's engine reads
it. Read it: the data is unambiguous.

The data ends at the first of these that applies:

1. **`/Length` is usable.** `/Length` is a non-negative integer, or a
   reference that resolves to one, and data of that length is followed,
   after optional whitespace, by `endstream`. This holds even when
   `endstream` also appears inside the data (test
   `wrong_and_missing_stream_lengths_are_recovered`, indirect-length case).
2. **`/Length` is unusable.** It is missing, negative, dangling, or does
   not land on `endstream`. The data ends at the next `endstream` keyword,
   minus one EOL before it (§7.3.8.1: the EOL is not part of the data),
   with one exception. If the stream's filters decode the data correctly
   only when that trailing CR or LF is kept, keep it: such a byte can be
   the last byte of compressed data (test
   `recovered_lengths_keep_eol_bytes_that_are_data`). The decode used for
   this check counts against `max_decoded_bytes`.
3. **The extent is bounded.** Data never runs past the start of the next
   complete object definition. If `/Length` lands where the stream's
   `endobj` follows, after whitespace and a damaged `endstream` keyword,
   use `/Length` and do not search further (experiment 16; app test
   `a_damaged_endstream_keyword_keeps_the_declared_length`). In experiment
   16, MuPDF searched on and drew the next page's text on page 1.

The output writes each stream's raw (still encoded) data with a correct
`/Length` in the same key position as before, or appended last if it was
missing. Filters are not applied, except to containers and xref streams
(which are decoded and not written) and to the check in rule 2.
`streams_length_recovered` counts the streams that took rule 2 or 3.

### 4.7 Trailer entries

The output trailer has `/Size`, `/Root`, and, when present and valid,
`/Info`, `/ID` and `/Encrypt`. Nothing else.

- Through the chain (4.3): each entry comes from the newest trailer in the
  chain that has it.
- During reconstruction: each entry comes from the trailer (4.4 item 7)
  latest in file order that has it. Experiment d1: the newest trailer lacks
  `/Info`, so `/Info` comes from the older trailer. MuPDF agrees. §7.5.6
  requires an update's trailer to repeat the previous trailer's entries,
  so keeping the older value restores what the writer meant.
- `/Info` is written only if it resolves to a dictionary that is in the
  output. Otherwise it is omitted (test
  `object_stream_members_compete_at_the_container_position`).
- `/Root` must name a catalog (4.8).
- `/ID` and `/Encrypt` are copied as they are. `/Encrypt` may be a direct
  dictionary or a reference. One that is not a dictionary stays as it is.
  A dangling reference becomes null and is omitted.

### 4.8 Catalog

The catalog is the trailer's `/Root` when that resolves to a dictionary
with `/Type /Catalog` or with `/Pages`. Otherwise, in this order:

1. The object latest in file order whose dictionary has `/Type /Catalog`,
   or else one with `/Pages` naming a page-tree node. Set
   `catalog_from_scan`.
   - Experiment 41: `/Root` names a page and the xref is broken. MuPDF
     shows 0 pages. Today's engine and this spec show 3.
   - Experiments 45 and 46: with two catalogs and no trailer, the later
     one in the file wins. MuPDF picked the same catalog in both orders,
     which contradicts P3.
2. **NEW.** If no catalog exists but page objects do (4.10), synthesize a
   catalog `<< /Type /Catalog /Pages n R >>` over a synthesized page tree.
   Give it a number above every number in use, and set
   `catalog_from_scan`. Experiments 43 and 44: MuPDF shows 0 pages and
   today's engine refuses. Showing the pages that exist is better for the
   user.
3. With neither a catalog nor a page, return `Err(Invalid("document catalog"))`.
   Empty input, a header alone, and non-PDF text all get this error, or
   another `Invalid` (experiments 90–92).

A catalog that carries a `/Linearized` key is still the catalog (app test
`a_catalog_carrying_linearized_survives_repair`).

### 4.9 References

- **Dangling.** A reference to an object number absent from the output is
  null (§7.3.10). In arrays that list objects (`/Kids`, `/Contents`,
  `/Annots`) the entry is removed. Elsewhere it becomes `null`. Experiments
  52 and 53, and test `truncated_bodies_keep_their_surviving_pages`
  (dangling contents).
- **Wrong generation.** A reference whose number exists in the output
  under another generation is pointed at that object. This holds on both
  the intact and the broken path (experiments 55 and 56, both engines
  show all pages; app test
  `a_reference_with_another_generation_names_the_object`). Without this,
  a single wrong digit would lose pages.
- Rewriting references is not damage by itself. It does count in
  `references_repaired`.

### 4.10 Page tree

The output's page tree must satisfy the strict page walk
(`pages::collect_pages`): one indirect root node, consistent `/Count` and
`/Parent` values, and every leaf a page dictionary.

1. **Classify kids** the way the app's page walk does (P4):
   - `/Type /Pages` is a node.
   - No `/Type`, or a null one: `untyped_kid_is_node`.
   - A `/Type` naming another kind of object that ISO 32000-2 defines
     (`/Catalog`, `/Font`, `/Annot`, `/XObject`, …) is a page only if it
     has `/Contents` as a reference or array, or a `/MediaBox` and no
     non-empty `/Kids`. Otherwise it is left out.
   - Any other `/Type` value is a page: a misspelled name, a number, a
     string, an array, or a name with a damaged byte.

   Tests: `a_page_whose_type_is_damaged_stays_a_page`,
   `the_repair_reads_untyped_nodes_with_kids_as_nodes`,
   `an_untyped_node_reads_alike_with_and_without_repair`,
   `a_kid_typed_as_another_object_that_looks_like_a_page_stays_a_page`.

2. **Keep the tree.** Pages reachable from the catalog's `/Pages` stay, in
   tree order. Kids that are null or dangling are removed (4.9).
   `/Count` and `/Parent` are corrected.
   - Experiment 54: `/Count 7` over 3 kids. MuPDF lists 7 pages, 4 of
     which fail to load. This spec gives 3.
   - Experiments 52 and 53: a dangling kid. MuPDF shows a blank page.
     This spec drops it, as the strict page walk does for the intact file
     (P4).
   - Repaired page dictionaries are typed `/Type /Page`, so every later
     reader classifies them the same way.

3. **Unreadable listed page.** A page object that the tree lists, whose
   header exists but whose value is unreadable, becomes a blank page that
   inherits from its parent. The page count stays the tree's (app test
   `a_listed_page_whose_object_does_not_parse_is_a_blank_page`). This
   differs from a dangling kid: here the page existed and lost its
   content, so its slot is kept.

4. **Rebuild the tree** only when the catalog's `/Pages` is missing, is
   not a node, or yields no page while page objects exist. Build a new
   root (a fresh number) over every page object found, in file order.
   - Experiments 50, 51 and 58: the `/Pages` node is lost. MuPDF shows 0
     pages. This spec shows 3, in order.
   - Pages that a usable tree does not reach are *not* added. They may be
     pages that an update deleted (P2, P3).

5. **Missing MediaBox.** A page with no `/MediaBox` of its own and none
   inherited gets `[0 0 612 792]`, US Letter (8.5 × 11 in at 72 units per
   inch). Experiment 57 measured MuPDF showing that size.
   §7.7.3.3 requires a `/MediaBox`, so a reader needs some default, and
   Letter is the common one. App test
   `a_stray_closing_bracket_in_a_page_dictionary_keeps_the_page`.

6. **Untouched catalog and Info.** A rebuilt tree never takes over the
   catalog object or the `/Info` dictionary, even when one of them carries
   `/Kids` (app test
   `a_rebuilt_tree_never_takes_over_the_catalog_or_the_info_dictionary`).

### 4.11 Structural objects dropped from the output

The following are dropped from the output:

- xref streams;
- object-stream containers (their members are written as top-level
  objects);
- the linearization parameter dictionary and its hint streams.

The linearization parameter dictionary is the dictionary that Annex F
defines: it has `/Linearized`, plus `/L`, `/H`, `/O`, `/E`, `/N` and `/T`.
Its hint streams are the streams that its `/H` locates. Linearization is
dropped because the rebuilt file is no longer linearized, and a reader
that trusted those offsets would misread it. Set `linearized` when this
happens.

Dropping these objects is not damage. Test
`valid_structures_the_strict_reader_refuses_are_not_damaged` requires
that the output of a rebuilt linearized or hybrid file has exactly the
original's object count.

### 4.12 Encryption

- **Copy encrypted objects as they are.** Strings and stream data stay
  ciphertext, and `/Encrypt` and `/ID` stay in the trailer, so the strict
  reader decrypts them on open. The encryption dictionary itself is never
  encrypted (§7.6.1).
- **Encrypted object streams (§7.6.2, §7.5.7).** Authenticate with the
  first password in the list that works. Decrypt each container with the
  file key. Write each member as a top-level object whose strings are
  encrypted under *that member's own* object key, because the reader will
  decrypt every top-level string once, with its object's key.
  - The output must be deterministic. AES initialization vectors come from
    `encrypt::Drbg` with a fixed seed. That is acceptable only because the
    rebuilt bytes never leave the engine: the app's save re-encrypts with
    fresh entropy.
  - Experiments `h-*-nowhere`: MuPDF shows 0 pages even with the right
    user password. Today's engine and this spec show all 3. App tests
    `repair_reads_object_streams_of_an_encrypted_file_and_decrypts_once`
    and `open_tolerant_rebuilds_encrypted_object_streams_with_its_password`.
- **No password works.** When no password authenticates and the file has
  object streams, write the structure only: containers as ordinary
  streams, no members, and references and the page tree left as they are.
  Set `locked`. The app opens that locked, and `authenticate` rebuilds
  from the original with the password (app test
  `an_encrypted_file_whose_object_streams_need_a_password_opens_locked_then_unlocks`).
- **Encryption dictionary found by scan.** When the trailer is lost, the
  encryption dictionary is found by scanning:
  - AES-256 (revisions 5–6) does not use `/ID`, so the file can be
    recovered.
  - Revisions 2–4 derive the key from `/ID`. If no `/ID` survives, return
    `Err(Invalid("encryption without the file identifier"))`.
    Experiments `g-encrypted-truncated-at-xref` and
    `g-encrypted-aes256-truncated-at-xref`: in both, MuPDF opened the file
    as unencrypted and showed ciphertext as the document title (P5).
- **Lenient reading of `/Encrypt`.** `/Encrypt` values are read leniently.
  For example, a non-hex digit in `/U` reads as 0 (app test
  `a_non_hex_character_in_the_encryption_dictionary_reads_as_zero`).
- **Wrong password.** For an encrypted file without object streams, the
  output must be identical for any password list (`password_dependent`
  false). The app relies on this to answer a wrong password without a
  reload (app test `a_wrong_password_for_a_rebuilt_locked_file_matches_a_reload`).

### 4.13 Truncated files

A file cut anywhere keeps every complete object before the cut, as
reconstruction does. A stream cut short keeps its data up to the cut only
if that data ends exactly at the end of the input. Otherwise it follows
4.6. A page whose contents were lost stays as a blank page.

- Experiment 17: cut in the third content stream. Both engines show 3
  pages, the third blank.
- Test `truncated_bodies_keep_their_surviving_pages`: for cuts of
  `large.pdf` at 50%, 75% and 90%, the surviving pages keep their content
  streams, except at most one.

---

## 5. Output contract

### 5.1 Bytes

- **One revision.**
  - Header `%PDF-x.y` (4.2), followed by a binary-marker comment line
    (§7.5.2).
  - Every object as `N G obj … endobj`, with the generation it was
    selected with.
  - One classic xref table that lists every number from 0 to the largest
    in use. Numbers not in use are free entries.
  - A trailer (4.7) and `startxref`, then `%%EOF`.
  - No xref streams, object streams, linearization or incremental
    sections.
- **Object numbers are preserved.** New objects (a synthesized catalog or
  page-tree root) take numbers above all numbers in use. `/Size` is one
  past the largest number written.
- **Strict.** The output opens with the strict reader: no duplicate keys
  and no lenient tokens. Each value is re-serialized from its parsed form
  with the crate's writer.
- **Within token limits.** Strings are written in a form that re-parses
  within the caller's token limits. For example, a 600 KB literal string
  must not be expanded to hex past a 1 MiB token limit (test
  `long_strings_stay_within_the_token_limit`). In that test, the output is
  smaller than the input.
- **Deterministic.** The same input and password list always give the
  same bytes. When `password_dependent` is false, every password list
  gives the same bytes. Test `encrypted_fixtures_with_broken_xref_keep_encryption`
  compares the bytes of a locked open with a rebuild.

### 5.2 The `damaged` flag

This flag means: *what you see required the engine to guess or to drop
content, or the original bytes cannot be safely extended.* sign-prepare
refuses to extend a damaged file, and the UI shows it as repaired.

| Situation | `damaged` |
|---|---|
| Header not at byte 0 of `input` | **true**. The output's offsets are header-relative. An incremental update appended to the original would mix two offset bases. |
| No usable chain (4.3), including a chain longer than `max_revisions` | **true** |
| An object the chain locates is unreadable (`objects_unreadable > 0`) | **true** (app test `an_object_header_inside_a_damaged_object_does_not_replace_the_object`, "unparsable" case) |
| Catalog not the trailer's `/Root` (`catalog_from_scan`) | **true** |
| Page tree rebuilt from page objects (4.10 rule 4), or listed pages made blank (rule 3) | **true** |
| Page-tree corrections that the app's strict walk also makes silently (null kids removed, `/Count` or `/Parent` corrected, `/Type` set, `/MediaBox` supplied) | false |
| Stream extent recovered (4.6) while the chain is usable | **false (NEW, open question Q1)**. Experiments 12–15: MuPDF does not flag a wrong or missing `/Length`. The original xref is valid, so signing by incremental update stays possible. |
| Linearized, hybrid, slack offsets, text after `%%EOF`, missing `%%EOF`, trailing NULs, junk between objects, duplicate keys, wrong-generation references, lenient tokens, dangling references | false |

Experiment 72 (3,000 trailing NUL bytes): MuPDF flags the file and today's
engine does not. NUL is whitespace (§7.2.3, Table 1), so it is not damage.

### 5.3 Diagnostics

The report's counts (2.2) are diagnostics. Their exact definitions above
are binding, because tests assert them. Nothing else is reported. There is
no logging and no warning text.

---

## 6. Limits and safety

- **No panics, no `unsafe`, no unbounded recursion.** Nesting is bounded
  by `parse.max_depth`. No allocation may be sized from a number the file
  declares (`/Size`, `/N`, `/First`, `/W`, `/Length`, `/Count`, xref
  subsection counts) before the bytes backing it are present.
- **Linear time.** Total work is O(input bytes + decoded bytes), with a
  small constant. Any input shape that makes some part of the input be
  re-read without bound is a defect. These acceptance tests must each
  finish in under 10 s in a debug test build:
  - the adversarial inputs in `adversarial_inputs_scan_in_linear_time`
    (unterminated strings, huge `/Length`, endless `stream` keywords,
    unclosed arrays, `obj` floods, `trailer << /Root` floods, comment
    chains, headers inside strings);
  - `object_stream_members_sharing_an_offset_are_parsed_once`
    (200,000 members pointing at one region);
  - app test `loose_dictionaries_are_read_in_bounded_time`
    (a megabyte of unterminated dictionaries).
- **Memory** is O(input + output + kept definitions), and bounded by the
  limits:
  - definitions kept ≤ `max_objects`;
  - parsed values ≤ `max_values`;
  - decoded structural bytes ≤ `max_decoded_bytes`;
  - containers decoded ≤ `max_object_streams`;
  - output ≤ `max_output_bytes`;
  - chain sections followed ≤ `max_revisions`.

  When a limit is exceeded, the function returns the matching error
  (2.3). It does not return a partial result. A chain that is too long is
  not an error: the file is reconstructed instead.
- **Garbage in, error out.** For any byte string, the function either
  returns an output that opens strictly (or needs a password), or returns
  an error. Test `mutated_fixtures_never_panic_and_rebuild_to_strict_files`
  checks this on 1,500 mutants, and at least 1,000 of them must rebuild.

---

## 7. Recorded black-box experiments

### 7.1 Method

All inputs are synthetic, built by scripts outside the repository in
`reviewer archive spec-03/scripts/`. These
scripts are self-authored and contain no MuPDF code, so they can go to the
implementer with the acceptance suite.

| Script | Builds |
|---|---|
| `build.mjs` | `cases/`: 69 files |
| `build2.mjs <repo>` | `cases2/`: 29 files. It uses MuPDF only as a *writer* for the `h-*` encrypted object-stream files. |
| `build3.mjs` | `cases3/`: 5 files |

Observer:
`tools/slot node scripts/observe.mjs <repo> [files]`, with `PW=<password>`
for encrypted cases. It opens each file:

- with **MuPDF 1.28.1** (npm `mupdf`): `wasRepaired()` at open and after
  loading every page, page count, structured text per page, the trailer
  `/Info` `/Title`, and `countVersions()`;
- with **today's Rust app** (the WASM package `engine/web/pkg`, through
  `RustApp.dispatch`): `open`, `page-ids`, `text-all` and `save`, then the
  save's `/Info` `/Title` read back through MuPDF.

Raw outputs are in `scratch/spec-03/observe-{1..4}.txt`.

pdf.js and qpdf are not installed on this machine and were not run (Q5).

### 7.2 Base document and notation

Unless a row says otherwise, the base document has:

- catalog 1;
- `/Pages` 2, with kids 3, 4 and 5;
- pages 3, 4 and 5 with contents 6, 7 and 8, which draw the text "Alpha",
  "Beta" and "Gamma";
- font 9;
- `/Info` 10, titled "Original";
- a classic xref.

In the table:

- **P** = page count.
- **T** = page texts, separated by `|`. A `-` marks a blank page.
- **R** = repaired flag (`wasRepaired` for MuPDF, `info.repaired` for
  today's engine).
- **title** = the `/Info` `/Title`.

"=" means the same as the cell to its left.

### 7.3 Results

| # | Input | MuPDF 1.28.1 | Today's engine | This spec |
|---|---|---|---|---|
| 00 | intact | P3 A\|B\|G, R no | = | = |
| 01 | `startxref` → 999999 | P3, R yes | = | = |
| 02 | `startxref` removed | P3, R yes | = | = |
| 03 | cut at `xref` (no table, trailer, `%%EOF`) | P3, title none, R yes | = | = |
| 05 | every offset +7 | P3, R yes | = | = |
| 06 | every offset −1 (at the EOL before the header) | P3, R no | = | = |
| 07 | every offset +1 | P3, R yes | = | = |
| 08 | 30 junk bytes before the header, offsets header-relative | P3, R no | = | R no via the app; `rebuild` alone R yes (5.2) |
| 09 | 2,000 junk bytes before the header | P3, R yes | = | = |
| 10 | `endobj` blanked (offsets kept) | P3, R no | = | = |
| 11 | `endobj` removed (offsets shift) | P3, R yes | = | = |
| 12–15 | content 7 `/Length` too long, too short, missing, or dangling; xref intact | P3, R no | P3, R yes | P3, **R no** (Q1) |
| 16 | content 6 `endstre#m`, `/Length` right | P3, page 1 text "Alpha Beta" (swallowed the next stream), R no | P3 A\|B\|G, R no | as today's engine |
| 17 | cut inside content 8 | P3 A\|B\|-, R yes | = | = |
| 20 | valid update: 7 → "Updated", title "Updated" | P3 A\|Updated\|G, versions 2, R no | = | = |
| 21 | update's `xref` → `xrex` | P3 A\|Updated\|G, R yes | = | = |
| 22 | update's objects present, no xref section | P3 A\|B\|G, title Original, R no | = | = |
| 23 | as 22, plus 1,100 bytes of tail | P3 A\|Updated\|G, R yes | = | = (the writer's locator finds no `startxref`) |
| 24 | update's trailer cut | P3 A\|B\|G, R no | = | = |
| 25 | update with `startxref` → 999999 | P3 A\|Updated\|G, R yes | = | = |
| 26 | valid update frees 8 | P3 A\|B\|-, R no | = | = |
| 27 | 26 with `startxref` → 999999 | P3 A\|B\|G, R yes | = | = (P3: the scan has no free entries) |
| 30 | two `7 0 obj`, the later one "Second", no xref | P3 A\|Second\|G | = | = |
| 31 | later `7 1 obj` "GenOne" | A\|GenOne\|G | = | = |
| 32 | earlier `7 3 obj`, later `7 0 obj` | A\|GenZeroLater\|G | = | = |
| 40 | `trailer` → `xrailer` | P3, title Original, R yes | = | = |
| 41 | `/Root` names page 4, `startxref` broken | **P0** | P3 | P3 |
| 42 | trailer removed | P3, title none | = | = |
| 43 | catalog `/Type /Katalog` with `/Pages`, no trailer | **P0** | refused | **P3** (4.8) |
| 44 | no catalog at all, no trailer | **P0** | refused | **P3** (4.8) |
| 45 | catalogs 12 (→ 1-page tree) then 1 (→ full tree), no trailer | P1 | P3 | P3 (later wins) |
| 46 | catalogs 1 then 12, no trailer | P1 | P1 | P1 |
| 50 / 51 | `2 0 obj` → `2 0 xbj`, xref intact or broken | **P0** | P3 | P3 |
| 52 / 53 | `/Kids [3 99 4 5]`, `/Count 4`; intact or broken | P4 A\|-\|B\|G | P3 | P3 |
| 54 | `/Count 7`, broken | P7, 4 pages fail to load | P3 | P3 |
| 55 / 56 | kid `3 1 R`; intact or broken | P3 | P3 | P3 |
| 57 | page 3 without `/MediaBox`, broken | P3, page 1 612×792 | P3 | P3, 612×792 |
| 58 | 50 plus 57 | P0 | P3 | P3 |
| 59 | page 4 `/Parent 9 0 R`, broken | P3 | = | = |
| 60 | title "see 4 0 obj and trailer << here", broken | title kept | = | = |
| 61 / 62 | font `/Junk 3 0 obj /Encoding …`; intact or broken | P3, R no / yes | = | = |
| 63 | `/Info` loses `>>`, next object follows; intact xref | P3, title none, R no | P3, R yes | P3, R yes (unreadable object) |
| 64 | 63, broken | P3, R yes | = | = |
| 70 | text after `%%EOF` | P3, R no | = | = |
| 71 | 1,100 junk bytes after `%%EOF` | P3, R yes | = | = |
| 72 | 3,000 NUL bytes after `%%EOF` | P3, **R yes** | P3, R no | R no (5.2) |
| 73 | no `%%EOF` | P3, R no | = | = |
| 74 | xref rows end in CRLF (19-byte rows) | P3, R no | = | = |
| 75 | subsection count one too large | P3, R yes | = | = |
| 76 | first subsection starts at 1 | P3, R no | = | = |
| 80 | object stream and xref stream, intact | P3 | = | = |
| 81 / 83 | 80 with `startxref` broken, or `/W [1 4 3]` | P3, **title none** | P3, title kept | P3, title kept (4.4 item 7) |
| 82 | 80 without its xref stream | P3, title none | = | = |
| 84 | container, then a top-level 10 "TopLevelLater" | title TopLevelLater | = | = |
| 85 | top-level 10 "TopLevelEarlier", then the container | title Original | = | = |
| 90–92 | empty / header only / "hello world 1 0 obj" | error | error | error |
| 93 | header blanked, offsets intact | P3, R yes | P3, R no | = (strict path) |
| 94 | last object an unterminated dictionary, no xref | P3, title none | = | = |
| a0 | CRLF before each header, offsets at the CR | P3, R no | = | = |
| a1 / a2 | `startxref` −1 / +1 | R no / R yes | = | = |
| a3 | `junk` line between objects | P3, R no | = | = |
| a4 | entry for 4 points at object 5 | P3, R yes | = | = |
| a5 | entry for 4 free, object 4 present | P3 A\|-\|G, R no | P2, R yes | P2. The chain frees 4 (P2), and the kid is then dangling (4.9). |
| b0 / b1 | `/Title (First) /Title (Last)`; intact or broken | title Last | = | = |
| c-cr / c-space / c-none | `stream` followed by CR / space / nothing | P3 / P3 / **page 2 blank** | P3 all (R no / yes / no) | P3 all, R no |
| d0 | update trailer without `/Info`, intact | title none | = | = (strict path) |
| d1 | d0 with `startxref` broken | title Original | = | = |
| e0 | hybrid, table marks compressed objects free | **P0** | P3, R no | P3, R no (§7.5.8.4) |
| e1 | hybrid with a wrong `/XRefStm` | P3, R yes | = | = |
| f0 | linearized (forward `/Prev`, hint stream, object stream) | P3, R no | = | = |
| f1 | f0 with a wrong main-xref `/Prev` | P3, **title none** | P3, title kept | as today's engine |
| g aes256 / rc4, `startxref` broken | user password | P1, text | = | = |
| g aes256 / rc4, trailer keyword damaged | user password | P1, text | = | = |
| g aes256, cut at `xref` | | P1 blank, **ciphertext title**, not encrypted | P1 text with the password | as today's engine |
| g rc4, cut at `xref` | | P1 blank, **ciphertext title** | refused | refused (4.12) |
| h rc4 / aes256 (MuPDF-written: object streams, user password), broken | password "user" | **P0** | P3 | P3 |
| h rc4, empty user password, broken | | P3, title none | P3, title kept | as today's engine |
| i0 / i1 / i2 | container `/Type /Other`, `/Fl`, junk after zlib; intact xref stream | P3, R no | = | = |
| j0 / j1 | loose `<< /Info 9 0 R /Root 99 0 R >>` between objects, or a loose `trailer` after `%%EOF`; intact | title Original, R no | = | = |

### 7.4 Where the viewers disagree, and why this spec chooses as it does

- **41, 43, 44, 45, 50, 51, 54, 58, e0, h-\*:** MuPDF shows 0 pages or
  pages that fail to load. This spec recovers the pages that exist (P1).
- **16, c-none:** MuPDF swallows the next object or drops the data. This
  spec bounds the extent by the next object (P1).
- **52 and 53 (dangling kid):** MuPDF inserts a blank page. This spec
  drops the kid, as the strict page walk does for the intact file (P4).
  This is open question Q2.
- **81, 83, f1, h-emptyuser:** MuPDF loses `/Info`. This spec keeps
  trailer entries from xref-stream dictionaries (4.7).
- **g-\*-truncated:** MuPDF shows ciphertext as plain text. This spec
  decrypts the file or refuses it (P5).
- **12–15 and 72:** MuPDF and today's engine disagree about the repaired
  flag. This spec follows 5.2: flag only guessing and loss.
- **23:** both engines change their reading when the last `startxref`
  moves beyond what the locator searches. This spec ties that to the
  incremental writer's locator, so reading and signing agree.

---

## 8. Acceptance

All tests below are black-box: they build or derive their own inputs and
assert observable results. None asserts byte-equality with MuPDF. Classes:

- **keep**: unchanged;
- **modify**: assertion changed by a decision in this spec;
- **replace**: rewritten;
- **scrub**: comment or name cites MuPDF internals or copied constants, so
  reword it as an observed behaviour (8.5).

### 8.1 Inventory of existing tests (68 tests)

**`engine/tests/repair.rs`: 18 tests (14 keep, 4 modify).**

| Test | Class | Note |
|---|---|---|
| `generic_damage_to_fixtures_is_repaired` | keep, scrub | The helper `complete_revision` hard-codes the locator's search distance. Make it call `incremental::startxref` instead (4.3). |
| `encrypted_fixtures_with_broken_xref_keep_encryption` | **modify** | The "encrypted.pdf length" sub-case has an intact xref with a wrong `/Length`. Expect `damaged == false` and `streams_length_recovered == 1` (Q1). |
| `wrong_and_missing_stream_lengths_are_recovered` | **modify** | The "long" and "short" cases and the hello.pdf case keep an intact xref. Expect `damaged == false` and `xref_intact`. The "missing", "negative", "unresolvable" and "CRLF" cases shift offsets, so they stay `damaged`. |
| `duplicate_keys_keep_the_last_value` | keep | |
| `incremental_update_with_broken_xref_keeps_the_newer_objects` | keep, scrub | Counts per 2.2. |
| `objects_only_in_an_object_stream_survive_a_broken_xref_stream` | keep | |
| `object_stream_members_compete_at_the_container_position` | keep | `(recovered, overridden) = (6, 2)`, `(expanded, references_repaired) = (1, 4)`, per 2.2. |
| `catalog_is_found_by_type_when_no_trailer_names_it` | **modify** | The `no_catalog` input now opens: 1 page, `catalog_from_scan`, `damaged`. Inputs `""`, `"%PDF-1.7\n"` and `"not a pdf 1 0 obj"` still return `Err`. |
| `open_tolerant_keeps_valid_files_and_passwords` | keep | |
| `limits_are_enforced` | keep | Reason strings from 2.3. |
| `adversarial_inputs_scan_in_linear_time` | keep | |
| `mutated_fixtures_never_panic_and_rebuild_to_strict_files` | keep | |
| `object_stream_members_sharing_an_offset_are_parsed_once` | keep | The defaults must admit 200,000 members (2.1). |
| `truncated_bodies_keep_their_surviving_pages` | keep, scrub | The Letter comment should cite 4.10 rule 5. |
| `recovered_lengths_keep_eol_bytes_that_are_data` | **modify** | The four EOL cases rebuild an intact xref, so expect `damaged == false`. The "objstm ending in CR" case stays `damaged`. |
| `keywords_inside_strings_do_not_cut_objects` | keep | |
| `long_strings_stay_within_the_token_limit` | keep | |
| `valid_structures_the_strict_reader_refuses_are_not_damaged` | keep, scrub | |

**`engine/tests/repair_trailer_reread.rs`: 2 tests, keep, scrub.** The
module comment describes MuPDF's repair. Reword it as: "a header inside an
unterminated trailer string is read as part of the string".

**`engine/tests/app_corpus_open.rs`: 21 of 37 tests depend on repair. All
keep.**

- encrypted object streams: `repair_reads_object_streams_of_an_encrypted_file_and_decrypts_once`,
  `an_encrypted_file_whose_object_streams_need_a_password_opens_locked_then_unlocks`,
  `open_tolerant_rebuilds_encrypted_object_streams_with_its_password`;
- lenient reading: `a_name_escape_without_two_hex_digits_stands_for_itself_in_repair`,
  `a_damaged_token_in_a_page_dictionary_keeps_the_page`,
  `a_damaged_token_keeps_annotations_forms_and_the_catalog`,
  `a_damaged_matrix_entry_keeps_the_form`,
  `a_catalog_with_a_damaged_type_opens`,
  `a_non_hex_character_in_the_encryption_dictionary_reads_as_zero`;
- containers: `containers_mupdf_reads_are_read_in_repair` (rename to
  `tolerated_containers_are_read_in_repair`);
- streams and pages: `a_listed_page_whose_object_does_not_parse_is_a_blank_page`,
  `a_damaged_endstream_keyword_keeps_the_declared_length`;
- revisions: `objects_appended_without_an_xref_leave_the_last_complete_revision`
  (scrub: it cites the locator's byte distance),
  `a_signed_file_with_an_appended_object_shows_the_signed_revision`,
  `a_catalog_carrying_linearized_survives_repair`;
- page tree: `a_stray_closing_bracket_in_a_page_dictionary_keeps_the_page`,
  `a_page_whose_type_is_damaged_stays_a_page` (scrub),
  `the_repair_reads_untyped_nodes_with_kids_as_nodes`,
  `a_reference_with_another_generation_names_the_object`,
  `an_object_header_inside_a_damaged_object_does_not_replace_the_object`
  (scrub);
- encryption: `a_missing_or_damaged_encryption_revision_is_inferred_from_v`
  (scrub; its `/R` rule belongs to class C `security.rs`).

**`engine/tests/app_accept_open.rs`: 10 of 11 tests depend on repair. All
keep.**

`a_dictionary_missing_its_end_keeps_the_next_object`,
`a_header_glued_to_a_damaged_object_keeps_an_intact_xref`,
`fixtures_with_a_dictionary_missing_its_end_keep_their_objects`,
`an_untyped_node_reads_alike_with_and_without_repair`,
`a_kid_typed_as_another_object_that_looks_like_a_page_stays_a_page`,
`a_damaged_trailer_keyword_keeps_the_file_identifier`,
`loose_dictionaries_are_read_in_bounded_time`,
`a_wrong_password_for_a_rebuilt_locked_file_matches_a_reload`,
`loose_dictionaries_do_not_override_an_intact_xref`,
`junk_between_objects_does_not_starve_rereads`.

Scrub these:

- the helper `mupdf_node` (rename to `reference_reader_node`, drop the
  citation of a MuPDF C function);
- the last test's name and comment, which describe internal budgets.
  Suggested name: `junk_between_objects_does_not_hide_a_later_page`.

**`engine/tests/m10_fuzz.rs`: 4 of 4 depend on repair** (each runs with
`broken = true`). All keep, scrub. The comments describe what MuPDF's
repair reads internally. Keep only the observed outcomes.

**Other tests that depend on repair: 13, all keep.**

- `engine/tests/app_sign.rs`: `a_damaged_signed_source_is_not_extended`,
  `hybrid_and_linearized_signed_sources_take_an_update_after_their_bytes`,
  `xref_quirks_mupdf_reads_without_repair_take_an_update` (rename to
  `xref_quirks_read_without_repair_take_an_update`),
  `an_update_after_rebuilt_bytes_keeps_the_original_numbers_and_size`,
  `a_signed_source_with_a_long_blank_tail_takes_an_update_after_it`.
  Scrub the comment naming the locator's byte band. With the unified
  locator (4.3), every distance in that band still gives the documented
  refusal.
- `engine/tests/app_core.rs`: `open_repairs_damaged_files_and_reports_them`,
  `many_revisions_open_and_past_the_cap_are_rebuilt` (needs
  `max_revisions`), `junk_before_the_header_is_skipped_without_repair`
  (scrub the junk-prefix comment).
- `engine/tests/app_info.rs`: `versions_count_the_file_as_mupdf_does`.
  The values were recorded black-box. Rename to
  `versions_count_the_files_own_sections`.
- `engine/tests/dictionary.rs`: `stream_length_and_repair_keep_positions`.
- `engine/tests/serialize_roundtrip.rs`:
  `documents_with_empty_names_reopen_identically_in_every_writer_style`.
- `engine/src/app/document.rs` unit tests:
  `a_value_past_a_parse_size_limit_refuses_the_open_instead_of_being_repaired_away`,
  `a_value_nested_past_the_parse_depth_is_still_repaired_away`.

**Totals:** 68 tests. 64 keep (about 20 of them need comment or name
scrubbing), 4 modify, 0 replace. No existing repair test asserts
byte-equality with MuPDF.

### 8.2 New cases

Add these to `tests/repair.rs` or a new `tests/repair_spec.rs`. Each is
built in the test, from the experiment of the same number (7.3).

| Case | Expect |
|---|---|
| N1 (45, 46) | Two catalogs, no trailer: the later catalog's pages. |
| N2 (43, 44) | Catalog untyped or absent, page objects present: opens, pages in file order, `catalog_from_scan`, `damaged`. |
| N3 (54) | `/Count` larger than the kids, broken xref: only the real pages. |
| N4 (50, 51, 58) | `/Pages` node lost: 3 pages in file order, `page_tree_repaired`, `damaged`. Letter size where no `/MediaBox` applies. |
| N5 (52, 53) | Dangling kid: the same page list on the intact and the broken file. |
| N6 (31, 32) | Later definition wins whatever the generations, and keeps its own generation. |
| N7 (d1) | Broken xref, newest trailer without `/Info`: `/Info` taken from the older trailer. |
| N8 (12, 13; c-space) | Intact xref, wrong `/Length` or a non-standard `stream` EOL: content equal to the intact file, `!damaged`, `xref_intact`, `streams_length_recovered == 1`. sign-prepare on a signed variant succeeds (Q1). |
| N9 (h-\*-nowhere, AES-256 and RC4) | Broken xref with encrypted object streams and a user password: all pages with the password; `locked` without it. |
| N10 (a4) | One xref entry pointing at another object's header: reconstructed, all pages, `damaged`. |
| N11 (a5) | An entry free but the object present: the object absent from the output (P2). |
| N12 (e1, f1) | Hybrid with a wrong `/XRefStm`, linearized with a wrong main `/Prev`: reconstructed, all pages, title kept. |
| N13 (81, 83) | Broken xref, xref stream gone or unreadable: `/Info` title from the xref-stream dictionary. |
| N14 (23, app_sign band) | A `startxref` the writer's locator does not find: `rebuild` reconstructs (`!xref_intact`). One it does find: read through the chain. |
| N15 (72) | 3,000 trailing NUL bytes after a strict-refused layout (for example, a hybrid): `!damaged`. |
| N16 | Determinism: `rebuild` twice gives equal bytes. For an encrypted file without object streams, `rebuild_with_passwords` with `[]`, `[b"wrong"]` and `[user]` gives equal bytes. |
| N17 | `max_revisions`: a chain one section over the limit is reconstructed (`damaged`, `xref_sections` reports the sections read). A chain at the limit is read (`!damaged`). |

### 8.3 Corpus acceptance

Reviewer-side gate: the runner is `engine/oracle/corpus-run.mjs` (AGPL,
moved there in Phase 0; it drives MuPDF), so the reviewer runs it, not the
implementer, and the baseline results (outputs of the replaced engine) stay
out of the hand-off tree. Run it on the external corpus
(`engine/reference/corpus-20261015/README.md`). Compare per file against
the baseline run
the reviewer's corpus run
(summary `summary-ba70615.json`).

Baseline open counts:

- Rust: 4,952 open, 27 locked, 70 refused;
- `open-rust-refuses`: 0;
- Rust marks 142 files repaired, and so does MuPDF.

Required:

1. No file the baseline opened is refused, and `open-rust-refuses` stays 0.
2. On every file, the page count is at least the baseline's Rust count.
   The `rust` text-all character count is not lower on any file that went
   through repair.
3. No new `timeout`, `oom`, `crash` or `rust-trap`. On files that go
   through repair, wall time and peak RSS are no worse than twice the
   baseline's.
4. Every save still validates and reopens (the save class counts do not
   regress).
5. The `repaired` count may change. Q1 lowers it. `repaired-differs` is
   recorded, not a failure.

Files to check by name: 29 baseline-repaired files where Rust and MuPDF
disagree on pages. Rust ≥ the listed count is required. Reaching MuPDF's
count where it is higher is a goal, not a requirement.

| Source | Path | SHA-256 prefix | Rust pages | MuPDF pages |
|---|---|---|---|---|
| qpdf | `fuzz/qpdf_extra/69969.fuzz` | 083e9db240e1 | 22 | 30 |
| qpdf | `fuzz/qpdf_extra/69977e.fuzz` | d35136962510 | 8 | 9 |
| qpdf | `fuzz/qpdf_extra/71624.fuzz` | fe4e80441943 | 6 | 30 |
| qpdf | `qpdf/qtest/qpdf/xref-compressed-in-compressed.pdf` | c99080e8d936 | 29 | 30 |
| qpdf | `qpdf/qtest/qpdf/append-xref-loop.pdf` | 7c67daccc433 | 30 | refused |
| qpdf | `fuzz/qpdf_extra/{440599107, 6310410941956096, 68668, 69977, 70306, 71689, 99999e}.fuzz` | (manifest) | 1 each | refused |
| qpdf | `fuzz/qpdf_extra/70306b.fuzz` | 86c53176bda3 | 6 | refused |
| qpdf | `qpdf/qtest/qpdf/bad40.pdf`, `object-stream-self-ref.pdf` | 6fd5c3e66fc2, c88761ef6acf | 1 each | refused |
| pdf.js | `test/pdfs/issue19484_1.pdf`, `issue19484_2.pdf` | 0f020604762f, 247215f0ca71 | 1 each | refused |
| pdf.js | `GHOSTSCRIPT-698804-1-fuzzed.pdf`, `issue18986.pdf`, `issue9418.pdf`, `poppler-395-0-fuzzed.pdf`, `poppler-91414-0-53.pdf`, `poppler-91414-0-54.pdf` | (manifest) | 1 each | refused |
| pdf.js | `issue3371.pdf`, `pr6531_1.pdf`, `print_protection.pdf` | 0c3fb45d9b61, a5584ab7da3a, e5497249e498 | 1 each | 0 |
| pyHanko | `…/test_data/data/pdf/broken-objstream{1,2,3}.pdf` | 739347ddba3c, 2a3e1b7041e4, 0fb75b109ea1 | 1 each | refused |

Also re-check these. The baseline Rust results must hold:

- qpdf `issue-99.pdf` (7613e0aa0732): 1 page;
- qpdf `stream-line-enders.pdf` (14c62cf7f0af): 1 page, text present. Q1
  makes it `!repaired`;
- pyHanko `broken-objstream4.pdf` (20b8c82a3622): 1 page, contents drawn;
- pdf.js `PDFBOX-4352-0.pdf` (ede76ce3aaa8): 1 page. (Superseded by §12 A9,
  owner to confirm: refused under A6.)

Corpus rules apply. Record no document text, and never read
`private-sample.pdf`.

### 8.4 Gates

- `cargo test` for the whole engine, run through `slot`.
- The WASM build.
- The corpus run (8.3).
- Independent review that this spec's implementation was written without
  the old file.

### 8.5 Comments and names to scrub

Phase 0 or 3 should reword these as observed behaviour, with no MuPDF C
function or constant names, no MuPDF internals, and no copied search
distances.

- `tests/repair.rs`:
  - the module comment and the "as MuPDF reads it" comments;
  - the `complete_revision` helper's search distance;
  - the Letter comment.
- `tests/repair_trailer_reread.rs`: the module comment.
- `tests/app_corpus_open.rs`:
  - the module comment;
  - the comments at `objects_appended_without_an_xref…` (locator distance);
  - comments that name MuPDF C functions (page lookup, crypt setup) or
    describe "MuPDF's repair";
  - the test name `containers_mupdf_reads_are_read_in_repair`.
- `tests/app_accept_open.rs`: `mupdf_node` and `mupdf_pages`, and the
  comment naming a MuPDF C function.
- `tests/m10_fuzz.rs`: the `a_header_read_as_a_damaged_value…` comment.
- `tests/app_sign.rs`:
  - the comments with the locator's byte distance and band;
  - the test name `xref_quirks_mupdf_reads_without_repair…`.
- `tests/app_core.rs`: the comment about how MuPDF offsets a junk prefix.
- `tests/app_info.rs`: the module comment and the test name.
- Class C code outside this workstream, flagged for its owner:
  - `src/app/document.rs`: comments that name MuPDF API methods, C
    functions and an object-number constant;
  - `src/app/ops/sign.rs`: same constant, locator comment;
  - `src/document.rs`, around line 918: "MuPDF ignores generations and
    the repair…";
  - `src/app/info.rs`: the `is_node` comment.

  `src/incremental.rs::startxref` and `app::document::without_leading_junk`
  use a fixed search distance carried over from the oracle. Section 4.3 depends on the
  locator but deliberately does not state that distance. Whether to keep
  it is the owner's call (Q7).

---

## 9. Behaviour changes users would notice (compared with today)

1. **Fewer "repaired" badges.** A file whose xref is intact but whose
   stream `/Length` is wrong or missing, or whose `stream` keyword has a
   non-standard EOL, is no longer shown as repaired. If it is signed,
   sign-prepare can add a signature by incremental update. This matches
   MuPDF. In the corpus, qpdf `stream-line-enders.pdf` is one such file.
2. **Files with no catalog now open.** If page objects survive, they open
   with those pages (experiments 43 and 44) instead of "the file could not
   be read".
3. **One rule for "last revision".** A file whose last `startxref` sits
   just beyond the incremental writer's search is now reconstructed and
   marked repaired, instead of being read through its xref. sign-prepare
   already refused such files, so its behaviour does not change.
4. **Different bytes after a rebuild** (object order, serialization). This
   is not visible: the save rewrites the file, and numbers are preserved.
   Report counts may differ on inputs no test pins.
5. **Default `RepairLimits`** now equal the app's limits (2.1). Direct
   callers of `RepairLimits::default()` get larger bounds.

---

## 10. Open questions for the owner

- **Q1.** Should a wrong or missing `/Length` (or a stream EOL quirk) on a
  file with an intact xref stop counting as damage? This spec says yes:
  MuPDF agrees, and signing is no longer blocked. The cost is that 4 tests
  change (8.1).
- **Q2.** A dangling page-tree kid: drop it (today's engine, this spec,
  and the strict walk), or show a blank page (MuPDF)?
- **Q3.** A kid typed as another kind of object, with neither `/Contents`
  nor a `/MediaBox`, is left out by the repair but shown as a blank page
  by the strict walk. That is an existing inconsistency with P4. Keep it
  (the current tests) or unify?
- **Q4.** Should a catalog be synthesized when none exists (4.8 rule 2)?
- **Q5.** pdf.js and qpdf are not installed. Allow `npm install pdfjs-dist`
  into the scratch directory (a download) for a second oracle on section
  7's inputs?
- **Q6.** Should the default `RepairLimits` become the app's limits, as
  2.1 says, or should explicit values be chosen by measurement?
- **Q7.** `incremental::startxref` and the app's leading-junk skip use a
  fixed search distance inherited from MuPDF-informed code. They are class
  C, so outside this workstream, but the implementer reads them (4.2,
  4.3). Re-derive the distances (and scrub their comments) before
  hand-off, or keep them as observed behaviour?

---

## 11. Leak-check change log (2026-10-18)

Independent leak check against SPEC-RULES.md. The spec states no MuPDF
internals, copies none of the replaced file's constants (scan windows,
inflate ratios, key lists), and records every MuPDF fact it uses in
section 7. Edits made:

1. 8.3: the corpus runner moved to `engine/oracle/` in Phase 0 and drives
   MuPDF; marked as a reviewer-side gate, with the baseline results (the
   replaced engine's outputs) kept out of the hand-off tree.
2. Q7: reworded (no claim about MuPDF's search distance) and made a
   hand-off blocker, because the implementer reads the class C locator and
   junk skip, which carry an inherited distance and MuPDF comments.

Hand-off prerequisites found (not spec text, so not edited here):

- `src/incremental.rs` (`startxref` and its comment near line 282) and
  `src/app/document.rs` (limits comments naming MuPDF constants and API
  methods) are surviving code the implementer reads; scrub them first
  (8.5 already lists them).
- `engine/design/` (for example `m10-fuzz.md`, `app-accept-session.md`,
  which tests cite) and `engine/oracle/` must not be in the hand-off tree.

Questions for the owner (gaps found, not filled):

- 4.9: is the list of arrays whose dangling entries are removed rather
  than nulled exactly `/Kids`, `/Contents`, `/Annots`, or should other
  arrays of object references (for example `/Fields`, `/OCGs`, `/Order`,
  `/ON`, `/OFF`) also drop dangling entries?
- 4.4 item 7: what counts as "a damaged keyword in that place" (any
  keyword-like token directly before a dictionary outside every object, or
  only near-misses of `trailer`)?
- 2.2: when a cut-short object is kept (4.4 item 3, implementer's choice),
  does it count in `objects_recovered`, and is it `damaged`?

## 12. Review addenda (2026-10-18)

- **A1. Object glued to `%%EOF`.** An update whose first object header follows
  `%%EOF` on the same line (`%%EOF1 0 obj`, no end-of-line between them) is
  read by the strict reader as two sections. Reconstruction must find that
  header too. `%%EOF` ends its comment at the marker itself, not at the end of
  the line. Test: `app_accept_nav.rs`
  `an_update_whose_first_object_follows_eof_on_its_line_is_read` (its
  repaired case, with a wrong final `startxref`, shows the glued object's
  bookmark).
- **A2. Xref subsections (5.1 erratum).** The output xref may be written as
  subsections over runs of used object numbers, not one section from 0 up.
  The strict reader reads both forms the same way, and the literal form
  breaks on a single huge object number.
- **A3. Kids of another ISO type (4.10 rule 2 erratum).** A page-tree kid
  typed as another ISO type that stays a page keeps its /Type as read.
- **A4. /Encrypt naming an absent object on the intact path (4.7 erratum).**
  P5 wins. When the chain is usable but /Encrypt names an absent object, the
  chain is treated as unusable, or the encryption dictionary is found by type
  as reconstruction does. Ciphertext is never shown as text.
- **A5. Work accounting (P6).** Bytes the lexer reads ahead (for example the
  look-ahead for `N G R` after a top-level integer) and bytes produced by
  decode attempts that fail both count against the scan and decode budgets.
  Inputs such as `1 0 obj 5 (` or `trailer 5 (`, repeated many times, scan in
  linear time.
- **A6. No encryption dictionary at all.** When /Encrypt dangles and
  reconstruction finds no encryption dictionary by type, `rebuild` refuses
  with `Invalid("encryption dictionary")`.

## 12 (continued). Corpus-run addenda (2026-10-18)

Found by the corpus run of the clean-room tree against the baseline. The
experiments are in `reviewer archive corpus-addenda-2026-10-18/`
(`exp_a7_a10.mjs`, self-authored, builds each input on the base document of
7.2; raw output `exp_a7_a10.txt`; reference program MuPDF 1.28.1 through
the npm `mupdf` package: page count and each page's text).

- **A7. Which stream is a hint stream (4.11).** /H gives the offset of the
  hint stream object from the beginning of the file (Annex F). The hint
  stream is the stream whose definition (its `N G obj` header) begins at
  that offset, with the same whitespace slack 4.3 allows for xref
  offsets. A stream that merely contains the offset is not a hint stream
  and is kept: files rewritten after linearization (incremental updates
  appended by another writer) keep a stale linearization dictionary
  whose /H points into some other object, often a page's content stream.
  The linearization dictionary itself is still dropped and `linearized`
  set. Experiment: linearization dictionary first, /H pointing ten bytes
  into the data of content stream 6, intact classic xref. MuPDF shows
  3 pages, `Alpha`, `Beta`, `Gamma`. This spec: the same, `linearized`.
  Test (`cleanroom_repair_corpus.rs`):
  `a7_a_hint_offset_inside_another_object_drops_nothing_else`.

- **A8. A `/Prev` loop over an incomplete chain (4.3).** When the chain
  ends at a `/Prev` loop, it is usable only if the sections read give an
  entry, in use or free, for every object number from 0 to the newest
  trailer's /Size − 1. Every complete file has such entries (§7.5.4: the
  original section covers every object; §7.5.6: updates add sections and
  keep the earlier ones reachable through /Prev). A loop that hides older
  sections leaves numbers without an entry: the chain is then not usable,
  and the file is reconstructed (4.4), `damaged`. A loop after sections
  that do cover every number is still not damage (4.3; control test
  `a8_a_prev_loop_after_a_complete_section_is_not_damage`). The 8.3 table
  already requires this: qpdf `append-xref-loop.pdf` must show 30 pages.
  Experiment: the base document, then an update (catalog 1 and /Info 10
  rewritten) whose /Prev names its own section. MuPDF shows 0 pages. This
  spec: 3 pages, title `Updated` (P3), `damaged`. Test:
  `a8_a_prev_loop_that_hides_the_older_sections_is_reconstructed` (the
  app opens it with 3 pages, repaired).

- **A9. Must-hold list erratum (8.3), owner to confirm.** pdf.js
  `PDFBOX-4352-0.pdf` names an /Encrypt object whose dictionary is damaged
  past reading, and no other encryption dictionary exists. Under A6 and P5
  the rebuild refuses with `Invalid("encryption dictionary")`. The 8.3
  line "pdf.js `PDFBOX-4352-0.pdf`: 1 page" is replaced by "refused (A6)".
  The reference viewer opens that file as one blank page; its content is
  undecryptable either way. Test (holds today):
  `a9_an_unreadable_encryption_dictionary_refuses_the_rebuild`.

- **A10. Damaged end keywords after a right `/Length` (4.6 rule 3).** When
  `/Length` lands, after optional whitespace, on a keyword that is not
  `endstream`, and the next complete object definition begins after it,
  the data ends at `/Length`. This holds whether or not the `endobj` that
  follows is damaged too. A stream whose extent 4.6 determines is never
  dropped as a cut-short object (4.4 item 3's option to drop applies to
  values that do not complete, not to such a stream), on the usable-chain
  path as in reconstruction. The object is still damage. Experiment:
  content stream 6 with the right `/Length`, then `enxstream` and
  `enxobj`, then the base document's objects. MuPDF shows page 1 as
  `Alpha` and `Beta` (it searched on into stream 7). This spec: page 1
  `Alpha`, pages 2 and 3 as before, `damaged`. Test:
  `a10_a_stream_with_damaged_end_keywords_keeps_its_declared_data`.

- **Corpus findings that are expected (no rule change).**
  - A fuzzed file defining one content stream several times, once whole and
    once with spliced garbage, now shows the whole definition's text on its
    first page, where the reference viewer and the baseline drew it blank.
    That follows 4.4 item 4 (a complete definition outranks a fragment) and
    is an improvement.
  - A file whose last xref stream was written with a sync flush (no final
    DEFLATE block) is read through the lenient decoder (02 §3.6) and is not
    damage. Its lost page was A7, not the filter.
