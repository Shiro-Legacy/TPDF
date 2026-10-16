# Acceptance review: redact-add, redact-set, redact-search, redact-preview, redact-apply (round 1)

October 16, 2026. Fixes for the acceptance review of these ops at ed62a16 (verdicts: redact-add, redact-set accepted; redact-search, redact-preview, redact-apply not accepted). Read with `../APP-ENGINE-DESIGN.md` and `app-redact.md` (corrected where these fixes change it).

## Optional content shown only while a group is off (blocker, redact-search; apply's raw check)

- **Verified** (review probe p14, rerun): `view_unlayered` dropped /OCProperties, which turned every group on, so a membership (/OCMD) with /P /AllOff or /AnyOff over a group /D turns off hid its content; MuPDF draws it. redact-search marked page 1 only, apply returned ok and the copy still held the text: the raw check reads marked pages only and its `search_left` built the same view. A document with memberships and no /OCProperties hid it too (the cached view was used: hayro turns every group on; MuPDF draws everything without /OCProperties), and so did a Form whose /OC is a membership written into it.
- **Fix.** `layers::ShowAll` (`show_all`: one pass over the objects): the catalog without /OCProperties, every membership dictionary object redefined as `<< /Type /OCMD >>` (no groups: shown by hayro's rule and the view's text reader, whatever /P or /VE say), a Form's own direct /OC dropped. `AppDocument::view_unlayered` applies it in the view's private update (over the view's own redefinition of the catalog, which it overwrote before: a document with a direct /Pages lost that fix in this view) and returns None when nothing is optional, so redact-search uses the cached view then. redact-apply's raw check decides "has optional content" the same way and `search_left` applies `ShowAll` to its plain copy. A membership or group written directly into /Properties is not optional content to hayro or the text reader (each needs a reference), so nothing needs redefining there.
- Recorded in `app-redact.md` (Differences): redact-search reads every optional content, whatever layer settings and memberships say; MuPDF searches what it draws.
- Tests (`tests/app_redact_apply.rs`): `text_a_membership_shows_only_while_a_group_is_off_is_found_and_removed` (/AllOff, /AnyOff, /AnyOff with /VE /Not, no /OCProperties, a Form's own membership: two hits, pages [0, 1], the copy holds no secret and keeps KEEPTEXT); `the_raw_check_reads_text_a_membership_shows_only_while_a_group_is_off` (page 2's search mark replaced by an area mark elsewhere, the text split over two shows so only a search finds it: `verify-failed`, raw check). Both fail on ed62a16 (one hit; apply ok).

## Forms with large /BBoxes in `shared` (major, redact-preview and redact-apply)

- **Verified** (review probes p17, p17b, p18, p20, rerun): a touched Form placement's parts were every mark rect inside its box, so with a page-size /BBox the mark over placement B lay inside placement A's box, and carried into B's frame landed where nothing was marked. A Form drawn twice on one page with both occurrences marked, and a letterhead under a mark over body text, refused `shared-content`; engine.ts applied. The design note's "the same marks on both pages share nothing" held only with one placement per page.
- **Fix** (`region::shared`, `form_parts`). A Form placement is touched when a mark touches a draw inside it (any depth; positive overlap), and its parts are, per mark rect, the rect clipped to the union of the bounds of the draws inside it the rect touches. A marked page with a Form whose box a mark touches is walked again with `Detail::Full` (within the 64-million-token budget of the marked pages); a box-touched Form nothing of which is touched is an untouched draw (reported when another draw of it is touched). The box still decides for images, when the full walk fails, for a Form with a draw that has no position or that was not entered (self-invoking), and past 16,777,216 (draw, Form, rect) visits or 262,144 distinct (Form, rect) parts. Apply edits only the draws inside a Form that marks touch, so a Form not touched this way is not changed.
- Differences from engine.ts, recorded: a full-page Form placed with `q cm Do Q` under a mark over other content applies (engine.ts refused it, review probe p18 lh2); invisible content inside a Form (draws without area) does not make it touched.
- Tests (`tests/app_redact_apply.rs`): `a_page_size_form_drawn_twice_and_marked_alike_applies` (page-size and tight /BBox: `shared` [], applied, ACCOUNT and holder kept twice; one occurrence's mark deleted: shared [{0, form, [0]}] and `shared-content`); `a_letterhead_form_is_not_touched_by_a_mark_over_other_content` (`q /LH Do Q`, `q cm Do Q`, bare `Do`: `shared` [], LETTERHEAD kept on both pages; a mark over the letterhead on page 1 alone is shared with page 2). Both fail on ed62a16. The existing `shared` tests (`a_shared_object_marked_in_different_places_is_shared`, `shared_graphics_are_found_at_any_depth_and_in_appearances`, `many_touched_draws_of_one_object_are_compared_by_what_their_marks_touch`, the redact-shared-only fixture) pass unchanged.

## Structure element title (major, redact-apply)

- **Verified** (review probe p7): `clean_structure` removed /ActualText, /Alt and /E only; MuPDF also drops the element's /T (and /E), so a title naming the redacted figure or table stayed in the Rust copy. The stage B note misstated MuPDF.
- **Fix.** The element of each redacted MCID also loses /T; its ancestors lose /ActualText, /Alt and /E as before and keep /T (a section title names more than the redacted content; MuPDF keeps ancestors' entries). `sanitize.struct` is unchanged (neither engine clears /T). Note corrected in `app-redact.md`.
- Test: `the_structure_element_of_redacted_content_loses_its_title` (/T, /ActualText, /Alt, /E gone from the MCID's element; /Lang, /ID, /K, /S kept; the parent /Sect's title and an untouched element's title kept). Fails on ed62a16.

## Minors

- **Annotations under marks (redact-apply): recorded.** Every annotation a mark touches goes (Text notes, Highlights, hidden ones, straddling ones whole); MuPDF keeps all but the marks, so `sanitized`/`kept` annots counts differ and pdfjs issue19424.pdf changes outside its marks. Deliberate (an annotation's text cannot be cut to the mark). `app-redact.md` Differences, and the stage A Preview wording ("MuPDF keeps Widgets") corrected.
- **Undrawn Form holding the search text (redact-apply, review s40): recorded, unchanged.** Apply refuses `verify-failed` (raw check) where MuPDF's rebuilt resources drop the Form; failing closed beats writing a file that holds the text, and dropping it would need private copies of shared resources and certainty that nothing else (Type 3 glyphs, groups without /Resources) reaches them. `app-redact.md` Differences.
- **Sanitize surfaces (parity): partly fixed, the rest recorded.** `fields` also removes /RV and /I (a choice's selected indices: probe z08's selection was still named by `/I [0]` once /V went; /Opt stays, the form's own options); `layers` also removes the /Name of /D and each /Configs entry; `links` removes /URI, /F, /Win, /Mac and /Unix along each Link action's /Next chain (direct or by reference; 4,096 action objects, 32 levels). Counts unchanged. Still kept by both engines (`app-redact.md` Gaps): /AF embedded files (neither counts them), link /JS, OCG /Usage, /OpenAction and /AA targets. Test: `sanitize_clears_rich_values_choices_configuration_names_and_chained_link_targets` (all on: none of eleven tokens left, the choice has no /V or /I and keeps /Opt; all off: each surface kept).

## Measured

- Regression tests on ed62a16's engine (`engine/src` stashed, the tests kept): the six new native tests fail (search one hit; apply ok with the secret left; `shared` reported and `shared-content`; /T left; /RV, /I, configuration names and chained targets left); all pass with the fixes.
- The review's probes (`scratch/accept-redact-r0`, copied to `scratch/fix-redact/probes` and run on this build through the adapter, engine.ts on mupdf.js beside it):
  - p14 and a new p30 (six optional-content cases): Rust marks both pages and leaves nothing in every case (/AllOff, /AnyOff, /AnyOff with /VE /Not, no /OCProperties, a Form's own membership, a plain hidden group); engine.ts misses /AnyOff (mupdf.js 1.28.1 does not draw it although the group is off) and the hidden group, as before. p13's m15 (a Form hidden by a membership) now marks and applies as engine.ts does.
  - p17, p17b, p18, p20: f2/t2-both, f3/t3-same, lh1, lh2, lh4, fs-samepage-13 apply with nothing left and nothing lost (lh2 applies where engine.ts refuses); f1/t1-cross (different parts marked at two placements) still refuse, as recorded (in-place rewriting).
  - p12 (an area mark on page 1 of each of 54 fixtures, preview and apply): p0b/edit-shared and p3/edit-objects now equal engine.ts (they were the recorded broader `shared`); the other 52 unchanged.
  - p7: the element's /T, /ActualText, /Alt, /E gone; /Lang, /ID and the parent's /T kept, as MuPDF (the parent's /Alt also goes, recorded). p6: z07 (links), z09 (layers), z10 (element title) leave nothing; z08 keeps only the choice option in /Opt; z01 (/AF) unchanged in both engines (recorded). p2 (22 scenarios): Rust results identical to the review's run, s40 still refusing (recorded).
  - p1 (contract), p8 (encryption), p9 (41 queries on 12 fixtures), p11 (the cap): identical to ed62a16's Rust results.
- Timing through the adapter (median of 5, machine shared; one large area mark on page 1): preview of p0b/edit-shared, p3/edit-objects and redact-shared-only 0.3 to 0.6 ms (0.06 to 0.13 before: the second, full walk), p6/quartz.pdf 2.6 to 2.9 ms (2.2 to 2.3), large.pdf within noise (1.3 to 2.6 ms either build); redact-search unchanged (redact.pdf 1.3 ms, large.pdf 159 ms, quartz.pdf 7.6 ms on both).
- Gates (CARGO_BUILD_JOBS=2, RUST_TEST_THREADS=2, memcap 3000): fmt; clippy `-D warnings` and `cargo test --release --all-features` on stable and 1.92 (1,537 passed, 3 ignored, each); `engine/bridge` untouched; WASM build, `npm run build`. App on Rust (`node tests/run-rust.mjs`): 663 tests, 560 passed, 103 skipped, 0 failed. Default suite (`CI_NO_ELECTRON=1 node --test tests/*.test.mjs`): 674 tests, 564 passed, 110 skipped, 0 failed. No app test edited.

# Round 2

October 16, 2026. Fixes for the round-2 review at 4a7c756 (redact-add, redact-set accepted; redact-search, redact-preview, redact-apply not accepted). Each finding was reproduced natively first (the new tests below fail on 4a7c756's engine sources and pass now).

## Text past the page edge inside a Form (blocker, redact-preview and redact-apply)

- **Verified** (probe p33): `form_parts` decided a Form's touch by positive overlap of unclipped bounds, while apply removes a glyph by `GlyphRule::removes_glyph`, which first cuts the glyph's box to the page, so a mark on the page edge takes a glyph wholly past it. The Form counted as untouched, `shared` was [], and apply removed OFFPAGE from every placement (another page, the same page, a tight box, a path beside it). The Gaps entry "as with the box before" was wrong.
- **Same cause, found while fixing** (form_parts did not use apply's tests):
  - A Form drawn wholly past the edge was not walked again, because its box touched no mark with area. Its glyphs that the edge mark takes went unreported. ed62a16 had the same hole.
  - Apply cuts paths, images and shadings with each rect's reach past the page edges it lies on (`reach`), but parts were the rect only. A placement showing the reached part was not compared for it.
  - A clip apply cuts inside a Form (`W n`, which `Detail::Full` does not record) hides what the Form draws under the cut at every placement.
  - The part of a run apply removes whole was the rect clipped to the run, not the run.
- **Fix** (`region::form_parts`, `reached_parts`, `shared`, `object_parts`, `covered`; `reach` moved from apply into region):
  - The marked page is walked again as apply walks it (`Detail::Apply`) when a Form's box reaches a mark (`GlyphRule::reaches`: cut to the page, edges included).
  - A Form is touched where apply changes it, by apply's own tests: glyphs by `removes_glyph`, unmeasured runs by `reaches`, painted paths, images and shadings by positive overlap, and cut clips for the draws of the same Form under the cut.
  - Each change's part is its box clipped to the reach of each rect it comes from. For a glyph past the edge that is its own box, so a placement that shows it must cover it.
  - `covered` cuts a part to the other placement's page as the glyph rule cuts. A placement past the edge with a mark on that edge covers it, as apply there would remove it. A part without area counts where it lies in the box.
  - Image parts, and a Form's box where it decides, use the reach too; the box reaches by `GlyphRule::reaches`.
- **Results** (as native tests): another page, a tight box, a path beside the glyph, the same page, and a placement wholly past the edge report `shared` and refuse `shared-content`, as engine.ts does. Both placements past the edge, marked alike on that edge: `shared` [], applied, OFFPAGE gone from the Form, LEFTPART kept twice.
- **Difference, recorded** (`app-redact.md` Preview): a glyph a mark only grazes, short of its middle, stays, so it no longer makes its Form touched. 4a7c756 reported the Form's other placements; apply changes nothing there (page 2 renders identically, test). Under a clip that the same mark cuts, the Form is shared again.
- **Reversed from round 1**: text without area (size 0) that apply removes inside a Form now makes it touched, so its other placements are reported. Round 1 recorded the opposite ("invisible content … does not make it touched"). The removal is invisible, but page 2's text would lose it. Fail closed.
- Tests: `text_past_the_page_edge_inside_a_form_is_shared_where_another_placement_shows_it`; `a_form_is_touched_where_apply_changes_it_clips_included`, which covers:
  - a letterhead with a page-size clip under a mark over body text: `shared` [];
  - a grazed M without a clip: applied, page 2 unchanged;
  - the same under a clip, and size-0 text under the mark: shared with page 2.

## Copies keeping a stream that names an external file (major, redact-apply)

- **Verified** (probe p32): `one_revision` and `check_copy` reopened the copy with `Document::open`, which refuses a stream with /F. Any copy keeping one (an unmarked page's content, a Form) refused `verify-failed` "output is not one compact revision". This was pre-existing at ed62a16. Also, hayro refuses /F streams, so the copy's view could not read a kept /F Form on a marked page.
- **Fix**:
  - Both reopen with `open_options()`, as load, validate and signing do.
  - When the copy keeps such a stream, its view and the layered raw check's view are a plain copy with /F, /FFilter and /FDecodeParms removed (`readable_view`), read as the session's view reads the document.
  - The saved copy keeps the stream's bytes and keys (`app-corpus-open.md`).
- Recorded in `app-redact.md` (stage B Differences).
- Test: `a_copy_keeping_a_stream_that_names_an_external_file_verifies` (page content and Form: applied, /F kept, KEEPEX kept).

## The unlayered view skipped the view's preparation (major, redact-search)

- **Verified**: `view_unlayered` built its bytes from `view_fixes::redefinitions` only. It skipped three steps of `view()`:
  - the resource budget;
  - reading external-file streams from their own bytes;
  - `drop_sample_streams`.
  
  Since 4a7c756 any document with optional content searched it, so on such documents redact-search:
  - found 1 of 2 hits when one was in an /F stream;
  - built a view of 1,200 pages sharing 4,000 fonts that the cached view refuses (1.06 s, 396 MB in review);
  - refused `scan-budget` on a page whose /Contents include an image-filtered stream.
- **Fix**: `AppDocument::view_redefinitions` holds `view()`'s preparation, in its order:
  - the budget refusal;
  - redefinitions, with the shared color memo;
  - widget appearances, when asked;
  - external-file streams from their own bytes;
  - image-filtered page content dropped.
  
  `view()` and `view_unlayered` both call it. The unlayered view skips widget appearances, which are annotations and so not part of page text. `ShowAll` is applied on top. `view()`'s result is unchanged.
- Tests: `search_with_optional_content_reads_what_the_view_reads` covers:
  - an /F page stream or Form, plain and layered: 2 hits, pages [0, 1], applied, nothing left (before, layered: 1 hit and a refusal);
  - image-filtered page content, plain and layered: 2 hits (before, layered: `scan-budget`).
  
  `pages_past_the_views_resource_budget_refuse_search_and_apply` covers 1,100 pages sharing 4,000 fonts: search refuses "larger than the view's limit", plain and layered.
- Recorded gap (`app-redact.md` Gaps; pre-existing, plain or layered, fail-closed): a marked page whose /Contents include an image-filtered stream refuses preview and apply (`invalid`, "stream N cannot be decoded"). The view and MuPDF leave such a stream out, and engine.ts applies (probe p35).

## Minor: the copy's view had no resource budget (redact-apply)

- **Fixed**: before opening the copy's view or the raw check's copy, apply refuses `too-large` when `view_cost::over_budget` holds: "the pages' resources are larger than the view's limit, so the copy cannot be checked". The session's own view refuses such a document too. engine.ts applies it, slowly. Recorded in `app-redact.md` (stage B Differences). Covered by the test above.

## Measured (round 2)

- The review's probes (`scratch/accept-redact-r1`), copied to `scratch/fix-redact-r2/probes`, were run on this build through the adapter, with engine.ts on mupdf.js beside it.
  - **p33**: all four loss cases now report `shared` and refuse `shared-content`, as engine.ts does. off-edge-search applies with nothing lost (the Form's off-page word is out of the mark's reach), where engine.ts refuses by box.
  - **p32**: the four cases equal engine.ts (applied, nothing left, nothing lost; layered search 2 hits).
  - **p35**: search marks 2 plain and layered. Preview and apply refuse `invalid` in both (recorded gap).
  - **p34**: unchanged.
  - **p39** (1,200 pages, 4,000 shared fonts): layered search refuses in 4 ms at 137 MB (was 1.06 s, 396 MB). Apply refuses `too-large` in 40 ms at 158 MB (was ok, 841 ms, 473 MB).
  - **p37** (100k memberships): search 123 ms, 272 MB.
  - **p38**: 1,500 placements of a page-size Form under 1,500 search marks preview in 41 ms; 20-deep Forms with 1,900 marks in 95 ms. Both have `shared` [] and apply ok.
- **Regression sweep**: the Rust results equal round 1's on every scenario of p2 (22), p3 (36), p4, p5, p6, p7, p8, p13, p14, p15, p17, p17b, p18, p20, p30 and p36, and on the text output of p1, p9 and p12 (54 fixtures). One exception, in p31: ext-stream-layered now marks 2 and applies, equal to engine.ts (it was 1 hit and "not one compact revision").
- **Corpus sample** (the review's 174 files, same selection; `scratch/fix-redact-r2/corpus.jsonl`, sha256 prefixes and counts only): every Rust result equals round 1's (155 area and 65 search redactions applied, the same refusals, outputs unchanged by every measure).
- **Gates** (CARGO_BUILD_JOBS=2, RUST_TEST_THREADS=2, memcap 3000):
  - fmt;
  - clippy `-D warnings` and `cargo test --release --all-features` on stable and 1.92: 1,542 passed, 3 ignored, each;
  - `engine/bridge` untouched;
  - WASM build and `npm run build`.
- **App on Rust** (`node tests/run-rust.mjs`): 663 tests, 559 passed, 103 skipped, 1 failed. The failure is the known load-sensitive p3-edit "/W expansion" timing test (2.34 s against 2 s under suite load); it passes alone (`tests/p3-edit.test.mjs`: 32 of 32).
- **Default suite** (`CI_NO_ELECTRON=1 node --test tests/*.test.mjs`): 674 tests, 564 passed, 110 skipped, 0 failed.
- No app test was edited.
