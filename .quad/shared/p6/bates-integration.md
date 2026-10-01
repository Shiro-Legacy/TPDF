# Bates integration (integrator, Opus 5.5, 2026-09-24)
Contract: contract-bates.md incl. §9; build record: bates-build.md. One lock hold (.quad/run.lock) for build + npm test + all drives. Logs: session scratchpad int/.

## Suite
- npx tsc --noEmit: clean (exit 0). npm run build: exit 0.
- npm test (once): tests 551, pass 550, fail 0, skipped 1. Baseline accept-debt2.out 544/543/0/1 -> +7 new (6 p2-decorate bates (a)-(f), 1 p6-bates; p2-shell pins added inside existing tests). No failing tests.

## Untouched files vs .quad/backup/pre-bates-20260924.tgz (all diffs EMPTY)
engine-core.ts, content-edit.ts, main.ts, preload.cts, policy.ts, scripts/drive.mjs, tests/p3-drive.test.mjs, tests/drive/dialogs.jsonl, tests/drive/refusals.jsonl.

## Drives (exit code / steps / red)
- bates.jsonl x3: 0/9/0, 0/9/0, 0/9/0.
- dialogs 0/11/0, refusals 0/29/0, help-fields 0/21/0, direct-edit8-chords 0/19/0, replace-page 0/7/0, direct-edit 0/75/0, direct-edit2 0/50/0, direct-edit3 0/79/0, direct-edit4 0/71/0, direct-edit5 0/35/0, direct-edit6 0/59/0, direct-edit7 0/66/0, direct-edit7-save 0/19/0, direct-edit8 0/115/0, direct-edit8-cut 0/51/0, direct-edit8-save 0/42/0, direct-edit9 0/43/0, object-edit 0/11/0, content-edit 0/4/0, debt1 0/22/0.

## Hashes
- fixtures.sha256: 9 OK before and after.
- pdf/png hash list vs bates-hash-before.txt (55 lines): diff empty. No new file under fixtures/.
- Note: fixtures/p0b/.ca/ca.srl (CA serial, not pdf/png) advanced ...FC -> ...FE during npm test; the p5 sign tests bump it on every run (pre-existing, also seen at debt2). Not a Bates change.

## Review of the implementer's deviation (§9.18 template dirty values)
- Confirmed: save-to is a save-a-copy and keeps dirty true, already pinned by direct-edit3.jsonl:64 (`save-to ... "dirty":true`). bates.jsonl S3/S6 dirty true and S7/S8 dirty false (journal back at never-saved position 0) follow the app, not a wrong renderer. Revisions/decorate values are verbatim from objections-bates-tests.md:19-31. Accepted.

## Applied fixes
- None needed.

## Open defects
- None. Not run here by instruction: shot-bates.mjs (real ⌥⌘F chord collision still UNCHECKED), npm run package, pre-bates-review backup.
- Cosmetic (not a contract violation): addStamp announcement shows a range for a single page ("Bates 000103–000103").

## Finish
- pgrep -fl -- '--drive=' empty; run.lock released.
