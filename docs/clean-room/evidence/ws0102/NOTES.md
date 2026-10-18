# Notes for the reviewer (workstreams 01 and 02)

## Acceptance status

All twelve tests HANDOFF.md lists are un-ignored and pass. Every test file
in `engine/tests/` was run with `cargo test --no-fail-fast`; the failures
left are listed below. All but one fail inside another workstream's stub.

### Fails because of an owner decision (needs a test change)

- `tests/document_app.rs` `huge_reals_survive_a_save_and_reopen` asserts the
  old binary32 real format (`/Z 340282350000000000000000000000000000000.`,
  `/V 15000001000`). Under owner decision S2 (spec 01 §3.7.5) the compact
  output is `/Z 1000…000.` (1e300 exactly), `/W 9300000000000000000.`,
  `/V 15000000000`, `/U -2500000000`, and those read back exactly. The test
  is not in spec 01's §6.2 replacement list but belongs there. Changing it
  is outside my files.

### Fail inside other workstreams' stubs (expected)

- Workstream 03 (`src/repair.rs`):
  - `tests/app_corpus_open.rs`: 21 tests (`a_catalog_carrying_linearized_survives_repair`,
    `a_catalog_with_a_damaged_type_opens`, `a_damaged_endstream_keyword_keeps_the_declared_length`,
    `a_damaged_matrix_entry_keeps_the_form`, `a_damaged_token_in_a_page_dictionary_keeps_the_page`,
    `a_damaged_token_keeps_annotations_forms_and_the_catalog`,
    `a_listed_page_whose_object_does_not_parse_is_a_blank_page`,
    `a_name_escape_without_two_hex_digits_stands_for_itself_in_repair`,
    `a_non_hex_character_in_the_encryption_dictionary_reads_as_zero`,
    `a_page_whose_type_is_damaged_stays_a_page`, `a_reference_with_another_generation_names_the_object`,
    `a_stray_closing_bracket_in_a_page_dictionary_keeps_the_page`,
    `a_tree_with_an_untyped_node_is_read_and_saved_as_a_node`, `an_intact_tree_is_saved_as_it_is`,
    `an_object_header_inside_a_damaged_object_does_not_replace_the_object`,
    `an_encrypted_file_whose_object_streams_need_a_password_opens_locked_then_unlocks`,
    `objects_appended_without_an_xref_leave_the_last_complete_revision`,
    `open_tolerant_rebuilds_encrypted_object_streams_with_its_password`,
    `the_repair_reads_untyped_nodes_with_kids_as_nodes`,
    `repair_reads_object_streams_of_an_encrypted_file_and_decrypts_once`,
    `tolerated_containers_are_read_in_repair`)
  - `tests/dictionary.rs` `stream_length_and_repair_keep_positions`
  - `tests/number_overflow.rs` `an_overflowing_length_is_refused_and_the_stream_found_by_its_end`,
    `an_overflowing_xref_offset_is_refused_not_wrapped`
  - `tests/review_r3.rs` `document_prefixes_and_mutations_do_not_panic`
  - `tests/serialize_roundtrip.rs` `documents_with_empty_names_reopen_identically_in_every_writer_style`
  - unit test `app::document::tests::a_value_nested_past_the_parse_depth_is_still_repaired_away`
- Workstream 06 (`src/app/annots/appearance.rs`): `tests/app_corpus_open.rs`
  `allocation_past_the_object_number_limit_takes_free_low_numbers_once`,
  `allocation_with_the_largest_object_number_takes_free_low_numbers`.
- Workstream 07 (`src/app/forms/appearance.rs`, `src/app/forms/text.rs`):
  `tests/app_corpus_open.rs` `a_missing_p_entry_reads_as_every_permission`,
  `a_page_list_other_readers_would_read_differently_is_saved_as_one_node`,
  `a_rebuilt_tree_never_takes_over_the_catalog_or_the_info_dictionary`,
  `a_rebuilt_tree_writes_what_each_page_inherited`,
  `a_missing_or_damaged_encryption_revision_is_inferred_from_v`,
  `a_signed_file_with_an_appended_object_shows_the_signed_revision`,
  `a_stream_naming_an_external_file_is_read_from_its_own_bytes`,
  `a_short_u_entry_is_zero_padded`, `an_encrypt_entry_that_is_not_a_dictionary_names_no_handler`,
  `an_object_an_update_frees_stays_gone_in_the_view`, `indirect_crypt_filters_resolve`;
  `tests/number_overflow.rs` `an_overflowing_integer_in_a_dictionary_and_an_array_keeps_its_value`;
  unit test `app::ops::scan::tests::numbers_tags_and_routes`.
- Workstream 08 (`src/app/ops/pages/images.rs`): unit test
  `app::ops::pages::images::tests::truncated_jpegs_and_pngs_are_refused_without_panicking`.

Still ignored: `tests/app_journal.rs` `journal_costs` (a timing harness,
`#[ignore]` without a clean-room reason; not mine).

## Spec ambiguities and how I resolved them

1. `ParseError` display for a lexical error: "the lexical error's text" is
   read as the lexer error's full `Display`, so the message is
   `PDF object error at byte N: PDF lexical error at byte N: …`.
2. `expected` labels in parse errors (spec gives only `found` labels):
   `object` for a value, `name` for a dictionary key, `]` and `>>` for
   unterminated containers.
3. Lenient parser, `>>` where a dictionary value belongs (`<</A >>`): spec
   §4.2 is silent; it stays an error, as in strict mode.
4. Lenient rule 4 when the token after the integer is a lexical error: the
   integer is returned and the error surfaces on the next read (the whole
   value then fails anyway).
5. Compact separators: the first token of a top-level `value` or
   `dictionary_entries` call takes no space after a regular character the
   writer itself wrote, as §3.7.2 says literally; after an empty name it
   does.
6. `dictionary_entries` and `stream_dictionary` count the dictionary itself
   as one object, plus one per key (including an added `/Length`).
7. S3 (`/Type`, `/Subtype` first in stream dictionaries) applies to the
   compact style only; canonical `stream_dictionary` after `keep_order()`
   keeps the dictionary's own order exactly.
8. Lenient ASCII85 without `~>`: a final partial group is decoded as at
   EOD; a final one-digit group is still `InvalidAscii85Group`.
9. Lenient LZW without EOD: accepted once the leading clear code was read;
   left-over bits shorter than a code are ignored. A missing leading clear
   is still `MissingLzwClear` (S4 relaxes end markers only).
10. Lenient decoders keep strict behaviour for everything except missing
    end markers and Flate damage: trailing data after an end marker, bad
    bytes, overflow and truncated runs still fail.
11. Preset dictionary (S6): strict gives `InvalidFlate` at offset 1;
    lenient gives empty output, which the test allows.
12. Flate error offsets are the number of input bytes the inflater had
    consumed (S6: need not be exact).
13. Limit-error offsets: a stage reports the input offset at which its
    output would pass the cap (for a predictor, the start of the first row
    that would not fit; for the no-filter copy, 0).
14. `Dictionary::is_sorted` is removed (spec 01 §2.3, D1 default); nothing
    in the tree calls it.
15. `Lexer::seek` after a stored failure does nothing, so the position stays
    where the failure left it (§2.1: "position() no longer changes").

## Items that need a reviewer's eye

- `Dictionary` operations allocate infallibly (their signatures return no
  `Result`); spec 01 §5 asks for fallible reservation where sizes depend on
  input. Parser and lexer growth is bounded by their limits first, so a
  dictionary never grows past `max_objects`. `with_capacity` treats its
  argument as a hint capped at 65,536 entries.
- The parser has no allocation-failure kind; a failed array reservation is
  reported as `ParseErrorKind::Lex(LexErrorKind::AllocationFailed)`.
- `Writer::keep_order` has no caller until workstream 03 (repair) lands; it
  carries `#[allow(dead_code)]` so my files stay clippy-clean.
- `cargo clippy --all-targets -- -D warnings` cannot pass for the crate as a
  whole while other workstreams' stubs are in place (dead code and
  unreachable-code warnings in `src/app/forms/*`, `src/app/ops/pages/*`,
  `src/view/*`); it reports nothing in `lexer.rs`, `parser.rs`,
  `object.rs`, `serialize.rs` or `stream.rs`.
- Two unit tests were added inside my files: lenient parsing of random
  bytes (`parser::tests`) and every filter and predictor, strict and
  lenient, on random bytes (`stream::tests`).
