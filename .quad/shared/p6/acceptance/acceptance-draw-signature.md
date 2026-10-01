# Acceptance — draw-signature (2026-09-30, fable51)

Order: human in chat — "I want to be able to just draw a signature and use that as my signature as a feature". Contract `.quad/shared/p6/contract-draw-signature.md` (D1–D10 by fable51), build note `draw-signature-build.md`, review `review-draw-signature.md`. One Opus 5.5 implementer, one Opus 5.5 reviewer.

| Check | Outcome |
|---|---|
| gate `accept-unit-nogui.sh draw-signature draw-signature,draw-signature-rotated - - tests/draw-signature.test.mjs 625` (`draw-signature-gate.log`) | `== result: fail=0` |
| tsc (`tsconfig.json`, `tsconfig.preload.json`) | clean |
| `npm test` (`draw-signature-suite.log`) | 637 tests / 636 pass / 0 fail / 1 skip (625 + 11 new) |
| red-before log | 6 `not ok` |
| new drives ×3 | `draw-signature` 25 steps, `draw-signature-rotated` 9 steps: exit 0, 0 red, every run |
| regression drives | all exit 0, 0 red |
| secrets in reports | none; `sign-draw` contents are `[redacted]` in reports |
| `fixtures.sha256` | 9 OK |
| fixture snapshot vs `checks/draw-signature-hash-before.txt` (56 lines) | identical |
| stray Electron | none |
| package | ok; installed with `ditto` to `/Applications/Tandem PDF.app` (the app was not running), `lsregister -f` |
| `checks/packaged-sign-check.mjs` on the installed app (`draw-signature-packaged-mcp.log`) | an agent session starts with no saved signature; `sign-draw` 2 strokes / 202 points; `sign-place` rect [200,634.92,400,665.08]; save validated; the file has one black Ink, subject "Drawn signature", no `/T`; nothing written under `~/Library/Application Support/Tandem PDF/` |
| source diff vs `pre-draw-signature-20260930.tgz` | read by fable51: `src/main.ts`, `src/preload.cts`, `src/renderer/engine.ts`, `src/sign/drawn.ts` |

**Review (5 fixed, each red before):** a `sign-draw` over 4096 characters killed the session (now up to 4 MiB for that one command, larger → `error`); stroke points were in the drive report (now `[redacted]`); `clear` + `strokes` together silently removed the signature (now refused); a page under 200×60 pt squashed the signature; the store kept a stale temp's 0644 mode, left a temp behind on failure and blocked on a FIFO.

**Contract amendments at acceptance (fable51, after the review's open finding that mupdf's Ink appearance is quadratic in the point count — 1.07 s at 4096 points, 16.9 s at 16384):** D3/D4 — a drawing holds at most **4096 points in all** (store check and pad; `sign-draw` over it → `error`, nothing saved); the pad's skip distance is **2 px** (was 0.5), so a real signature stays far below the cap; a tap (1-point stroke) is **kept as a dot** (two points 0.5 px apart; mupdf draws Ink with round caps — probe `.quad/scratch/draw-signature/dot-probe.mjs`) instead of being dropped. The 8 build-note deviations and the reviewer's ninth (`sign-draw` never opens the pad) are accepted.

Not done: every GUI proof (real pointer drawing in the pad, click to place, handle drag, the "(filled)" save sheet, the signature surviving a relaunch, the pad in dark theme) — `checks/shot-draw-signature.mjs` is written, never run; the real userData file was never written by a test (the store is tested in a temp dir). Backups: `.quad/backup/pre-draw-signature-20260930.tgz`, `pre-draw-signature-review-20260930.tgz`.
