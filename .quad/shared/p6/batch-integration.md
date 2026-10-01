# Batch — integration (Opus 5.5 integrator, 2026-09-25)
Contract .quad/shared/p6/contract-batch.md (§9 read) + batch-build.md. All runs in one .quad/run.lock hold (lock free at start and released). Logs: .quad/shared/p6/acceptance/int-batch-*.log / .err, summary int-batch-summary.txt.

## Step 1 — suite
- `npx tsc --noEmit`: exit 0. `npm run build`: exit 0.
- `npm test` once: 563 tests / 562 pass / 0 fail / 1 skip (Bates baseline 553/552/0/1 + 10 new p6-batch tests). No failing tests.
- `node --test tests/p6-batch.test.mjs` ×3 (stages batch-in / batch-c mkdtemp folders, --out=): 10/10/0/0 each run.
- Untouched-file diffs vs .quad/backup/pre-batch-20260924.tgz — all EMPTY: scripts/drive.mjs, src/renderer/content-edit.ts, src/safe-write.ts, tests/p2-decorate.test.mjs, tests/p6-bates.test.mjs, tests/p3-drive.test.mjs, tests/drive/{bates,bates-pad,dialogs,refusals}.jsonl.

## Step 2 — drives
- batch.jsonl direct form ×3, each staged per §9.18 (mktemp -d; batch-in + batch-c with encrypted/hello/nav/rotated; `--out=$d`): exit 0 ×3; `ls $d/batch-in/pdf4all-out` = hello.pdf nav.pdf report.txt rotated.pdf each time.
- Regression drives once, all exit 0: bates, bates-pad, dialogs, refusals, help-fields, direct-edit8-chords, replace-page, direct-edit, direct-edit3, direct-edit8-cut, direct-edit9, object-edit, content-edit, debt1.
- No red steps.

## Step 3 — fixes
- None applied. batch.jsonl expects checked against §9.13/9.14/9.16/9.17 (Bates order hello 1, nav 2-4, rotated 5, next 6; rerun all `exists`, report `exists`; cancelAfter 2 → done 2, rows 3-4 queued).

## Step 4 — fixtures
- fixtures.sha256: 9 OK before and after.
- `find fixtures -type f (pdf|png) | shasum` vs batch-hash-before.txt: diff EMPTY before and after.
- `find fixtures -name pdf4all-out`: empty. Nothing new under fixtures/.
- Note: fixtures/p0b/.ca/ca.srl (openssl CA serial, not a pdf/png, not hashed) is rewritten by the existing p5 signing tests during `npm test` — pre-existing behaviour, not Batch.

## Step 5
- shot-batch.mjs NOT run (fable51 runs it; ⌥⌘B System Events chord still unchecked). `pgrep -fl -- '--drive='` empty at finish.

## Open defects
- None found. Residual risks from the build report stand (saveToBuffer throw → row fails without a reason, queue continues; stampPages validation order changed only for multi-error requests).
