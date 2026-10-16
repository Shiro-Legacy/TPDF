# Rust engine acceptance report

October 16, 2026. This is the final report of the independent acceptance review of the Rust app engine (workflow run `wf_47a8f38e-1a3`). The review covered the 67 operations of the app's `SessionRequest` contract (`src/renderer/engine.ts`) as the Rust engine implements them: `engine/src/app`, the WASM bridge `engine/bridge` and the adapter `src/renderer/engine-rust.ts`. It also ran an app integration check, which is not a ledger row. The owner's instruction was to stop after acceptance, so this report records where things stand and what the owner has to decide.

How the review ran: review 0 checked every group at ed62a16 (the wave C integration d352fcd plus its corpus summary). A group with an op not accepted got a fix round and a re-review, at most twice. "Round 1" and "round 2" below are those fix rounds. The session design note calls its two fix rounds "round 0" and "round 1", and c4d2181's commit subject says "round 1"; this report uses the workflow's numbering.

## 1. Bottom line

- **64 of 67 ops are accepted**: 41 at review 0, 57 after round 1, 64 after round 2. Not accepted: `open` (session group), and `redact-preview` and `redact-apply` (redaction group). The integration check is not accepted either.
- **Each has one blocker left.** Two are regressions from round 2: `open` (c4d2181, the repair's dictionary-reparse resume rule) and redaction (67ed038, unbounded clip entries in `form_parts`). The integration blocker predates the review: decoded forms still chain through inherited patterns and Type3 fonts toward the 4 GiB WASM limit. Both round-2 commits fixed their round-1 blockers, so they were merged.
- **Merged locally, nothing pushed.** The evidence branch and all 10 fix branches are merged into the local branch `rust-engine-app`, head 3d1a300. That is 135 commits ahead of local `main`. The branch has no upstream, and no remote branch contains it.
- **"When it is done" (completion plan), criterion 1, every ledger row accepted:** 64 of 67. The ledger records 64 accepted and 3 not accepted.
- **Criterion 2, every app workflow on Rust:** not met. The desktop build runs Rust by default, with no fallback. Mobile, releases and CI still build MuPDF, M9 long-operation recovery is not done, and the integration check is not accepted.
- **Criterion 3, M10 release evidence:** not started.
- **Gates on the merged head:**

  All pass at `3d1a300` (2026-10-16; every step under memcap 3000 MB, 2 build jobs, 2 test threads; logs in `tpdf-wt/gates-out/final-3d1a300/`, not committed):
  - `cargo fmt --check` and `cargo clippy -D warnings`, engine and bridge, on Rust 1.98.1 and 1.92.0.
  - Native tests: 1,618 passed, 0 failed in 131 test binaries on 1.98.1, and the same on 1.92.0 (peak 1,521 MB).
  - Bridge tests: 4 passed.
  - The WASM package (`engine/tools/build-workflow.sh`) and `npm run build` (Rust build) both build.
  - App unit suite without Electron: 681 tests, 568 passed, 0 failed, 113 skipped.
  - App unit suite on Rust (`tests/run-rust.mjs`): 670 tests, 567 passed, 0 failed, 103 skipped.
  - Full suite with Electron on the Rust build, including the 20 test files that run drive scripts (`node --test --test-concurrency=2 tests/*.test.mjs`): 681 tests, 670 passed, 0 failed, 11 skipped (peak 1,721 MB).

- **What the owner decides:** section 6.

## 2. Results by group

| Group | Ops | Accepted at review 0 | Accepted now | Fix rounds | Branch tip | Design note |
|---|---|---|---|---|---|---|
| session | open, close, page-ids, authenticate, security-set | 4/5 | **4/5** | 2 | `app-p7-fix-session` c4d2181 | `engine/design/app-accept-session.md` |
| journal | save, validate, saved, undo, redo | 3/5 | 5/5 | 1 | `app-p7-fix-journal` 0d1b280 | `engine/design/app-accept-journal.md` |
| view | render, thumbnail, search, text-all, layers, layer-set | 6/6 | 6/6 | 0 | none | none |
| pages | page-rotate, page-delete, page-move, insert-pages, extract-pages, pdf-from-images, batch-file, stamp | 8/8 | 8/8 | 0 | none | none |
| nav | outline, outline-add, outline-set, outline-delete, links, link-add, link-set, link-delete, labels, labels-set, labels-clear | 3/11 | 11/11 | 1 | `app-p7-fix-nav` fe0fd91 | `engine/design/app-accept-nav.md` |
| annots | annots, annot-add, annot-set, annot-move, annot-delete | 3/5 | 5/5 | 1 | `app-p7-fix-annots` d8d6cb6 | `engine/design/app-accept-annots.md` |
| forms | fields, field-set, field-add, field-props, field-delete | 2/5 | 5/5 | 1 | `app-p7-fix-forms` a021319 | `engine/design/app-accept-forms.md` |
| edit | edit-targets, edit-text, edit-paragraph, text-add, edit-replace-preview, edit-replace-apply | 2/6 | 6/6 | 2 | `app-p7-fix-edit` fc7ef3b | `engine/design/app-accept-edit.md` |
| objects | edit-object, edit-objects, edit-batch, clip-copy, clip-paste | 3/5 | 5/5 | 2 | `app-p7-fix-objects` 78208e5 | `engine/design/app-accept-objects.md` |
| compare | compare-open, compare-close, compare-render, compare-text | 4/4 | 4/4 | 0 | none | none |
| redact | redact-add, redact-set, redact-search, redact-preview, redact-apply | 2/5 | **3/5** | 2 | `app-p7-fix-redact` 67ed038 | `engine/design/app-accept-redact.md` |
| sign | sign-list, sign-prepare | 1/2 | 2/2 | 1 | `app-p7-fix-sign` 71ef03c | `engine/design/app-accept-sign.md` |
| **Total** | 67 | 41 | **64** | | | |
| integration (not a ledger row) | build, worker, adapter, recovery, packaging, robustness | no | **no** | 2 | `app-p7-fix-integration` 82c8693 | `engine/design/app-accept-integration.md` |
| evidence (not reviewed) | | | | | `app-p7-evidence` 06e717d | `engine/reference/app-acceptance-20261016/README.md` |

Accepted ops carried 30 minor findings at the final review, listed by title in `verdicts.json`. None blocks acceptance. Five of them are marked `resolvedAfterReview` there, so 25 remain open. Edit's "the stale-package check does not cover `engine/vendor`" is closed by the integration merge (f69f43a): `scripts/build.mjs` and `engine/tools/workflow-sources.sh` now count `vendor/`. Compare's four "Ledger row is stale" findings are closed by the October 16 ledger rewrite.

## 3. What the fix rounds changed

Since ed62a16, the merged head adds 87 native test functions (`engine/tests` and unit tests in `engine/src`) and 7 app tests. Tests changed on purpose, each named in its note: two repair tests (session), one app_pages test (nav), one forms test, one annotation unit test and two edit tests from round 1. No app test was edited. The evidence branch changed no test either. It changed the runner `tests/run-rust.mjs`, which now exports its arguments, and the loader `tests/engine-select.mjs`, which loads an op-log hook only when `TPDF_OP_LOG` is set, and it added three op-log helpers.

### journal: 1 round (0d1b280)

- The 32 MiB token limit is gone; a token may be as long as the input. A valid string over 32 MiB no longer makes the open repair the object away and the save write the loss. `validate` now accepts a save that writes a 17 MiB binary string as a 34 MiB hex token.
- A strict parse that stops at a size or count limit now refuses the open, instead of falling back to a repair that drops the object silently. Nesting past 256 levels still repairs, which matches MuPDF (it nulls objects past 251 levels). No corpus file reaches these limits.
- Recorded: the renderer does not send the save ticket, so `saved` keeps engine.ts semantics in the app.
- Tests: `app_core` `a_string_past_32_mib_survives_open_save_and_validate` and `validate_accepts_a_binary_string_the_save_writes_as_hex_past_32_mib`, plus three in-crate tests of the open policy.

### nav: 1 round (fe0fd91)

- Name and number trees whose /Kids array names itself are walked once (`pagetree::tree_nodes`, `forest_nodes`). Before the fix, 581- and 651-byte files gave no reply within 30 s (2^64 visits on the 651-byte one), and a 456-byte file trapped the WASM heap; all now answer in 0-2 ms. Attachment, signature and page-list counting at open use the same bounded walk.
- link-add and link-delete first copy a shared /Annots array into the page, so only that page changes.
- An update glued to `%%EOF` on the same line now opens strictly and unrepaired. Three corpus files have this shape; their saves no longer lose catalog keys.
- labels-set refuses starts above 2^31-1. A /PageLabels entry that names the catalog, the page tree or a page is replaced, not written into.
- Tests: `engine/tests/app_accept_nav.rs` (8).

### annots: 1 round (d8d6cb6)

- annot-add gives a page whose /Annots array other pages share its own copy, as MuPDF does. The same applies to link-add and redaction marks.
- FreeText appearances use MuPDF's per-script fonts: re-encoded base-14 fonts for Greek and Cyrillic, CID fonts for CJK, and a re-encoded Latin font for letters WinAnsi lacks. Before, all of these were drawn and saved as `?`. MuPDF renders both engines' saves 0.0% different for Greek, Cyrillic, Japanese, Korean, Polish and Czech. Vietnamese and emoji still differ, and the difference is recorded.
- Icon stamps fit MuPDF's 190:50 box inside the rect. Identical stamp images share one object: 16 stamps of one 3.76 MB JPEG now save at 3.77 MB instead of hitting the 50 MiB ceiling.
- Deleting an annotation cuts a name or number tree pair whole.
- Tests: five native regression tests (for example `an_add_on_a_page_sharing_its_annots_array_lands_on_that_page_alone`, `identical_stamp_images_share_one_image_object`) and unit tests. All five fail on ed62a16.

### forms: 1 round (a021319)

- Text that no fallback font encodes now removes only that widget's /AP, and /NeedAppearances is no longer set for the whole document. On the corpus, 0 of 1,109 other widgets change; before the fix, 291 changed in 65 files.
- A check box or radio fill turns on only the widgets with that state. A radio group without /AP ends with one kid on, where MuPDF turns all of them on.
- field-delete works on a page written directly into /Kids.
- Tests: native `a_lossy_text_leaves_every_other_widget_as_it_was`, `a_button_fill_turns_on_only_widgets_with_that_state` and `a_field_on_a_page_written_into_kids_can_be_deleted`, plus an app test in `tests/p3-forms.test.mjs`.

### sign: 1 round (71ef03c)

- sign-prepare gives the page a direct /Annots (its entries plus the new widget), as MuPDF's update does. A /Fields array that is also some page's /Annots gets a direct copy. In pyHanko, every Rust signature chain is INTACT and docmdp-ok, including the two where /Fields is shared with another page's /Annots; on those, pyHanko flags MuPDF's revision.
- The incremental writer finds `startxref` before trailing whitespace, so a signed file with a long NUL tail can take another signature.
- Tests: 3 new in `engine/tests/app_sign.rs`, plus NUL-tail cases in a unit test.

### objects: 2 rounds (fece880, then 78208e5)

- Round 1: clip-copy keeps each mark's quads and clip-paste writes them shifted, so a pasted search mark redacts its words, not its whole rect. A mark past 1,024 quads is copied as its rect.
- Round 1: a run move's page check starts at the run's first glyph when the structured-text bounds belong to other characters (govdocs1 `000370.pdf`).
- Round 2: the edit scan tracks text rise (Ts), so a raised run is checked where its glyphs draw. Before, moves that took glyphs off the page were accepted and valid moves were refused; round 1 had introduced this.
- Tests: `a_pasted_mark_keeps_its_quads_and_redacts_what_the_copied_one_does`, `a_run_matched_to_other_characters_moves_within_its_own_extent`, `a_raised_run_is_checked_where_its_glyphs_are` and `a_form_run_is_checked_with_the_rise_of_its_do`.

### edit: 2 rounds (77ded9b, then fc7ef3b)

- Round 1: ToUnicode CMaps with Flate damage are read as leniently as MuPDF reads them, so runs in those fonts are editable again. On 12 corpus files, 4,085 verdicts and 115 edits now match MuPDF.
- Round 1: content streams that no decoder reads are treated as empty, and image-filtered content is not decoded. A JBIG2 bomb page now scans in 34 ms at 132 MB, where it was refused after peaking at 1,125 MB. The scan's code expansion is bounded (2^22 since round 2), and the renderer rescans after security-set.
- Round 2: runs are matched to structured text by the view's font name. Runs in fonts with non-ASCII names, names longer than 31 bytes, and Type0 fonts named other than their descendant can now be edited.
- Round 2: the vendored hayro-interpret sets /W and /W2 CIDs only up to 0xFFFF, and at most 2^18 codes per array. On the U3 file, edit-targets went from 0.56-0.63 s to 10-19 ms.
- Tests: `engine/tests/app_accept_edit.rs` (9) and a `font.rs` unit test.

### session: 2 rounds (f04e094, then c4d2181); `open` not accepted

- Round 1: the repair reads damaged tokens as MuPDF 1.28.1 does (measured rules), so one bad token no longer blanks a page or drops an annotation, a form, the catalog or /Encrypt. The re-review found one exception, a damaged page /Type, which still dropped the page until round 2. In the content-loss fuzz on seed 202, lossy saves went from 8 to 0.
- Round 1: objects appended after the last `%%EOF` without an xref belong to no revision, so the last complete revision is shown. A signed file with an appended object shows the signed text. A wrong password is now answered without a reparse: 19 ms instead of 740-870 ms on a 200,000-object file. A NUL in a security-set password is refused. page-ids got an app test.
- Round 2: page-tree kids are classified as MuPDF classifies them, so a page with a damaged /Type stays a page. References with another generation are read through a rebuild from the file's own xref. An object header inside a damaged object no longer replaces that object, but only partly: see section 4. A missing encryption /R is guessed from /V.
- Tests: in round 1, 14 native tests (lexer 2, parser 1, `app_corpus_open` 9, `app_core` 2) and `tests/p7-page-ids.test.mjs` (2); in round 2, 5 native tests in `app_corpus_open`.

### redact: 2 rounds (4a7c756, then 67ed038); preview and apply not accepted

- Round 1: redact-search and apply's raw check read all optional content, so text a layer membership shows only while a group is off is found and removed. A Form counts as touched by what it draws, not by its /BBox, so a letterhead under a mark over body text can be redacted. The structure element of redacted content loses /T. Sanitize also clears /RV, /I, configuration names and chained link targets.
- Round 2: a Form is touched where apply changes it, by apply's own tests. Text past the page edge inside a Form is now compared at every placement; before, it was removed from placements nobody marked. Copies that keep an external-file (/F) stream now verify. The unlayered view now uses the view's own preparation and budget.
- Round 2's Form-touch change is where the new blocker comes from (section 4).
- Tests: 6 native in round 1 and 5 in round 2, in `engine/tests/app_redact_apply.rs`.

### integration: 2 rounds (f69f43a and 3cf6182, then 767bb5e and 82c8693); not accepted

- Round 1 added recursion caps in the vendored hayro-interpret: PostScript procedures 100 levels, colour spaces 8, stitching functions 32, and each function built once per render. The 8 files that trapped the engine no longer trap.
- Round 1 bounded decoding in a vendored hayro-syntax: 128 MiB per filter, images by their declared size within 512 MiB, and a page's content 128 MiB. It also added a cap of 4,096 graphics states, path budgets (2^20 elements, a 512 MiB estimate) and a 64 MiB limit on sampled-function tables, and fixed a predictor division by zero.
- Round 2: the content walk bounds the decoded content held along one chain (128 MiB) and read in all (1 GiB), the fonts a page selects (256 MiB), and the page content a document keeps (32 MiB).
- Also fixed: Date and toJSON values cross the wire as engine.ts reads them. A real WASM trap is now driven through the adapter in a test.
- Tests: `engine/tests/view_hostile.rs` (17), `tests/rust-engine-bounds.test.mjs` (2 WASM tests), `tests/rust-engine-trap.test.mjs` (1) and one adapter test in `tests/rust-engine-smoke.test.mjs`.

## 4. Not accepted, in detail

### Where each open finding came from

| Item | Finding | Severity | Origin |
|---|---|---|---|
| open | A dictionary missing `>>` hides the next object | blocker | **round 2 regression** (c4d2181) |
| open | Header glued to a damaged object drops an intact xref | major | pre-existing; round 2 fixed it only partly |
| open | Untyped page-tree node shown as one blank page | major | pre-existing (ed62a16 and f04e094 behave the same) |
| open | One damaged byte in `trailer` makes RC4/AES-128 files unopenable | major | pre-existing (ed62a16 behaves the same) |
| open | Pages typed as other objects are dropped by the repair | minor | pre-existing (f04e094 behaves the same); round 2 recorded a wrong rationale |
| open | Cost of the stale-generation rebuild | minor | round 2 (new code path) |
| redact-preview, redact-apply | `form_parts` clip entries grow without a bound | blocker | **round 2 regression** (67ed038) |
| redact-preview, redact-apply | Region queries quadratic with many file-borne rects | major | pre-existing (ed62a16) |
| redact-apply | Pixel check slow with many large quads | minor | pre-existing (ed62a16) |
| integration | Forms chained through inherited patterns and Type3 fonts | blocker | pre-existing: the rest of review 0's memory class |
| integration | M9 recovery for long operations | major | owner decision |
| integration | Mobile on MuPDF | major | owner decision |
| integration | A refused large image keeps about 1 GB of WASM | minor | not recorded before (the 512 MiB image limit dates from round 1) |
| integration | Releases and CI build MuPDF | minor | owner decision |

Why the round-2 commits were merged anyway:

- **c4d2181 (session).** It closes both round-1 blockers, verified by the reviewer: byte-damage fuzz went from 16 lossy saves to 0, token fuzz from 7 to 0, and damaged-/Type and stale-generation probes now match MuPDF and engine.ts. Its regression is one damage shape, a dictionary missing its final `>>`, which no corpus file has. Reverting the commit would bring both blockers back.
- **67ed038 (redact).** It closes the round-1 blocker: redaction removed text past the page edge from Form placements nobody marked. It also closes both majors, and the reviewer re-verified this independently, finding no secret left in any output. Its regression is a memory blow-up on crafted files where clip paths times mark rects run into the millions (for example 4,000 clips under 2,000 marks, or 10 clips under 262,144 file-borne rects). Past the 4 GB heap that would trap the worker and lose unsaved edits; it does not produce a wrong redaction.
- **767bb5e (integration).** It bounds the three held-stream cases that round 1 left open, and it refuses no corpus page. Eight nested forms went from 1,253 MB to a refusal at 229 MB, ten padded fonts from 1,189 MB to 266 MB, and nine pages sharing content from 1,285 MB to 261 MB. The remaining blocker is a path the commit does not cover, not one it opened.

### `open` (session group): 1 blocker, 3 majors, 2 minors

1. **Blocker, round 2 regression: after a dictionary missing its closing `>>`, the repair loses the next object, even with an intact xref, and the save writes the loss.**
   - Cause: `engine/src/repair.rs`. `Scan::object` rereads the dictionary to the end of the input, and when that parse fails on the next object's number, its new resume rule skips that object's header. `select_by_xref` then finds an xref entry at a header it neither read nor recorded as failed, so it drops the whole intact xref.
   - Impact, synthetic: on a 3-page file, 8 of 9 damage shapes lose page 2 and its text ("Beta"). engine.ts, f04e094 and ed62a16 keep it.
   - Impact, real fixtures with only the two `>>` bytes blanked: `p6/links.pdf` shows 3 links of 4 and `p6/tabs.pdf` 12 fields of 13, where engine.ts and f04e094 show all. `nav.pdf` shows 2 pages, where f04e094 shows 3 and engine.ts refuses the file.
   - Impact, cut fuzz: 63 of 639 inputs regress; 36 of the 47 classified ones have an intact xref. No corpus file has this shape.
   - Proposed fix: in `select_by_xref`, read an in-use entry at its own offset instead of dropping the xref. In the scan, do not resume past a header that starts a line and whose object parses through `endobj`. Add tests with each kind of last value (integer, name, string, reference), followed by a page, a content stream, a link and a widget, with an intact and a broken xref.
2. **Major, pre-existing: an object header glued to the damaged tail of the previous object still makes the repair drop an intact xref.**
   - Cause: the same `select_by_xref` branch. Round 2 rereads only entries recorded as failed.
   - Impact: 60 intact-xref inputs over 3 seeds lose pages or text on both f04e094 and c4d2181, where engine.ts keeps them. In one example MuPDF shows 3 pages, Rust shows 2, and the save loses "Alpha".
   - Proposed fix: the same as finding 1, plus a test with a glued header.
3. **Major, pre-existing: an untyped intermediate page-tree node (with /Kids, no /Type) shows as one blank page and hides its pages.**
   - Cause: `engine/src/app/info.rs` `walk_tree` treats only `/Type /Pages` as a node. The repair reads the same tree as a node, so the same file reads two ways depending on damage elsewhere.
   - Impact: engine.ts shows 3 pages, Rust 2. Deleting the blank page deletes the two hidden pages, and MuPDF reads the save with 1 page. No corpus file has this shape (`engine/design/app-corpus-open.md`).
   - Proposed fix: classify kids in `walk_tree` as the repair does. `app-corpus-open.md` notes that `pagetree` and `view_fixes` must change with it. Add a test covering page count, page-ids, text, page-delete and page-rotate.
4. **Major, pre-existing: one damaged byte in the `trailer` keyword makes an RC4 or AES-128 file unopenable.**
   - Cause: the repair takes /Root, /ID and /Encrypt only after a `trailer` keyword or from an xref stream. Without /ID it refuses ("encryption without the file identifier").
   - Impact: deterministic on 3 of the 4 encrypted fixtures that use these older ciphers (`encrypted.pdf`, `encrypted-noedit.pdf`, `p3/encrypted-edit-noannot.pdf`). engine.ts opens each one locked, and the password shows its text. The session note recorded it as a rare fuzz case.
   - Proposed fix: take /ID, /Root, /Encrypt and /Info from a dictionary outside any object when no `trailer` keyword precedes it, as MuPDF's repair does.
5. **Minor, pre-existing: the repair drops a page whose /Type names another kind of object** (/Font, /Annot, /XObject, /Catalog) even when it has /Contents and a /MediaBox. engine.ts and the strict walk show its content, so the recorded rationale ("MuPDF shows a blank page") is wrong. The damage is contrived. Proposed fix: keep as a page any kid with /Contents, or with a /MediaBox and no non-empty /Kids; otherwise correct the rationale.
6. **Minor, round 2: the stale-generation rebuild's cost is not recorded.**
   - On a 16 MB plain file, open goes from 562 to 1,385 ms and RSS from 351 to 776 MB.
   - On a 27 MB AES-256 file, a wrong password goes from 14 to 3,129 ms, because `password::all_wrong` applies only to strict opens.
   - Proposed fix: record the cost, and optionally apply `all_wrong` to rebuilt locked documents.

### `redact-preview` and `redact-apply` (redaction group): 1 blocker and 1 major each, 1 minor on apply

1. **Blocker, round 2 regression, both ops: `form_parts` records every clip path inside a touched Form once per mark rect, before any work bound.**
   - Cause: `engine/src/app/redact/region.rs` `form_parts`. `Detail::Apply` now records clip-only paths, and `clips.extend` adds a 72-byte entry for each rect a clip touches. The work budget (`MAX_FORM_PART_WORK`) is never checked while this list grows. apply runs the same step first, so it is affected before any apply budget.
   - Impact, p46 (a 3,972-byte file, one search with 100 hits, a Form of 50,000 clips): preview takes 950 ms and RSS rises from 140 to 973 MB. At ed62a16 the same preview took 53 ms at 146 MB, and engine.ts refuses it as `scan-budget` at 191 MB. apply reaches 983 MB before its own path budget refuses.
   - Impact, p40: memory grows linearly, to 1,345 MB at 4,000 clips × 2,000 marks.
   - By extrapolation (not run, because it exceeds the 3,000 MB memcap), a file of tens of KB passes the 4 GB wasm32 heap. A failed allocation there is a trap, which loses the open document and its unsaved edits.
   - Proposed fix: count clip cuts into the work budget and fall back to the box rule past it, or keep one entry per clip draw. Add native and WASM tests (50,000 clips × 100 marks, 2,000 × 2,000, and 262,144 file-borne rects), and correct the bound statement in `app-redact.md`.
2. **Major, pre-existing, both ops: region queries are quadratic when a file brings many mark rects.**
   - Cause: in `region.rs`, `PageRects::candidates` clones and sorts the `wide` list on every query, and `covered` runs one query per part. A file can bring 262,144 rects through Redact annotations that share one /QuadPoints array.
   - Impact, p41 (a 67 KB file): preview takes 74.5 s at ed62a16. At 67ed038 it takes 73.8-89.0 s once the Form part work falls back to the box rule (8-9 clips), or for any image under such marks. engine.ts takes 0.55 s.
   - Proposed fix: dedupe identical rects, test wide rects directly instead of cloning and sorting them (or index them separately), and union parts past a bound. Charge each part's query to `MAX_PAIR_WORK`, and add a bounded-time test with 262,144 identical quads.
3. **Minor, pre-existing, redact-apply: the pixel check walks every pixel of every rect of every mark** (`engine/src/app/redact/verify.rs` `pixels`). With 16,384 page-size quads, apply takes 18.8 s (engine.ts takes 118.7 s). With 262,144 quads it takes 290 s at a 1,287 MB peak. Proposed fix: check each distinct rect once, or bound the work and refuse past it; otherwise record the cost.
4. Also open, on the accepted `redact-search` (minor): past the view's resource budget, it refuses with an untagged plain error instead of the recorded `scan-budget`. The cause is `engine/src/app/ops/redact.rs` `search`.

The rest of the redaction package held. On the 174-file corpus sample, Rust applies 155 area and 65 search redactions (engine.ts 139 and 52), with 0 characters left inside marks, 0 search hits left and 0 changed unmarked pages. No probe output holds a secret in any object or revision.

### Integration check: 1 blocker, 2 majors, 2 minors

1. **Blocker, pre-existing class: decoded forms still chain past the held-content budget through inherited pattern fills and Type3 fonts.**
   - Cause: `engine/src/view/walk.rs` starts every form's run with `PaintState::default()`, but hayro's forms inherit the caller's fill and stroke pattern and its Type3 font. A pattern cell or glyph procedure is counted where it is set, then interpreted inside the form while the form's decoded content is held.
   - Impact: 8 levels reach 1,157 MB of WASM from a 6,923-byte file through patterns, and 1,158 MB through Type3 fonts, about 128 MB per level. MuPDF draws the same pages at 129 MB. hayro draws such chains to 50 levels, so about 31 levels (a file of roughly 27 KB) would reach the 4 GiB abort. That figure is extrapolated, not run.
   - The round-2 note's statement that this residual is bounded at "about two or three times 128 MiB" is wrong.
   - Proposed fix: bound memory where hayro holds it. One option is a per-render decoded-content counter in the vendored hayro-interpret (`FormXObject::new`, `TilingPattern::interpret`, Type3 glyphs, soft masks) that marks the render over budget past 128 MiB. The other is to make the walk carry inherited pattern, glyph and soft-mask cost into forms. Either way, add 8-level × 120 MiB pattern and Type3 cases to `view_hostile.rs` and `rust-engine-bounds.test.mjs`, and correct the notes.
2. **Major: M9 recovery for long operations is not done.** The 20 s watchdog on ordinary requests restarts the worker and drops unsaved edits. Saves and other long jobs are only rejected after 120 s, thumbnails after 10 s. This waits on the owner (section 6).
3. **Major: mobile still runs on MuPDF, with no agreed exclusion.** This waits on the owner (section 6).
4. **Minor: refusing an oversized image costs about 1 GB of WASM that the worker keeps.** On a 561 KB file holding one 24000 × 24000 grayscale Flate image, Rust refuses the render, but WASM grows to 1,030 MB and stays there, because linear memory never shrinks. MuPDF draws the page in 1.3 s at 133 MB. Proposed fix: check the declared size before decoding, decode within a reserve cap, or subsample while decoding.
5. **Minor: releases and CI still build MuPDF.** Rust packages are verified on darwin-arm64 only.
6. Also recorded but unresolved: JBIG2 symbol and halftone decoding inside a declared page is still unbounded (`engine/design/app-corpus-robust.md`).

What held at 82c8693:
- The WASM, rebuilt from scratch, is byte-identical to the fixer's (sha256 `0a48d9ae…`).
- `node tests/run-rust.mjs`: 667 tests, 564 pass, 0 fail.
- The full suite with Electron and drives: 678 tests, 667 pass, 0 fail.
- The smoke runs on rust, sandboxed.
- No trap on review 0's 61 hostile files or round 1's 14 cases.

## 5. Decisions made on the owner's behalf

These are deliberate choices and differences from MuPDF or engine.ts, recorded during the work. The acceptance reviewers listed them as recorded and justified, except where a line says otherwise. Paths are under `engine/` (`APP-ENGINE-DESIGN.md` means its "Differences from MuPDF and engine.ts" section).

**Open, repair and save**
- The repair reads damaged tokens by MuPDF 1.28.1's measured rules, not by the reviewer's longest-prefix rule. (`design/app-accept-session.md`)
- Objects appended after the last `%%EOF` without an xref are in no revision, so the last complete revision is shown, as MuPDF shows it within its 1,024-byte window. (`design/app-accept-session.md`)
- References with another generation are read through a rebuild from the file's own xref, reported `repaired: false`. (`design/app-accept-session.md`)
- A missing /R is guessed from /V, as MuPDF does. `algorithm` names the key actually used: `rc4-128` where engine.ts says `rc4-40`. (`APP-ENGINE-DESIGN.md`)
- The repair reads an untyped dictionary with /Kids and a /MediaBox as a node, and leaves out a kid typed as another object. The reviewer found the second rule's rationale inaccurate (section 4, `open` minor 5). (`APP-ENGINE-DESIGN.md`)
- A page listed twice in /Kids is shown and saved once; MuPDF shows it twice. A decision is needed only if MuPDF's count is wanted. (`design/app-corpus-open.md`, `APP-ENGINE-DESIGN.md`)
- An empty owner password opens a file as owner only when it is also the user password (Acrobat's rule). A NUL in a security-set password is refused. (`APP-ENGINE-DESIGN.md`)
- A parse size or count limit refuses the open instead of repairing. One object with more than 8 million values is refused where MuPDF opens it. (`design/app-accept-journal.md`, `APP-ENGINE-DESIGN.md`)

**Rendering and limits**
- Decompression is bounded in the vendored hayro-syntax: 128 MiB per filter, images by their declared size within 512 MiB, and a page's content 128 MiB together. This answers the "Decision needed" on decompression bombs, except JBIG2 symbol and halftone decoding, which is still unbounded. (`design/app-accept-integration.md`, `design/app-corpus-robust.md`)
- Images over budget make the render fail with an error; they are not silently dropped. MuPDF draws them. (`design/app-corpus-robust.md`, `design/app-accept-integration.md`)
- A page holding more than 128 MiB of decoded content along one chain, reading more than 1 GiB in all, or selecting fonts that decode past 256 MiB is refused. No corpus page is refused. (`design/app-accept-integration.md`)
- Recursion caps: PostScript procedures 100 levels (MuPDF's limit), colour spaces 8, and stitching functions 32 (MuPDF: 255). Graphics states are capped at 4,096 (MuPDF: about 2,048). (`design/app-accept-integration.md`)
- Paths over 2^20 elements or the 512 MiB estimate are refused, and the page fails, not the document. A 450,000-point plot is refused in 0.2 s; MuPDF takes 70.7 s, past the app's 20 s watchdog. Dashes under half a pixel stroke solid, as in MuPDF. (`design/app-accept-integration.md`)
- Annotations with the Invisible flag are hidden, as MuPDF and pdf.js hide them. (`design/app-corpus-fidelity.md`; `design/app-view.md` still calls this an open decision, which is stale)
- Optional-content memberships follow their /P policy (MuPDF treats every one as AnyOn), and each layer is listed once. (`APP-ENGINE-DESIGN.md`)
- CJK text without an embedded font draws with Droid Sans Fallback, a 3.45 MB asset loaded on demand (not compiled into the WASM). Where a code's ToUnicode character and its CID disagree, the ToUnicode character wins; MuPDF draws by CID. No verdict names this rule; it was decided in the CJK font work. (`design/app-view.md`, `design/app-corpus-cjk.md`)
- The test-only bridge hooks stay in the production WASM, so the app suite tests the binary that ships. The worker cannot reach them. (`design/app-accept-integration.md`)

**Text and search**
- Text in clipping render modes 4 to 6 is extracted once; MuPDF extracts it twice. (`design/app-corpus-text.md`, `APP-ENGINE-DESIGN.md`)
- Text shown before any font is set is drawn in Helvetica and extracted as drawn; MuPDF draws and extracts nothing. (`design/app-corpus-text.md`)
- Search gives one hit per match, with no 500-quad cap per page. (`APP-ENGINE-DESIGN.md`, `design/app-corpus-text.md`)

**Pages, navigation, annotations and forms**
- A page edit normalizes the page tree in the same step. page-move keeps /PageLabels; MuPDF deletes them. insert-pages and extract-pages copy only the selected pages and drop signature /V. (`APP-ENGINE-DESIGN.md`)
- Name trees are read from every leaf, whatever /Limits says; this matters on hostile files only. link-delete on a shared /Annots array removes the link from the named page only; MuPDF removes it from every page that shares the array. labels-set refuses starts above 2^31-1. (`design/app-accept-nav.md`)
- FreeText uses a re-encoded base-14 font for Latin letters outside WinAnsi, not an embedded font program, because the engine has no font writer. Characters no font has show as `?`; MuPDF draws nothing for them. Stamp images are shared among stamps only. An annotation listed twice is listed once. (`design/app-accept-annots.md`, `design/app-annots.md`)
- Form text that no font encodes removes only that widget's /AP and sets no /NeedAppearances. Embedding a font subset was rejected: there is no font writer, and no new dependencies are allowed. (`design/app-accept-forms.md`)
- A radio fill turns on only the filled kid when the kids lack /AP; MuPDF turns all of them on. (`design/app-accept-forms.md`)
- sign-prepare always writes a direct /Annots for the page and copies a /Fields array shared with a page's /Annots. A signed file with a long blank tail takes an update, where MuPDF refuses. (`design/app-accept-sign.md`, `design/app-sign.md`)

**Content and object editing**
- Runs are matched by the view's font name, so long names and Type0 fonts named other than their descendant can be edited, where engine.ts refused them. (`design/app-accept-edit.md`)
- The edit scan expands at most 2^22 CMap and /W codes, and the vendored hayro reads /W CIDs only up to 0xFFFF. (`design/app-accept-edit.md`)
- An unproven text matrix refuses edit-text (`unmeasurable-width`). A non-numeric /UserUnit is read as 1. `unsupported-encoding` is decided by code in the font the view draws with. (`design/app-edit.md`)
- A pasted mark keeps its quads and gets no search tag. A mark past 1,024 quads is copied as its rect. (`design/app-accept-objects.md`)

**Redaction**
- Widgets, and every other annotation a mark touches, are removed with the redaction; MuPDF keeps everything but the marks. (`design/app-redact.md`)
- redact-search reads all optional content, hidden layers included; MuPDF searches only what it draws. (`design/app-redact.md`)
- A Form counts as touched where apply changes it, not by its /BBox. A glyph a mark only grazes, short of its middle, stays. Size-0 text that apply removes inside a Form makes the Form touched; this reverses round 1's choice, and it fails closed. (`design/app-accept-redact.md`, `design/app-redact.md`)
- The structure element of redacted content loses /T; its ancestors keep theirs. (`design/app-accept-redact.md`)
- Refuses rather than risk a leak, failing closed:
  - an undrawn Form that holds the search text refuses `verify-failed`;
  - a marked page with image-filtered content refuses `invalid`;
  - a copy past the view's resource budget refuses `too-large`.

  (`design/app-redact.md`)

## 6. Decisions needed from the owner

### 6.1 A third fix round for `open`, `redact-preview`, `redact-apply` and the integration blocker

Scope, estimated from the reviewers' proposed fixes:
- **open.** Changes to `engine/src/repair.rs`:
  - `select_by_xref` reads any in-use entry at its own offset instead of dropping the xref;
  - the resume rule in `Scan::object`;
  - trailer entries taken from a dictionary outside any object;
  - the NOT_PAGES rule.

  `engine/src/app/info.rs` `walk_tree` must read an untyped node with /Kids as a node, together with `pagetree` and `view_fixes`. Optionally, `password::all_wrong` for rebuilt files. Then a full corpus run; round 2's full run took 1,933 s at one job.
- **redact.** `engine/src/app/redact/region.rs` only: bound the clip entries in `form_parts`, and make `PageRects::candidates` and `covered` independent of duplicate and wide rects. Optionally, `verify.rs` `pixels` and the search error tag. Then the 174-file sample.
- **integration.** One bound where hayro holds inherited pattern cells and Type3 glyphs, either in the vendored hayro-interpret or in the walk. Add tests at 8 levels × 120 MiB, correct the notes, and run the full corpus to show that no page is newly refused; round 2's run took 1,482 s.
- Each fix is about the size of a round-2 fix: one commit per group, then one re-review. None needs a new dependency.
- Not recommended: reverting c4d2181 or 67ed038. Each revert reopens a verified blocker that loses or wrongly removes content.
- No corpus run exists yet on the merged head 3d1a300. The third round's runs would provide one.
- Optional in the same round: some of the 30 minors on accepted ops. One example is giving widgets the re-encoded Latin font (`Fonts::latin_extended`), a one-line change that the forms note proposed once both branches were merged.

### 6.2 M9: recovery for long operations

The 20 s watchdog restarts the worker and drops unsaved edits, on both engines. Hostile-input budgets now refuse in milliseconds most of what used to take seconds. The review's 25,000-quad Highlight still reaches the watchdog: 46 s on Rust, over 90 s on MuPDF, as measured at review 0. The options:
- **Implement replay** (the preferred design in `engine/design/app-accept-integration.md`). The renderer keeps the opened bytes and the mutating requests since the last save. After a watchdog restart it reopens the file, replays those requests without the long one, and says which request was dropped. This is renderer work, with its own UX and app tests.
- **Move the item to M10** and record the risk.

### 6.3 Mobile

`mobile/package.json` builds the desktop app with `build:mupdf`, and `mobile/scripts/build.mjs` refuses a Rust build. The scanner (`mobile/src/scan-worker.ts`, `scan-pdf.ts`) composes its PDF with `vendor/mupdf.js` directly. The options:
- **Move mobile to Rust.** Scanning needs a Rust page-from-images path, and the move must be verified on a device: an iOS WebView, Safari 15+ for the worker's top-level await.
- **Record an agreed exclusion** in the completion plan. Criterion 2 allows only explicitly agreed exclusions.

### 6.4 Release and CI engine switch, and licence wording

- `npm run package`, which `.github/workflows/release.yml` runs, builds and ships MuPDF. `npm run package:rust` exists and is verified on darwin-arm64 only. Switching means adding the Rust toolchain and `engine/tools/build-workflow.sh` to `release.yml`, with Windows and Linux packaged smokes.
- A Rust build drops the IJG sentence from `NOTICE.txt` and the About panel. It names the engine as AGPL-3.0-or-later, "informed by MuPDF, Copyright Artifex Software, Inc." The owner should confirm both texts before a Rust release (`engine/design/app-worker.md`).
- The release-time items in `docs/gotchas.md` ("Still undecided") stay open. One of them, the MuPDF source obligation, applies only while MuPDF ships.

### 6.5 CJK fallback font packaging: no decision needed

This is settled. The Rust build ships Droid Sans Fallback (3,451,900 bytes, pinned by sha256) as an asset loaded on demand. Its Apache-2.0 notice ships under `licenses/third-party`, and `NOTICE.txt` names it (`engine/design/app-corpus-cjk.md`). Review 0 of the integration check confirmed that the `package:rust` app carries the font and its licence, and draws CJK text. The only open part is the NOTICE wording in 6.4.

### 6.6 The remaining M10 work

None of it is started:
- a fixed benchmark set on named machines, with frozen budgets;
- fuzz, mutation and malformed-input campaigns, plus save-failure and recovery tests;
- an independent review of everything since this checkpoint;
- real packaged builds on Windows x64 and Linux x64. macOS ARM64 has a packaged Rust smoke, but not the full M10 checklist.

The plan also lists replacing the preview limits (16 MiB, 64 pages) with the app's limits. That item was written for the preview pipeline (`engine/desktop`); the app engine already enforces the app's 120 MiB input limit, which the `open` review checked. All of this is held by the October 15 instruction to ask before any remaining M9 or M10 work.

## 7. Evidence index

**Gates on the merged head 3d1a300:**

All pass at `3d1a300` (2026-10-16; every step under memcap 3000 MB, 2 build jobs, 2 test threads; logs in `tpdf-wt/gates-out/final-3d1a300/`, not committed):
- `cargo fmt --check` and `cargo clippy -D warnings`, engine and bridge, on Rust 1.98.1 and 1.92.0.
- Native tests: 1,618 passed, 0 failed in 131 test binaries on 1.98.1, and the same on 1.92.0 (peak 1,521 MB).
- Bridge tests: 4 passed.
- The WASM package (`engine/tools/build-workflow.sh`) and `npm run build` (Rust build) both build.
- App unit suite without Electron: 681 tests, 568 passed, 0 failed, 113 skipped.
- App unit suite on Rust (`tests/run-rust.mjs`): 670 tests, 567 passed, 0 failed, 103 skipped.
- Full suite with Electron on the Rust build, including the 20 test files that run drive scripts (`node --test --test-concurrency=2 tests/*.test.mjs`): 681 tests, 670 passed, 0 failed, 11 skipped (peak 1,721 MB).

**App suites on both engines** (`engine/reference/app-acceptance-20261016/README.md`, `op-evidence.json`, `suite-rust.json`, `suite-mupdf.json`). These were measured at e747ef6, whose app and engine code are d352fcd's, so before any fix:

| Run | Pass | Fail | Skip |
|---|---|---|---|
| Rust unit run | 560 | 0 | 114 (103 Electron, 11 `RUST_SKIPS`) |
| MuPDF unit run | 561 | 0 | 113 |
| Full suite, Rust build | 666 | 0 | 8 |
| Full suite, MuPDF build | 661 | 0 | 13 |

- All 50 drive scripts pass on both builds.
- The 421 tests that pass on both engines dispatch identical ops with identical ok, refused and threw counts. 49 of 50 drives send identical requests and get identical replies; the exception is a thumbnail count that depends on timing.
- Flags: `page-ids` has no unit test in that run (it has one since round 1 of the session group). `close`, `security-set`, `extract-pages`, `pdf-from-images`, `text-all` and `layer-set` have no drive script.
- After the fixes, each branch reran `cargo test` on stable and 1.92 (1,533-1,550 passed), `node tests/run-rust.mjs` (663-667 tests, 0 failures except the known load-sensitive p3-edit "/W expansion" timing test in some runs, which passes alone) and the default suite (674-678 tests, 0 failures). Each design note has its own numbers.
- The integration re-review at 82c8693, on its own branch, ran `node tests/run-rust.mjs` (667 tests: 564 pass, 0 fail, 103 skip) and the full suite with Electron and drives (678 tests: 667 pass, 0 fail, 11 skip).

**Corpus differential** (`engine/reference/corpus-20261015/`: README, `manifest.json`, summaries; tool `engine/tools/corpus-run.mjs`). The corpus is 5,049 public PDFs from 7 sources and is kept outside the repository.

| Summary | What it measures | Failing files | Rust trap, timeout, OOM, crash |
|---|---|---|---|
| `summary-426a2c6.json` | October 15 baseline | 723 | 8 traps, 1 timeout, 1 OOM |
| `summary-d352fcd.json` | wave C integration, before acceptance | 238 | 0 |
| `summary-f69f43a.json` | integration round 1, first commit | 238 | 0 |
| `summary-3cf6182.json` | integration round 1, final | 239 (one file newly compared, not changed) | 0 |
| `summary-767bb5e.json` | integration round 2 | 239 | 0 |

- At 767bb5e, Rust opens 4,949 files and MuPDF 4,923, and Rust refuses none that MuPDF opens.
- Within-16 render agreement: median 99.96%, p10 95.80%.
- Page text is equal on 5,848 of 5,904 pages.
- 4,903 Rust saves validate, and MuPDF reopens 4,898 of 4,900 without repair.
- The edit round trip adds a Square on 4,882 files, and MuPDF finds it on 4,877, with 0 failures.
- These runs are on the integration branch, not the merged head.
- Other groups' corpus checks are in their notes:
  - session: all 5,049 files on an intermediate build, and the 300 repaired or refused files on the final build;
  - redact: a 174-file sample;
  - objects: samples of 236, 381 and 677 files;
  - edit: the reviewer's 12 files, and the font names on pages 0-1 of 4,956 files;
  - forms: 195 and 288 files;
  - nav: a 4,868-file tree-walk comparison;
  - journal: a 5,049-file strict-open scan.

**Design notes** (`engine/design/`):
- `app-accept-journal.md`, `app-accept-session.md`, `app-accept-nav.md`, `app-accept-annots.md`, `app-accept-forms.md`, `app-accept-objects.md`, `app-accept-edit.md`, `app-accept-sign.md`, `app-accept-redact.md`, `app-accept-integration.md`.
- View, pages and compare were accepted at review 0 and have no note.
- The final re-review findings for `open`, redaction and integration are in `verdicts.json`, and in this report's section 4.

**Verdicts:** `engine/reference/app-acceptance-20261016/verdicts.json` lists, per group, the fix rounds, branch, tip, design note and each op's verdict, with every finding open at the final review, minors included; the five closed since are marked `resolvedAfterReview`. The review's full output (evidence, differences, fix reports and every round's verdicts) is outside the repository, in `tpdf-wt/scratch/accept-final/workflow-result.txt`. The reviewers' probes are in `tpdf-wt/scratch/accept-<group>-r<n>/`.
