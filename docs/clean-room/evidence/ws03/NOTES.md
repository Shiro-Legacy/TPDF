# Notes from workstream 03 (repair)

## Acceptance status

Run through `tools/slot`, debug build, 2026-10-18.

- `tests/repair.rs`: 21 of 21 pass (the 3 un-ignored included).
- `tests/cleanroom_repair.rs`: 22 of 22 pass (the 7 un-ignored included).
- `tests/app_sign.rs`: 26 of 26 pass (`a_tail_just_inside_the_locator_takes_an_update` included).
- `tests/dictionary.rs` 14/14, `tests/serialize_roundtrip.rs` 3/3, `tests/app_info.rs` 2/2.
- Unit tests: `repair::` 8/8 (addenda A1, A4, A5 and the 4.12 password case included), `app::document::` 3/3 (both repair cases).
- `cargo clippy --all-targets`: no finding in `src/repair.rs` (`-D warnings`
  fails on dead code in other workstreams' stubs).

## Tests that fail only inside other workstreams' stubs

Each panics in a `todo!()` of workstream 06 or 07 when the app opens the file
(`src/app/forms/appearance.rs:65` unless noted), before any repair assertion:

- `tests/app_accept_open.rs` (9): `a_damaged_trailer_keyword_keeps_the_file_identifier`,
  `a_dictionary_missing_its_end_keeps_the_next_object`,
  `a_header_glued_to_a_damaged_object_keeps_an_intact_xref`,
  `a_kid_typed_as_another_object_that_looks_like_a_page_stays_a_page`,
  `an_untyped_intermediate_node_is_a_node_everywhere`,
  `an_untyped_node_reads_alike_with_and_without_repair`,
  `fixtures_with_a_dictionary_missing_its_end_keep_their_objects`,
  `junk_between_objects_does_not_hide_a_later_page`,
  `loose_dictionaries_do_not_override_an_intact_xref`.
- `tests/m10_fuzz.rs` (4): all four.
- `tests/repair_trailer_reread.rs` (2): both.
- `tests/app_core.rs` (16): `a_large_overlay_rebases_onto_a_rewrite`,
  `a_rebased_view_follows_a_reference_whose_target_appears_later`,
  `a_view_inside_an_operation_sees_its_writes_and_never_outlives_an_abandon`,
  `abandon_restores_and_drops_the_redo_branch_only_after_a_write`,
  `authenticate_refuses_then_unlocks_a_fresh_copy`,
  `batch_run_joins_the_open_operation_and_plan_skips_the_work`,
  `encrypted_documents_render_from_a_plaintext_base_and_snapshot_their_own_encryption`,
  `hayro_reads_an_overlay_on_classic_xref_stream_and_incremental_bases`,
  `junk_before_the_header_is_skipped_without_repair`,
  `many_revisions_open_and_past_the_cap_are_rebuilt`,
  `one_operation_is_one_step_and_undo_redo_restore_exact_bytes`,
  `open_repairs_damaged_files_and_reports_them`,
  `page_geometry_matches_the_view_on_every_fixture_and_odd_boxes`,
  `page_lists_follow_the_view_through_odd_trees`,
  `random_mutate_undo_redo_abandon_sequences_match_a_model`,
  `the_view_reads_a_removed_object_as_null`.
- `tests/app_corpus_open.rs` (32): every test but the 5 that pass; three stop
  elsewhere: `a_damaged_token_keeps_annotations_forms_and_the_catalog`
  (`src/app/annots/view.rs:45`), and the two `allocation_*` tests
  (`src/app/annots/appearance.rs:47`).

To check the repair-relevant parts of these anyway, a temporary test file
(not committed) loaded the same inputs through `AppDocument::load` and
checked page ids, page-content words, the repaired and locked flags, kept
objects and /Info titles for: all of `m10_fuzz.rs` and
`repair_trailer_reread.rs`; the app_accept_open tests above; the corpus tests
on encrypted object streams (locked, then unlocked), tolerated containers,
blank listed pages, a damaged `endstream`, name escapes, damaged tokens, a
damaged catalog /Type, a stray `]`, a catalog with /Linearized, damaged page
/Type values, untyped nodes, references to another generation, headers inside
damaged objects, missing or damaged /R and a non-hex /U digit; and app_core's
70,000 revisions and offsets that count junk before the header. All held.

## A test whose expectation conflicts with spec 03 section 4.3 (for the reviewer)

`tests/app_corpus_open.rs` `objects_appended_without_an_xref_leave_the_last_complete_revision`,
last step: it appends 1,100 spaces and expects the file to be repaired (and
"Tampered" shown), because an older locator searched about 1 KiB. Spec 4.3
requires the reader to use `incremental::startxref`, which skips trailing
white space and searches 16 KiB, so the last `startxref` is still found: the
file reads its last complete revision, unrepaired ("Body"). The test is
currently masked by a stub panic; once the stubs exist it will fail at that
step. The spec's scrub list (8.1, 8.5) already marks this test; the tail
should be computed from the locator, as `n14_reading_follows_the_writers_locator`
and `app_sign.rs` `locator_limit` do.

## Choices where the spec leaves room (R7, R9 and gaps)

1. Distances re-derived (R7), all private constants in `repair.rs`:
   header search 64 KiB (the app's own skip is shorter); an xref offset or
   `startxref` may land on at most 32 bytes of white space before its header
   or keyword; read-through work is budgeted at 4 × input + 1 MiB; a loose
   dictionary is read 32 levels deep; a damaged `endstream` keyword is at
   most 16 bytes. The locator distance itself is `incremental::startxref`'s.
2. R9, damaged trailer keyword: no keyword matching. Any dictionary outside
   every object that holds /Root, /Info, /ID or /Encrypt is a trailer during
   reconstruction (4.4 item 7), which covers `xrailer`, `8railer` and others.
3. R9, cut-short objects (4.4 item 3): dropped, never kept partially. They
   count in `objects_dropped`. With an intact chain they are
   `objects_unreadable` (damage); without one the result is damaged anyway.
4. "Complete definition": a value followed by `endobj`, `stream` (with a
   data extent), the next candidate header or `trailer`, `xref`,
   `startxref` or the end. A value followed by other tokens is a fragment
   (kept, but outranked by any complete definition or member). For bounding
   stream data (4.6 rule 3) only `endobj` or `stream` after the value counts.
5. Addendum A1: in the scan's first pass `%%EOF` ends at the marker, so a
   header glued to it (`%%EOF1 0 obj`) is a candidate; other bytes there are
   not. Otherwise the first pass does not track comments: a header after `%` on the
   same line is still a candidate. `%` inside strings (URIs, titles) is
   common, and treating it as a comment would hide later headers on that
   line in single-line files.
6. Output xref table (5.1, addendum A2): written as subsections over runs
   of numbers in use (gaps up to 8 filled with free rows).
7. Kids typed as another ISO type that are kept as pages (4.10 rule 1,
   addendum A3) keep their /Type; untyped, null-typed and
   damaged-typed pages get /Type /Page. Untyped nodes get /Type /Pages.
8. A stream whose data cannot be delimited (no usable /Length, no
   `endstream` before the next complete definition, not ending exactly at
   the end of the input) makes its object unreadable: dropped, or a blank
   page when listed.
9. Rule 4.6: a lone CR, a space (or spaces before an EOL) or nothing after
   `stream` are accepted and counted in `streams_length_recovered` even when
   /Length is right (test N8 c-space requires this).
10. An encrypted stream whose length was recovered (rule 2) is decode-checked
    with the key only when `password_dependent` is true (4.12, N16).
    Otherwise, and without the key, AES data takes the end that leaves whole
    16-byte blocks and RC4 data drops the EOL.
11. /Encrypt naming an absent object (addendum A4): the chain is treated as
    unusable; reconstruction uses an encryption dictionary found by type, and
    when there is none returns `Invalid("encryption dictionary")` (a reason
    string the spec's table does not list), so ciphertext is never read as
    plain text (P5).
12. `max_values` and `max_objects` are counted per path: the scan (always
    run, also to count `objects_overridden` with an intact chain) and the
    chain each stay within them, so memory for parsed values can reach about
    twice `max_values`.
13. A compressed xref entry whose container does not decode, or whose
    member at that index has another number, makes the chain unusable. When
    no password authenticates, compressed entries are taken as they are
    (the output is `locked`).
14. `objects_overridden` with an intact chain: the scan's readable
    definitions at positions the chain does not locate (xref streams,
    containers and linearization excluded), plus members of decoded
    containers the chain does not select.
15. Budgets (addendum A5): a candidate whose value begins with a number is
    read only up to the next candidate and charged that whole window, since
    the parser reads a token ahead for `N G R`. Each choice among a stream's
    candidate ends first decodes the shortest one leniently (charged), then
    tries each end strictly with output capped near that size; a failed
    attempt is charged its cap.
16. The output is written by repair.rs's own serializer, not the crate's
    `serialize::Writer`: section 4.5 requires every dictionary's key order as
    read, which only the canonical style keeps, and the canonical style
    writes strings in hex, which would double long strings past the token
    limit (5.1).
