# P6 U8 integration (integrator, Opus 5.5, 2026-09-24)
All build/test/drive runs were under .quad/run.lock. There was no stale lock.

## Suite
- `npx tsc --noEmit`: clean. `npm run build`: exit 0.
- `npm test`, run once: 536 tests / 534 pass / 1 fail / 1 skip.
- The only fail is `tests/p6-drive8.test.mjs:236` "drive P6 U7: Cut of links, runs, run + image and a radio; ..." with `red(steps).length` 1 !== 0 (see the open defect below).
- `p6-text8` (5) and `p6-drive9` (1) pass.

## Untouched-file diffs against .quad/backup/pre-p6u8-20260924.tgz
- src/renderer/engine.ts: EMPTY
- scripts/drive.mjs: EMPTY
- src/main.ts: EMPTY

## Drives (node scripts/drive.mjs)
- direct-edit9.jsonl, run 3 times: exit 0 / 0 / 0; 30 steps, 0 red each time
- direct-edit3: exit 0 (79 steps, 0 red). direct-edit8: exit 0 (115 steps, 0 red).
- content-edit: exit 0 (4/0). debt1: exit 0 (22/0). object-edit: exit 0 (11/0).
- direct-edit8-cut: exit 1, 51 steps, 1 red: step 26 (file line 32), `key Meta+v`.
  - expect `content.edited {kind selection, action paste, name "PA1", count 2, skipped 0}`
  - seen `edited.name "Move me"`, `direct.selection [{image PA1 p0.c1.29.c43c2a9e}, {text "Move me" p0.c1.121.6c51e7e2}]`, objects 3, rev 2, announcement "Pasted 2 items"

## Fixture hashes
- `fixtures.sha256`: 9 OK before and after.
- The `u8-hash-before.txt` diff is empty before and after. There are no `--drive=` stragglers. shot-direct9.mjs was not run (it belongs to acceptance).

## Applied fixes
None.

## Open defects
1. `tests/drive/direct-edit8-cut.jsonl:32` (step 26), pinned by `tests/p6-drive8.test.mjs:236`. The expect is stale; the renderer is correct.
   - Cause: D1 makes the pasted duplicate "Move me" (the original stays, because a Cut of a run is a copy) a candidate with bounds. `pasteClipboard` (renderer.ts:3120-3126) therefore adds it to the picks, and it becomes the anchor, so `edited.name = picks.at(-1).name = "Move me"` (§10.34: anchor = the last). This is the same flip as p6-objects7:450-467 (§9.5).
   - Proposed flip: `"name":"PA1"` → `"name":"Move me"`, and add `"selection":[{"kind":"image","name":"PA1"},{"kind":"text","name":"Move me"}]` to the step's `direct` expect. This makes the step stricter, not weaker.
   - Why it is not applied: the integrator's in-place edit of that file was refused by the session's permission classifier. It needs orchestrator or human sign-off, and then `accept-u8.sh` reaches 0 fail.
2. Carried over from the build report, outside this unit, not fixed: engine.ts `text-add` → undo → `text-add` leaves the new run with a dangling /PDF4AllF1 (freed object 14 0 R). The repro is .quad/scratch/p6u8/impl/probe4.mjs.
3. Carried over, pre-U8: a GUI-added run cannot grow when edited (refused as `overflow`), because its limit is its own right edge. The §9.10 ledger was adapted: B is edited to "Pair".
