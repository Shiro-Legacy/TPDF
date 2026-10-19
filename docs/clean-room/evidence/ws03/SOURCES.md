# Sources used for workstream 03 (repair)

Everything below was read or used while writing `engine/src/repair.rs`.
No source outside this directory was opened; no web search was made.

| What | Where from | Used for |
|---|---|---|
| `specs/03-repair.md` (incl. section 12 addenda A1 to A10), `specs/01-syntax.md`, `specs/02-filters.md`, `specs/OWNER-DECISIONS.md`, `specs/SPEC-RULES.md`, `HANDOFF.md` | this directory | The behaviour, interface, limits and acceptance of the component |
| ISO 32000-2:2020 §7.2.3 (Tables 1 and 2) | the standard, from the implementer's own knowledge (no copy is in the tree) | White space and delimiter classes of the scan and the serializer |
| ISO 32000-2 §7.3.4, §7.3.5, §7.3.8.1, §7.3.10 | as above | Literal/hex string and name syntax of the output; the EOL after `stream` and before `endstream`; references to absent objects read as null |
| ISO 32000-2 §7.5.2, §7.5.4, §7.5.5, §7.5.6, §7.5.7, §7.5.8 (incl. §7.5.8.4 hybrid files) | as above | Header, classic xref table and trailer, incremental updates (newest section wins, free entries), object streams, xref streams (/W, /Index, field defaults), hybrid sections |
| ISO 32000-2 §7.6.1, §7.6.2 | as above | Which objects are encrypted: every top-level string and stream with its own object key; the encryption dictionary and xref streams are not |
| ISO 32000-2 §7.7.2, §7.7.3 (§7.7.3.3, §7.7.3.4) | as above | Catalog, page-tree nodes and pages, required /MediaBox, inherited attributes (/Resources, /MediaBox, /CropBox, /Rotate) |
| ISO 32000-2 §8.9.7 | as above | Filter-name abbreviations (`/Fl`, `/AHx`, `/A85`, `/RL`, `/LZW`) accepted for containers |
| ISO 32000-2 Annex F | as above | The linearization parameter dictionary (/Linearized, /L, /H, /O, /E, /N, /T) and its hint streams (/H offsets) |
| ISO 32000-2 object /Type values (catalog, fonts, annotations, XObjects, structure, signatures, collections, multimedia, 3D, DPart, …) | as above | The list `ISO_TYPES` of "another kind of object that ISO 32000-2 defines" (spec 03 section 4.10 rule 1) |
| Surviving engine code in this tree: `document.rs`, `incremental.rs` (`startxref`), `parser.rs`, `lexer.rs`, `object.rs`, `serialize.rs`, `stream.rs`, `stream_object.rs`, `security.rs`, `encrypt.rs`, `app/document.rs`, `app/info.rs`, `app/ops/sign.rs`, `app/pagetree.rs` | this directory | The APIs the component builds on and what the strict reader accepts |
| Acceptance tests in `engine/tests/` and the fixtures in `fixtures/` | this directory | Expected behaviour; inputs for local checks |
| Rust standard library behaviour (`f64` `Display` prints the shortest round-trip digits without an exponent) | the implementer's knowledge of the Rust documentation | Writing reals that read back exactly |
