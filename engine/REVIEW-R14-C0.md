# C0 independent mixed-program review

The standalone mixed coordinator is accepted by the independent gate after the failed-preflight work-accounting fix described below. Reviewed `program.rs`, the minimal P0/T0 introspection/context/budgeted-push changes, and `cli_program.rs`. This is a mixed owned path/source-code trace with explicitly injected fonts; it does not establish document-page execution, glyph rendering, reading order or the earlier page-inventory ceiling.

## Independent and native evidence

`tools/program_model.py` maintains independent path/color/clip state alongside the previously verified exact Fraction text-state model. `tools/differential-program.py` compares raw Rust child arenas, path/clip/dash identities, original global operation indices/spans, complete T0 output, ordered global event references, every text color/clip/CTM snapshot, and exact operation/child/work-unit accounting. It also compares a separately normalized native callback sequence, without using native results to derive model expectations.

All220 programs pass:5,280 ordered Rust/model events,3,960 native semantic paint/clip/text events and2,200 source codes. The generator includes Gray/RGB/CMYK fill contexts, fill-plus-stroke-plus-clip order, nested clipping, Q restore ancestry, color and stroke-style changes inside BT, Tr3 invisible intent, font/spacing/scale/rise/mode restoration around whole text objects, and noncommuting CTMs. The initial native/CLI domain is identity CTM with unbounded clip; separate library regressions cover initial Empty clipping. All26 malformed or unsupported CLI suffixes after a successful path/text prefix refuse atomically with diagnostics and no stdout.

Path geometry/color/style/clip ancestry and semantic order compare exactly for these dyadic cases. Native text matrix and normalized-advance fields use the predeclared T0 float32 tolerance: absolute0.002 plus relative0.00002*max(1,abs(expected)), with bounded generated coordinates. Maximum observed component error is0.000003800000001774606. Normalized native advances can report0.75000006 for a750-unit width; this is explicitly recorded as float32 arithmetic, while Rust width/position/context expectations remain exact. Native text batching is normalized to primary source-code records with Unicode continuations, retaining the raw callback output hashes. Tr3 color is not observable in native ignore_text and is compared only to the independent state model. No native paint-color claim is made for invisible text.

Final report: `reference/program-differential-c0.json`. Seed2026100314, all binary/harness dependencies hashed, zero failures. Dedicated frozen binary `target/program-gate/debug/pdf-inspect` SHA256: `13b6650c289bac3701c304b3bb8bbacdbb7628a146024cfc0dc4a04df3127412`.

Required regressions against the same unchanged binary also pass:

- `reference/path-differential-c0-regression.json`:310 P0 cases/5,835 events and20 strict refusals; the prior four-native-versus-zero-Rust empty-path callback difference remains explicitly recorded.
- `reference/text-position-c0-regression.json`:246 exact T0 programs/2,906 placements,160 native programs and48 strict refusals.

Prior accepted stage reports were preserved separately.

## Source and adversarial tests

All seven tests in `tests/review_program.rs` pass with strict targeted Clippy:

1. The second broadcast child can fail after P0 has saved state. Parent usage retains first-child allocation and both attempted child calls; the error remains sticky and finish publishes nothing.
2. Empty pending W/W* cannot disappear at BT. Active paths and path operations inside BT refuse; an explicitly ended/restored empty clip is legal.
3. Path-only/style operations do not reset text/global operation indices. Path and text events retain original logical source spans.
4. Color changes within BT, nested clip ancestry and Tr3 intent survive q/Q restoration with immutable context snapshots.
5. Aggregate operation/work/event/context ceilings and the minimum shared allocation boundary are exercised at/beyond the edge. Child and coordinator allocation charges stay within the aggregate cap on failure. Boundary calibration uses identical other limits because their geometric capacity ceilings affect allocation requests.
6. A late event-limit failure during TJ preflight retains the already admitted array-traversal work.
7. Initial Empty clipping preserves position/paint tracing and cannot hide invalid or unsupported suffixes.

The coordinator retains one P0/T0 pair for the whole program, checks finite numeric CTM equality and save/text-object invariants, rejects unsafe boundary transitions, and routes shared commands P0 first then T0. It snapshots P0 context before text execution, preflights exact event counts for one-byte text, and exposes only a successful final owned result. q/Q/cm never recreate an interpreter or reset child counters.

P0 now charges full requested targets, strengthening its earlier capacity-growth allocation policy. Both budgeted child calls lower the effective allowance to their existing charge plus parent remaining budget before allocation, then parent records deltas even on error. Coordinator vectors likewise charge full target requests before reserve. The parser and borrowed input/font storage remain separately bounded prerequisites; C0 does not claim a document-session aggregate cap yet.

## Finding resolved

Initial `preflight` computed TJ work locally and committed only after all event checks succeeded. A large array of empty strings followed by a numeric element could fail the event ceiling after traversing the array but report only the global operation charge. Although the failed interpreter was sticky, this underreported consumed work for any future session accounting. The implementation now commits admitted child/array traversal work before scanning and charges source-byte work before later checks. The independent regression verifies retained work on this late failure.

No remaining implementation blocker was found for the approved C0 scope. Future document orchestration must still implement the separately reviewed ownership, content/resource decoding and multi-page aggregate limits, and prove actual page execution rather than reuse the operator-inventory ceiling.
