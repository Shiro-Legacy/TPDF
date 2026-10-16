# Corpus: opening, repair and saving

October 15, 2026. Fixes for the external corpus differential's open, repair and save classes (`reference/corpus-20261015/README.md`, baseline `summary-426a2c6.json`), in `serialize.rs`, `lexer.rs`/`parser.rs`, `stream.rs`, `security.rs`, `document.rs`, `repair.rs`, `writer.rs` and `src/app/{document,edit,journal,save_tree,overlay}.rs`, with `ops/core.rs`, `ops/sign.rs` and `ops/stamp.rs` calling them. Read with `../APP-ENGINE-DESIGN.md` and `app-core.md`. Corpus files are named by source, path and SHA-256 prefix only; every regression test builds its own file (`tests/app_corpus_open.rs`, `tests/serialize_roundtrip.rs`).

Owner decisions applied: the editor opens what MuPDF opens wherever that is safe, and every save is a valid file that MuPDF and Rust both reopen with the pages Rust showed.

## Serializer

- An empty name (the solidus alone) is a complete token that a regular character would extend. The compact printer separated tokens only when neither side was a delimiter, and `/` is one, so `<</ 5>>` was written `<</5>>` (key and value merged) and `[/ 1 0 R]` as `[/1 0 R]`. A token now takes a space after a solidus that ends the output, which only an empty name leaves (a name's own solidus is escaped `#2F`). The canonical style already separated every token.
- `tests/serialize_roundtrip.rs` serializes 20,000 generated objects (empty names as keys, values and array items next to every kind of token, delimiters, `#`, escapes and arbitrary bytes in names and strings) in both styles and parses them back: equal in the canonical style; in the compact style equal up to its documented real printing (f32 rounding, integral reals as integers, integral f32 beyond i32 written without a point). Both tests fail before the fix. A third writes a whole document with empty names as a catalog key, a value, array items and a stream-dictionary key through the app's compact save, the canonical rewrite and the repair's output, and reopens each with the same objects and stream bytes.

## Allocation past MuPDF's object-number limit

- Rule: allocation takes the floor (past every /Size up to 8,388,608, every object of the file and every number allocated before; never lowered) while the floor is at most 8,388,607 (`MAX_OBJECT_NUMBER`, MuPDF's `PDF_MAX_OBJECT_NUMBER`). Once it passes that (a file holding a larger number, such as 2,147,483,647, or using the numbers up to it), allocation takes the lowest number from a cursor that only rises which no object holds or references, in the document or in any state the journal holds (both branches and the open operation, trailer states included), and which was never handed out (`LowNumbers`, a 1 MiB bitset built once per session on the first such allocation, then each handed-out number marked). Later changes reference only objects that existed when it was built or numbers handed out since, so, as with the floor, a stale or dangling reference never resolves to a new object and undo or abandon never lead to a reuse.
- The view's private objects (widget appearances, redefinitions) keep numbering past the floor; they are never saved.
- A document edited with no journal (`edit::Builder`) and a save's rebuilt page tree (below) use the same `LowNumbers` (without journal states) once their numbers pass the limit, so no new object anywhere gets a number MuPDF does not read.
- Not changed: an object the file numbers past 8,388,607 stays in the document under its number (MuPDF's repair drops it); a save writes it if it is reachable, and MuPDF then repairs the save. None of the corpus files has one reachable.

## Encrypted object streams in repair (`rebuild_with_passwords`)

- MuPDF repairs object streams after authenticating the empty password. The repair now authenticates the encryption dictionary (its indirect values resolved) with the first password that works (the empty one by default; `AppDocument::load_any` passes the encodings of the password being tried), decrypts each container with that key, and writes the members as top-level objects whose strings are encrypted again under their own object keys (`OutputCipher::strings`, IVs from a fixed-seed DRBG: the output is internal), so the reader decrypts every string once. The encryption dictionary itself is never re-encrypted. Containers named by a type-2 xref entry are read whatever their /Type, as MuPDF reads them.
- When no password authenticates, the repair writes the structure only (`RepairReport::locked`): the containers as ordinary streams, no members, references, catalog and page tree untouched. The app opens that locked (`open_shared_locked` semantics); `authenticate` reloads from the source with the password, which repairs again with the key. `open_tolerant` keeps refusing that case ("object streams in an encrypted document"), as before.
- Linearized files: the strict reader still refuses a first-page xref whose `startxref` is the dummy one and whose /Prev points forward. Tolerating that layout in the strict reader would move every linearized file from the repair's xref path (`Opened::Rebuilt`, not damaged, versions counted as MuPDF counts them) to the strict path, which changes `versions`, render bases and incremental signing for hundreds of files the baseline reads correctly; it was not done. The repair's xref path already reads them without damage, and now reads their encrypted object streams too.

## Streams naming an external file

- MuPDF ignores /F, /FFilter and /FDecodeParms on a stream and decodes the stream's own bytes with /Filter (measured: a content stream with `/F (missing.bin)`, with and without `/FFilter`, draws the same 2,500 pixels). The app reads it so (`OpenOptions::external_streams`, `app::document::open_options()`: load, locked load, validate, signing); the bytes and the three keys are saved unchanged. `Document::open` (strict, default options) still refuses such a document, as the existing tests require; `stream_object` decoding no longer refuses /F, since only a document opened with the option can hold one.

## Encryption dictionaries (measured against MuPDF 1.28.1 on generated revision 3 and 4 files)

- A missing /P, or one that is not an integer (a real included), reads as -4 (every permission) in the key and in the permissions, as MuPDF's integer check reads it: MuPDF opens a file whose key was hashed with -4 and no /P, and asks for a password when it was hashed with 0; with `/P 100.0` it opens a key hashed with -4 and asks for a password for one hashed with 100 (measured).
- Revisions 2-4: a /U shorter than 32 bytes is zero-padded (MuPDF warns "encryption password key too short"); revisions 3 and 4 compare 16 bytes, so 16 suffice and 8 do not (MuPDF: the same). A short /O is still refused (MuPDF: "encryption dictionary missing owner password"). Revisions 5 and 6 are unchanged.
- Indirect values in /Encrypt (/CF, its filters and their entries, three levels) are resolved before authenticating (`security::resolve_encryption`), as MuPDF resolves them.
- An /Encrypt that is not a dictionary (an array, a number, a reference to a non-dictionary) names no handler: the document is read as it is (MuPDF the same; its strings and streams are not decrypted). `DocumentInfo` says `encrypted: true`, `algorithm: "unknown"` (engine.ts on MuPDF says `rc4-40`, reading V, R and /Length of a non-dictionary as undefined); `keep` writes plaintext without it, and `validate` expects that (`keep` expects encryption only where a handler was authenticated). The repair keeps such an /Encrypt as it is (a dangling reference becomes null) instead of refusing the file.
- Authentication is not weakened: every password is still checked against the file's own /O, /U (or /OE, /UE).

## Lexical damage the repair reads through (as MuPDF does)

- A `#` in a name without two hex digits after it stands for itself (`/A#G0` is `A#G0`, `/A##41` is `A#A`; measured). Only the repair's parser does this (`Parser::with_lenient_names`); the strict lexer still refuses, as its tests require. The serializer escapes the `#` (`#23`), so saves are valid.
- Flate in object and xref streams: data after the end of the zlib stream is ignored, and a truncated or corrupt stream or a bad checksum keeps the bytes produced (`stream::decode_filter_specs_lenient`), tried after the strict decode of every candidate end. Stream-length recovery still decodes strictly.
- /Fl names FlateDecode in object and xref streams (MuPDF accepts the abbreviations anywhere). The strict structural decoder and `stream_object` (whose tests refuse `Fl`) are unchanged.
- A stream whose `endstream` keyword is damaged (`endobj` within 32 bytes after its /Length, before the next `endstream`) ends at its /Length. The repair used to search on for the next `endstream`, swallowing the objects after the stream (qpdf `xref-compressed-in-compressed.pdf` c99080e8d936 lost a page this way; MuPDF lists that page but cannot read it either and draws it blank). A /Length that lands on `endstream` is used as before; otherwise the search is unchanged.
- A page the tree lists whose object header was found but whose value cannot be parsed (for example a key without a value) is kept as an empty dictionary, so it stays a blank page with what it inherits, as MuPDF shows it, and the page count is the tree's. A /Kids entry naming no object at all is still dropped (MuPDF shows a blank Letter page for it).

## Objects freed by an incremental update

- hayro ignores free xref entries, so a render base that was the input file showed an object an update had freed (qpdf `incremental-3.pdf`: a deleted content stream still drawn, while the document, MuPDF and the save had none). `Document::freed_by_update` records whether a newer section frees an object an older one has in use; the render base is then a rewrite of the document (`Base::input` declines), as for encrypted inputs.

## Saved page trees (`app::save_tree`)

- Decision: save time, not open time. The save writes a copy of the document whose page tree is one /Pages node listing exactly the app's pages (`info::pages`) when MuPDF's reading of the tree could differ from that list; the live document, its journal (undo, redo, abandon) and the incremental saves that keep a signed prefix never see it. Open-time normalization would have to be written into the session's base (changing what `snapshot_bytes` and the signed prefix hold) or journaled (a step the user did not take).
- Intact (written as it is): the catalog's /Pages is an indirect, non-stream dictionary typed /Pages without /Parent; every /Kids entry is a reference to a distinct non-stream dictionary, or a page dictionary written directly into /Kids (MuPDF draws it; measured), whose /Parent names its node (a node written directly into /Kids is rebuilt: its pages have no /Parent to inherit through, which MuPDF and the app both read as nothing inherited); MuPDF and the app classify each kid alike (MuPDF: a /Type other than /Pages is a page, an untyped dictionary with /Kids and no /MediaBox a node; the app: /Type /Pages with /Kids); every node's /Count is the number of pages under it; and the pages in order are the app's.
- Otherwise each page gets /Type /Page, /Parent the node, and what it inherited through its /Parent chain written onto it (/Resources, /MediaBox, /CropBox, /Rotate; a direct /Resources dictionary becomes one new object the pages share); a page written directly into /Kids gets its own object; the catalog's node object is reused unless it is one of the pages; a catalog the page list takes for a page keeps its /Type. Other nodes become unreachable and are not written. New objects take numbers past every number an object holds or a reference names (a dangling reference must not come to name a new node), or, when that would pass 8,388,607, the lowest numbers none of them uses; the copy is never journaled, so the session's allocation floor is not involved. Used by `save`, `sign-prepare`'s full save and `batch-file`.
- Cost: an intact check walks the tree once per save (O(pages)); a rebuild clones the document's object map once.

## Page lists that differ from MuPDF's (decisions)

- A page object listed twice in /Kids (qpdf `duplicate-page-inherited.pdf` 5ef7073b12f8 and `page_api_2.pdf` 170dc821a5b5): MuPDF (and Acrobat, and qpdf after copying the object) show it twice; the app shows it once (hayro's walk, `pagetree::normalize` before every page edit; recorded in APP-ENGINE-DESIGN). Kept: page ids are object ids, so two positions of one object would be one page under two indices, and an edit through either would change both. The save writes it once, so MuPDF reopens the save with the pages the app showed (`save-pagecount` against MuPDF's reading of the source is the expected consequence). Decision needed only if MuPDF's count is wanted: open time would have to copy the repeated object (qpdf's fix), which changes the document the user opened, its page ids and a signed prefix.
- A /Kids entry naming no object: dropped, where MuPDF shows a blank Letter page (it cannot read the page either). Fuzzed files (qpdf `fuzz/qpdf_extra/69969.fuzz` 083e9db240e1, `69977e.fuzz` d35136962510, `71624.fuzz` fe4e80441943) are spliced from several files; most of MuPDF's 9-30 pages there are such blank placeholders or other files' objects, and the app shows the pages it can read (22, 8 and 3). Left as they are.
- qpdf `xref-compressed-in-compressed.pdf` (c99080e8d936): one page is listed twice (above) and one followed a stream with a damaged `endstream`; the repair now keeps that page (with its content, which MuPDF draws blank), so the app shows 29 of MuPDF's 30.
- pdf.js `issue22011.pdf` (176ef523a383): one page object does not parse; MuPDF shows a blank page there, and so does the app now (placeholder above).
- qpdf `issue-99.pdf` (7613e0aa0732): MuPDF draws a blank Letter page; the repair recovers the page's own 595 × 842 MediaBox. The app's reading is kept, so MuPDF draws the app's save at that size (`save-render-differs`).
- pdf.js `issue3371.pdf` (0c3fb45d9b61), `pr6531_1.pdf` (a5584ab7da3a) and `print_protection.pdf` (e5497249e498): MuPDF opens them with 0 pages; the app (which refused them) now shows their page.

## Readings kept where the app differs from MuPDF

- qpdf `stream-line-enders.pdf` (14c62cf7f0af): content streams whose data follows `stream` after a lone CR, a space or nothing. MuPDF draws the page blank; the repair reads every stream and the app draws its text, as qpdf's test expects. MuPDF draws the app's save (streams rewritten with an LF) with the text.
- pyHanko `broken-objstream4.pdf` (20b8c82a3622): the xref names a slot of an object stream for the page contents that holds another object. MuPDF finds nothing there and draws the page blank; the repair takes the contents stream the file defines at top level and draws it. MuPDF draws the app's save the same way.
- In both, `save-render-differs` (MuPDF's drawing of the save against its drawing of the source) is the app's reading carried into the save, which the owner decision asks for.

## Gaps left to other pieces

- qpdf `name-pound-images.pdf` and `.qdf` (3b04f40397cf, ebe3d80c772e) now open (the repair reads resource names whose `#` is not followed by two hex digits as MuPDF does), but hayro's content parser refuses a name whose `#` lacks two hex digits, so the `Do` operators are dropped and the page draws blank (MuPDF draws three images). Fix belongs to the view (an upstream hayro change, or rewriting such names in the view's content); the save is valid (resources keys escaped, content unchanged, which MuPDF reads alike).
- pdf.js `PDFBOX-4352-0.pdf` (ede76ce3aaa8): the trailer's /Encrypt names an object whose value does not parse. MuPDF reports `encrypted: true`, `algorithm: "rc4-40"` (engine.ts reading an unloadable object); the repair drops the object and the app reports `encrypted: false`. Neither decrypts the content (both draw the page blank). Recorded, not changed (`info-differs`).
- pyHanko `malformed-encrypt-dict2.pdf` (4c19d85e3e7b): /Encrypt is not a dictionary; `algorithm` is `unknown` where engine.ts says `rc4-40` (recorded below).

## Measured

Corpus (5,049 files, `--jobs 2 --rss-mb 1200`, outside the repository; no summary committed). Columns: the recorded baseline (`summary-426a2c6.json`), this branch's parent rebuilt here (f0945be), and this work (the full run was taken before the last allocation refactor, which changes nothing below the object-number limit; the 66 cluster files and 461 others, all of govdocs1 and IRS and 250 of pdf.js, were rerun on the final build with the same classes for every file).

| class | 426a2c6 | f0945be | after |
| --- | --- | --- | --- |
| open-rust-refuses | 37 | 37 | 0 |
| validate-refused | 3 | 3 | 0 |
| edit-failed | 12 | 12 | 8 (all after a WASM trap in render: robust piece) |
| save-unreadable | 1 | 1 | 0 |
| save-unreadable-as-source (recorded) | 8 | 8 | 2 (MuPDF's WASM trapped earlier in the child) |
| save-pagecount | 5 | 5 | 6 |
| save-render-differs | 7 | 7 | 6 |
| save-text-differs | 6 | 6 | 5 |
| pagecount-differs | 7 | 7 | 6 |
| pagesize-differs | 3 | 3 | 2 |
| save-refused | 8 | 8 | 8 (the same trap files) |
| info-differs | 0 | 4 | 6 (+2 files that now open, above) |
| failing files | 723 | 732 | 695 |

- 39 files open that the parent refused; none it opened is refused. 43 files lost a failure class; 8 gained one: the two `info-differs` and three `render-pixels` of files that now open (gaps above; the third is a veraPDF output-intent colour difference), `save-pagecount` (and on `page_api_2.pdf` `save-text-differs`) for the two files with a page listed twice (decision above), and pyHanko `form-tree-circular-ref-input.pdf` (7d7847f4c69f) `render-pixels`, whose Rust render is byte-identical on both builds: MuPDF's WASM trapped before drawing it in the parent's run, so the forms difference was not measured there.
- Owner decision, checked on every record: of the 4,892 files Rust opened and saved, 4,890 saves reopen in MuPDF and in Rust, unrepaired, with the page count Rust showed (the parent: 4,846 of 4,858); the other two are files on which MuPDF's WASM had trapped earlier in the same child.
- Every save of the 66 cluster files reopens in both with Rust's page count; the edited saves of pdf.js `bug1980958.pdf`, `issue3188.pdf`, `issue5896.pdf` and pyHanko `CompactedPDFSyntaxTest.pdf` reopen in MuPDF unrepaired with the added Square.

Fixtures: all 83 pages of the 54 fixtures render byte-identical through the app's view (`examples/view_probe`, `TPDF_VIEW_APP=1`, 72 DPI; pages 1-4, 251 and 500 of each fixture that has them) before and after.

Tests: Rust 1,333 passed before, 1,354 after (3 ignored; stable and 1.92): `tests/app_corpus_open.rs` 18, `tests/serialize_roundtrip.rs` 3. App on Rust (`tests/run-rust.mjs`): 524 passed, 20 failed before; 525 and 19 after: the redaction and mark tests, whose ops are on a sibling branch; the twentieth before is the timing-bound /W test, which fails in some full runs of either build. Default suite: 553 passed, 0 failed, 102 skipped.

## Differences from MuPDF

- An object numbered past 8,388,607 is kept (MuPDF's repair drops it); new objects never get such a number.
- An encrypted file whose object streams no password opens is locked with its structure only; MuPDF's repair reads such containers as garbage after failing the empty password.
- /Encrypt that is not a dictionary: `algorithm` is `unknown` (engine.ts: `rc4-40`), and a save drops it (MuPDF's `keep` writes it back).
- A /Kids entry naming no object is dropped; MuPDF shows a blank page.
- A page listed twice is shown and saved once.
- A stream whose `endstream` is damaged ends at its /Length when `endobj` follows within 32 bytes; MuPDF searches on and loses the objects after it.
- Saves of a tree MuPDF would read differently are flattened to one node; MuPDF saves the tree as it is (and reads its own save differently from the app).
