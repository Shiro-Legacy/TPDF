# Acceptance — housekeeping B (2026-09-25): ACCEPTED

- Contract: `.quad/shared/p6/contract-housekeeping-b.md`; build notes `housekeeping-b-{src,tests}-build.md`; review `review-housekeeping-b.md` (ACCEPT-WITH-FIXES, 2 fixes in scripts/drive.mjs).
- Gate: `acceptance/accept-housekeeping-b.out` — `accept-unit-nogui.sh` minus the `npm run package` line (human's packaged app was running from `out/`). tsc ok, build ok, suite 595 / 594 pass / 0 fail / 1 skip (≥ 592 + 2 new), red-before 2 `not ok`, 23 legacy drives green, no secrets, fixtures.sha256 OK, hash-after == hash-before (55 lines), no `--drive=` stragglers, fail=0.
- Extra: new `$TMPDIR/pdf4all-*` after the whole gate = 3 × `pdf4all-new-*` (src/main.ts `newBlank`, by design); `/tmp/pdf4all-accept-*` = 0; `fixtures/p0b/.ca/ca.srl` sha256 eca06ff8eccd42843198490ada7566113dd800325b431cc53d1ad6df7e4fd814 unchanged.
- New baseline: 595 / 594 / 0 / 1. Rollback: `.quad/backup/pre-housekeeping-b-20260925.tgz` (pre-build), `pre-housekeeping-b-review-20260925.tgz` (pre-review).
