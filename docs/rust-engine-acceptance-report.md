# Rust engine acceptance report

October 16, 2026. This is the final report of the independent acceptance review of the Rust app engine (workflow run `wf_47a8f38e-1a3`). The review covered the 67 operations of the app's `SessionRequest` contract (`src/renderer/engine.ts`) as the Rust engine implements them: `engine/src/app`, the WASM bridge `engine/bridge` and the adapter `src/renderer/engine-rust.ts`. It also ran an app integration check, which is not a ledger row. The owner's instruction was to stop after acceptance, so this report records where things stand and what the owner has to decide.

**Updated October 17, 2026.** The owner then asked for a third fix round, M9 recovery and the M10 work. Section 8 records that work and its reviews: the macOS ARM64 package checklist (8.7), the final independent review and its fixes (8.8), the benchmarks (8.9) and the gates on the final head (8.6). Sections 1 and 6 are brought to the final state at b6bc7b9, and items resolved since are marked in sections 2, 4 and 5. Later on October 17 the owner decided that mobile runs fully on Rust: the mobile and release branch and the scanner port were merged (8.11), and sections 1, 4 and 6 are brought to 3d8a6ff. The rest of sections 2 to 5 and section 7 describe the October 16 review as it was.

How the review ran: review 0 checked every group at ed62a16 (the wave C integration d352fcd plus its corpus summary). A group with an op not accepted got a fix round and a re-review, at most twice. "Round 1" and "round 2" below are those fix rounds. The session design note calls its two fix rounds "round 0" and "round 1", and c4d2181's commit subject says "round 1"; this report uses the workflow's numbering.

## 1. Bottom line

As of October 17, 2026, at 3d8a6ff:

- **69 of 69 ops are accepted.** Of the 67 the October 16 review covered, 41 were accepted at review 0, 57 after round 1, 64 after round 2, and 67 after round 3. `open` was accepted in round 3, and `redact-preview` and `redact-apply` at the third re-review of round 3 (section 8). The scanner's two ops, `scan-compose` and `scan-measure`, came with the scanner port and were accepted by an independent review with three rechecks at 8ce01d7 (8.11). The integration check is accepted too, at its round 3 re-review; the three minors it left were fixed after acceptance on `app-p10-followups`.
- **M9 recovery for long operations is done and accepted** (replay after a worker restart, `app-p9-m9-replay`, after 3 review rounds).
- **M10:** done, except the packaged builds for Windows and Linux.
  - The save-failure tests and a campaign with the existing fuzzers are done to their stated scope and accepted after 3 review rounds (`app-p10-fuzz`).
  - The macOS ARM64 package checklist passes on the package, and the WASM builds reproducibly (`app-p10-pkg`, section 8.7).
  - The final independent review of 46f1e95..11bf970 accepted it with no blocker or major. Its five minors were fixed, and the recheck's blocker and minor in the first fix too (`app-p10-final`, section 8.8).
  - The fixed benchmark set ran on one named machine, and its budgets are frozen (`app-p10-bench`, section 8.9).
  - Windows x64 and Linux x64 packages need CI, so a push, and wait for the owner.
- **Mobile and the release engine switch are merged.** On October 17 the owner decided that mobile can be fully Rust and merged. `app-p9-mobile-release` (tip 5a00457, re-gated on the final head) is merged at 92e7612: mobile builds with Rust, `npm run package` builds only Rust, and `release.yml` builds one reproducible WASM with pinned Rust and checks it against `engine/reference/wasm-release.sha256`. The scanner then moved to the Rust engine (`app-p11-rust-scan`, merged at 97fa6dc), and the TypeScript PDF writer is deleted (8.11). The Rust NOTICE and About wording is merged but needs the owner's sign-off before a release, no GitHub Action has run, and nothing has run on a device.
- **Merged locally, nothing pushed.** Every round 3, M9 and M10 branch, the integration follow-up, the mobile and release branch and the scanner port are merged into the local branch `rust-engine-app`, head 3d8a6ff. That is 250 commits ahead of local `main`. The branch has no upstream, and no remote branch contains it.
- **"When it is done" (completion plan), criterion 1, every ledger row accepted:** met. The ledger records 69 rows accepted.
- **Criterion 2, every app workflow on Rust:** met in the build, for desktop and mobile; device verification is pending. The desktop build runs Rust by default, with no fallback, and M9 recovery is done. Mobile builds with Rust and its scanner composes with the Rust engine. `npm run package` and `release.yml` build only Rust; `build:mupdf` and `test:mupdf` stay for the MuPDF oracle. Mobile has not run on an iOS or Android device, and no GitHub Action has run the release workflow.
- **Criterion 3, M10 release evidence:** not met. What is missing is the Windows and Linux packages (they need CI, so a push). The full corpus ran on the final head ba70615 (section 8.10): no Rust trap, timeout or error, and no file newly refused.
- **Gates on 39b0699** (`app-p10-pkg` and `app-p10-final` merged; October 17, 2026, every step under memcap 3000 MB; logs in `tpdf-wt/gates-out/final-39b0699/`, not committed): all 12 gates of `tools/gates.sh` pass.
  - `cargo fmt --check` and `cargo clippy -D warnings`, engine and bridge, on Rust 1.98.1 and 1.92.
  - Native tests: 1,683 passed, 0 failed on 1.98.1 and on 1.92.
  - Bridge tests: 4 passed.
  - The WASM package (sha256 `3c18b0e8cb8dcf868fa13c4aacb4f5a75e29efe3744b353245c298e94783de93`) and `npm run build` both build.
  - App unit suite without Electron: 760 tests, 616 passed, 0 failed, 144 skipped.
  - App unit suite on Rust (`tests/run-rust.mjs`): 749 tests, 623 passed, 0 failed, 126 skipped.
  - Peak 1,977 MB.
  - The benchmark merge b6bc7b9 adds only `engine/tools/bench` and `engine/reference/bench-20261017`, outside the WASM's sources. The full suite with Electron last ran at 11bf970, in the final review (749 tests, 0 failed). No run of it on 39b0699 is recorded here.
- **Gates on 92e7612** (the mobile merge): all 12 gates pass. Native tests 1,683 passed, 0 failed on Rust 1.98.1 and 1.92; app unit suite 760 tests, 616 passed, 0 failed; app suite on Rust 749 tests, 623 passed, 0 failed (8.11).
- **Gates on 3d8a6ff: see the coordinator's run.** (PLACEHOLDER: replace this line with the coordinator's results on 3d8a6ff.)
- **What the owner decides or does:** section 6.7 is the complete list. The earlier gates (3d1a300, 9ef8f49, 11bf970) are in sections 7 and 8.6.

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

After round 3 (October 17, section 8): session 5/5 (`app-p8-fix-open` 94be413), redact 5/5 (`app-p8-fix-redact` 608fd9d), total **67**, and the integration check accepted (`app-p8-fix-integration` 8ca0c05; its three minors fixed on `app-p10-followups`, 0dacd84).

Accepted ops carried 30 minor findings at the final review, listed by title in `verdicts.json`. None blocks acceptance. Five of them are marked `resolvedAfterReview` there, so 25 remain open. Edit's "the stale-package check does not cover `engine/vendor`" is closed by the integration merge (f69f43a): `scripts/build.mjs` and `engine/tools/workflow-sources.sh` now count `vendor/`. Compare's four "Ledger row is stale" findings are closed by the October 16 ledger rewrite.

Since round 3, 18 of the 30 are resolved, 8 are kept and recorded as differences (`recordedAfterReview`), and 3 were left as they were (`unchangedAfterReview`: one already recorded, two skipped). One, on `render`, was in no round 3 lane. Section 8.4 lists what is left.

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

This section is the October 16 state. Every finding below is resolved since, mobile and releases by the owner's decision of October 17 to merge them; the last column says where (section 8 has the detail).

### Where each open finding came from

| Item | Finding | Severity | Origin | October 17 |
|---|---|---|---|---|
| open | A dictionary missing `>>` hides the next object | blocker | **round 2 regression** (c4d2181) | fixed in round 3 |
| open | Header glued to a damaged object drops an intact xref | major | pre-existing; round 2 fixed it only partly | fixed in round 3 |
| open | Untyped page-tree node shown as one blank page | major | pre-existing (ed62a16 and f04e094 behave the same) | fixed in round 3 |
| open | One damaged byte in `trailer` makes RC4/AES-128 files unopenable | major | pre-existing (ed62a16 behaves the same) | fixed in round 3 |
| open | Pages typed as other objects are dropped by the repair | minor | pre-existing (f04e094 behaves the same); round 2 recorded a wrong rationale | fixed in round 3 |
| open | Cost of the stale-generation rebuild | minor | round 2 (new code path) | recorded; wrong passwords fast |
| redact-preview, redact-apply | `form_parts` clip entries grow without a bound | blocker | **round 2 regression** (67ed038) | fixed in round 3 |
| redact-preview, redact-apply | Region queries quadratic with many file-borne rects | major | pre-existing (ed62a16) | fixed in round 3 |
| redact-apply | Pixel check slow with many large quads | minor | pre-existing (ed62a16) | fixed in round 3 |
| integration | Forms chained through inherited patterns and Type3 fonts | blocker | pre-existing: the rest of review 0's memory class | fixed in round 3 |
| integration | M9 recovery for long operations | major | owner decision | done (`app-p9-m9-replay`) |
| integration | Mobile on MuPDF | major | owner decision | merged (`app-p9-mobile-release` at 92e7612; the scanner on Rust at 97fa6dc); not verified on a device |
| integration | A refused large image keeps about 1 GB of WASM | minor | not recorded before (the 512 MiB image limit dates from round 1) | fixed in round 3 (drawn reduced) |
| integration | Releases and CI build MuPDF | minor | owner decision | merged (`app-p9-mobile-release` at 92e7612); no GitHub Action has run |

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
4. Also open, on the accepted `redact-search` (minor): past the view's resource budget, it refuses with an untagged plain error instead of the recorded `scan-budget`. The cause is `engine/src/app/ops/redact.rs` `search`. Fixed in round 3: it refuses `scan-budget`.

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
6. Also recorded but unresolved: JBIG2 symbol and halftone decoding inside a declared page is still unbounded (`engine/design/app-corpus-robust.md`). Still the case after round 3.

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
- The repair reads an untyped dictionary with /Kids and a /MediaBox as a node, and leaves out a kid typed as another object. The reviewer found the second rule's rationale inaccurate (section 4, `open` minor 5); round 3 keeps such a kid as a page when it has /Contents, or a /MediaBox and no non-empty /Kids, and corrects the rationale. (`APP-ENGINE-DESIGN.md`, `design/app-accept-session.md`)
- A page listed twice in /Kids is shown and saved once; MuPDF shows it twice. A decision is needed only if MuPDF's count is wanted. (`design/app-corpus-open.md`, `APP-ENGINE-DESIGN.md`)
- An empty owner password opens a file as owner only when it is also the user password (Acrobat's rule). A NUL in a security-set password is refused. (`APP-ENGINE-DESIGN.md`)
- A parse size or count limit refuses the open instead of repairing. One object with more than 8 million values is refused where MuPDF opens it. (`design/app-accept-journal.md`, `APP-ENGINE-DESIGN.md`)

**Rendering and limits**
- Decompression is bounded in the vendored hayro-syntax: 128 MiB per filter, images by their declared size within 512 MiB, and a page's content 128 MiB together. Since round 3, Flate images too large to hold are drawn reduced a row at a time, as MuPDF subsamples (section 8). This answers the "Decision needed" on decompression bombs, except JBIG2 symbol and halftone decoding, which is still unbounded. (`design/app-accept-integration.md`, `design/app-corpus-robust.md`)
- Images over budget make the render fail with an error; they are not silently dropped. MuPDF draws them. Since round 3 this applies only to images over 512 MiB behind other filter chains, masks of that size, and pages past 2^30 reduced samples. (`design/app-corpus-robust.md`, `design/app-accept-integration.md`)
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

Updated October 17, 2026, at 3d8a6ff. 6.1 and 6.2 were decided and are done. 6.3 and 6.4 were decided on October 17: the owner merged mobile and the release switch, and the scanner moved to the Rust engine. The licence sign-off, the mobile notice, the remaining scanner fonts and device verification still wait for the owner. 6.5 needs no decision. 6.6 is done except the Windows and Linux packages. 6.7 is the complete list of what the owner must decide or do.

### 6.1 A third fix round for `open`, `redact-preview`, `redact-apply` and the integration blocker: done

The owner asked for it, and it ran as proposed here: one fix branch per group, each re-reviewed, with no new dependency and no revert of c4d2181 or 67ed038. `open`, `redact-preview`, `redact-apply` and the integration check are accepted (section 8). The optional minors on accepted ops ran in a separate lane, `app-p8-fix-minors`, which also gave widgets the re-encoded Latin font.

- Corpus runs exist on the branches: the open lane ran all 5,049 files on its first build and reran the 300 repaired or refused files on its final build; the integration lane has `summary-bb68bf1.json` and `summary-62a9b21.json`. No corpus run on the merged head is recorded.

### 6.2 M9: recovery for long operations: done

The owner chose replay. The renderer keeps the opened bytes and a log of the requests that changed the session; after a worker restart (watchdog, trap, worker error or a start that times out) it reopens the bytes, replays the log without the request that hung, checks every answer against the original, and says which request was dropped. If the replay fails it reopens the last saved state and says so. It is accepted after 3 review rounds (`engine/design/app-m9-recovery.md`, section 8.2). One deviation from the brief is recorded there: the log is not cleared on save, because a reopened save cannot reproduce ids, revisions and undo history.

### 6.3 Mobile: merged, device verification pending

On October 17 the owner decided that mobile can be fully Rust and merged. `app-p9-mobile-release` (tip 5a00457, refreshed onto the final head and re-gated there) is merged at 92e7612, and the scanner port `app-p11-rust-scan` at 97fa6dc. The note is `engine/design/app-m9-mobile-release.md` (sections 1, 6, 8 and 9), and the scanner ops are in `engine/APP-ENGINE-DESIGN.md`, "Scanner".

- `mobile/package.json` builds the desktop app with the Rust build, and the mobile build refuses a MuPDF build and any file named after MuPDF.
- The scanner no longer imports `vendor/mupdf.js`. It composes its PDF with the Rust engine through two ops, `scan-compose` and `scan-measure`, which the ledger records as accepted. The TypeScript PDF writer (`scan-pdf.ts`, `scan-font.ts`, `scan-metrics.ts`) is deleted, which settles the duplicated writer.
- Font coverage in rebuilt pages, characters drawn per block (all were boxes with the TypeScript writer): Latin Extended-A 121 of 121, Greek 72 of 135, Cyrillic 137 of 256, mathematical operators 28 of 256, arrows 7 of 112, currency 4 of 33. Vietnamese, Hebrew and Arabic letters, Thai and emoji still draw as boxes; their text extracts and searches.
- Right-to-left runs are written in visual order, with numbers placed by UAX #9 W4-W6 and combining marks before their base, so the typed words search in MuPDF, the Rust engine and PDFKit.
- Verified: mobile `tsc` and build, the 16 mobile tests, and `electron tests/scanning-browser.cjs`. Not verified: anything on a device (iOS WKWebView, Android System WebView).
- Still for the owner:
  - the mobile notice: a proposed "Licences and source" screen and file, and whether to distribute an AGPL app through the app stores (AGPL section 6; legal);
  - scanner fonts for the remaining scripts: covering Vietnamese, Hebrew, Arabic, Thai and emoji needs a new font file, such as Noto Sans (OFL, not in the repository);
  - other output differences: non-embedded Helvetica and DeviceRGB;
  - device verification on iOS and Android (the existing UI already needs iOS 15.4).

### 6.4 Release and CI engine switch, and licence wording: merged, sign-off and CI pending

Merged with the same branch at 92e7612:

- `npm run package` builds and ships only the Rust engine (`package:rust` is an alias) and refuses a MuPDF build. `build:mupdf` and `test:mupdf` stay for the MuPDF oracle.
- `release.yml` has an `engine` job that builds the WASM once with pinned Rust (1.98.1, with a 1.92 check) and wasm-bindgen-cli 0.2.129, and checks it against `engine/reference/wasm-release.sha256`. The three installer builds use that one WASM, and each checks its installed package with `scripts/check-package.mjs` (Rust engine, no MuPDF file or code, the WASM's hash, the engine's notices). The MuPDF source archive step is removed.
- The desktop `NOTICE.txt` and About panel of a Rust build, the README and mobile footer, and the `engine/NOTICE` paragraph for the WASM bridge carry the Rust wording. It is merged, but the owner has not signed it off.
- Validated locally on darwin-arm64 only, after the refresh: `npm run package` builds, the packaged smoke reports Rust, `check-package.mjs` passes, the full macOS checklist passes, and `release.yml` parses.
- **Windows and Linux packaging gaps.** Nothing was pushed, so no GitHub Action ran. The toolchain install, the artifact hand-off, the Windows and Linux packaged smokes, `check-package.mjs` on those platforms, and signing and notarization with the Rust WASM are all unverified. No Windows or Linux Rust package exists.
- Still for the owner:
  - sign off the licence wording before a release;
  - corresponding source for releases: the tag alone, or also a `cargo vendor` archive of the engine's crates (legal);
  - follow-ups: move `mupdf` to devDependencies (a lockfile refresh), align the `actions/cache` major, and decide whether a tag build should fail, rather than warn, without a current `wasm-release.sha256` record.
- **Gates.** The branch was refreshed onto the final head and re-gated there (that note's section 8), and the merge 92e7612 passes all 12 gates (8.11).
- The release-time items in `docs/gotchas.md` ("Still undecided") that concern MuPDF go away with it (that note's section 4); the Electron and process items stay.

### 6.5 CJK fallback font packaging: no decision needed

This is settled. The Rust build ships Droid Sans Fallback (3,451,900 bytes, pinned by sha256) as an asset loaded on demand. Its Apache-2.0 notice ships under `licenses/third-party`, and `NOTICE.txt` names it (`engine/design/app-corpus-cjk.md`). Review 0 of the integration check confirmed that the `package:rust` app carries the font and its licence, and draws CJK text. The only open part is the NOTICE wording in 6.4.

### 6.6 M10: done, except Windows and Linux

- **Done, to its stated scope:** save-failure tests for the app's save path, and a fuzz and mutation campaign with the existing review fuzzers (section 8.2, `engine/design/m10-fuzz.md`). Its "Not covered" list stands: editing operations on damaged inputs, encrypted inputs beyond the p7 fixtures, large inputs and real-world damage.
- **Done:** the macOS ARM64 package checklist, with reproducible WASM build instructions (section 8.7); the final independent review, with its minors fixed (section 8.8); the fixed benchmark set, on one named machine, with frozen budgets (section 8.9).
- **Preview limits:** the plan's item to replace the preview limits (16 MiB, 64 pages, 64 steps, 64 MiB history) was written for the preview pipeline (`engine/desktop`). The app engine enforces the app's limits: 120 MiB input and 50 MiB save (`engine/src/app/mod.rs`, `MAX_PDF_BYTES` and `MAX_SAVE_BYTES`, as engine.ts), with no page cap (`engine/APP-ENGINE-DESIGN.md`, "Limits"), which the `open` review checked. Its journal has no step cap, as engine.ts has none; the benchmarks measured its retained history and session growth (section 8.9).
- **Waiting on the owner:** packaged builds on Windows x64 and Linux x64 (6.7).
- **Left open, no decision pending:** the campaign's three found-not-fixed cases and the other residual minors (section 8.4).

### 6.7 Everything the owner must decide or do

This is the complete list; each item appears once. 6.3, 6.4 and sections 8.7, 8.9 and 8.11 have the detail. Decided on October 17 and removed from this list: the mobile and release branch merge, no MuPDF fallback in packaging (merged with it), and the duplicated scanner writer (replaced by `scan-compose`).

Decisions:

1. **Licence wording.** Sign off, before a release, the desktop `NOTICE.txt` and About panel of a Rust build (no IJG sentence), the README and mobile footer wording, and the `engine/NOTICE` paragraphs for the WASM bridge and for the wasm-bindgen and bumpalo licences added by `app-p10-pkg`. The wording is merged (6.4, 8.7).
2. **Corresponding source and the NOTICE source tag.** Decide whether the tag alone is enough or a `cargo vendor` archive of the engine's crates is also attached, a legal question. `NOTICE.txt` points to `https://github.com/Shiro-Legacy/TPDF/releases/tag/v0.1.0`, which does not exist yet. The AGPL source offer holds only once that tag and its source are published with the release, so decide when and where to publish them (6.4, 8.7).
3. **Mobile notice.** Decide on the proposed "Licences and source" screen and file, and whether to distribute an AGPL app through the app stores (AGPL section 6), a legal question (6.3).
4. **Scanner fonts for the remaining scripts.** Vietnamese, Hebrew and Arabic letters, Thai, emoji and most symbols draw as boxes in rebuilt pages; their text extracts and searches. Covering them needs a new font file, such as Noto Sans (6.3).
5. **Other scanner output differences.** Accept non-embedded Helvetica and DeviceRGB (6.3).
6. **Electron fuses.** The packaged binary honours `ELECTRON_RUN_AS_NODE` and `--inspect`, and does not check its asar's integrity. Setting the fuses for a signed release needs `@electron/fuses`, a new dev dependency (8.7).
7. **Performance follow-ups.** The benchmarks show where Rust is slower than MuPDF: renders at 0.5x and 1x and thumbnails (a fixed per-call cost), warm open of large files (an eager parse against MuPDF's lazy one) and search on some files (8.9). Decide whether to work on them before a release. The budget check is timing-dependent and is not one of the default gates.
8. **Package follow-ups.** Move `mupdf` to devDependencies (a lockfile refresh), align the `actions/cache` major, and decide whether a tag build should fail without a current WASM record (6.4).

Actions only the owner can take:

9. **Device verification.** Run mobile on iOS and Android devices (iOS WKWebView, Android System WebView); the existing UI already needs iOS 15.4 (6.3). On macOS, the packaged app's native open and save panels, print dialog and About window were never shown (8.7).
10. **Windows x64 and Linux x64 packaged builds.** They need CI, so a push. No Windows or Linux Rust package exists. The `release.yml` switch is merged, but its toolchain install and artifact hand-off, the packaged smokes, `check-package.mjs` on those platforms, the installer scripts, and signing and notarization with the Rust WASM are unverified (6.4, 8.7).
11. **Push and merge.** Push `rust-engine-app` (3d8a6ff, 250 commits ahead of local `main`, no upstream) and merge it to `main`. Nothing is pushed or published.

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
| `summary-62687fd.json` | open round 3, first build (October 16) | compared per file in `app-accept-session.md` | 0 |
| `summary-bb68bf1.json` | integration round 3 (October 16) | 238 (pyHanko `form-tree-circular-ref-input.pdf` trapped MuPDF's open again) | 0 |
| `summary-62a9b21.json` | integration round 3 re-review (October 17) | 238; every file's Rust-side record equals bb68bf1's | 0 |
| `summary-4280502.json` | integration follow-up (October 17) | 239; every file's Rust-side record equals 62a9b21's, and MuPDF opened the pyHanko file again | 0 |

- At 767bb5e, Rust opens 4,949 files and MuPDF 4,923, and Rust refuses none that MuPDF opens.
- Within-16 render agreement: median 99.96%, p10 95.80%.
- Page text is equal on 5,848 of 5,904 pages.
- 4,903 Rust saves validate, and MuPDF reopens 4,898 of 4,900 without repair.
- The edit round trip adds a Square on 4,882 files, and MuPDF finds it on 4,877, with 0 failures.
- These runs are on the integration branch, not the merged head. At bb68bf1 and 62a9b21, Rust opens 4,952 files and MuPDF 4,922, with no Rust render, text or save error. The folder's README describes every run, 4280502 included.
- Other groups' corpus checks are in their notes:
  - session: all 5,049 files on an intermediate build, and the 300 repaired or refused files on the final build;
  - redact: a 174-file sample;
  - objects: samples of 236, 381 and 677 files;
  - edit: the reviewer's 12 files, and the font names on pages 0-1 of 4,956 files;
  - forms: 195 and 288 files;
  - nav: a 4,868-file tree-walk comparison;
  - journal: a 5,049-file strict-open scan.

**Design notes** (`engine/design/`):
- `app-accept-journal.md`, `app-accept-session.md`, `app-accept-nav.md`, `app-accept-annots.md`, `app-accept-forms.md`, `app-accept-objects.md`, `app-accept-edit.md`, `app-accept-sign.md`, `app-accept-redact.md`, `app-accept-integration.md`. Round 3 is in their "Round 3" and "Round 3 minors" sections.
- M9 and M10 (October 17): `app-m9-recovery.md`, `m10-fuzz.md` and `m10-macos-package.md`, with `docs/building-rust-release.md` for the release build. The benchmarks: `engine/reference/bench-20261017/` (README, `results.json`, `budgets.json`, `comparison.md`) and `engine/tools/bench/`. Mobile, the release switch and the scanner port (October 17): `app-m9-mobile-release.md`, and `APP-ENGINE-DESIGN.md`, "Scanner".
- View, pages and compare were accepted at review 0 and have no note.
- The final re-review findings for `open`, redaction and integration are in `verdicts.json`, and in this report's section 4.

**Verdicts:** `engine/reference/app-acceptance-20261016/verdicts.json` lists, per group, the fix rounds, branch, tip, design note and each op's verdict, with every finding open at the final review, minors included; the five closed by October 16 are marked `resolvedAfterReview`. Its `round3` section (October 17) adds each round 3, M9 and M10 lane's branch, tip, merge and review outcome, the findings those reviews raised, the gates on 9ef8f49 and the totals; the October 16 findings are marked `resolvedAfterReview`, `recordedAfterReview` or `unchangedAfterReview`. The review's full output (evidence, differences, fix reports and every round's verdicts) is outside the repository, in `tpdf-wt/scratch/accept-final/workflow-result.txt`. The reviewers' probes are in `tpdf-wt/scratch/accept-<group>-r<n>/`.

## 8. Round 3, M9 and M10 (October 17, 2026)

After the October 16 report the owner asked for a third fix round for `open`, `redact-preview`, `redact-apply` and the integration blocker, then for M9 recovery and the M10 work. Each lane worked on its own branch from 46f1e95 (M10 from 034eead, after the minors merge), was reviewed independently, and was merged into the local `rust-engine-app` in this order (`git log --first-parent 46f1e95..de49f8a`). The integration follow-up came after acceptance (`app-p10-followups`, merged at 11bf970). The final independent review then covered 46f1e95..11bf970, and the package checklist, the review's fixes and the benchmarks were merged after it (`git log --first-parent de398ab..b6bc7b9`). After the final-head verification, the mobile and release branch and the scanner port were merged on the owner's decision (`git log --first-parent 5c21bf3..3d8a6ff`; 8.11).

| Branch | Tip | Merged at | Work | Reviews | Outcome |
|---|---|---|---|---|---|
| `app-p8-fix-open` | 94be413 | d0980a4 | `open`, round 3 | 1; its two minors fixed in 94be413 | `open` accepted |
| `app-p8-fix-minors` | 9356748 | 034eead | minors on accepted ops | 1, plus the dash-array follow-up (9356748) | accepted, no regressions |
| `app-p8-fix-redact` | 608fd9d | 714396c | redaction, round 3 | 3 re-reviews | `redact-search`, `redact-preview`, `redact-apply` accepted |
| `app-p9-m9-replay` | 0fd6015 | fb9e163 | M9 recovery for long operations | 3 | accepted |
| `app-p8-fix-integration` | 8ca0c05 | 9ef8f49 | integration, round 3 | review and re-review | accepted at re-review; 3 minors to a follow-up |
| `app-p10-fuzz` | b251a79 | de49f8a | M10 save failures and fuzz campaign | 3 | accepted |
| `app-p10-followups` | 0dacd84 | 11bf970 | the three integration minors left at acceptance | after acceptance | fixed; gates pass on 88634a7 |
| `app-p10-pkg` | 94f3ca9 | 2efe665 | M10 macOS ARM64 package checklist, reproducible WASM, missing licence files | none recorded; the gates on 39b0699 | every checklist item passes (8.7) |
| `app-p10-final` | 4d65808 | 39b0699 | the final review's five minors | the final review, then a recheck | the recheck's blocker and minor fixed and accepted (8.8) |
| `app-p10-bench` | 03341e0 | b6bc7b9 | M10 fixed benchmark set and frozen budgets | measurement only | results on one named machine (8.9) |
| `app-p9-mobile-release` | 5a00457 | 92e7612 | mobile on Rust, `npm run package` and `release.yml` on Rust, licence text | review fixes on the branch; re-gated on the final head | merged on the owner's decision; licence sign-off and device verification pending (8.11) |
| `app-p11-rust-scan` | 8ce01d7 | 97fa6dc | `scan-compose` and `scan-measure`; the TypeScript scan writer deleted | an independent review with three rechecks | both ops accepted at 8ce01d7 (8.11) |

### 8.1 Bottom line

- **69 of 69 ops are accepted, and so is the integration check.** The ledger records 69 rows `accepted`, the scanner's two ops included (8.11); `verdicts.json` has a `round3` section with every lane above.
- **M9 recovery is done.** A worker restart no longer loses the open document and its unsaved edits, unless the replay fails, in which case the window reopens the last saved state and says so.
- **M10 is done except the Windows and Linux packages:** save-failure tests and a campaign with the existing fuzzers, to the scope in 8.4; the macOS ARM64 package checklist (8.7); the final independent review, accepted with no blocker or major, and its fixes (8.8); the benchmarks on one named machine (8.9).
- **Mobile, the release switch and the licence wording** are merged (92e7612), and the scanner composes with the Rust engine (97fa6dc). The licence wording waits for the owner's sign-off, and nothing has run on a device or in CI.
- Everything is local, at 3d8a6ff. Nothing is pushed or published.

### 8.2 What each lane changed

**open** (62687fd, 49e76c7, 770efd6, 94be413; `engine/design/app-accept-session.md`, "Round 3"):
- The round-2 blocker and the glued-header major: `select_by_xref` now reads any in-use entry the scan did not define at its own offset, as MuPDF reads an object at its xref offset, instead of dropping the whole xref. An entry that does not parse there is null (`repaired: true`) and the rest of the xref is kept. The scan no longer reads a dictionary past a header that starts a line and whose own object parses whole, and the xref's rereads have their own one-pass budget.
- The untyped page-tree node: one rule (`info::is_node`) now serves the repair, the page walk, page edits, the save and the view, so a node without /Type but with /Kids is a node everywhere. One test that encoded the old reading was changed.
- The damaged `trailer` keyword: when the file is scanned, /Root, /Info, /ID and /Encrypt are taken from dictionaries between objects, as MuPDF's repair takes them. RC4 and AES-128 fixtures with that damage open locked, and their passwords work.
- A kid typed as another object stays a page when it has /Contents, or a /MediaBox and no non-empty /Kids; the wrong rationale is corrected.
- The stale-generation rebuild's cost is recorded (a 16 MB, 200,000-object file opens in 1,405 ms instead of 705 ms), and a wrong password on a rebuilt locked file is answered without a reload: 12 ms where the review measured 3,129 ms.
- Measured on the review's probes: every truncated-dictionary row (54) keeps 3 pages and all text (round 2 lost a page in 8 of 9 shapes); `nav.pdf`, `p6/links.pdf` and `p6/tabs.pdf` with `>>` blanked read as engine.ts reads them; on 9,600 paired robustness inputs, 0 traps and 504 refusals of files MuPDF opens (round 2: 526). Corpus: all 5,049 files on the first build and the 300 repaired or refused files on the final build: no file newly refused, none with fewer pages or less text, three files changed to MuPDF's reading.
- Tests: `engine/tests/app_accept_open.rs` (11).

**Redaction** (6731d75, 469ccef, f75db77; `engine/design/app-accept-redact.md`, "Round 3" and its two re-reviews):
- The round-2 blocker: `form_parts` keeps one entry per clip draw and charges parts, rects and glyphs to the work budget as it finds them, falling back to the Form's box past it. Found while fixing, in apply: each touched path held its own copy of every rect, so the regions planned edits hold are now counted and refused `too-large` past a cap.
- The quadratic queries: identical rects count once, and the rects are indexed in a stack of grids with no clone or sort per query. Finding parts and each part's query are charged to `MAX_PAIR_WORK`. Apply draws each distinct black box once.
- The pixel check reads each row once; its cost is the page's pixels plus the rects' heights. `redact-search` refuses `scan-budget` past the view's resource budget.
- Measured (adapter, node RSS): the 3,972-byte p46 preview went from 950 ms and 973 MB to 130 ms and 186 MB; p40 at 4,000 clips × 2,000 marks from 1,345 MB to 217 MB; the 67 KB p41 preview from 74-89 s to 0.25 s; 262,144 page-size quads apply in 0.13 s natively (the review measured 290 s at 1,287 MB).
- On the review's 174-file corpus sample every round gave the same result: 155 area and 65 search redactions applied, 0 characters left inside marks, 0 search hits left, 0 changed unmarked pages.
- Tests: `engine/tests/app_redact_bounds.rs` (12) and `tests/rust-redact-bounds.test.mjs`.

**Integration** (90ec371 to 62a9b21; `engine/design/app-accept-integration.md`, "Round 3" and "Round 3 re-review"):
- The blocker: a content budget inside the vendored hayro-interpret, at the five places hayro decodes content (forms, soft mask groups and appearances, tiling patterns, Type3 glyphs, page content). It counts what is held and releases it when the decoded bytes are dropped: 128 MiB held at once and 1 GiB read per page. The sweep found a third inheritance path, soft mask groups kept by graphics states, which the same counter bounds. Eight levels of inherited patterns or Type3 fonts went from 1,157-1,158 MB of WASM, drawn, to a refusal in 1.3-1.6 s at 261 MB. Round 2's "about two or three times 128 MiB" statement is corrected.
- The 1 GB image minor: Flate-only image data too large to hold (over 512 MiB declared) is inflated a strip at a time and reduced a row at a time into the drawn image, as MuPDF subsamples. `gray-24000.pdf` draws in 1.0 s with 6-8 MB of WASM, where it was refused at 1,030 MB.
- The re-review's findings, fixed: large images drawn tiny gathered nearly the whole image (blocker); type 4 functions named by stitching functions were held without a bound (blocker: 218 MB from 74 KB); a time cliff on oversize images (now at most 2^30 reduced samples per page); held memory counted by length while buffers kept spare room; and, raised by the redaction re-review, render fill memory (the scene estimate's bound went from 1 GiB to 384 MiB, so 28,000 fills no longer reach 844 MB).
- Corpus `summary-bb68bf1.json` and `summary-62a9b21.json`: no Rust render, text or save error, 238 files on the failing list (one fewer than 767bb5e, a file whose MuPDF open trapped), and no page newly refused by any new budget; at 200 % on 1,207 files the only Rust render errors are pages past the render's pixel size, which MuPDF fails too.
- Tests: `view_inherited.rs`, `view_large_images.rs`, `view_function_budget.rs`, `view_scene_budget.rs`, and the WASM tests `rust-engine-inherited`, `rust-engine-large-image` and `rust-engine-scene`.
- **Follow-up after acceptance** (`app-p10-followups`: 5a5eda9, 4280502; the note's "Follow-up after acceptance"). The three minors the acceptance left are fixed:
  - Rows of an image decoded reduced have at most 2^20 samples (`MAX_REDUCED_ROW_SAMPLES`), and the working set counts every buffer. A row 2^25 samples wide had taken 108-208 MB; a wider image is now over budget before anything is decoded, where MuPDF draws it.
  - Strokes are estimated by the 4-pixel tiles their outline crosses (80 bytes a tile), not by their bounding box. `vert-4000` and `hatch-4000` at 400 % now draw (292 and 164 MB of WASM); the synthetic `plot-200k` (200,000 points stroked) is now refused, where it drew at 454 MB. Fills, clips, glyphs and images keep the box estimate.
  - The wall-clock bounds of eleven app tests scale with the machine's measured speed at that moment (`tests/timing-budget.mjs`), with 3x headroom. `tools/gates.sh` keeps the full suite logs, prints every `not ok` line and fails with the suite.
  - Corpus `summary-4280502.json`: every file's Rust-side record equals 62a9b21's; 239 failing files (MuPDF opened the pyHanko file again). Engine tests 1,666 passed on stable and 1.92.

**Minors on accepted ops** (4002a8b to 9356748; the "Round 3 minors" sections of the forms, annots, journal, edit, objects and sign notes; pages in `APP-ENGINE-DESIGN.md` Differences; compare in `app-misc.md`):
- forms: widgets take the re-encoded Latin font, so Polish, Czech, Turkish, Romanian and other Latin Extended-A fills store an appearance; the radio-kid fix has app tests on both engines; the check-box rule is recorded.
- annots: the no-break space and soft hyphen are WinAnsi; default-ignorable and format characters draw as nothing; combining marks are composed when a font has the result; a dashed FreeText border is regenerated dashed, and an invalid dash array falls back to `[3]` (the review's follow-up). /DS and DA font names are recorded.
- journal: the values-limit history is corrected; validate names the values limit instead of "needed repair".
- pages: a valid PNG past the decoding budget is `too-large`; PNG chunks with a bad CRC are read as MuPDF reads them; a missing IEND and the insert-pages messages are recorded.
- edit and objects: an empty /BaseFont is listed as null; #00 names, non-UTF-8 names and malformed Ts operands are recorded.
- compare: a dangling /Encrypt reference and vertical-writing line boxes are recorded.
- sign: a rebuilt source whose `startxref` the writer cannot find gets the documented refusal; `app-sign.md` is corrected.
- Tests: `engine/tests/app_accept_minors.rs` and `tests/p8-minors.test.mjs`. Four forms tests that encoded Polish as lossy were changed.

**M9 recovery** (060627d to 0fd6015; `engine/design/app-m9-recovery.md`). Nothing in `engine/src` changed; the renderer side is shared, so both engines recover the same way.
- The renderer keeps the opened bytes, a log of every request that changed the session (with a fingerprint of its answer) and the last acknowledged save in place. After a stop (watchdog, trap, worker error, or a worker that does not start) it replays the log in a fresh worker, checks every answer, and restores the page, unlock, layers, clipboard, undo history and clean state. The request that hung is not replayed, and the notice names it.
- Long jobs (120 s) that time out now restore instead of leaving the worker computing; a page whose drawing hangs is paused with a Try again button, so a restore cannot loop.
- The log is not cleared on save (a recorded deviation from the brief), and recovery has a memory budget: 256 MiB on desktop, 128 MiB on a 4 GiB machine, 96 MiB for mobile. Past it, recovery turns off for that document and says so at a restart.
- Replay is deterministic: `annot-add` carries the renderer's date and `redact-apply` its entropy. On large.pdf, 240 logged changes replay in 422 ms on Rust (619 ms on MuPDF).
- **Save tickets, a pre-existing bug fixed on the way.** The renderer sent `saved` without the save's ticket, so an edit made while a save in place was written was reported saved on both engines. `save` now returns a ticket, the renderer sends it back with `saved`, and engine.ts takes it as Rust does.
- Tests: `tests/m9-recovery.test.mjs`, `tests/m9-recovery-drive.test.mjs` (Electron), `tests/save-ticket.test.mjs` (both engines), and updated trap and startup tests in `tests/worker-build.test.mjs`.

**M10 save failures and fuzz campaign** (02d41af to b251a79; `engine/design/m10-fuzz.md`):
- Save failures, all in the app's save path (`src/safe-write.ts`, `main.ts`), each fixed with a test in `tests/m10-save-failure.test.mjs`: Save on a read-only file, and Save As over one, silently replaced it; an in-place Save through a symlink replaced the link; a link retargeted during the write gave the window the other file's identity; Save As over an existing file reset its permission bits (b251a79); exports and signing reported permission failures as "(unreadable)".
- Campaign: the existing review fuzzers, copied into `engine/tools/fuzz`, with new seeds: 20,200 generated inputs per run, run before and after the fixes, plus 92 probe rows. After: 0 traps, 0 hangs, 0 memory-cap hits (peak 393 MB), 0 saves that fail or do not validate. Saves with less text than MuPDF reads went from 4 to 1 (a page listed twice, a recorded difference), with fewer annotations from 3 to 0.
- Engine fixes, each with a test in `engine/tests/m10_fuzz.rs`: in the repair, a header read through as a damaged value, or a whole object read through inside a later object, no longer replaces an object, and an object-stream member outranks such a header; in the view, a /Contents entry that is no stream no longer blanks the page.

### 8.3 Review rounds and what they caught

| Lane | Found by the reviews of round 3 | Resolved |
|---|---|---|
| open | Loose dictionaries overrode an intact xref (a plain file opened locked, or the save took the wrong /Info); junk between objects used up the rereads' budget and left a page blank (both minor) | 94be413 |
| minors | The FreeText dash pattern was written unchecked | 9356748 |
| redaction, 1st re-review | A Form without a usable /BBox was decided by a box cut at the page edge, so apply removed text past the edge from a placement nobody marked (blocker, pre-existing); the same for shadings in Forms; planned regions held per clip; glyph queries charged to nothing | 469ccef |
| redaction, 2nd re-review | The keys of shared region lists were not charged (blocker, from 469ccef); the query budget was checked only between draws | f75db77 |
| redaction, 3rd re-review | No new finding; all three ops accepted. The uncharged keys are closed (2,000 zero-width quads apply at 60 MB of WASM at 2,000 fills); the per-glyph checks fail closed, since apply works on a scratch copy; the 39-probe sweep is unchanged and all 18 bbox-less Form variants refuse `shared-content`. The render's growth per fill went to integration (scene budget) | |
| integration, re-review | Large images drawn tiny (blocker); type 4 functions held without a bound (blocker); a time cliff on oversize images; held memory counted by length; render fill memory | a39d4cf, e599fb3, 62a9b21 |
| M9, round 1 | Edits queued behind a hung save were dropped; a page whose drawing hangs looped through every restore (majors); a failed replay during a save in place, main's dirty flag during a restore, no replay limit or progress, a trap during the replay, the wrong op named, the mobile budget | 8e6f58b |
| M9, round 2 | After a save in place, any hang lost every edit since the save, because tickets replayed at the wrong generation (major); late `saved`, paused pages, the Untitled save dialog | 0fd6015 |
| M9, round 3 | No new data-loss path; accepted. Probe C (save in place, edit, hang, generation 2 or more) and probe D (several documents, two saves in place, undo and an edit) restore every edit; the ticket rebuild is correct in 14 scenarios on both engines. Minors left: a paused page stays blank until Try again or a reopen; `generation()` scans the log on each edit | |
| M10, round 1 | In-place Save through a symlink replaced the link; exports and signing showed "(unreadable)"; the first repair rule skipped every read-through header and could lose the next page or a sole content stream (major) | 9e6c8a2, 08d747c |
| M10, round 2 | A link retargeted mid-save gave the window the wrong identity; Save As onto a symlink was refused; an object-stream member lost to a read-through header (major) | 9345425, 9e21d93 |
| M10, final recheck | Accepted: the object-stream case holds for hybrid xrefs, and Save As through a link never bypasses the permission checks (0444 file, 0555 folder, `uchg`, `/etc/hosts`, a link to a folder all refused). Three nits fixed after it: Save As reset an existing file's mode; signing mapped `broken-link` to "unreadable"; the broken-link text did not cover a link loop | b251a79 |
| Final review, 46f1e95..11bf970 | Accepted, no blocker or major. Five minors: the redaction pixel check failed pages the view refuses at full scale; verify-failed messages named search texts; a header inside a trailer's string dropped the trailer; Save As bound to a re-inspected destination; integers past i64 wrapped | 49eb124, 4a665fe, 33d36ef, a51477e, 32f0e81 |
| Final review, recheck | /UserUnit 1e36 sent the pixel check's retry into an endless loop (blocker, from 49eb124); the unread edge band grew at reduced scales (minor). Both fixed and accepted | 297c53a, 4d65808 |

Each lane's design note records its review rounds, the third ones included (`app-accept-redact.md`, "Third re-review of round 3"; `app-m9-recovery.md`, "Review round 3"; `m10-fuzz.md`, "Final recheck"). The final review's fixes are in `app-accept-redact.md` ("Final review", "Re-review of the final review's fix") and `m10-fuzz.md` ("Final review (minors)", "Final re-review: integers past i64").


### 8.4 Residual minors and recorded differences

Left open:
- **On accepted ops:** `render`: exponent-notation numbers and bare keywords in object dictionaries blank a page MuPDF renders (in no round 3 lane); `annot-delete`: a cut inside a /Nums value array shifts later items (skipped: malformed trees only); `edit-replace-preview`: the renderer's rescan after security-set has no automated test (skipped: no drive command reaches the dialog).
- **Found by the M10 campaign, not fixed:** a copied object-stream header at a line start replaces the real object stream, and an object header split by junk is not found (one input in 4,000 each); a damaged form /BBox hides the form in the view (9 view text losses in p22; the save keeps it). p25's 20 view-only text losses are not classified, because p25 keeps no inputs.
- **M9 minors (round 3 review):** a paused page stays blank until Try again or a reopen; `generation()` scans the log on each edit, at negligible cost.
- **Recorded, unchanged:** JBIG2 symbol and halftone decoding inside a declared page is still unbounded.

Differences recorded in round 3, M9 and M10 (each in the note named):
- **open** (`APP-ENGINE-DESIGN.md`): an untyped node with a /MediaBox is a node (MuPDF: a page hiding its kids), and an untyped page with empty /Kids and /Contents is a page (MuPDF: a node hiding its content). A complete `trailer` after the xref chain no longer overrides it, as in MuPDF.
- **Redaction:** with a degenerate /BBox MuPDF draws nothing of the Form, and Rust still refuses `shared-content` (fail closed).
- **Integration:** a page whose inherited content would hold more than 128 MiB at once or read more than 1 GiB is refused, and so is the text of a page whose graphics states keep soft mask groups past 128 MiB; images over 512 MiB behind other filter chains, and masks that large, are refused; a page needing more than 2^30 reduced samples, or whose fills, strokes and clips are estimated past 384 MiB, is refused. MuPDF draws all of these. A streamed Flate image that breaks ends where it breaks. After acceptance (`app-p10-followups`): an image whose rows would pass 2^20 samples on the reduced path is refused before decoding, and strokes count the tiles they cross, so the synthetic `plot-200k` is refused where it drew; MuPDF draws both.
- **Minors:** composed combining marks (MuPDF draws the base letter only); the WinAnsi baseline for Latin Extended-A widget text (MuPDF 1.28.1 embeds Nimbus Sans and its clip cuts the glyphs); FreeText /DS ignored; DA font names beyond MuPDF's five draw their own face; the check-box rule for widgets without /AP; validate refuses a copy past the values budget where MuPDF saves; a PNG missing IEND is refused; insert-pages source messages; a dangling /Encrypt read as unencrypted, so compare-open and batch-file accept the file; vertical-writing line boxes of zero width; #00 and non-UTF-8 font names; malformed Ts operands.
- **M9:** the log is not cleared on save; the compare slot, search hits, the selection and open dialogs are not recovered.
- **M10** (campaign): a /Kids entry that names no page is dropped (MuPDF counts a blank page); a page listed twice is shown once; 200 of the 201 refusals where MuPDF opens are files where MuPDF shows blank pages without text or opens locked with no pages; an annotation with a damaged /Subtype is not listed but is saved. After the final review: an integer past i64 is read as the real its digits name, where MuPDF wraps it.

Scope note on M10, under what is not covered: the campaign reuses the existing review fuzzers. A first attempt that wrote a new mutator was stopped by a safety filter, so no new fuzz or mutation code was written. Not covered, from `m10-fuzz.md`: editing operations (annotations, forms, page edits, redaction) on damaged inputs; encrypted inputs beyond the p7 fixtures and its three MuPDF-written seeds; large inputs (every input is under 16 KB except p6 meshes up to 200 KB); real-world damage (the external corpus is not used); a full disk and EIO from a real device, network and FUSE folders, Windows ACLs, and a power cut between the rename and the folder fsync.

### 8.5 What the owner still decides

Section 6.7 is the complete list: the licence wording sign-off, the mobile notice (AGPL section 6), corresponding source, scanner fonts for the remaining scripts, the scanner's output differences, the package follow-ups, the NOTICE source tag, Electron fuses, the performance follow-ups, device verification, the Windows and Linux packages, and pushing and merging `rust-engine-app`. The campaign's found-not-fixed cases and the other residual minors (8.4) stay open with no decision pending.

### 8.6 Gates

- On 9ef8f49, the merged head before `app-p10-fuzz`: all 12 gates pass (logs in `tpdf-wt/gates-out/merged-9ef8f49/`, not committed: native 1,664 passed on 1.98.1 and 1.92, app suite 728 tests with 593 passed and 0 failed, app on Rust 717 tests with 598 passed and 0 failed, peak 1,957 MB).
- Each lane also ran the gates on its own branch (logs in `tpdf-wt/gates-out/`, not committed); `app-accept-integration.md` records its numbers, including the follow-up's on 88634a7 (all pass; 1,666 native tests).
- Before the follow-up, `tools/gates.sh` piped each app suite into `tail`, so a gate line could pass with a failing suite. It now keeps the full logs, names every `not ok` test and fails with the suite. The earlier counts in this report were read from the logs' `# fail 0` lines, so they stand.
- `app-p9-mobile-release` was re-gated on the final head before its merge, and the merge 92e7612 passes all 12 gates; the scanner lane ran the gates on its own branch (8.11).
- **Gates on 3d8a6ff: see the coordinator's run.** (PLACEHOLDER: replace this line with the coordinator's results on 3d8a6ff.)
- **Gates on 11bf970, the head the final review covered: all 12 pass** (October 17, 2026; every step under memcap 3000 MB, 2 build jobs, 2 test threads, with the fixed gate script; logs in `tpdf-wt/gates-out/final-11bf970/`, not committed). fmt and strict clippy, engine and bridge, on Rust 1.98.1 and 1.92.0; native tests 1,670 passed, 0 failed on both toolchains; bridge tests; the WASM package and `npm run build`; app unit suite 749 tests, 609 passed, 0 failed, 140 skipped; app suite on Rust 738 tests, 614 passed, 0 failed, 124 skipped; peak 1,987 MB. The docs merge 525ad90 changes `vendor/hayro-interpret-0.8.0/PATCH.md` prose, which the WASM source hash covers, so the package was rebuilt there (sha256 `6e900969…`), `npm run build` passed and the app suite on Rust passed again (738 tests, 614 passed, 0 failed; logs in `gates-out/post-docs-525ad90/`). The final review ran the full suite with Electron on 11bf970: 749 tests, 0 failed (8.8).
- **Gates on 39b0699, the final head's engine and app (all 12 pass).** `app-p10-pkg` and `app-p10-final` merged; October 17, 2026; every step under memcap 3000 MB; logs in `tpdf-wt/gates-out/final-39b0699/`, not committed.
  - fmt and strict clippy, engine and bridge, on Rust 1.98.1 and 1.92.
  - Native tests: 1,683 passed, 0 failed on both toolchains.
  - Bridge tests: 4 passed.
  - The WASM package (sha256 `3c18b0e8cb8dcf868fa13c4aacb4f5a75e29efe3744b353245c298e94783de93`) and `npm run build` both build.
  - App unit suite: 760 tests, 616 passed, 0 failed, 144 skipped.
  - App suite on Rust: 749 tests, 623 passed, 0 failed, 126 skipped.
  - Peak 1,977 MB.
  - b6bc7b9 adds only the benchmark tools and reference, which are outside the WASM's sources. The benchmarks used the same WASM bytes.

### 8.7 M10: the macOS ARM64 package and the reproducible WASM

`app-p10-pkg` (cac04b8, 817f938, 94f3ca9; merged at 2efe665). `engine/design/m10-macos-package.md` has the checklist and `docs/building-rust-release.md` the build recipe. It was checked on darwin-arm64 only, on the package `npm run package:rust` makes from that branch (base de398ab), and was not rerun on 39b0699.

- **Checklist tool.** `scripts/check-macos-package.mjs` launches the packaged binary about 25 times, each with its own userData, through the app's `--smoke` and `--drive` paths. `scripts/drive.mjs --app=<binary>` (or `TPDF_DRIVE_APP`) drives a packaged app.
- **Every item passes:**
  - WASM startup: launch to first page drawn, median 764 ms cold and 731 ms warm. The first launch of a new bundle took about 4.2 s, most of it before the process's own clock started.
  - Fonts and assets: the CJK fallback font is packaged, requested only by a page that needs it, and its export is byte-identical to the dev build's. All 16 served assets are in `app.asar`.
  - Dialogs, with every picker pre-answered: open, Save a copy, Save in place, reopen, export and print to PDF. 48 of the 51 drive scripts pass on the package on their own, and the other three pass through their tests.
  - Orphan temp cleanup.
  - The M9 replay after a hang, on a patched clone of the package, because the hang hooks are in no build.
  - Printing to PDF.
  - High-DPI at device scale factors 1 and 2.
  - Attribution: every licence file matches its source.
  - Reproducible build instructions.
- **Not exercised:** the AppKit open and save panels, the print dialog and the About window were never shown, and printing to a printer was not tried. Signing and notarization were not run for a Rust package.
- **Fixed: missing licence files.** wasm-bindgen, wasm-bindgen-shared, wasm-bindgen-macro, wasm-bindgen-macro-support and bumpalo had no licence folder in `engine/reference/licenses` and no `engine/NOTICE` entry. They have both now, and `tests/m10-package.test.mjs` fails when any crate of the WASM's tree lacks a folder.
- **Fixed: the WASM depended on the checkout path.** It held absolute checkout and `$HOME/.cargo` paths, and cargo hashed path dependencies by absolute path into `-C metadata`. `build-workflow.sh` now adds `--remap-path-prefix` flags, and `tools/rustc-stable-metadata.sh`, a `RUSTC_WRAPPER` for the wasm32 build, gives those crates a path-independent hash. Three builds were byte-identical: two from empty target folders at different paths, and one incremental. The module holds no `/Users/` path. The package as a whole is not byte-reproducible (asar integrity hash, file times, ad-hoc signature).
- **Owner decisions** (6.7): Electron fuses are at their defaults, and setting them needs `@electron/fuses`, a new dev dependency. The source URL in `NOTICE.txt` (tag `v0.1.0`) does not exist yet.
- **Windows and Linux: not covered.** Cross-packaging on this Mac proves nothing about running the result, and `release.yml` has never run a Rust package because nothing is pushed. The installer scripts are untested with the Rust engine, and the checklist tool is macOS-specific.

### 8.8 M10: the final independent review and its fixes

The final review covered 46f1e95..11bf970, everything since the October 16 checkpoint. It **accepted** it with no blocker or major. As part of the review:

- the full suite with Electron ran: 749 tests, 0 failed;
- a 603-file corpus sample gave no new Rust error;
- fresh fuzz seeds gave 0 traps.

It left five minors, fixed on `app-p10-final` (49eb124 to 4d65808, merged at 39b0699):

- **Redaction pixel check.** A page the view refuses at full scale for its drawing's cost (12,000 or 20,000 page-size fills) failed every mark, so apply refused `verify-failed`. Such a page is now drawn again at a fixed half scale. A page whose size is not finite and positive, or that cannot be drawn at 36 dpi, refuses `scan-budget`, and so does a page drawable at neither scale. The band along a rect's edges the check does not read stays under 2.02 pt at both scales. (`app-accept-redact.md`, "Final review" and "Re-review of the final review's fix"; `tests/app_redact_pixel_scale.rs`, `tests/rust-redact-page-size.test.mjs`)
- **Plain verify-failed messages.** They carried the marks as JSON, search texts included, and the window shows a refusal's message as it is. They now name only how many marks failed and on which pages. engine.ts builds the same messages. (`app-redact.md`)
- **Trailers get the reread rule.** A header spliced into an unterminated string of the trailer cut its parse short, so the trailer, /Encrypt and /Info were dropped and the header replaced the page tree. Trailers now get `object()`'s reread and read-through rule. (`m10-fuzz.md`, "Final review (minors)"; `tests/repair_trailer_reread.rs`)
- **Save As rebinds to the written identity.** After a Save As of an Untitled or filled document, `main.ts` re-inspected the destination, so a link retargeted in between bound the window to a file it never wrote. Every mode of `writeCopy` now returns the identity of the file written, and `main.ts` binds to it. (`tests/save-as-rebind.test.mjs`)
- **Integers past i64 are read as reals, never wrapped.** The lenient reader wrapped them as MuPDF does, so a save wrote another value (/UserUnit 10^30 became 5076944270305263616). Such an integer is now the real its digits name, in the strict reader too. Where an integer is required (/Length, xref offsets, object numbers), it is refused. This is a recorded difference from MuPDF. (`m10-fuzz.md`, "Final re-review: integers past i64"; `tests/number_overflow.rs`)

The recheck of the first fix found a **blocker** and a minor:

- Blocker: /UserUnit 1e36 made the page size infinite and the full scale 0, so the retry loop never ended and redact-apply never returned, natively or in the WASM worker.
- Minor: the unread edge band grew to 4 pt at half scale and 8 pt at quarter scale.

Both were fixed (297c53a, then 4d65808 for the render's real edge limit) and accepted: the scales are a fixed list, quarter scale is no longer tried, and an audit of the other loops in redaction found no other retry on a computed condition. Fuzz reruns against the previous build were identical, except one input where the app now shows text that MuPDF clips away by wrapping a /BBox number.

### 8.9 M10: the fixed benchmark set

`app-p10-bench` (03341e0, merged at b6bc7b9) is measurement only, with no engine change. The reference is `engine/reference/bench-20261017/` (README, `results.json`, `budgets.json`, `comparison.md`), and the tools are in `engine/tools/bench/`.

- **Machine and build.** MacBookAir10,1 (Apple M1, 8 cores, 8 GB), macOS 26.6.2, node 22.23.1, mupdf 1.28.1, commit 39b0699, WASM `3c18b0e8…`. This is one named machine.
- **Method.** 18 documents: 7 repository fixtures and 11 public corpus files, named by source, path and SHA-256. The harness drives both engines through the app's own adapters, as the worker does. Per document and engine:
  - 3 cold processes and 10 warm passes;
  - median, p90 and max for open and first page, zoom renders, thumbnails, search, an edit sequence, save, reopen, peak RSS and WASM memory, retained history, and growth over 200 annot-add and undo cycles.
- **Where Rust is faster or smaller** (geometric mean of the per-document ratios to MuPDF):
  - the edit sequence, 0.14-0.32x;
  - the annot-add and undo cycle, 0.11-0.13x;
  - render at 4x, 0.57-0.60x;
  - engine load, 0.61x;
  - peak RSS, 0.77-0.81x, lower on 16 of 18 documents cold and 17 warm.
- **History and growth.** Retained history grew WASM memory by 0 on every document. Over the 200 cycles, Rust's RSS grew at most 15.7 MiB; MuPDF's grew at most 57 MiB.
- **Where Rust is slower** (`comparison.md` lists every case over 1.5x):
  - Renders at 0.5x and 1x, and so the first page, are 2-12x slower on most real documents. Thumbnails are 2-20x slower (`gov-scan` 0.94 s against 0.12 s for 50 thumbnails). Rust's render time barely changes with scale, so a fixed per-call cost dominates at low zoom. The Rust render reply also carries the page text.
  - Warm open of large files is up to 26x slower (`gov-images` 83 ms against 3 ms), and cold open 1.6-9.6x. Rust parses more of the file at open, while MuPDF opens lazily. Reopen of the saved file is slower in the same way.
  - Search is slower on some files (`gov-mosaic` 376 ms against 75-79 ms; `embedded` 23 ms against under 1 ms).
  - Cold save is 1.9-2.2x slower on the small fixtures; warm save is at parity.
  - Cold WASM memory is 1.5-1.8x on several documents.
- **Budgets.** `budgets.json` freezes, per document and metric, 1.5x the Rust p90, rounded up, with floors. `check-budgets.mjs` reran right after the measurement and passed (764 budgeted p90 figures). It is timing-dependent, so it is not one of the default gates: run it by hand on the named machine, and freeze another machine's own budgets before using it there.

### 8.10 Final-head verification

Run on ba70615 (branch `app-p10-verify`, October 17, 2026) to close the gaps the docs pass found. Every heavy step ran under memcap 3000 MB; logs are in `tpdf-wt/scratch/p8/verify/`, not committed.

- **Reproducible WASM on the final head.** The WASM was built in this worktree, at another checkout path than the gates' build, twice: incrementally in a copied target folder, then from an empty scratch `CARGO_TARGET_DIR`, deleted afterwards. Both gave `3c18b0e8cb8dcf868fa13c4aacb4f5a75e29efe3744b353245c298e94783de93`, the bytes the gates built on 39b0699. The `.js`, `.d.ts` and `sources.sha256` files were identical too, and the module holds no `/Users/` path (`docs/building-rust-release.md`, "Final head ba70615").
- **Full suite with Electron, Rust build.** `npm run build`, then `node --test --test-concurrency=2 tests/*.test.mjs`: 760 tests, 740 passed, 0 failed, 20 skipped, in 188 s at a peak of 1,743 MB. 18 of the skips are the Rust bounds tests that run under `npm run test:rust`, one is the opt-in package checklist (`TPDF_PACKAGE_CHECK`), and one is the legacy p12 test without rc2-40-cbc.
- **Full corpus differential on the final head** (`engine/reference/corpus-20261015/summary-ba70615.json` and README, "Final head"). 5,049 files in 1,635 s. There was no Rust trap, timeout, OOM, crash, render, text or save error, and Rust refuses no file MuPDF opens. The failing list fell from 239 at 4280502 to 237, and no file joined it.
  - pyHanko `form-tree-circular-ref-input.pdf` left it because MuPDF's open trapped again, as in earlier runs.
  - qpdf `split-content-stream-errors.pdf` left it because Rust now draws its page blank, as MuPDF does. The page's only content stream is marked LZW but is not LZW. Rust drew its raw bytes before the M10 fuzz fix f2087af.
  - Per file, only save lengths otherwise differ, by 1 to 7 bytes, from the random bytes each save gets. The same check against the final review's 603-file sample finds 3 differences: the pyHanko file and 2 save lengths.
- **Review of `app-p10-pkg` (tooling and docs): no blocker or major.**
  - The stable-metadata hash covers the package name and version and every rustc argument: crate name and type, source path, `--cfg` features, target and flags. The machine roots and cargo's hashes are left out. A fake-rustc probe confirmed that the hash changes with features and version and not with the checkout path.
  - Two versions of one crate differ by version and source path; both miniz_oxide 0.8.9 and 0.9.1 are in this build. rustc refuses two crates of one name with the same metadata in one crate graph, so such a collision cannot pass silently.
  - Host proc macros and build scripts go through the wrapper but do not get the wrapper-version flag. Dependents name them by crate name, and cargo's fingerprints and rustc's crate hash checks still guard the linking.
  - Minors:
    - The wrapper's branch for a joined `-Cmetadata=` argument turns it into a bare `metadata=` argument. That would fail loudly, and cargo never passes the joined form.
    - `-C incremental=<dir>-<cargo hash>` stays in the hash, so `CARGO_INCREMENTAL=1` would make the WASM path-dependent again.
    - A caller whose `RUSTC_WRAPPER` is the wrapper itself would exec it in a loop.
    - In `check-macos-package.mjs`, the licence-folder check counts as passed when `cargo tree` fails, and log problems do not affect `ok`.
  - `drive.mjs --app` is a development script. The packaged app's `--drive` and `--smoke` flags predate the lane, and its only production change is the smoke report's `startup` field. The 10 added licence files are byte-identical to the registry's, and every crate of the WASM's tree has a folder.
- **Review of `app-p10-bench`.**
  - Recomputed from `results.json`, all 764 budgets equal the larger of the floor and 1.5x the Rust p90, rounded up to two significant figures; `freeze` reproduces `budgets.json` exactly.
  - `comparison.md` contains `compare`'s output verbatim, and the README's summary table recomputes.
  - Median ≤ p90 ≤ max holds everywhere, and the cold p90 is the max (3 samples), as stated.
  - The outputs hold only ids, the lane's own descriptions, corpus source paths, SHA-256s and a repository fixture's test password; no corpus bytes or text.
  - Minor: `check` passes a budgeted metric that a rerun did not produce at all (only an engine error fails it).
  - `check-budgets.mjs` was rerun on Rust once, with the 1-minute load average at 1.60-1.99: PASS, 764 of 764 within budget. The closest was `forms` render2 cold, at 81% (10.6 of 13 ms).

### 8.11 Mobile merge and the scanner on Rust (October 17)

On October 17 the owner decided: "mobile can be fully rust and merged". Two branches were merged after the final-head verification (`git log --first-parent 5c21bf3..3d8a6ff`). The detail is in `engine/design/app-m9-mobile-release.md`, sections 1, 8 and 9, and in `engine/APP-ENGINE-DESIGN.md`, "Scanner".

**Mobile and the release switch** (`app-p9-mobile-release`, tip 5a00457, merged at 92e7612). The branch was refreshed onto the final head and re-gated on its final tip before the merge. It brings:
- mobile on the Rust build; the mobile build refuses a MuPDF build;
- `npm run package` building only Rust (`package:rust` is an alias), with `build:mupdf` and `test:mupdf` kept for the MuPDF oracle;
- `release.yml` building one reproducible WASM with pinned Rust and checking it against `engine/reference/wasm-release.sha256`, then checking each installer with `scripts/check-package.mjs`;
- the Rust NOTICE and About wording, which still needs the owner's sign-off before a release.

No GitHub Action has run, so the Windows and Linux jobs are unverified.

Gates on 92e7612: all 12 pass.
- Native tests: 1,683 passed, 0 failed on Rust 1.98.1 and 1.92.
- App unit suite: 760 tests, 616 passed, 0 failed.
- App suite on Rust: 749 tests, 623 passed, 0 failed.

**The scanner on the Rust engine** (`app-p11-rust-scan`, tip 8ce01d7, merged at 97fa6dc). The mobile scanner composes its PDF with the Rust engine through two new ops, `scan-compose` and `scan-measure`. The TypeScript PDF writer (`scan-pdf.ts`, `scan-font.ts`, `scan-metrics.ts`) is deleted. This settles the owner decision between the duplicated TypeScript writer and a Rust compose op.
- Font coverage in rebuilt pages, characters drawn per block: Latin Extended-A 121 of 121, Greek 72 of 135, Cyrillic 137 of 256, mathematical operators 28 of 256, arrows 7 of 112, currency 4 of 33. All were boxes with the TypeScript writer.
- Vietnamese, Hebrew and Arabic letters, Thai and emoji still draw as boxes, but their text extracts and searches. Covering them needs a new font file, an owner decision (6.7).
- Right-to-left runs are written in visual order. Numbers are placed by UAX #9 W4-W6 and combining marks before their base, so search works in MuPDF, the Rust engine and PDFKit.
- Where the fonts are the same (WinAnsi and CJK text), the old and new outputs read back identically in MuPDF (the note's section 9 has the comparison).

**Review.** An independent review with three rechecks accepted both ops at 8ce01d7. Its round 2 and round 3 minors were fixed on the branch. Two cosmetic notes remain:
- a trailing U+200F after Arabic that ends in a marked letter;
- combining marks that no font has are not drawn.

**Ledger.** The ledger now has 69 rows, all `accepted` (3d8a6ff), and `engine/tools/operation-ledger.py` passes.

**Gates on 3d8a6ff: see the coordinator's run.** (PLACEHOLDER: replace this line with the coordinator's results on 3d8a6ff.)
