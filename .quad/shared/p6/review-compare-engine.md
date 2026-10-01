# Compare review — ENGINE lens (Opus 5.5, 2026-09-25)
Verdict: REVISE (1 blocking, 1 should, 1 nit). Probes: `.quad/scratch/compare/review-engine/{probe,mem}.mjs`, `mutants.sh` (the mutant copies of build/ were deleted after the run).
Diff vs `.quad/backup/pre-compare-20260925.tgz`: engine.ts only adds lines (the SaveReason union, the Reply union and the open reply are extended). engine-core.ts, worker.ts, preload.cts and drive.mjs are unchanged. tsc clean; fixtures.sha256 9 OK; the 55-line hash list is identical; no `--drive=` stragglers.

## Findings
**E-R1 (blocking) engine.ts:4363-4368, §9.3 + untruthful STATE.** `open` destroys the compare slot BEFORE `openDocument(pdf)`. When that open then throws, the error reply has no `compareClosed`.
Repro (probe.mjs): main hello.pdf, compare-open paddedPdf(MAX-hello), then `open` a 4 KiB "%PDF-1.4 garbage" buffer. Result: `{ok:false, compareClosed:null}` with `session.compare === null`, and the main doc is still hello.pdf.
In the app: File ▸ Open of a corrupt file after comparing a large one. renderer.ts:1765-1767 returns without clearing the slot, so STATE `compare.open` stays true while the engine slot is gone, and the next compare-render fails `no-compare-document`.
Fix (smallest): when `compareClosed` is set, wrap `openDocument` + `detect` in a catch that sets `err.extra = {compareClosed:true}` (dispatch already spreads `extra`, engine.ts ~:6282). The renderer's catch in openDocument calls `clearCompare()` when the rejection carries it.

**E-R2 (should) tests/p6-compare.test.mjs:157-177 / :215, §9.7 store-then-destroy.** Mutant M1 deletes `if (previous) closeDocument(previous.doc)` in compare-open, and all 9 engine tests still pass. The previous compare doc then leaks a whole WASM copy on every re-compare, and no test notices.
Fix: in test (c), after a successful second compare-open, assert that the old doc was destroyed: keep `s.compare.doc` before the call, then `assert.throws(() => old.countPages())`, or count destroys through a wrapped `mupdf.Document.openDocument`.

**E-R3 (nit) tests/p6-compare.test.mjs:179, §9.3 boundary.** Mutant M7 (`>` changed to `>=`) survives, so the cap itself (== MAX) is not pinned. The probe shows that paddedPdf(MAX-hello) beside hello opens ok and that re-opening hello keeps the slot.
Fix: add `compare-open paddedPdf(MAX_PDF_BYTES - helloBytes)` → ok to (c2).

## Mutants (scratch copy of build/, CI_NO_ELECTRON=1)
| Mutant | Result |
|---|---|
| M2 swap removed/added | caught (2 tests fail) |
| M3 drop normalisation | caught (g). stext keeps "a  b" verbatim, so the test really bites |
| M4 drop copy gate | caught (c) |
| M5 drop pair-close on open | caught (c2) |
| M6 drop locked check | caught (reason becomes invalid). Note: `hasPermission('copy')` is true on the locked doc |
| M8 destroy the slot before the compare open | caught (c) |
| M1 no destroy of the previous slot | SURVIVES → E-R2 |
| M7 cap off-by-one | SURVIVES → E-R3 |

## checkedFine (probe.mjs output)
- §9.1: no compare-* row in engine-core.ts. No requireMutable, mutate, beginOperation or enableJournal between engine.ts:4413 and :4481.
- §9.1 with no main doc: compare-open ok, compare-close ok, compare-render gives no-compare-document then ok, compare-text gives no-document. A locked main (encrypted.pdf) gives read-only; signed-intact as main is ok (test).
- §9.2: the rc4 permissions=-20 main has copy false; compare-text → read-only (text-all gives the same) and compare-render is ok. encrypted.pdf as compare → encrypted: 1 open + 1 destroy, both of the new doc. The slot is the same nav doc and still live. An invalid compare → invalid, slot kept.
- §9.3: at cap+1 → too-large with 0 opens (openDocument wrapped). At exactly the cap it opens. The oversize main open closes the slot with compareClosed:true (test c2).
- §9.3/§5 memory, `/usr/bin/time -l`: 120 MiB single open + render → max RSS 603.7 MB. 60+60 MiB pair + compare-render + compare-text → 478.1 MB.
- §9.4: raw stext gives hello {72,53,132,49} and nav {72,66,40,32}. The engine returns [72,53,204,102] and [72,66,112,98], i.e. x0,y0,x1,y1 y-down.
- §9.5 LCS: [alpha,beta,gamma] vs [alpha,gamma,delta] → 2/beta/delta. Duplicates [x,x,y,x] vs [x,y,x,x] → same 3, 1 removed/1 added. Empty vs page, page vs empty and both empty are all correct. A trailing space counts as same.
- §9.5 line cap: 2000 vs 2000 → ok, same 2000, 51 ms. 2001 lines on the B side → too-many (the stext counts are exactly 2000/2001). The table is Uint16, (n+1)(m+1).
- Render refusals (scale 0, scale 99, page 7) are ok:false with no reason token. `request.pdf` is null after compare-open. The compare-open reply keys are {id,ok,name,pageCount,pages}.
- (e)/build risk: the compare-text reply revision is 1, equal to journalState().revision after page-rotate. Every compare op AND every refusal (encrypted, too-large, bad page, no slot) leaves journalState byte-identical. The save bytes equal a control session's (rotate only). Save → reopen width is 792. One undo gives 612 and canUndo false.
