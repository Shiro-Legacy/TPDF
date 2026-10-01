# Acceptance — fill-anywhere (2026-09-30, fable51)

Contract `.quad/shared/p6/contract-fill-anywhere.md`; build note `fill-anywhere-build.md`; review `review-fill-anywhere.md`. Rollback `.quad/backup/pre-fill-anywhere-20260930.tgz`, `pre-fill-anywhere-review-20260930.tgz`.

**Gate** (`accept-unit-nogui.sh fill-anywhere fill-anywhere,fill-anywhere-form - - tests/fill-anywhere.test.mjs 600`, output `acceptance/fill-anywhere-accept.out`): tsc ok, build ok, suite `# tests 607 / # pass 606 / # fail 0 / # skipped 1` (600 + 6 new), red-before log 5 `not ok`, `fill-anywhere` (34 steps) and `fill-anywhere-form` (14 steps) ×3 exit 0 / 0 red, 23 regression drives 0 red, no secrets, fixtures.sha256 9 OK, 55-line hash-after identical, no `--drive=` stragglers, package ok → `== result: fail=0`.

**Review** (Opus 5.5, refute-first) fixed 3, each with a red-first test: `annot-set fit` re-fitted non-Helv-12 boxes (now Helv 12 only); `fill-text` silent on empty text / no-annotate documents (now refuses in `error`); a trailing-blank-line re-edit journaled a no-op.
**Orchestrator fixes after review:** F4 a fitted line wider (or a text taller) than the page is refused `overflow` before `mutate` ("The text does not fit on the page; use shorter lines"; test in `tests/fill-anywhere.test.mjs` (a)); F5 `src/main.ts` `save-file`: a Save-As (Untitled or filled) that could not rebind the window returns `write-failed` instead of ok, so the renderer never marks "copied" while main is still bound to the original (no test: needs the copy to vanish after the write).

**Trigger file** (headless, read only): 2 pages, 0 fields → `fill-text` → 1 FreeText, `fill.filled true` → `save-to` scratch ok, reopens with the box.

**Deferred / not verified:** GUI proof (`checks/shot-fill-anywhere.mjs` written, never run): real click → type → Enter → drag, `#fill-textbox-add`, ⌘S sheet default "… (filled).pdf". F4's red-before was the reviewer's measurement, not a logged run. Leftovers: progress.md §10.
