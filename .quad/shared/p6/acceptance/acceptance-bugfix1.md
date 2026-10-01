# Acceptance — bugfix1 (2026-09-29): ACCEPTED (GUI proof deferred)

- Contract: `.quad/shared/p6/contract-bugfix1.md` (D1–D8, human-ordered in chat); build note `bugfix1-build.md`; review `review-bugfix1.md` (ACCEPT after 1 reviewer fix: rotated page + app-written run could grow off the page, `content-edit.ts` `editAttachGrowthLimits`, test (f)).
- Gate: `acceptance/accept-bugfix1-nogui.out` — `accept-unit-nogui.sh bugfix1 bugfix1 - - tests/bugfix1.test.mjs 594`: tsc ok, build ok, suite 601 / 600 pass / 0 fail / 1 skip (≥ 594 + 6 new), red-before 5 `not ok`, `bugfix1` drive ×3 green (11 steps), every regression drive green, no secrets, fixtures.sha256 9 OK, hash-after == hash-before (55 lines), no `--drive=` stragglers, package ok (the human's packaged app was closed; asar 2026-09-29 16:01), fail=0.
- fable51 read the source diff against `pre-bugfix1-20260929.tgz`: only contract-owned files changed.
- **Deferred:** GUI proof (real ⌘N → type → ⌘S shows the save sheet with `Untitled.pdf`; cancel keeps the document dirty). No shot script exists yet; run when the human is away.
- New baseline: 601 / 600 / 0 / 1. Rollback: `.quad/backup/pre-bugfix1-20260929.tgz` (pre-build), `pre-bugfix1-review-20260929.tgz` (pre-review).
