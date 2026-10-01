# Debt sweep 2 — review, ENGINE lens (Opus 5.5, 2026-09-24)
Verdict: ACCEPT (one should, two nits). Probes: `.quad/scratch/debt2/review-engine/probe.mjs` → `probe.out`, `u6.mjs` (pure node on build/ 22:46, current with src).

## Diff vs pre-debt2-20260924.tgz
engine.ts: ClipItem `tr` (:812), tfState from `record.renderMode` (:3398), block `${tr} Tr 0 g 0 G 1 w 0 J 0 j [] 0 d` (:3588-3592), ALLOW pass (:3819-3829). content-edit.ts: `renderMode?` (:193-194), record (:2408). objects7: :411/:645 wraps + 5 tests (:756-823). objects6: :232 box. Nothing else.

## Findings
- E-R1 (should) engine.ts:3908 — §1.1 second sentence ("the dispatch … builds the single-op request from `{ id: request.id, page, op, ...picked }` so nothing else can leak even if shaped() is bypassed") is not built and debt2-build.md does not say so; `runAnnotation` still spreads `...item`. §9.1 amends only the refusal shape, so the clause stands. Unreachable today (ALLOW refuses first, probe.out 48/48 PASS), but the belt-and-braces half is missing. Fix: `const { op, ...rest } = item; dispatch({ id: request.id, page, op, ...Object.fromEntries(ALLOW[op].map(k => [k, rest[k]])) }, …)`, or record the omission in debt2-build.md "Not done".
- E-R2 (nit) tests/p6-objects7.test.mjs:759-796 — the key rule is pinned only on field-props, redact-set, link-set. field-delete and annot-delete are also dispatched through `...item` and have no test; dropping their ALLOW entry to a wider list would not go red. Fix: one loop test over all 8 ops with a stray `page` key (probe.mjs body).
- E-R3 (nit) tests/p6-objects7.test.mjs:810-822 — the N2 tests do not save → reopen. The probe shows it survives: after reopen the pasted run rescans renderMode 1/2, candidate, last /Contents still holds `0 G 1 w`. Fix: add `scanEditTargets(reopen(s).doc, 0)` renderMode assert.

## checkedFine
- §9.1 ALLOW: the 8 lists match §9.1 and the dispatch reads. The renderer builders (renderer.ts:2926-2931, :2951-2958) send only allowed keys. Only objects7 builds batches; every existing shape passes (51/51). Stray id/page/name/target/zz(undefined)/`__proto__` on EACH of the 8 ops → invalid-selection, "edit-batch: unexpected key <k> on <op>", index 1, journal (revision/savedPosition/canUndo/canRedo) unchanged: 48/48 PASS. The 8 legit controls land. Extra key + repeat → key refusal (pass order right).
- §9.3/§9.4 paste: Tr 0/1/2 last element = `0 Ts <tr> Tr 0 g 0 G 1 w 0 J 0 j [] 0 d`. Tr 0 still writes 0 Tr (U7 pin). Rescanned record renderMode = tr, candidate. Save → reopen keeps renderMode and the block. With a trailing page `1 0 0 RG 6 w 2 J`, the pasted run draws 0 red px (stroke state isolated). Tr 3/7: the scan marks them unsupported-graphics-state and clip-copy refuses not-candidate, so no paste (the contract says nothing here; behaviour is right).
- content-edit renderMode: one record build site (:2399). It is always a number and 0|1|2 for candidates. It is worker-only (never in a reply), so edit-targets shapes are unchanged. p6-objects6/7 pass 51/51.
- N3: `still` (objects7:158) and `nothingWritten` (:81) deep-equal the journal and compare the page dict / contents bytes. They are not vacuous.
- U6-N2: [90,0,110,200] lands for rotate-cw on the :232 state (Im1 back at [72,600,172,650]). spin/ROTATE-CW/null with that box → invalid-transform "transform must be rotate-cw, rotate-ccw, flip-h or flip-v", journal unchanged. The refusal happens for the right reason.
- Bite: acceptance/debt2-red-before.log 6/6 red on the pre-fix build. The N2 regex fails on both a missing stroke state and a default-0 tr.
- tsc clean; fixtures.sha256 9 OK; the 55-line list matches debt2-hash-before.txt; I edited no src/tests/fixtures file.
