# Contract — housekeeping B (2026-09-25)

Human: "do B" (after the 2026-09-25 audit; see progress.md §12 and §10 "Cleanup candidates"). No behaviour change for users. Rollback: `.quad/backup/pre-housekeeping-b-20260925.tgz`; fixture snapshot `.quad/shared/p6/checks/housekeeping-b-hash-before.txt` (55 lines).

## Owners (no file has two owners)

- **Implementer SRC** — `src/**`, `tsconfig.json`, `tsconfig.preload.json`.
- **Implementer TESTS** — `tests/**`, `scripts/drive.mjs`, `scripts/session.mjs`.
- **fable51** — `.gitignore` (done), `progress.md`, `.quad/**`.

## SRC scope

S1. WinAnsi: delete the copies inside engine.ts (`winAnsiByte`, 2-arg `checkStampText`, `winAnsiBytes` near engine.ts:3096-3128, and the `WIN_ANSI_EXTRA` table near engine.ts:6481-6491); use the engine-core.ts:190-231 versions (call site ≈ engine.ts:3276 becomes `checkStampText(action.text, "stamp", MAX_STAMP_CHARS)` or equivalent). One WinAnsi map remains. Move the orphan JSDoc at engine-core.ts:223 onto `winAnsiBytes`.
S2. Unused imports/locals found by `tsc --noUnusedLocals`: engine.ts `fieldPath` (≈2412), imports `winAnsiByte`/`winAnsiBytes` (33-34, unless S1 makes them used), `MAX_SIG_CONTENTS_BYTES` import (40; keep the `export … from`), content-edit.ts imports `num`, `str` (16-17), renderer.ts `STAMP_ICONS` import (25), consts `contentApplyButton`/`contentApplyObjectButton` (≈450/474; keep the HTML), sign-prepare.ts `enc` (14), sign/verify.ts import `kids` (2). Also unused exports `editIsAscii` (content-edit.ts ≈1312), types `LabelStyle` (engine.ts ≈153), `ClipCopyReply`/`ClipPasteReply` (≈502-503).
S3. Dead IPC: `confirm-discard` (main.ts ≈683, preload.cts ≈66, renderer.ts type ≈122) and `trust-list` (main.ts ≈905, preload.cts ≈129, renderer.ts type ≈158). `DISCARD_BOX` stays (used at main.ts ≈370, 591).
S4. Trivial duplicates: worker.ts `copyBytes` (≈123) imports from engine-core; renderer.ts `normaliseLine` (≈7420) imports engine.ts's; drop the inner `round2` copy at renderer.ts ≈6933.
S5. Stale comments in preload.cts:23-24 ("duplicate of policy.ts" → renderer/limits.ts; drop "grok owns").
S6. Guard: `"noUnusedLocals": true` in both tsconfigs so this cannot regrow; `npx tsc --noEmit -p tsconfig.json` and `-p tsconfig.preload.json` clean.

Out of scope: the "A1 step 1" re-export block and limit re-exports (engine.ts 46-66, 6605-6607), `geometryBlocked` inherited-/Rotate divergence (separate unit, needs a fixture), package rename.

## TESTS scope

T1. No `pdf4all-*` temp dirs survive a run: every `mkdtemp` in tests/*.test.mjs is removed after its file's tests (`after()` / `t.after()` / `finally` with `rm(dir, { recursive: true, force: true })`), never before the assertions that read it. `scripts/drive.mjs` removes its per-run userData dir (≈:44) on every exit path, including failure; session.mjs likewise if it creates one.
T2. `tests/p5-verify-crypto.test.mjs` ≈192-193: stop writing `fixtures/p0b/.ca/ca.srl` (`-set_serial <n>` instead of `-CAcreateserial`, or `-CAserial` pointing into its temp dir). sha256 of `fixtures/p0b/.ca/ca.srl` before this unit: `eca06ff8eccd42843198490ada7566113dd800325b431cc53d1ad6df7e4fd814` — it must be unchanged after a full run.
T3. New test file `tests/tmp-cleanup.test.mjs`: runs `scripts/drive.mjs` on a tiny drive script with `TMPDIR` set to a fresh empty directory and asserts that directory holds no `pdf4all-*` entry afterwards; plus the T2 invariant (ca.srl sha unchanged after running the openssl step, or a static check that no test passes `-CAcreateserial` without `-CAserial`). It must fail before T1/T2 and pass after. Red-before output: `.quad/shared/p6/acceptance/housekeeping-b-red-before.log` (≥1 `not ok`).

## Acceptance

`zsh .quad/shared/p6/scripts/accept-unit-nogui.sh housekeeping-b '' - - tests/tmp-cleanup.test.mjs 592` (package step only when the human's packaged app is closed): tsc ok, build ok, suite fail 0 / skip 1, pass ≥ 592 + new, red-before present, all legacy drives green, fixtures unchanged (55 lines), no `--drive=` stragglers. Plus: `find $TMPDIR -maxdepth 1 -name 'pdf4all-*'` count after the suite ≤ count before + nothing new from the suite (the gate's own batch staging in /tmp excepted); ca.srl sha unchanged.
