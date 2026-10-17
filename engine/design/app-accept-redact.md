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

# Round 3

October 16, 2026. Fixes for the round-2 review at 82c8693 (merged as 46f1e95; redact-add, redact-set, redact-search accepted; redact-preview and redact-apply not accepted: one blocker, one major each, a minor on apply, and a minor on the accepted redact-search). Report section 4, "`redact-preview` and `redact-apply`". The bound statements in `app-redact.md` are corrected with these fixes.

## Form clip parts grew without a bound (blocker, round-2 regression, both ops)

- **Verified** (probes p46 and p40, rerun at safe sizes): `form_parts` pushed a 72-byte entry for each (clip, mark rect) pair while it walked, and checked `MAX_FORM_PART_WORK` only afterwards, in the pass over what the clips hide. Apply runs the same step first, inside `shared`.
- **Fix** (`region::form_parts`):
  - A clip a rect touches is kept as one entry, its draw.
  - Its rects are found again in the pass over what it hides. That pass charges the rects found, and each draw visited once per rect, as it goes, and falls back to the box rule past the budget.
  - Every other part is charged once per Form it is inside, as soon as it is found, glyph by glyph. Before, a run's parts were gathered first.
  - Glyphs and clips read are charged too.
  - The 262,144 kept (Form, rect) parts are counted across the document, not per page.
  - Memory is so bounded by the walk's draws plus the kept parts. The box fallback is the same rule that already applied past the budget (round 2), so the round-1 blocker (text past the page edge removed from Form placements nobody marked) stays closed. Probes p33 and p42 (every past-edge case) report `shared` and refuse as in round 2.
- **Found while fixing, same class, in apply.** Each touched clip or path held its own copy of every rect it touches, as 64-byte regions, before the 20-million-visit cut budget was checked. After the `form_parts` fix, p46's apply still peaked at 386 MB natively. With the walk's 500,000 draws under 2,000 marks, this would pass 4 GiB.
  - Fix: `Planner::hold` counts the regions the planned path, shading and image edits hold. Past 1,048,576 (64 MiB), apply refuses `too-large` ("the paths under the marks are too complex"). A path that goes whole holds none.
  - Each cut spends at least one visit per region, so the cut budget alone allowed up to 20 million. Plans between the two limits, with cuts that cheap, now refuse earlier. No probe or corpus result changed.
- **Pair comparisons** (`Pairs::of`):
  - Finding a draw's parts is charged to `MAX_PAIR_WORK`.
  - Each part's query against each other draw is charged and checked as it is made. Before, the budget was checked only before a class's comparisons.
  - Past the budget, a draw's parts are not found, and its class shows on every touched draw of its object (fail closed).
- Tests (`tests/app_redact_bounds.rs`; process RSS sampled while the op runs):
  - `clips_in_a_form_under_many_search_hits_stay_bounded` (p46: 50,000 clips, 100 hits): preview `shared` [] in 60 ms, +23 MB. Apply refuses `too-large` at +124 MB, 0.26 s.
  - `clips_in_a_form_under_the_mark_cap_stay_bounded` (p40 at 2,000 × 2,000): preview +3 MB. Apply refuses `too-large` at +66 MB.
  - `tests/rust-redact-bounds.test.mjs` runs the same cases on the WASM build, where memory never shrinks, so its size is the peak. The whole file, five cases, grows WASM memory from 4 MB to 138 MB. The test bounds the growth at 256 MB.

## Region queries were quadratic in file-borne rects (major, both ops)

- **Verified** (p41): 256 Redact annotations share one /QuadPoints array of 1,024 copies of one quad, 262,144 rects. Every rect spans the whole grid, because the grid's area is the rects' bounds. So every query cloned and sorted all 262,144 indices, and `covered` made one such query per part.
- **Fix** (`region::PageRects`, `Grid`):
  - A page's identical rects count once.
  - The index is a stack of grids, 32 down to 1 cells a side, over the finite coordinates' range. Each rect is listed at the finest grid where it spans at most 16 cells. A query visits its cells in each grid, with no clone or sort; `select` sorts only what it found.
  - A coordinate past the range, infinite ones included, maps to the nearest cell. That mapping never decreases as the coordinate grows, so closed overlap survives it and every rect a query meets is a candidate. Before, a rect with an infinite coordinate turned the index off for its page.
  - Past a bound, parts are not unioned. Instead, finding parts and each part's query are charged to `MAX_PAIR_WORK` (above), which is the bound the review asked for.
- **Found while fixing**: apply drew one black box per rect, so p41's copy held one path of 262,144 boxes. The view refuses to draw a path that long, so the pixel check failed every mark. Each distinct box is now drawn once.
- Tests:
  - `file_borne_rects_by_the_hundred_thousand_are_answered_once` covers p41 with 10 clips, and with 8 clips and an image under the quad. Preview takes 0.1 s, apply 35 ms, and the copy holds no MARKEDWORD but keeps FORMTEXT.
  - `the_rect_index_answers_as_a_scan_would_for_odd_and_repeated_rects` checks 2,000 rects, some infinite, inverted, NaN, huge or repeated, against 4,000 queries. `touches`, `contains`, `meets` and `touching` all equal a scan of the distinct rects.

## The pixel check walked every pixel of every rect (minor, redact-apply)

- **Fix** (`verify::pixels`, `uniform_rows`):
  - Marks are grouped by page once.
  - Each distinct pixel rect is checked once, for all the marks it belongs to.
  - Each row the rects cross is read once, recording where each pixel's run of one colour ends. A rect costs one step per row: it is one colour when its row's run from its left edge reaches its right edge, and that colour is its first row's. The cost is the page's pixels plus the rects' heights, not their areas.
  - Past 134,217,728 rect rows in all, the page's marks fail. Apply then refuses `verify-failed` rather than passing them unchecked. (Corrected in the final review: it now refuses `scan-budget`, below.)
- Tests:
  - `the_pixel_check_answers_as_a_walk_over_every_pixel_would` compares 600 random marks, some with repeated quads, some off the page or without area, against the old walk over every pixel.
  - `page_size_quads_are_checked_once_each_row_by_row` covers 262,144 copies of a page-size quad (0.13 s) and 1,024 distinct page-size quads under each of 256 marks (0.22 s). The review measured 290 s and 1,287 MB for the first; 16,384 quads took 18.8 s. In WASM, 262,144 page-size quads apply in 76 ms.

## Minor: redact-search refused past the view's budget without a tag

- **Fix** (`ops::redact::search`): the view's `TooComplex` refusals, the resource budget among them, now refuse `scan-budget`. The message is unchanged ("redact-search: page is too complex: the pages' resources are larger than the view's limit"). Other view errors stay plain.
- Test: `search_past_the_views_resource_budget_refuses_scan_budget` covers 1,100 pages sharing 4,000 fonts, plain and layered. Probe p39 now reports `scan-budget`.

## Measured (round 3)

- **Review probes, through the adapter** (`scratch/p8/redact/outs/p40-46.out`; RSS of the node process, about 140 MB at rest):

  | Probe | Op | Round 2 | Round 3 |
  |---|---|---|---|
  | p46 (3,972 bytes) | preview | 950 ms, 973 MB | 130 ms, 186 MB, `shared` [] |
  | p46 | apply | 983 MB, then `too-large` | 831 ms, 283 MB, `too-large` |
  | p40, 4,000 clips × 2,000 marks | preview | 1,345 MB | 111 ms, 217 MB |
  | p40, 4,000 clips × 2,000 marks | apply | not run | 424 ms, 319 MB, `too-large` |
  | p40, 2,000 × 2,000 | preview | not run | 81 ms, 217 MB |
  | p41 (67 KB), 10 or 8 clips | preview | 74-89 s | 0.25 s, 196 MB |
  | p41 | apply | not run | ok in 353 ms, 228 MB |

  At ed62a16, p46's preview took 53 ms at 146 MB. engine.ts refuses it `scan-budget` at 191 MB.
- **Regression sweep**: Rust's results equal round 2's on every scenario of p2, p3, p4, p5, p6, p7, p8, p13, p14, p15, p17, p17b, p18, p20, p30, p31, p32, p33, p34, p35 and p36, on the text of p1, p9, p11, p12 and p16, and on p37 and p38. One exception: p39's search now reports `scan-budget`. p42 (Forms past the page edge) refuses `shared-content` in every past-edge case, as round 2 did, and removes and keeps the same text as engine.ts where it applies. No probe output holds a secret beyond the surfaces round 2 already kept with sanitizing off, and engine.ts keeps more of those.
- **Corpus sample**: the review's 174 files, same selection, in `scratch/p8/redact/corpus.jsonl` (sha256 prefixes and counts only).
  - Every Rust result equals round 2's: 155 area and 65 search redactions applied, with the same refusals.
  - 0 characters left inside marks, 0 search hits left, 0 changed unmarked pages.

## Re-review of round 3

The re-review found one blocker (pre-existing, not a round-3 regression) and two minors. All three are fixed on the same branch.

### A Form without a usable /BBox, decided by its box (blocker, both ops)

- **Verified** (probe r3-bboxless, natively first). The Form holds 5,000 clips, LEFTPART and OFFPAGE. It is placed at `300 600 cm` on page 1, where OFFPAGE lies past the right edge, and at `50 300 cm` on page 2. Preview said `shared` []. Apply removed OFFPAGE from page 2, which went from "LEFTPART OFFPAGE PAGETWO" to "LEFTPART PAGETWO". With 0 clips it refused `shared-content`, which is correct.
- **Cause.** Past the part budget, `form_parts` hands the page's Forms to the box rule. For a Form without /BBox, the walk uses the clip at that placement, or the page area, as the box. That stand-in stops at the page edge, so each mark's reach was cut there. The part that apply's glyph rule takes past the edge was never compared with page 2.
- **Fix** (`region::form_box`). In `shared`, a Form without a usable /BBox has no bound. Unusable means missing, non-finite, or without width or height: `[0 0 420]` reads as `[0 0 420 0]`.
  - Its box rule takes each meeting rect's whole reach.
  - The other placements compare parts without clipping.
  - Its touch by box and the decision to walk the page again use the same unbounded box.
  - Apply and the walk are unchanged.
- **Audit for the same class.** Where else does a box or fallback rule cut to a box the file did not give?
  - Images use their unit square, a real box.
  - Forms with a usable /BBox clip what they draw to it. That is the accepted rule; p42 `bbox-hidden-text` matches engine.ts.
  - Shadings did cut to a stand-in. `sh` paints the current clip, so the walk's bounds for a shading are the clip at that placement, or the page. `form_parts` cut its part to that, and the clip pass did the same. A clip from outside the Form differs at another placement, and apply cuts the shading by each rect's whole reach.
  - Repro: a bbox-less Form painting an axial shading, placed and marked as above. Page 2 lost a band right of its own mark, with `shared` [], with or without clips. This hole is pre-existing.
  - **Fix** (`shading_extent`): a shading inside Forms is compared within the usable /BBoxes of the Forms it is inside, and unbounded without one. Clips inside the Form are not counted, which fails closed.
- **Difference, recorded.** With a degenerate /BBox, MuPDF draws nothing of the Form, so page 2 shows only "PAGETWO". The view and apply also read nothing there. Rust still refuses `shared-content` (fail closed).
- Tests:
  - `a_form_without_a_bbox_keeps_text_past_the_edge_shown_elsewhere_when_its_box_decides`: missing, degenerate and page-size /BBox, each with 0 and 5,000 clips, all `shared` and `shared-content`.
  - `a_shading_in_a_form_is_compared_where_it_paints_at_every_placement`.
  - The WASM test covers the bbox-less case with 0 and 5,000 clips.
  - Both native tests fail with the fix taken out.
  - The reviewer's probe gives `shared` [page 1, other page 2] and refuses `shared-content` for every /BBox (none, bad, `[0 0 420 100]`) at 0 and 5,000 clips.

### Planned regions counted across the document (minor, redact-apply)

- **Verified** (probe r1-clipruns). Every run sits in its own page-size clip, and the search is for "the". Each clip held its own copy of every hit's region. 3 pages of 600 runs held 1.08 million copies and refused `too-large`, as did 1 page of 1,100 runs. Round 2 applied both.
- **Fix** (`Planner::regions`). Paths with the same inverse matrix under the same rects of a page share one region list, charged once. Its key, 32 bytes a rect, counts toward the cap, so the bound is now 96 MiB. A Form's path drawn at two placements under different rects merges both lists into one copy, which is charged. The cap and the cut budget are otherwise unchanged.
- **Results.**
  - r1-clipruns: both shapes apply. 3 × 600 runs takes 0.7 s at +10 MB, and 1 × 1,100 takes 0.7 s. "the" is left in no view text.
  - p46's apply now stops at the cut budget, not the cap. It takes 2.3 s in WASM (0.6 s before) at less memory: WASM grew to 50 MB, was 113 MB.
- Test: `page_size_clips_around_every_run_share_their_regions`, and the WASM case.

### Glyph queries charged to nothing (minor, both ops)

- **Verified** (probe r2-cluster): 4,096 distinct tiny file-borne rects in one index cell and 100,000 zero-width glyphs in that cell, under none of them. Every glyph's query tested every rect, 409 million tests and 3.4 s, linear and unbounded.
- **Fix** (`PageRects::visits`, `GlyphRule::visits`/`spent`). Every rect a query tests is counted. Past `MAX_QUERY_VISITS` (2^27), the following refuse `scan-budget`:
  - the preview, after each marked page and after the pair comparisons;
  - apply's plan, per draw;
  - verify's walk of the rewritten pages, per draw;
  - the copy's text check, after it runs.

  Each rule (preview, apply, checks) has its own count.
- **Result**: r2-cluster's apply refuses `scan-budget` in 1.7 s natively and 2.1-2.4 s through the adapter. The preview takes 64 ms.
- Test: `glyph_queries_past_their_budget_refuse`, and the WASM case.

### Measured (re-review)

- **Native redaction tests**: app_redact 38, app_redact_apply 49 and app_redact_bounds 11 (four new), all passing. `tests/rust-redact-bounds.test.mjs`: 2 of 2. WASM memory peaks at 73 MB across both tests.
- **Probe sweep**: every result equals the first round-3 sweep. The only differences are timings, in p37.
- **Corpus sample**: every Rust result on the 174 files equals the first round-3 run, and so round 2's.
  - 155 area and 65 search redactions applied, with the same refusals.
  - 0 characters left inside marks, 0 search hits left, 0 changed unmarked pages.

## Second re-review of round 3

The second re-review accepted redact-preview and redact-search. redact-apply was not accepted: one blocker from 469ccef, plus one minor.

### The keys of shared region lists were not charged (blocker, redact-apply)

- **Verified** (probe r6w, small P only). There are 2,000 distinct zero-width file-borne quads and P page-size fills, each fill under its own matrix.
  - `Planner::regions` kept each fill's key: 32 bytes for each of the 2,000 rects the fill touches.
  - It charged only the regions. `Region::of` makes none for a rect without area, so those keys were never charged.
- **Fix**:
  - A list with no region is returned without being kept.
  - A kept list is charged its regions plus its key's rects (`MAX_HELD_REGIONS`, now at most 64 MiB in all).
- **Results**:
  - With 2,000 zero-width quads, WASM memory after apply is 60 MB at 2,000 fills. That is the same as with 2 quads (59 MB), so the quads add nothing per fill.
  - With 0.1-pt quads under 2,000 fills, apply refuses `too-large` in 0.2 s. Each fill is charged 4,000 units.
  - Past about 3,000 fills under 2,000 quads, the fills' queries reach the query budget first, and apply refuses `scan-budget` in 1.4 s. The review measured 24,000 fills at 1,493 MB over 78 s.
- **Not redaction, recorded for integration.** The rest of the growth per fill is the view's rendering. A plain `render` of one page of P page-size fills grows WASM memory by about 40 KB a fill: 32 MB at 1,000 fills, 111 MB at 3,000 (`scratch/p8/redact/probes2/fills-stages.mjs`). With 2 quads instead of 2,000, the same 53 KB a fill appears in apply, because apply renders the copy for the pixel check. This was not changed here.
- **Tests**:
  - `region_list_keys_are_charged_and_empty_lists_not_kept`: RSS grows by less than 32 MB from 500 to 2,000 fills (it grew 62 KB a fill before). 24,000 fills refuse `scan-budget` at +0 MB. 0.1-pt quads refuse `too-large`.
  - The WASM test applies 200 and then 2,000 fills and allows under 8 KB of growth a fill; it measured 0 bytes a fill.

### The query budget was checked only between draws (minor, both ops)

- **Verified** (probe r2c): 4,096 rects in one cell under a single run of 1,000,000 glyphs. Apply refused after 25.3 s, and preview, with the run inside a Form, after 13.7 s.
- **Fix**:
  - The counter is now one atomic per rule, so a check is two reads.
  - The budget is checked per glyph in apply's plan, verify's walk and `form_parts`, per character in the copy's text check, per draw in the preview, and per comparison in the pairs.
- **Results**:
  - r2c: apply refuses in 0.36 s through the adapter.
  - With the run in a Form, preview and apply refuse in 0.43 s and 0.23 s.
  - The WASM test's Form case refuses in 0.26 s (preview) and 0.22 s (apply).
- Test: `glyph_queries_past_their_budget_refuse`, extended with a million-glyph run on the page and in a Form.

### Measured (second re-review)

- Native redaction tests: 38, 49 and 12 pass. WASM bounds test: 3 of 3.
- Probe sweep: every result equals the previous sweep; only p9's and p37's timings differ.
- Corpus sample: every Rust result on the 174 files is unchanged.
  - 155 area and 65 search redactions applied.
  - 0 characters left inside marks, 0 search hits left, 0 changed unmarked pages.

## Third re-review of round 3

The third re-review (of f75db77, with 608fd9d) accepted redact-preview, redact-apply and redact-search. It raised no new redaction finding.

- **Uncharged region-list keys (the second re-review's blocker): closed.** With 2,000 zero-width quads, apply at 2,000 fills uses 60 MB of WASM. The growth that remains per fill is the view's render, not the keys.
- **Per-glyph budget checks fail closed.** Apply works on a scratch copy, so a refusal partway through leaves nothing partly applied.
- **r2c** refuses in 0.17-0.28 s.
- **Sweeps:** the 39-probe sweep is unchanged. All 18 variants of the bbox-less Form blocker (B1) refuse `shared-content`.
- **Render growth, handed to integration.** The render grows by about 40 KB per page-size fill, up to about 850 MB before the old render limit. It was fixed in the integration lane: the render's scene estimate is bounded at 384 MiB (`app-accept-integration.md`, "Round 3 re-review", "Render fill memory").

## Final review: the pixel check on pages past the scene budget, and plain messages

- **Verified** (review probe `fills-redact.mjs`): a letter page of 12,000 or 20,000 page-size fills plus a text line. The view refuses it at scale 1 (the render's 384 MiB scene budget, `view_scene_budget.rs`) and draws it at 0.5. `verify::pixels` drew each marked page at one scale and failed every mark of a page on any render error, so redact-apply refused `verify-failed` "pixels check failed" (9,000 fills applied). The text and raw checks had passed: nothing was left, the check just could not see.
- **Fix** (`verify::pixels`, `ViewError::scale_bound`):
  - A page the view refuses for what its scale costs (its drawing past the render's scene or pixmap budget, or the image not allocated) is drawn again at half the scale, then a quarter. The walk's refusals (content too complex at any scale) are not retried.
  - Soundness at a reduced scale: the check is the same (each distinct rect, two pixels in, exactly one colour). Ink left inside a rect still shows: a line thinner than a pixel is antialiased into a different colour, which the exact comparison catches (test: a 0.5 pt white hairline over the box fails its mark at half and at a quarter, an eighth of a pixel). What a lower scale loses is resolution at the rect's edges: the two-pixel margin is 4 pt at half and 8 pt at a quarter. So at a reduced scale every rect the full scale would check (one with pixels inside its margin at full scale) must keep at least two pixels each way inside its margin; a thinner one could hide a line between the pixels checked, and apply refuses `scan-budget` ("page N is too complex to draw at a resolution that verifies its thinnest marks"). Rects the full scale does not check either (under about 4 pt across, inside the margin) are unchanged: the text and raw checks cover them, as before. A 12 pt line of text marked by search is about 14 pt tall: checked at half (3 pixels inside), refused at a quarter.
  - A page that still cannot be drawn (refused at a quarter scale, or by the walk) refuses `scan-budget` ("page N is too complex to draw, so its marks cannot be verified"), not `verify-failed`. Other render errors still fail the page's marks.
  - Past 134,217,728 rect rows the check now refuses `scan-budget` ("the marks are too many to verify against the drawing of page N") instead of failing the marks: the marks were not found dirty, they were not checked.
- **Messages.** `verify-failed` refusals carried the failing marks as JSON, search texts included (`text check failed for [{"page":0,"rect":[...],"search":"..."}]`, `raw check failed for "..."`), and the window shows a refusal's message as it is. They now name only how many marks failed and on which pages (at most ten listed, then "and N more"): "text check failed: text is still under 2 marks on pages 1 and 3", "pixels check failed: something is still drawn over 1 mark on page 2", "raw check failed: a searched text is still in the marked pages' content". engine.ts (`redact.ts`) builds the same messages. The other redaction refusals were checked: they name pages, counts, limits, object numbers and fixed reasons; none carries a search text or document text (content-walk errors are the lexer's byte and limit messages).
- Tests (`tests/app_redact_pixel_scale.rs`): `page_size_fills_past_the_scene_budget_apply_at_half_scale` (the review's 12,000 and 20,000 fills: render at 1 refused, at 0.5 drawn; apply ok, `verified.pixels` 1, no SECRET in the copy's text; failed `verify-failed` before); `a_thin_mark_on_a_page_past_the_scene_budget_refuses_scan_budget` (a 6 pt area mark on the 12,000-fill page); with test budgets (`verify::pixels_with`): `a_page_refused_at_full_scale_is_checked_at_half_and_at_a_quarter` (clean box passes, the hairline fails at both), `a_mark_too_thin_for_the_reduced_scale_is_refused_not_passed` (8 pt: `TooThin`; 3 pt, unchecked at full scale too: passes), `a_page_refused_at_every_scale_is_refused_too_complex`. The three raw-check message assertions in `app_redact_apply.rs` now expect the plain text. Native redaction suites: `app_redact` 38, `app_redact_apply` 49, `app_redact_bounds` 12, `app_edit_objects` 22, the new file 5: all pass (the new file in 2.1 s in all).
- Cost: a page refused at full scale is drawn up to three times; each refused draw stops at the budget, and the half and quarter scale images are a quarter and a sixteenth of the full one.
- Not changed: the text check (`view_text`) still fails a page's marks when the copy's view cannot read its text at all (the walk's refusal); the view's text has no scale, so the reduced-scale retry does not apply there. Apply's own content walk has its own limits and usually refuses such a page first (`scan-budget`); not measured against the view's limits here.

