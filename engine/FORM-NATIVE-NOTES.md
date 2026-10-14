# R22 ordinary Form reference

This is a test-only reference for the declared bounded ordinary Form capability. It does not implement a production native dependency or claim support for images, composite fonts, transparency groups, annotation appearances, or every valid Form placement. R19, R20 and R21 sources, policies and reports remain frozen.

## Declared gate

The gate uses 58 positive generated original PDF object graphs, four deliberately missing local bindings accepted by native outer-resource fallback but refused by strict Rust scope lookup, and ten malformed/cyclic/state examples. Positive graphs cover noncommuting caller/Form matrices; inheritance and restoration of selected font, text parameters and paint; local F1 collisions; explicit empty Resources with a previously selected font; absent/null Resources; nested repeated streams under different caller scopes; clip parentage and transformed BBox sides; line paint; page crop, rotation and UserUnit; actual page-tree resource providers; aliases; and two Forms sharing an indirect Resources dictionary while retaining separate provider scopes.

Each positive case compares the native original-page ordered path/code events, fonts and unhinted outlines using the already declared P0/T0/G1 numeric policies. Its independent exact-rational state model additionally checks Rust ordered path/text state and placement exactly; generated state inputs and selected width fractions keep those matrices dyadic. Canonical glyph-coordinate arithmetic keeps the preexisting R20 independent floating-point enclosure rather than asserting impossible exact rational equality after arbitrary CFF coordinate division.

Every positive graph also has an 18 DPI native pixel diagnostic. The eight path/clip/stroke cases additionally run at72DPI against independent fixed4x sample certificates. Two text cases use independent canonical glyph geometry and exact model transforms at18DPI to check inherited/scoped font, clipping and noncommuting placement. Together these are66 generated render invocations and10 independent pixel models. Curved glyph fills use the frozen R19 Bernstein crossing/enclosure model. Straight polygons keep its exact1/256-pixel lattice branch. Strokes keep the frozen R21 true-boundary certificates. The new R22 compositor only selects the appropriate existing line/curve certificate; every unknown sample contributes bounded0..255 channel uncertainty, not a whole-pixel omission. It preserves the existing1/32-pixel curve/stroke uncertainty and exact integer sample resolve.

All572 readable fixture pages run at18DPI. All43 previously accepted R20 render pages, the three R21 stress representatives, and both new actual Form-only pages also run at72DPI. Every one of the589 prior R21 successful render invocations must retain its exact Rust PPM hash. The expected narrow gain is two pages:545 rendered and27 explicit refusals at18DPI. All500 stress pages remain required. This is620 corpus attempts, with48 supplementary72DPI attempts. Every one of the29 old refusals also receives a full original-page Form-aware geometry/provenance census; unsupported pages are not counted as geometry matches.

Pixel diagnostics reuse the frozen R21 fixed4x AA0/native AA8 references, native width/flattening policy accounting, unchanged independently generated boundary bands, and the exact source-derived stress-rectangle color certificate. Raw native differences remain visible. No new blanket color tolerance, expanded band, hinted-text parity claim, or assumption of full72DPI corpus coverage is introduced.

## Independent native execution and provenance

`tools/mupdf-form-page-oracle.c` is a separate bounded copy of the R20 unhinted/hinted page oracle. The pinned MuPDF run processor itself executes Forms. Its actual `op_Do_form` callback is wrapped at entry and return; real device path/text/clip callbacks remain globally ordered. q/Q/cm callbacks track the saved CTM, while Form entry composes its own Matrix before that CTM. Each record reports original object/generation, parent invocation, caller/effective Resources provider, Matrix, BBox, composed CTM, native event interval, restored clip depth, and decoded source digest. A Form event interval must start with its real BBox clip and end with its corresponding clip pop. Delayed native text is flushed at ET before these admitted outside-text Form calls.

A separate bounded native-lexer preflight checks the admitted operation/state grammar, validates used Form metadata, detects active object ancestry cycles, and gathers actual used font dictionaries across resource frames before execution. It records every original operation's local byte span directly from native lexer positions, plus explicit entry/leave records for Do. Thus generated exact source spans and Rust output provenance are checked against both independently authored operations and the native lexer, not inferred from production flattening. Repeated uses retain distinct invocation origins while sharing source hashes. Synthetic boundaries retain their original Do call spans separately.

Provider identity is the actual object carrying explicit Resources: a page-tree ancestor, leaf Page or Form. Absent/null Form Resources inherits the immediate caller's provider. Two Forms sharing one indirect resource dictionary remain distinct providers. Native preflight intentionally follows outer resource frames for missing names, matching MuPDF's lookup behavior; four dedicated cases prove this differs from the narrow Rust replacement-scope contract. These are classified native-accepted/Rust-refused, not claimed as native refusal parity.

Source pins include MuPDF1.28.1 `pdf-op-run.c` (actual Form save/Matrix/BBox/restore), `pdf-interpret.c` (resource frame lookup and recursive processing), `pdf-xobject.c` (metadata retrieval), `geometry.c` (matrix composition), native lexer and font/outline implementations, and both existing licensed pinned Base14 assets. The new build script checks them before linking the existing test-only static libraries. Native code remains under its AGPL notice; existing OFL asset notices remain separate. No native runtime execution is part of the Rust engine.

Bounds are16MiB input,128MiB native heap,4MiB per decoded preflight stream,128MiB cumulative decoded work,64MiB output,4096 attempted Form invocations,32 Form depth,100,000 execution origins,256 used indirect fonts,256 tracked q/Q matrices, and the prior token/event/outline caps. CPU/wall limits remain30/45seconds. Device output is buffered atomically. Native warnings, repaired inputs, malformed envelopes, signals and unexpected exits fail the gate. An expected semantic page refusal is a nonempty explicit JSON refusal; it does not hide a native process crash. Dedicated invalid-wrapper checks separately require documented exact exits and zero stdout.

## Artifacts and reproduction

The fixed comparison policy is `reference/form-reference-policy-f0.json`; this note may later append measured results. Planned final evidence reports are `reference/form-differential-f0.json` and `reference/form-reference-validation-f0.json`. Independent model selfchecks are `reference/form-review-model-selfcheck.json`. Raw compressed PDF trace and pixel evidence is retained under `/private/tmp/tandem-r22-form-evidence`.

```sh
sh engine/tools/build-form-page-reference.sh /private/tmp/tandem-mupdf-reference
python3 engine/tools/differential-forms.py --rust /private/tmp/pdf-inspect-r22-oracle-final --report engine/reference/form-differential-f0.json --evidence-dir /private/tmp/tandem-r22-form-evidence
python3 engine/tools/test-form-reference.py --rust /private/tmp/pdf-inspect-r22-oracle-final --report engine/reference/form-reference-validation-f0.json
```

The independent graph/state model is reviewer-owned `tools/form_review_model.py`. `tools/form_cases.py` encodes original object graphs and source-local spans; `tools/form_reference.py` checks identity/state/source tables and delegates already accepted outline comparisons. `tools/form_raster_reference.py` and `tools/form_pixel_model.py` add Form-specific orchestration while preserving prior sampling policies. The final driver must verify its input/source/binary evidence hashes before and after execution.

## Final measured results — 2026-10-06

R22 ordinary Form XObject execution is accepted after the complete native/model/corpus gate. Scoped resources and fonts, nested Forms, affine Matrix/BBox clipping, state restoration, owned provenance and cumulative failure budgets are verified. These are reusable drawing streams, not interactive AcroForm fields.

- Rust 1.98.1 and minimum 1.85.1: **724 tests pass each**. Formatting, strict all-target/all-feature Clippy and minimum-toolchain wasm32 library compilation pass.
- **58 positive generated Form graphs**, four explicit native-fallback/strict-scope differences and ten malformed/cycle/state refusals pass. **66 generated renders and 10 independent pixel models** pass with zero certified channel-interval violations.
- **545/572 corpus pages render at 18 DPI**, with 27 explicit refusals; all 500 stress pages pass. All **48 supplementary 72-DPI renders** pass. The two new Form-only pages render at both resolutions. All **589 previously accepted render hashes remain identical**; all 29 old refusal pages receive a Form-aware census.
- Reference validation: 23 native comparator mutations, 25 Rust comparator mutations, 13 native process faults, six Rust process faults, three actual native negative-wrapper cases and eight pixel/baseline mutations are rejected as expected.
- The new `tools/verify-form-gate.py` requires exact corpus identities/counts, all generated/model/census coverage, preserved hashes and current evidence hashes. Ten deliberately shortened or corrupted reports are rejected. It prevents a successful `--skip-corpus` or `--native-only` run from being treated as final acceptance.

Native AA/edge differences and the existing source-certified 229-versus-230 color policy remain diagnostic. No tolerance or policy was widened. This is a bounded Forms milestone, not complete PDF rendering or application parity.
