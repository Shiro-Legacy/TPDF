# Review — P3 U1 Gate A (fable51, 2026-09-16 21:20)

Reran `node scripts/p3/journal-probe.mjs` myself: 20 pass / 5 fail, identical to grok. Then reproduced the redo claim with a 30-line script of my own (edit A, edit B, undo, then a written-and-abandoned op C) on `edit-transformed.pdf`, mupdf 1.28.1 wasm.

## What is true (independently confirmed)
| Scenario after `undo` (pos 1, steps [A,B], redo available) | journal after | content | verdict |
|---|---|---|---|
| begin C, **write**, `abandonOperation` | pos 1, steps **[A]**, redo=false | restored (BRAVO) | redo branch B dropped; undo of A still works |
| begin C, write, `endOperation` (today's `mutate`) | pos 2, steps [A, C-fail] | **partial write kept** (ALPHA) | worse: bogus step on the undo stack, redo of B also gone |
| begin C, write, `endOperation` on a **successful** op | pos 2, steps [A, C] | new content | redo of B gone — ordinary editor branch semantics |

So the "failure" is: a mutation that fails *after* a native write, *after* the user has undone something, drops the redo branch exactly as a successful mutation would have. The only difference from normal is that the user got an error instead of a result. `abandonOperation` is strictly better than the current `finally { endOperation }` on every axis.

**Undo history and `savedPosition` stay sound.** `journal.position` is untouched by abandon. The engine's dirty flag is `position !== savedPosition` (engine.ts:948). If the user saved at pos 2, undid to 1, then a failed op wiped step B: position 1 ≠ savedPosition 2 → dirty=true, which is correct (the document differs from the saved bytes). If saved at pos 1: dirty=false, correct. No case reports clean when it is not.

The 5th failure (`tagged-extract`) is not a journal failure: the probe rewrote the operator but not `/ActualText`, so mupdf's stext still says ALPHA while the stream says BRAVO. That is the U2 tagged-semantics requirement, already in the phase contract; drop it from Gate A.

## Recovery options, smallest first
1. **Accept the ceiling and land Gate C as measured** (recommended). `mutate`: begin → work → `endOperation` on success → revision bump; on throw → `abandonOperation` → rethrow. Validation stays before `beginOperation` (an abandoned op with no writes keeps redo — measured). Every mutation reply already spreads `journalState`, so `canRedo=false` reaches the renderer and the Redo item greys out. Contract text: replace "pre-existing undo **and redo history** are unchanged" with "undo history and position unchanged; the redo branch is dropped iff the abandoned op wrote, identical to a successful op". Gate C regression asserts exactly that.
2. Keep a JS-side copy of the redo branch and replay it — impossible: the wasm API has no way to push journal steps.
3. Snapshot the whole document before each mutation to restore the branch — doubles memory at the 120 MiB ceiling for a case the user cannot distinguish from a normal new edit. Rejected.
4. Keep today's `finally { endOperation }` — rejected; it leaves a partial write on the undo stack (row 2).

Option 1 needs the human's OK because the U1 contract §5 says the failure path cannot be accepted without a reviewed recovery decision. This file is that review. No source edits made; no contract text changed pending the decision.

## Addendum 21:25 — astra's false-clean case (m10), measured
Scenario: edits A, B; save (savedPosition=2); undo (pos 1); then any op that cuts the redo branch; then a successful op C (pos 2).

| path | pos | savedPosition | engine dirty | content vs saved |
|---|---|---|---|---|
| **today's engine**, success after undo (no abandon involved) | 2 | 2 | **false** | DELTA vs GAMMA — **false clean** |
| abandon-with-write, then success | 2 | 2 | **false** | same false clean |
| with a branch guard (below) | 2 | -1 | true | correct |
| redo back to the saved state (guard must not fire) | 2 | 2 | false | correct |

**This is a pre-existing P2 defect, not introduced by abandon.** `journal.position` is a step count, not an identity; once the redo branch is cut, the saved state is unreachable, but the count can land on it again. Consequence today: save → undo → new edit → close shows no prompt, work lost. `tests/p2-edit.test.mjs:286` covers undo-back-to-saved, not edit-after-undo.

**Fix (ds2, inside Gate C, same helper):** in `mutate`, capture `branched = pdf.canRedo()` and `before = position` before `beginOperation`; after `endOperation` *or* `abandonOperation`, `if (branched && !pdf.canRedo() && session.savedPosition > before) session.savedPosition = -1` (unreachable → dirty until the next save resets it at engine.ts:2764). An abandon with no writes keeps `canRedo()` true, so the guard does not fire and the saved state stays reachable. Regression: save → undo → edit → `dirty === true`, and the same through abandon-with-write.

Recommendation unchanged: approve option 1, with this guard and its test added to Gate C's scope. Nothing else in U1 changes.

## Gate C source review 22:20 (fable51) — PASS pending dynamic run
`mutate` (engine.ts:1307–1328) is the measured sequence verbatim: `canRedo`/position read before `beginOperation`; `endOperation` only on success; `abandonOperation` on throw; branch guard on both paths (`savedPosition = -1` when a redo branch existed, is gone now, and the saved position lay beyond `before`); no revision bump on failure. `endOperation`/`abandonOperation` occur nowhere else (grep). Error replies spread `journalState` so `canRedo:false` reaches the renderer. `savedPosition` writers: open (2211), close (2289), saved (2895), guard (1323) — the sentinel is only ever compared for equality, so `-1` reads dirty until the next save.
`tests/p3-journal.test.mjs`: 7 tests cover brief groups 1–6; failure injection patches `PDFPage.prototype.createAnnotation` for one dispatch and restores in `finally`; no production hook. Deliberate non-assertion of whole-file byte identity (empty xref slots after a rolled-back write) is documented and within the contract.
Two notes, no change requested: (a) if `abandonOperation` itself threw, the original error would be replaced and the guard skipped — that is the contract's "stop and report" path, and it never threw in any probe; (b) ds2's page-wrapper finding (a held wrapper sees a phantom annot after rollback) is a mupdf cache behaviour; production destroys wrappers in `finally`, recorded in `p3-unit1-ds2.md`.
Dynamic re-check (focused + full suite) runs after grok releases the slot.
