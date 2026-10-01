# housekeeping-b — SRC build note (2026-09-25)

Pure housekeeping, no behaviour change. Diff vs `.quad/backup/pre-housekeeping-b-20260925.tgz` (src + tsconfigs): 115 lines removed, 12 added (net -103). Files: main.ts, preload.cts, renderer/{content-edit,engine-core,engine,renderer,sign-prepare,worker}.ts, sign/verify.ts, tsconfig.json, tsconfig.preload.json.

## S1 WinAnsi
- Equivalence checked before the swap: `diff` of the two `WIN_ANSI_EXTRA` bodies is empty (27 identical entries). `winAnsiByte` and `winAnsiBytes` have the same logic. `checkStampText` has the same error strings; the only difference is `max`, which is now passed as `MAX_STAMP_CHARS` (200). Observable behaviour is identical.
- engine.ts: deleted the in-closure `winAnsiByte`, 2-arg `checkStampText`, and `winAnsiBytes` (were ≈3096-3128), plus the module-level `WIN_ANSI_EXTRA` table and the 10 blank lines around it (were ≈6475-6495). Added `checkStampText` to the engine-core import (engine.ts:35). The call site is now engine.ts:3227 `checkStampText(action.text, "stamp", MAX_STAMP_CHARS)`. The existing `winAnsiByte`/`winAnsiBytes` imports (33-34) are now used, at 3148, 5637 and 5669.
- engine-core.ts: moved the orphan JSDoc "WinAnsi bytes escaped…" from above `BASE14_FONTS` onto `winAnsiBytes`.

## S2 Unused imports, locals and exports
- engine.ts: removed `fieldPath` (≈2412), `MAX_SIG_CONTENTS_BYTES` from the `import` (the `export … from` stays), `type LabelStyle` (the `LABEL_STYLES` const stays), `type ClipCopyReply` and `type ClipPasteReply`.
- content-edit.ts: removed the `num` and `str` imports and `editIsAscii` (≈1312).
- renderer.ts: removed the `STAMP_ICONS` import and the `contentApplyButton`/`contentApplyObjectButton` consts. The HTML is unchanged.
- sign-prepare.ts: removed `enc`. sign/verify.ts: removed the `kids` import.
- Grep of src/, tests/ and scripts/ found no references to any of these.

## S3 Dead IPC
- Grep of the repo (excluding node_modules/out/build/.quad) found no caller of `confirmDiscard`, `trustList`, `'confirm-discard'` or `'trust-list'` in the renderer, tests/*.mjs, tests/drive/*.jsonl or scripts/.
- Removed the handlers in main.ts, the bridges in preload.cts and the optional type members in renderer.ts. `DISCARD_BOX` is still used at main.ts:64/370/591, and `trustStore.list()` is still used by `verify-signatures`. The renderer.ts:117-119 comment was reworded from "declares the seam and never asks twice" to "only reports its dirty state and never asks".
- NOT touched (out of my scope): `docs/gotchas.md:72-73` still lists `trust-list`/`trustList` among the signing channels. fable51 should drop them from the docs.

## S4 Trivial duplicates
- worker.ts: removed the local `copyBytes` and added `import { copyBytes } from "./engine-core.js"` (worker.ts:3). engine-core.js is already in the policy.ts asset map (:36).
- renderer.ts: removed the local `normaliseLine` and now imports it from engine.js (renderer.ts:16). Dropped the inner `round2` in `boxAt`, so it uses the module-level `round2` (renderer.ts ≈6022, same body).

## S5 preload.cts:23-24
- Comments now read "duplicate of renderer/limits.ts; change both together" and "duplicate of renderer/engine.ts; change both together". The "grok owns" notes are gone.

## S6 Guard
- Added `"noUnusedLocals": true` to tsconfig.json:11 and tsconfig.preload.json:7. `noUnusedParameters` is not enabled.

## Verification
- `npx tsc --noEmit -p tsconfig.json`: clean. `npx tsc --noEmit -p tsconfig.preload.json`: clean.
- `npm run build`: ok.
- `node --test tests/p2-decorate.test.mjs tests/p6-bates.test.mjs tests/p6-text8.test.mjs tests/p3-edit.test.mjs tests/p3b-content.test.mjs tests/p5-sign.test.mjs tests/p5-verify.test.mjs tests/p1-worker.test.mjs tests/p2-shell.test.mjs tests/p6-compare.test.mjs`: 125 pass / 0 fail / 0 skip.
- Did not run the full suite or `npm run package`. No Electron process was killed.
