# Compare review — TESTS / DRIVE / EVIDENCE lens (Opus 5.5, 2026-09-25)
Verdict: REVISE (no blocking; 3 should, 5 nit). Scratch: .quad/scratch/compare/review-tests/. No src/tests/scripts/fixtures edited.

## Ran (all Electron/test runs under .quad/run.lock)
- compare.jsonl ×3 (scripts/drive.mjs): exit 0/0/0, 12 steps, 0 red; normalised per-step {ok, revision, dirty, error, announcement, compare} sha1 identical ×3 (436a7420…).
  Pixels: S3 1590/484704 0.0033 S=1; S4-S7 1670 0.0034; S9/S10 0; S11 466 0.001. Revisions [0,1,1,1,1,1,1,2,0,0,0,0].
- node --test tests/p6-compare.test.mjs ×3: 10/10/0/0 each.
- npm test once: 574 / 573 / 0 fail / 1 skip (reproduces compare-integration.md).
- Red-before: tsc'd the backup (pre-compare-20260925.tgz) into scratch, ran the CURRENT p6-compare.test.mjs with CI_NO_ELECTRON: engine (a)(b)(c)(c2)(d)(e)(f)(g)+§9.1/§9.7 all red (9/9); drive test skipped.
- Backup diff: tests/ only p2-shell.test.mjs changed (additions: REQUIRED_IDS +10, Help sentence tag-stripped pin + order pin, READ_ONLY +4); no existing drive/test line changed; matrix still 11. src/main.ts diff = the menu item only.
- Hashes after every run: fixtures.sha256 9 OK; pdf/png list identical to compare-hash-before.txt (55). No --drive= stragglers.
- shot-compare.mjs: read fully, node --check OK, NOT run (per brief).
- Regression drives once: dialogs 11, debt1 22, direct-edit 75, direct-edit8-save 42, verify 17, sign-field 45, help-fields 21, nav-zoom-find 20, redact 8, bates 9, direct-edit3 79, refusals 29, replace-page 7 — all exit 0, 0 red (matches compare-build.md).

## Findings
S-1 (should) tests/drive/compare.jsonl:9 (§9.16 "pageB 9 clamps to 2"): step 5 is vacuous. Step 4 already left pageB 2 and step 5 pins no error, so a
  build that ignores or refuses pageB 9 (showError, no change) passes step 5. Fix: step 4 `{pageB:1}` → addedLines ["two"], then step 5 `{pageB:9}` → pageB 2,
  addedLines ["three"], "error":"" (p6-compare steps[3] pixelsOk/differing>0 still holds).
S-2 (should) no evidence for §9.12/§9.14 controls: #compare-page-b (1-based change handler renderer.ts:9529), PageUp/PageDown and Escape in #compare-view
  (:9519) have no drive step, test or shot case; the drive `pageB` calls setComparePageB directly, so a 0/1-based slip in the input handler passes everything.
  Fix: shot-compare.mjs: CDP focus #compare-page-b, Input.insertText "3" + Tab → STATE pageB 2; CDP Escape keyDown on #compare-view → compare.open false.
S-3 (should) renderer.ts:7442-7443 (§9.2/§9.7): every text refusal (too-many, read-only from a locked main, no-document) is summarised and announced as
  "this document does not permit copying"; no test covers the refused branch (text.refused never reached in drive or test). Fix: word by reason; pin it in the drive half or a unit.
N-1 (nit) shot-compare.mjs:175: if ⌥⌘K does not open compare the run fails "timeout: null", not naming the chord (§9.10). Fix: catch and throw
  "⌥⌘K (key code 40, option+command) did not open compare".
N-2 (nit) shot-compare.mjs:186-193: no elementFromPoint hit-test / fallback log before the CDP clicks (shot-bates.mjs:205 pattern); fixed sleep(600/800) instead of #status waits.
N-3 (nit) shot-compare.mjs:199-202: the fixtures.sha256 check runs only on the success path; move it into a finally so a FAIL still verifies hashes.
N-4 (nit) acceptance/compare-red-before.log: predates the last test edit (locations 103 vs current 104) and has no drive-half entry; the drive test has no red-before evidence (engine half re-proven above).
N-5 (nit) design-agent-drive.md:85: "(or locked)" — a needsPassword main never reaches compare in the renderer ("Open a document first…"); S formula omits the floor to 0.01.
N-6 (nit) compare.jsonl:11: STATE error "Could not compare with encrypted.pdf…" survives the undo (step 8) and is unpinned; carried risk, pin "error" once clearError lands.

## checkedFine
compare.jsonl = objections template + close step exactly; header "12 steps, 0 click lines" equals file (test also asserts). Steps 1-4, 7-12 fail if the feature were absent
(pageB, slot kept on refusal with last detail, undo rev 2 with slot open, open-as-A slot survives, {page:1} → 1/1, close nulls + last close). p6-compare drive half: differing>0,
ratio=differing/total 4dp, total=ceil(612S)·ceil(792S), S from STATE; announcement pinned. shot-compare: absolute --insert-from, key code 40 {option,command} by unix id with
frontmost check in one osascript call, exit 2 on -1712/1002, exit 1 on blocked, SIGTERM in finally. design-agent-drive.md "## Compare drive notes" appended at EOF after
"## Batch drive additions", names step, line-level from, --insert-from=, STATE slice, S, pageB rules, close. Help sentence matches pin exactly; READ_ONLY 4 ops; REQUIRED_IDS 10.
