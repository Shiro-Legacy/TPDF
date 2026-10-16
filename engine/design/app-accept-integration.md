# Acceptance review: integration (rounds 1 and 2)

Round 2 (review of 0fd61d3) is the last section. Round 1:

October 16, 2026. Fixes for the integration acceptance review at ed62a16 (verdict: not accepted; two blockers, two majors, five minors). Read with `../APP-ENGINE-DESIGN.md`, `app-worker.md` and `app-corpus-robust.md` (its "Decision needed" on decompression bombs is taken below). Patches: `vendor/hayro-syntax-0.8.0/` (new), `vendor/hayro-interpret-0.8.0/`, `vendor/hayro-0.8.0/` (each `PATCH.md`, diff regenerated). Tests: `tests/view_hostile.rs` (native), `tests/rust-engine-bounds.test.mjs` and `tests/rust-engine-trap.test.mjs` (app root, WASM, `npm run test:rust`).

## Blocker 1: recursion traps

- **Verified** on ed62a16's WASM through the review's `run-case.mjs`: the 8 files trap (`ps-function-deep`, `ps-tint-deep`, `ps-image-tint-deep`, `ps-annot-ap-deep`, `stitch-deep`, `cs-indexed-deep`, `cs-separation-deep`, `cs-icc-deep`). Root causes as reported, plus two the sweep found:
  - a stitching function naming itself recursed forever;
  - parts shared across levels (30 levels each naming the next twice) took 2^30 builds, and hayro then collected the stitching bounds anew for every path through them (2^30 walks, 1.7 GB natively).
- **Fixed in hayro-interpret** (cuts as MuPDF: past a cap the function or space is not read, so the shading or fill is not drawn and the page draws):
  - PostScript procedures nest at most 100 (MuPDF 1.28.1 draws 100, refuses 101, measured), programs at most 2^14 operators.
  - Colour spaces nest at most 8 (Indexed base, Separation/DeviceN alternate, ICCBased alternate, Pattern space).
  - Stitching functions nest at most 32 (MuPDF draws 128, not 256; real files nest 1-2); one build reads at most 4,096 functions.
  - A function is built once per render, cached by where its dictionary is; stitching bounds are collected once per function, at most 1,024.
- **Differences recorded:**
  - MuPDF refuses an Indexed base that is Indexed and a Separation alternate that is a Separation (the whole page fails, measured); hayro reads them, so such chains draw until 8 deep.
  - Stitching functions past 32 levels (MuPDF: 255) and programs past 2^14 operators are not read.
- After the fix: the 19 files of the review's `hostile2/` and `ps-function-deep`, `q-nest`, `form-deep`, `pattern-self` run with no trap and WASM memory at most 33 MB; `ps-function-deep.pdf` renders (the shading is not drawn, as in MuPDF).
- Tests: `postscript_procedures_nest_at_most_as_deep_as_mupdf_reads` (100 draws, 101 and 20,000 do not), `a_postscript_program_past_its_operator_budget_is_refused`, `colour_space_chains_are_cut_where_they_nest_too_deeply` (3,000 Indexed, Separation, ICCBased), `colour_spaces_nested_as_files_nest_them_draw`, `stitching_functions_are_bounded_in_depth_and_parts` (31 draw, 40 and 3,000 do not, a cycle, the shared 30-level DAG drawn), and the WASM cases.

## Blocker 2: unbounded memory from tiny pages

- **Verified**, and the sweep found more of the same class. WASM: the production adapter over the package (ed62a16's, this branch's); native: release `view_probe`, peak RSS.

  | Input | Before (ed62a16) | After |
  | --- | --- | --- |
  | 600,000 nested `q` (2,806 bytes) | 1,237 MB WASM (review) | refused in 11 ms, 13 MB WASM |
  | content behind `[/FlateDecode /FlateDecode]` decoding to 768 MB (1,871 bytes) | 2,054 MB WASM (review) | refused in 93 ms, 262 MB WASM |
  | stroke of 2,000,000 tiny segments (24 KB) | 2,289 MB WASM; 8,000,000 (94 KB) trapped | refused by the walk, 37 MB WASM |
  | stroke of 600,000 tiny segments (7 KB) | 739 MB WASM | refused by the path budget, 118 MB WASM |
  | stroke of 400,000 tiny segments (5 KB) | drawn | drawn, 508 MB WASM (the budget's edge) |
  | the same stroked 1,000 units wide | 2,428 MB WASM, 22 s (on the first commit too) | refused in 0.2 s, 62 MB WASM |
  | 20,000 tiny segments stroked 1,000 units wide | | drawn, 153 MB WASM, 2 s |
  | `[0.00001] 0 d 0 150 m 300 150 l S` (50 bytes) | WASM trap after 1.6 s; natively 1,406 MB, 6 s | solid, as MuPDF, 14 ms |
  | `[0.01] 0 d` along a 10^6-unit line | WASM trap; natively 1,575 MB, 12 s | solid, as MuPDF, 14 ms |
  | Type 0 function of 2^31 one-bit samples | 4 bytes per data bit (8 GB from 256 MB of data) | entries within 64 MiB a render |
  | `/Predictor 2 /Columns 0` on any Flate stream | panic (division by zero): WASM trap (measured) | read unpredicted, as MuPDF |
  | `/Predictor 12 /Columns 4294967295` | 4 GB zero row allocated | decodes to nothing |

- **Decompression bombs: decision taken** (was "Decision needed" in `app-corpus-robust.md`). hayro-syntax is vendored with a decoding limit: Flate (zlib, raw deflate and the lenient inflater), LZW, run-length and ASCII85 stop once one filter's output passes 128 MiB (`MAX_DECODED_BYTES`), or for an image what its declared size takes, within 512 MiB (`MAX_IMAGE_DECODED_BYTES`); buffers grow within the limit. A page's content streams together are held to 128 MiB (one stream listed many times in /Contents was decoded and copied each time). Past it:
  - content (page, form, pattern, glyph): the walk refuses the page (`TooComplex`, "page content decodes past the size limit");
  - images: over budget (`BudgetExceeded`, the render fails);
  - fonts, profiles, functions, CMaps: not read (as for corrupt data).
  - Why a patched crate: hayro decodes with zlib, then raw deflate, then a lenient inflater, so the engine's own decoders cannot predict the size hayro's reach.
  - Why 128 MiB: every corpus file peaked under 456 MB RSS in the Rust worker (`app-corpus-robust.md`), and the patched decode holds at most 1.5× its limit; images keep their declared size up to 512 MiB, so the large scans `MAX_IMAGE_SAMPLES` decodes reduced still draw.
  - Difference recorded: MuPDF streams decoding and has no limit. A content stream past 128 MiB, or image data past its declared size and 128 MiB (or past 512 MiB), is refused where MuPDF draws it.
  - Buffers grow within the limit: with `read_to_end`, the 768 MB bomb still took 517 MB of WASM memory (its buffer doubled to 256 MiB past the limit); now 262 MB.
- **Graphics states** (`view::walk`): a page keeps at most `MAX_GRAPHICS_STATES` (4,096) states at once, `q` nesting plus one for each content run along forms, patterns, glyphs and soft masks; past it the page is refused (render and text). MuPDF refuses past about 2,048 (measured: 2,046 nested `q` draw, 2,047 fail, "too many nested graphics states"), so pages of 2,047 to 4,095 draw here and not in MuPDF. MuPDF also fails search and text-all for such a page; here text extraction skips it like any page it refuses.
- **Paths** (new class): hayro builds a path whole (56 bytes an element) and vello flattens and strokes it at once (on WASM about 140 bytes a segment of a fill and 1,240 of a stroke; natively 1.5-4 bytes a device pixel of length: 20,000 page-sized curves 155-302 MB at 288 dpi).
  - The walk refuses a painted path of more than `MAX_PATH_ELEMENTS` (2^20) elements (render only; text extraction builds no paths).
  - hayro checks each fill, stroke and clip against `MAX_PATH_BYTES` (512 MiB), estimated from its segments, the lines its curves flatten into and its device length within the pixmap (vello culls the rest while flattening: a line or curve 10^24 units long draws in 14 ms on ed62a16), and for a stroke its dashes along the whole path (kurbo dashes it all before culling) and about three widths of outline for each segment near the pixmap (its sides and join; added in the second commit after the first still let a 5 KB page of 400,000 segments stroked 1,000 units wide reach 2.4 GB); past it the render is over budget. The first estimate counted whole lengths and refused `app_forms.rs` `hostile_fields_never_panic`'s widget at `/Rect [72 1e30 90 90]`, which draws; the estimate now counts what vello flattens.
  - A dash pattern shorter than half a pixel or 0.01 units strokes solid: MuPDF 1.28.1's rule (measured: `[0.001]` and `[0.3]` at scale 1 draw solid, `[1]` dashed). Before, Rust drew such lines at about half intensity; they now match MuPDF.
  - Differences recorded: MuPDF draws a 2,000,000-segment stroke (600 MB) and fails at 8,000,000 (a realloc error); here paths past 2^20 elements, or past the per-path estimate (about 400,000 stroked tiny segments, 3,300,000 filled), are refused. A dashed line whose dashes would pass the estimate (a 10^9-unit line with `[1]`) is refused; MuPDF culls the dashes off the page and draws it. 20,000 page-sized curves stroked still draw (192 MB WASM, 1.5 s).
- **Sampled functions**: a Type 0 table holds its declared entries, at most what its data holds (hayro kept 4 bytes for every data bit at one bit a sample, past the declared size), and one render builds at most 64 MiB of tables (`MAX_SAMPLED_BYTES`; functions are built once per render, so shared ones count once); past it the render fails. MuPDF allows 100 M samples per function.
- **Predictors** (hayro-syntax): with a row of no bytes the data passes unpredicted (MuPDF 1.28.1 draws such content, measured), data shorter than a row decodes to nothing before the zero row is allocated, and bits per pixel are a usize.
- `tools/workflow-sources.sh` and `scripts/build.mjs` now count `vendor/` among the WASM package's sources: a change in a patched crate left the package "fresh" before.
- Tests: `graphics_states_nested_past_the_limit_are_refused`, `a_path_past_the_element_limit_is_refused`, `a_wide_stroke_counts_its_outline_against_the_path_budget`, `dash_patterns_are_drawn_within_the_path_budget`, `sampled_function_tables_are_held_to_a_budget`, `a_function_shared_by_many_colour_spaces_is_built_once`, `predictor_parameters_hayro_divided_by_or_allocated_from_draw`; on WASM, `rust-engine-bounds.test.mjs` runs 15 hostile pages (each under 1 MB of file, most a few KB) through the production adapter and checks no trap, `too complex` refusals and WASM memory under 1 GiB.

## Major 1: recovery for long operations (not fixed: owner decision)

- **Verified.** The 20 s watchdog terminates the worker and `resetDocument` drops unsaved edits, on both engines. design/app-worker.md said so.
- Not fixed here: recovery is renderer work (keeping the session's bytes and journal outside the worker, or replaying its requests after a restart, or making long operations cancellable) with its own UX and app tests, and the completion plan lists it under M9. This round only lowers the chance of reaching the watchdog on hostile input (the budgets above refuse in milliseconds what took seconds); the review's 25,000-quad Highlight (46 s on Rust, over 90 s on MuPDF) still reaches it.
- Proposal for the owner: either implement it (preferred design: the renderer keeps the opened bytes and the list of mutating requests since the last save; after a watchdog restart it reopens and replays them, the long request excluded, and says which request was dropped; the engine is deterministic given the adapter's injected entropy and date, which a replay would pass again), or move the item to M10 with the risk recorded. The plan is unchanged until the owner decides.

## Major 2: mobile on MuPDF (not fixed: owner decision)

- **Verified.** `mobile/package.json` builds the desktop app with `build:mupdf`; `mobile/scripts/build.mjs` refuses a Rust desktop build; `mobile/src/scan-worker.ts` and `scan-pdf.ts` use `vendor/mupdf.js`. The completion plan never mentions mobile.
- Not fixed here: moving mobile needs a Rust page-from-images path for scanning and verification on a device (iOS WebView, Safari 15+ for the worker's top-level await), neither available in this round. Criterion 2 allows only agreed exclusions, so the owner must either schedule the move (the engine worker is the desktop `worker.js`; scanning needs `new-from-images`-like composition in Rust) or record mobile as an agreed exclusion in the plan.

## Minors

- **Releases and CI build MuPDF (recorded, unchanged):** already the owner's decision (app-worker.md); M10 adds the Rust toolchain and `build-workflow.sh` to release.yml with Windows and Linux packaged smokes.
- **Date/toJSON on the wire (fixed):** `encode` sends an object with `toJSON` (a `Date`, the only one structured clone carries) as its own properties, as engine.ts reads it, so Rust refuses it where engine.ts does (`search.query` a Date: "query must be 1..256 characters" on both). APP-ENGINE-DESIGN.md "Wire format" updated. Test: `rust adapter: a Date crosses as its own properties, ...` (`tests/rust-engine-smoke.test.mjs`, fake bridge, always runs).
- **Trap recovery tested only with an injected throw (fixed at the adapter):** `tests/rust-engine-trap.test.mjs` grows the WASM memory (untouched pages) until less than 32 MiB more can be had and sends a request buffer larger than all the memory Rust had: Rust's allocation in `push_input` aborts, a real trap. The request and later ones on that app answer `reason: "worker"`, a new session gets a fresh `RustApp` that opens and draws. The renderer's restart on that reply stays covered by worker-build's Electron test (injected) and the review's drive (real); with blocker 1 fixed no known document traps, so no real-trap document is driven in Electron.
- **Test-only bridge hooks ship (recorded, unchanged):** `session_debug`, `session_set`, `snapshot_bytes` and `compare_bytes` stay in the production package because `node tests/run-rust.mjs` runs the app suite against the very WASM the app ships; a test feature would test another binary. The worker forwards only op requests to `dispatch` (worker.ts), so the renderer cannot reach them; they read or set that RustApp's own session state only.

## Measured

- Corpus (`reference/corpus-20261015/summary-f69f43a.json`, the first commit, against `summary-d352fcd.json`): the same 238 failing files with the same classes, no new Rust render, text or save error, the same pixel agreement on all 5,938 compared pages (no page moved by more than 0.5 points within 16 levels), crashes none from Rust. One MuPDF-only change: qpdf `shared-unnamed-field.pdf` 361cb5b24126 ran out of MuPDF's heap at open after 48 s instead of timing out at 60 s (Rust alone ok, as before). The fine-dash rule and the budgets change no corpus page.
- Corpus on the final commit (`summary-3cf6182.json`): the same, but one file newly compared, not changed: pyHanko `.../pdf/form-tree-circular-ref-input.pdf` 7d7847f4c69f is `render-pixels`. MuPDF's WASM overflowed its stack opening it in the d352fcd and f69f43a runs (no comparison) and opened it this time (it traps later, at save); Rust's render is the same in all three runs (571 non-white pixels against MuPDF's 999). No Rust error, no page's agreement moved by more than 0.5 points, all 3 timeouts MuPDF's as before.

- Native and WASM: the table above. The review's `run-case.mjs` (23 requests each) on all 61 of its hostile files: no trap (the 8 traps gone); WASM at most 33 MB but for `annot-repeat` (264 MB), `path-huge` (504 MB) and `annot-quads` (529 MB, which timed out at 90 s on both engines in the review and now completes), within the per-path budget.
- `tests/view_hostile.rs` against ed62a16's engine (each test alone, memcap 2,500 MB): 10 of 12 fail, the colour-space chains by a stack overflow, the predictor by hayro-syntax's division by zero, the rest on their assertions; the two that pass there (`colour_spaces_nested_as_files_nest_them_draw`, `a_function_shared_by_many_colour_spaces_is_built_once`) guard what the caps must keep.
- Gates on the final code (CARGO_BUILD_JOBS=2, RUST_TEST_THREADS=2, memcap 3000): `cargo fmt --check`; clippy `-D warnings` and `cargo test --release --all-features` on stable and 1.92 (1,543 passed, 3 ignored, each); bridge fmt, clippy and tests (4 passed); WASM build; `npm run build` (the package fresh by the new digest).
- App on Rust (`node tests/run-rust.mjs`): 666 tests, 562 passed, 103 skipped, 1 failed: p3-edit's "/W expansion" timing bound (2 s; 8 s under the parallel run), the known load-sensitive test, which runs engine.ts's font scan on MuPDF and passes alone (32/32 for the file). On the first commit the same run had 563 passed and 0 failed. Before: 663, 560, 103, 0; the three new tests pass. Default suite (`CI_NO_ELECTRON=1 node --test tests/*.test.mjs`): 677 tests, 565 passed, 112 skipped, 0 failed. No app test edited.

## Round 2 (review of 0fd61d3)

Verdict: not accepted; one blocker (the residual of round 1's blocker 2), two majors (unchanged owner decisions), three minors. Tests: `tests/view_hostile.rs` (native, five new) and the second test of `tests/rust-engine-bounds.test.mjs` (WASM, production adapter).

### Blocker: decoded streams held together

- **Verified**, and the same class found twice more. hayro-syntax's limit is per stream, but three things hold several decoded streams at once. WASM memory on 0fd61d3's package (`fixer-built.wasm` of the review; files of 1-2 KB unless said):

  | Input | 0fd61d3 | After |
  | --- | --- | --- |
  | forms of 120 MiB each drawing the next, 2 / 4 / 8 levels | 357 / 613 (review) / 1,253 MB | refused in 0.4 s, 229 MB |
  | a form of 120 MiB drawn 1,000 times | not done after 125 s (120 GB to inflate; the app's watchdog would drop the document at 20 s) | refused in 0.34 s |
  | 3 pages sharing one content stream of 100 MiB, each drawn | 261, 389, 517 MB (128 MB a page, kept for the life of the view) | 261 MB flat (drawn, `text-all`, drawn again) |
  | 9 pages sharing 120 MiB, drawn | 1,285 MB | 261 MB, all drawn; `text-all` 3.2 s |
  | 3 TrueType fonts padded to 100 MiB (Arial, 1.3 MB file) | 493 MB | refused in 0.17 s, 264 MB |
  | 10 such fonts (the app's DroidSansFallback, 1.7 MB file) | 1,189 MB | refused in 0.09 s, 266 MB |

- **Forms along a chain** (`view::walk`): the walk's cost gains `held` (decoded bytes a run holds while what it invokes runs, its own plus the most of any chain below) and `read` (content bytes interpreted, each run counted each time it runs). A page is refused when its content and the forms, pattern cells, glyphs, soft masks and appearances along one chain hold more than `MAX_HELD_CONTENT_BYTES` (128 MiB, as hayro-syntax's limit for one stream and for a page's content together; "page content decodes past the size limit"), or when it reads more than `MAX_READ_CONTENT_BYTES` (1 GiB; "expands past the work limit"). Bytes hayro borrows from the file (no filter) are read, not held; the page's content is always held (hayro-syntax copies it). Each nested stream is decoded within what the chain leaves (`Stream::decoded_within`, new in the hayro-syntax patch), so the walk itself stops early.
- **Forms hayro decodes and skips**: hayro decodes a form before it checks its bounding box and its nesting depth; the walk skipped such forms unread. They now count (read and held a moment each time); one that goes through an image filter is refused like any other (hayro decoded its samples).
- **Pages' kept content** (hayro-syntax `page.rs`): the pages of one document keep at most `MAX_KEPT_CONTENT_BYTES` (32 MiB) of decoded content together; past it a page decodes its content each time it is read (walk, hayro, text: two to four decodes a request for such a page). `Page::page_stream` returns a `Cow`; `operations` and `typed_operations` are gone; hayro-interpret's `interpret_page` reads `page_stream`.
- **Fonts** (`walk::FontBudget`): hayro keeps every font of a render (and the text reader's cache, across pages within 64 MiB) with its program copied whole. The fonts a page selects (`Tf`, and `gs` /Font, in forms, glyphs and patterns too; once per font dictionary, as hayro copies a program for each) may decode to `MAX_FONT_BYTES` (256 MiB) together, counting FontFile, FontFile2, FontFile3, ToUnicode and an embedded Encoding CMap; past it the page is refused ("page fonts decode past the size limit"), for render and text. A stream past hayro-syntax's own limit counts nothing (hayro does not read it). Sizes are measured once per view (`walk::FontSizes`, by object). The edit measurer, which keeps the fonts it loads for a page, holds them to the same budget and measures a font past it as unloadable (no width or missing-glyph check from the view).
- Why refuse, not stream: hayro reads content and fonts whole; streaming content would mean rewriting its interpreter. Why these limits: content held is then at most about 2 × 128 MiB while the walk measures the next stream, and fonts 256 MiB (several full CJK fonts embedded take tens of MB); the corpus run is under "Measured (round 2)".
- **Differences recorded** (MuPDF streams content and draws these pages): a page holding more than 128 MiB of decoded content along one chain (say 60 MiB of page content drawing a 70 MiB form), reading more than 1 GiB of content in all (a 2 MiB form drawn 600 times), or selecting fonts that decode past 256 MiB is refused. A form without a bounding box or nested past 50 that goes through an image filter is refused (it was skipped).
- **Known residual, bounded**: the walk starts each form with its own paint state, while hayro's forms inherit the fill and stroke pattern, the Type3 font and the soft mask. Such an inherited pattern cell or glyph (whose own chain was counted where it was set) can be held on top of a chain the walk counted, so a contrived page holds up to about two or three times 128 MiB, not more; their repeated runs are not in `read`, and what they draw stays within the render's scene budget (a pattern of 10,000 fills inherited into a form of 1,000 fills is refused by it in 2 s). Modelling the inheritance would change the run budget of real pages (hatched maps drawn through forms) and needs a corpus run of its own.
- Tests: `nested_forms_hold_their_decoded_content_within_one_budget` (two forms of 70 MiB refused, of 30 MiB drawn and read; a 70 MiB pattern cell inside a 70 MiB form refused), `content_read_again_and_again_is_held_to_a_budget` (a 4 MiB form drawn 300 times refused with and without a bounding box, 20 times drawn), `the_fonts_a_page_selects_decode_within_one_budget` (one dictionary under two names counts once; two padded fonts draw, three are refused, render and text), `a_measurer_loads_fonts_within_the_font_budget`, `pages_past_the_kept_content_draw_from_content_decoded_again` (four pages sharing 20 MiB drawn and read twice, the same); the first three and the WASM test fail on 0fd61d3's engine (run: native with the source changes stashed, WASM on its package). WASM: eight nested forms, nine pages and ten padded fonts each stay under 1 GiB.
- `app-corpus-robust.md`'s "the small hostile inputs that trapped are fixed" now says that held streams were bounded only here.

### Majors: owner decisions, unchanged

- **Recovery for long operations** and **mobile on MuPDF**: verified unchanged; both are remaining M9 work, which the owner put on hold on October 15 (stop after the engine's acceptance; ask before the remaining M9/M10 work). The round 1 proposals stand. `docs/rust-engine-completion-plan.md` M9 now states the facts: the watchdog on ordinary requests is 20 s and drops unsaved edits (long jobs are only rejected, after 120 s; thumbnails after 10 s); mobile (`mobile/`) builds and scans with MuPDF and is listed as an open item, to move or to exclude by the owner's decision.

### Minors

- **Budget refusals fail the page** (recorded, unchanged): measured on this build at scale 0.5 with the review's probes: an R-style plot of 200,000 points draws (323 MB WASM, 1.2 s); 450,000 points (2.4 MB file), a smooth sine of 450,000 and a plot of 1,000,000 are refused in 0.2-0.4 s ("page drawing is larger than the render's limit"), where MuPDF draws them. Not chunked: vello keeps every stroke's strips until the page is composed, so chunks save only the flattening; a fill cannot be split (winding), and a translucent stroke split in chunks blends its overlaps twice. Not skipped: a page drawn without its data line, silently, is worse than a page that says it is too complex. The user sees the render error for that page; other pages draw. Revisit with incremental composition (M10 benchmarks).
- **Releases build MuPDF** (recorded, unchanged; owner).
- **60 s in the plan** (fixed): the plan says 20 s, as `renderer.ts` and `app-worker.md`.

### Measured (round 2)

- Gates on the final code (CARGO_BUILD_JOBS=2, RUST_TEST_THREADS=2, memcap 3000): `cargo fmt --check`; clippy `-D warnings` and `cargo test --release --all-features` on stable and 1.92 (1,548 passed, 3 ignored, each); bridge fmt, clippy and tests (4 passed); WASM build (package sha256 0a48d9ae…); `npm run build`.
- Corpus (`reference/corpus-20261015/summary-767bb5e.json`, the round 2 commit, against `summary-3cf6182.json`): the same 239 failing files with the same classes, the same failure classes, no Rust render, text or save error (`rustErrors` empty, `rustOnly` render errors 0), and the render, text, save and edit aggregates equal but for timings (both engines faster this run, a less loaded machine): no page newly refused by the new budgets. One MuPDF-only change, as in the f69f43a run: qpdf `shared-unnamed-field.pdf` 361cb5b24126 ran out of MuPDF's heap at open (43.7 s) instead of timing out; Rust alone ok. Timeouts: the other two, MuPDF's, as before. Wall time 1,482 s.
- App on Rust (`node tests/run-rust.mjs`): 667 tests, 564 passed, 103 skipped, 0 failed (the p3-edit "/W expansion" timing test passed this time). Default suite (`CI_NO_ELECTRON=1 node --test tests/*.test.mjs`): 678 tests, 565 passed, 113 skipped, 0 failed. No app test edited.

