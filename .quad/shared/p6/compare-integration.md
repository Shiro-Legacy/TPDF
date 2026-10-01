# Compare integration (integrator, Opus 5.5, 2026-09-25)
Scope: contract-compare.md (§9 overrides) against compare-build.md. All runs under .quad/run.lock (mkdir/trap rmdir); no stale lock found.

## Suite
- npx tsc --noEmit: clean.
- npm test (build + node --test tests/*.test.mjs), ONCE: 574 tests / 573 pass / 0 fail / 1 skip (skip = "legacy p12 parses when rc2-40-cbc exists", pre-existing).
  Baseline after Batch was 564/563/0/1; +10 = tests/p6-compare.test.mjs. No failing test.

## Untouched files (tar -xOzf .quad/backup/pre-compare-20260925.tgz <p> | diff - <p>): all EMPTY
src/preload.cts, src/policy.ts, src/safe-write.ts, src/renderer/content-edit.ts, scripts/drive.mjs,
tests/p6-batch.test.mjs, tests/p6-bates.test.mjs, tests/p2-decorate.test.mjs, tests/drive/{batch,bates,bates-pad,dialogs,direct-edit3}.jsonl.

## Drives (node scripts/drive.mjs; exit code, steps, red)
- compare.jsonl x3: 0/0/0, 12 steps each, 0 red; per-step STATE (ok, revision, compare) identical across the 3 runs (same sha1).
  Ledger: revisions [0,1,1,1,1,1,1,2,0,0,0,0]; hello-vs-nav p0 differing 1590 / 484704 ratio 0.0033 S=1; pageB 2 → 1670 (0.0034);
  pageB 9 clamps to 2; encrypted.pdf refused (last.detail "encrypted"), slot kept; undo keeps compare; open nav as A → same 1, differing 0;
  {page:1} → A1/B1 differing 0; {pageB:2} → 466; close → open false, last {close, ok}.
- Regression, once each, all exit 0 and 0 red: dialogs 11, debt1 22, direct-edit 75, direct-edit8-save 42, verify 17, sign-field 45,
  help-fields 21, nav-zoom-find 20, redact 8, bates 9, direct-edit3 79, refusals 29, replace-page 7.

## Hashes
- shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256: 9 OK before and after.
- fixtures *.pdf/*.png vs checks/compare-hash-before.txt: diff EMPTY. No new file under fixtures/.
- Note: fixtures/p0b/.ca/ca.srl serial bumped ...506 → ...507 by tests/p5-verify-crypto.test.mjs:193 (-CAcreateserial) during npm test;
  pre-existing behaviour of every npm test run, not a pdf/png, not in either hash list.

## Applied fixes
None (nothing red).

## Open defects
None blocking. Carried risks from the build (not contract violations): renderer ignores reply.revision on compare-text (compareGen only);
pixel diff on the renderer main thread O(W·H); overlay stretched to B's wrapper when page sizes differ; refused-open error survives undo.

## Not run
checks/shot-compare.mjs (fable51, acceptance); §9.3 memory measurement (review). pgrep -fl -- '--drive=' empty at finish.
