# draw-signature — build note

Files (new): src/sign/drawn.ts · tests/draw-signature.test.mjs (6 tests) · tests/drive/draw-signature.jsonl (25 steps, 1 click) · tests/drive/draw-signature-rotated.jsonl (9 steps, 0 clicks) · .quad/shared/p6/checks/shot-draw-signature.mjs (written, NOT run) · acceptance/draw-signature-red-before.log (6/6 not ok) · .quad/scratch/draw-signature/probe.mjs.
Files (edited): src/renderer/engine.ts (D1 `signature`, `DRAWN_SIGNATURE_SUBJECT`, `signatureLineWidth`; D2 `annot-move` width/ink; request types) · src/main.ts (store, `signature-get`/`signature-set`, Edit ▸ "Place Signature") · src/preload.cts (`signatureGet`/`signatureSet`) · src/renderer/renderer.ts (D4–D8) · src/renderer/index.html (#fill-sign, #fill-sign-draw, #sigpad-dialog; Help text in the existing Fill cell) · src/renderer/styles.css · docs/agent.md (Fill recipe + ops list) · .quad/shared/p3/design-agent-drive.md (## draw-signature) · tests/p2-shell.test.mjs (REQUIRED_IDS += fill-sign, fill-sign-draw, sigpad-dialog/-heading/-canvas/-status/-clear/-remove/-save/-cancel).
Not touched: engine-core.ts, worker.ts (no request-type change needed), scripts/*, tsconfigs, safe-write/policy/trust, mutate/save paths.

Numbers pinned: line width clamp(w×0.0075, 0.6, 4) (200 pt → 1.5, 100 → 0.75); resize min width 24 pt; noop/resize threshold 0.01; placement box 200×60 pt; pad 520×180 CSS px, ≥8 px wide, ≤64 strokes, 2..2048 points, 0.5 px skip; store width [8,4096], height [0,4096], ±0.01, 2 decimals, 4 MiB read cap, mode 0600; +/- = ±10 % width; arrows 1 pt (Shift 10).
Existing test values flipped: none. (REQUIRED_IDS only grew; no drive line inserted or deleted; no pinned value changed.)

Deviations:
1. Rotated script: fixtures/rotated.pdf has no /Rotate (rotated text only, page rotation 0), so the script turns its ${OUT} copy with `rotate-right`, saves it as turned.pdf, reopens it, then `sign-draw` / `sign-place` → the rotated sentence, dirty false (the fill-anywhere pattern).
2. Gesture `min`: contract says `min:24`. gestureRect's corner lock keeps BOTH sides ≥ min, so a 200×40 signature could never shrink below 120 wide. The target passes min = 24 × min(1, h/w), which keeps the width ≥ 24 without forcing a short signature taller; the engine still clamps width to 24.
3. Shift keeps the aspect via a new optional `RectGesture.keepAspect` (startRectGesture: `lockAspect: (keepAspect || lockAspect && !shift) && !within`); other callers are unchanged.
4. A side handle (n/s) that only changes the height commits width = w0 × newH/h0 (the engine keeps the aspect from the new top-left); e/w commit the new width. The drag ghost of a side handle stretches until the commit.
5. Refusal order in `sign-place` is the contract's (required → page range → no drawing → rotated → permission); a `width` that is not a finite number answers "sign-place: width must be a number" (not listed in the contract). The GUI-only refusals read "Draw your signature first" / "This document does not permit annotations".
6. `sign-draw` bad contents: "sign-draw: strokes must be a list of strokes of [x, y] points, or clear must be true"; too-small/empty drawing: "sign-draw: the drawing needs a stroke and must be at least 8 px wide"; >64 strokes: "sign-draw: a signature can have at most 64 strokes".
7. `annot-add` refusal text for a misplaced/non-boolean `signature`: "annot-add: signature applies to Ink only and must be true or false". `annot-move` adds "width must be a finite number" and "this signature is too narrow to resize".
8. The renderer's STATE slot is `fillSignLast` (an existing `signLast` belongs to certificate signing); STATE key is `fill.lastSignature` as specified, built through the existing `fillOp()` helper.

Ceiling: `sign-draw` JSON over 4096 characters is dropped by preload (MAX_NOTE_CHARS) — documented in docs/agent.md and the design note.
Not done / unverified: GUI proof (real pointer drawing, real click, handle drag, "(filled)" sheet, relaunch persistence) — shot script written, not run. The on-disk store is verified only by the unit test in a temp dir; the real userData path was never written.
Results: tsc (both) clean · tests/draw-signature.test.mjs 6/6 · each new drive ×3 green (exit 0, 0 red) · npm test 632 tests / 631 pass / 0 fail / 1 skipped (baseline 626/625/0/1 + 6) · fixture hashes 56/56 OK.
