# Acceptance — sign-merge (2026-09-30, fable51)

Order: human in chat — "merge the old Sign… dialog into the new pad". Contract `.quad/shared/p6/contract-sign-merge.md` (D1–D8 by fable51, §4 amendments), build note `sign-merge-build.md`, review `review-sign-merge.md`. One Opus 5.5 implementer, one Opus 5.5 reviewer.

| Check | Outcome |
|---|---|
| gate `accept-unit-nogui.sh sign-merge sign-merge - - tests/sign-merge.test.mjs 636` (`sign-merge-gate.log`) | `== result: fail=0` |
| tsc (`tsconfig.json`, `tsconfig.preload.json`) | clean |
| `npm test` (in the gate log) | 644 tests / 643 pass / 0 fail / 1 skip (636 + 7 new) |
| red-before logs | implementer `sign-merge-red-before.log` 5 `not ok`; F1 `sign-merge-f1-red-before.log` 1 `not ok` ("no EXIF block in the saved PDF") |
| new drive ×3 | `sign-merge` 18 steps: exit 0, 0 red, every run (the gate's `stage()` now stages `signature.png`); `sign-merge-exif` (6 steps) runs inside `tests/sign-merge.test.mjs` |
| regression drives | all exit 0, 0 red |
| secrets in reports | none; `sign-draw` contents `[redacted]`, no image base64 or file name in STATE or reports |
| `fixtures.sha256` | 9 OK |
| fixture snapshot vs `checks/sign-merge-hash-before.txt` | identical; nothing new under `fixtures/` |
| stray Electron | none |
| package | ok, repackaged once more after the gate for the `docs/agent.md` sentence (`tests/agent-session.test.mjs` 6/6 rerun; bundled `agent.md` = repo copy); installed with `ditto` to `/Applications/Tandem PDF.app` (the app was not running), `lsregister -f` |
| `checks/packaged-sign-check.mjs` on the installed app (`sign-merge-packaged-mcp.log`) | an agent session starts with no saved signature; a drawing places as an Ink (`kind:"drawn"`), then `sign-draw {image:true}` replaces it (`kind:"image"`) and places a Stamp at [255,370,345,430] (300×200 px → 90×60 pt); save validated; the file has the Ink ("Drawn signature") and the Stamp ("Signature appearance"), neither with `/T` or `/Contents`; nothing written under `~/Library/Application Support/Tandem PDF/` |
| source diff vs `pre-sign-merge-20260930.tgz` | read by fable51: `src/main.ts`, `src/preload.cts` (unchanged), `src/sign/drawn.ts`, `src/renderer/engine.ts` |

**Review:** F2 fixed (a signature Stamp larger than the page is shrunk to fit on move), F3 fixed (pad heading "Your signature"), each red before. **F1 (Medium) fixed by fable51 at acceptance:** a phone JPEG with an EXIF orientation tag was measured upright by Chromium but drawn unrotated by mupdf (placed squashed and sideways). `padUseImage` now stores the app's own PNG of the decoded pixels (`OffscreenCanvas`, long side ≤ 1000 px) instead of the picked file: upright, and none of the file's EXIF/GPS reaches the store or a PDF. This also bounds the bitmap the pad holds (most of F5). Contract §4 records it.

**D-decisions (fable51, reported to the human):** the old dialog's typed Name line is dropped (an image signature has no `/Contents`); appearances added by the old dialog in existing documents become movable/resizable/deletable in Fill forms; the image background is not made transparent; the file name is never stored; the picture is kept as a re-encoded PNG capped at 1000 px.

Left (progress.md §10 "sign-merge leftovers"): F4 (late image decode overrides a stroke/Clear/close), F6 (`#sign-add` title erased when annotating is allowed), F7 (Cmd/Ctrl+Shift+J over an open dialog), F8–F10 notes. Not done: every GUI proof — the real file sheet, the image in the pad, click to place, relaunch persistence (`checks/shot-draw-signature.mjs` written, never run). Backups: `.quad/backup/pre-sign-merge-20260930.tgz`, `pre-sign-merge-review-20260930.tgz`.
