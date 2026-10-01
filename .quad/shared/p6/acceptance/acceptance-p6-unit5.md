# P6 U5 acceptance — fable51, 2026-09-24 (lean mode, Opus 5.5 subagents)

Contract `.quad/shared/p6/contract-p6-unit5.md` (§9 approved by the human). Build `wf_e3f5ac09-012`, review `wf_d2f312ff-7a5`.
Backup `.quad/backup/pre-p6u5-20260924.tgz`; baseline `checks/u5-hash-before.txt` (54 files).

## After the review fixes, by fable51
- E-1 root cause moved into `editPrivatise` (content-edit.ts): an indirect `/Contents` array gets a direct copy on the page before the owned
  stream goes in. The engine seat's `directContents` helper and its two call sites were removed (redundant). The E-1 test gained an
  edit-text run on the shared two-page array (p6-objects5). Targeted: p6-objects5, p3-edit, p6-direct3, p3-objects = 69/69.
- Sign-dialog close race (P5 code): under a full `npm test` the `p5-drive-field` sign step 43 returned `no-certificate` with
  `sign.dialog null` twice (u5-suite.log, u5-suite2.log) and passed alone. The dialog's queued `close` event from the previous drive Escape
  fired after the next step re-opened the dialog, so the handler nulled the field/path and fired `certificateClear` under the running sign.
  Fix: the close handler returns when `certSignDialog.open` (renderer.ts). No deterministic test reproduces the race.

## accept-u5.sh under one lock hold (u5-accept.out)
- tsc clean; build ok; direct-edit6 ×3: exit 0, 59 steps, 0 red; direct-edit, direct-edit2, direct-edit3, direct-edit4, direct-edit5, debt1,
  object-edit, content-edit, help-fields: exit 0, 0 red; no secrets in reports; `fixtures.sha256` 8 OK; `u5-hash-after.txt` = before,
  nothing changed, nothing added (D2); `shot-direct6.mjs` PASS (real CDP right-click + ⌥-drag); no `--drive=` Electron.
- Full suite on the final code (u5-suite3.log): 480 tests, 479 pass, 0 fail, 1 skipped (legacy RC2).
- Shots viewed: `review-out/u5-a-multi-select.png` (Im1 + Im2 outlined, Inspector on the anchor Im2 = R2), `u5-b-menu-fm1.png`
  (menu on Fm1, Replace… dimmed, Duplicate/Delete enabled), `u5-c-alt-duplicate.png` (copy selected at X 460 Y 659.988, "Edited — not saved").
- `npm run package` exit 0, asar 16:00.

## Left in progress.md §10
flip/rotate/crop, alignment guides, Form pass-through, group resize, clipboard; engine test-strength nits E-2/E-3; probe sequencing FAILs
d0/d6/d8/d9/d10 (reviewer reruns pass); shot B ~20 px scroll offset (cosmetic).
