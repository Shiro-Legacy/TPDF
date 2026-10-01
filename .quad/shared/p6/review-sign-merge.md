# sign-merge — review (single reviewer, 2026-09-30)

Diff read in full: `.quad/backup/pre-sign-merge-20260930.tgz` extracted to `.quad/scratch/sign-merge/pre/`; `diff -ruN` of `src`, `tests`, `docs/agent.md`, `design-agent-drive.md` → `.quad/scratch/sign-merge/rev-{src,tests,docs}.diff` (581 + 359 + 43 lines). My own edits are checked against `.quad/backup/pre-sign-merge-review-20260930.tgz` (`.quad/scratch/sign-merge/prerev/`). Probes: `review-probe.mjs` (engine: reopen-then-move, foreign stamps, page box, oversize/zero-height, undo/redo, 9-byte PNG → `rev-probe.out`), `review-bomb.mjs` (hostile PNGs through `sign-draw {image:true}`, timed via `--session` → `rev-bomb.out`), `review-exif.mjs` + `review-exif-run.mjs` + `review-exif.jsonl` (EXIF-rotated JPEG → `rev-exif.out`), all under `.quad/scratch/sign-merge/`. Red-before logs: `review-red-before.log` (F1 attempt + F3), `review-red-before-2.log` (F2).

## Findings, by severity

**F1 — Medium — LEFT (needs a decision). A phone JPEG with EXIF orientation is placed squashed and unrotated.** `src/renderer/renderer.ts:4121` measures the image with `createImageBitmap`, which applies the EXIF Orientation tag; the engine (`decodeStampImage` → `setStampImage`) draws the stored pixels and ignores it. For Orientation 6 (a portrait-held phone, the common case for "photo of my signature on paper") a 300×100 stored JPEG is saved as `width 100, height 300` and placed into a portrait rect, so the picture is squeezed 9× sideways and not rotated, while the pad preview showed it upright. Proof (`rev-exif.out`): mupdf `getWidth/Height` 300×100; driven `sign-place {300,400}` → `fill.signatures[0].rect [290,370,310,430]` (20×60); rendering that Stamp: the stored image's black left third fills the rect's left third (dark left column 1.00, right 0.00) — unrotated, squashed. I tried `createImageBitmap(blob, {imageOrientation:"none"})`: Electron 44.4.0 still honours EXIF with it (my test stayed red: aspect 0.333 vs 3), so I reverted it and removed my test steps (no red test left in the tree). Options, each a behaviour decision: (a) refuse a JPEG whose EXIF Orientation ≠ 1 in `padUseImage` (walk APP1 → IFD0 tag 0x0112; ~20 lines, a new refusal sentence); (b) take width/height from the file header (PNG IHDR / JPEG SOFn) and paint the pad preview unrotated, so the pad shows what the PDF gets (also gives a pre-decode size bound, see F5); (c) rotate in the engine (not in this contract). PNGs are unaffected.

**F2 — Low — FIXED. `annot-move` left a signature Stamp larger than the page hanging off the page.** D3: "rect clamped inside the page box". Without `width`, `newW = w0`; for a foreign `/Subj (Signature appearance)` Stamp of 750×950 on a 612×792 page a move to (10,10) gave `[0,0,750,950]` (`rev-probe.out` §4). Such a stamp is also painted as a Fill-forms target covering the page. Fix `src/renderer/engine.ts:5083-5084`: for a signature Stamp a move without `width` shrinks it to fit the page, aspect kept (journal "resize signature" then, one op, undoable). Drawn Inks keep draw-signature's behaviour (its review's L4). Test: `tests/sign-merge.test.mjs` "(review) annot-move on a foreign signature Stamp larger than the page…" — red before (`review-red-before-2.log`: `inside the page: 0,0,750,950`), green after.

**F3 — Low — FIXED (brief item). The pad's heading said "Draw your signature" though it now also takes an image.** `src/renderer/index.html:641` → "Your signature" (id `sigpad-heading` kept). Nothing pinned the old text (grep of `src`, `tests`, `docs`, `scripts`, `.quad/shared/p6/checks`, `design-agent-drive.md`). Test: static (d) asserts `<h2 id="sigpad-heading">Your signature</h2>` — red before (`review-red-before.log`), green after. Not changed: the refusal "Draw your signature first" (`renderer.ts:4190`, only reachable if the saved signature vanishes between press and placement).

**F4 — Low — LEFT. "Use an image…" is not cancellable and can land late.** `padUseImage` (`renderer.ts:4111-4128`) awaits the picker and then `createImageBitmap` (424 ms measured for a 16000² PNG). A stroke drawn, Clear pressed or the pad closed during the decode is then overridden: the stroke is wiped by `padStrokes = []`, or the bitmap is set on a closed pad. Not a data leak: a reopened pad runs `clearPad()`, and `sign-draw {strokes}` drops it, so a stale image is never saved. A second click while one is in flight runs two pickers. Fix: a generation counter checked after each `await`. By reading only; not run in the GUI.

**F5 — Low — LEFT. Decoded bitmap held after a driven save; decode bound is after decoding.** A `sign-draw {image:true}` saves with the pad never open, so the dialog's `close` handler (`renderer.ts:4326`) never runs and `padImage.bitmap` (up to 16384²×4 B = 1 GiB) stays allocated until the next pad use or stroke. Fix: `dropPadImage()` after a successful `saveDrawnSignature`; I could not pin it through STATE, so I left it. The 1..16384 check runs after `createImageBitmap`; measured (`rev-bomb.out`): a 30000×30000 1-bit PNG (151 KB) is refused by Chromium in 4 ms ("…not a PNG or JPEG this app can read."); a 16000×16000 one (51 KB) is accepted in 424 ms and placed in 1.43 s; no crash, no step timeout. F1 option (b) would bound it before decoding.

**F6 — Low — LEFT (pre-existing pattern). `#sign-add`'s new title is erased once a document is open.** `renderer.ts:2205` `signButton.title = permissionTitle(canAnnotate)` sets `""` when annotating is allowed, so D5's title shows only while the button is disabled. Same for every Annotate button since before this unit.

**F7 — Low — LEFT (pre-existing class). Cmd/Ctrl+Shift+J over an open dialog.** Electron menu accelerators fire under a modal `<dialog>`: with the pad open and nothing saved, `pressSign` → `openSigpad(true)` → `clearPad()` wipes the drawing or image in progress; over another modal it switches to Fill forms beneath it and stacks the pad. The removed accelerator opened `#sign-dialog` the same way. Accelerator registered once: one `Shift+CmdOrCtrl+J` in `src/main.ts` (1359), no renderer keydown for J.

**F8 — Note. The `images` pre-answer is sticky across drive lines** (`src/main.ts:1911`, unchanged): after any line with `images`, a later `sign-draw {image:true}` without `images` reuses that file. "sign-draw: no image was given" (deviation 1's purpose) appears only when no earlier line set it. `docs/agent.md` ("uses the line's `images` pre-answer") reads as per-line.

**F9 — Note.** Stored `width`/`height` are not checked against the image (contract: "as the renderer measured them"). A hand-edited store claiming 16384×1 places a 200×0.012 pt sliver. No crash. Refusals hold. A 9-byte magic-only "PNG" passes the store and is refused by the engine before `mutate` (`dirty false`, `revision 0`; `rev-probe.out` §5).

**F10 — Note.** A bare `sign` / `sign-place` with no document switches to Fill forms and returns `error ""` (draw-signature review L2, unchanged).

Checked and found right:
- **Store bounds.** `data.length` is bounded before `Buffer.from` (`drawn.ts:47`). The canonical-base64 round trip and magic check work, and extra keys are dropped. A value with no `kind` goes to the unchanged `checkDrawnSignature`. A `kind` other than "image" → null.
- **Store hardening.** JSON and structured clone carry no prototype; a prototype-inherited `kind` gives a fresh object. Read cap: `statSync` size ≤ 8 MiB on a regular file before reading. The file name is never stored, logged or put in STATE (the `pick-images` name is ignored, and the engine names the image "signature"). IPC handlers are `isTrustedOpen`-guarded and validate with `checkSavedSignature`.
- **Renderer base64.** Converted in 32 KiB chunks.
- **Pad image lifecycle.** The image is dropped by a stroke (`padBegin`), by Clear and by `close`. Save is enabled with an image and disabled after Clear. A missing or throwing `pickImages` gives a refusal, not a throw.
- **Engine `annot-add`.** The old `sign:{name}` path is byte-identical: the new checks only fire with `signature` present, and the label is the same expression. Refusals come before `mutate`.
- **Engine `annot-move`.** Clamping works at all four edges. Page boxes with a non-zero origin are normalised to (0,0) by mupdf (probed). The listed rect equals `annotRect`, so a move to the listed top-left is `noop`. Undo and redo restore the rect.
- **Reopened stamps.** An old-dialog appearance saved, reopened in a new session, then moved and resized keeps its image XObject. An icon Stamp carrying the signature `/Subj` keeps its appearance when moved. Zero-height stamps move without error. Icon stamps keep the verbatim refusal.
- **Old dialog removed.** No leftover id, CSS, Help, doc, test or listener. `noUnusedLocals` is clean. `addSignature`/`signatureBox` are reachable only from `smokeShapes`, which always has `--images`.
- **Certificate `sign`.** A contents-carrying `sign` keeps `OP_MODE.sign` "annotate".
- **`pick-images` callers.** New-from-images, insert/replace, the shapes smoke and the GUI shot scripts all pass `--images`/`images`. A non-smoke run takes the unchanged `showOpenDialog` branch.
- **Placement.** It disarms on a mode change (`cancelPendingTools`) and on document reset. Targets are painted only on unrotated pages in Fill forms.
- **STATE and reports.** `kind` keys are new, no existing key changed shape, `sign-draw` contents are `[redacted]`, and there is no base64 or file name in STATE or the report (test c).
- **Tests and fixtures.**
  - Pre-existing drive scripts are byte-identical (48 files, `cmp`).
  - Fixtures are byte-identical to the backup; `fixtures/.DS_Store` dates from 2026-09-29 and is outside the 56-line snapshot.
  - The tests use temp dirs only.
  - The Electron test is one `test()` with `{ skip: CI_NO_ELECTRON, timeout: 180_000 }`.
- **D7.** Every D7 item is asserted. (b)'s "over 4 MiB" case exercises the length bound, not the decoded-size check; both refuse.

## Deviations
1. `pick-images` cancelled under smoke/drive without a pre-answer: **accept**. Every headless caller passes `--images`/`images` (before, a headless run would have hung on a native sheet). GUI runs take the unchanged branch. Caveat: F8.
2. Bare `sign` switched to Fill forms in the edit-request listener and `OP_MODE.sign` unchanged: **accept**. The certificate `sign` mode is unchanged, and `sign*.jsonl` stay green in `npm test`.
3. New Stamp refusal sentence (also for `signature:false` on a Stamp): **accept**. The other types keep the old sentence verbatim as D3 says, though "Ink only" is now inexact.
4. Extra keys beside `kind:"image"` dropped: **accept** (pinned).
5. Drive refusal texts: **accept**.
6. Dialog-gone / `#sign-add`-present asserted statically in (d): **accept** (no DOM probe op exists).
7. `kind:"image"` for every signature-subject Stamp, old appearances included: **accept**. It is consistent with D3/D4.
8. Pad heading kept "Draw your signature": **reject**. The brief asks for "Your signature" (F3, fixed).

Flipped values: (1) `draw-signature.test.mjs:239` gains `kind:'drawn'`: **accept** (D6 key; edited in place). (2) `:367` one-of sentence: **accept** (D6 names it). (3) `p2-shell.test.mjs:18` `+ 'sigpad-image'`: **accept** (D8; same line).

## Results
tsc (`tsconfig.json`, `tsconfig.preload.json`) clean · `tests/sign-merge.test.mjs` 6/6 (was 5; +1 review test, and (d) gained the heading assertion) · `npm test` 643 tests / 642 pass / 0 fail / 1 skipped (was 642/641/0/1) · `tests/drive/sign-merge.jsonl` ×3 (`review-drive3.mjs`, `${OUT}/signature.png` staged like the test): exit 0, 18 steps, 0 red each · no `--drive=` stragglers · lock released. Logs: `.quad/scratch/sign-merge/review-{after-2,npmtest,drive3}.log`. Files I changed: `src/renderer/engine.ts` (5083-5084), `src/renderer/index.html` (641), `tests/sign-merge.test.mjs` (+1 test, +2 lines in d). `tests/drive/sign-merge.jsonl` is byte-identical to the pre-review backup (F1 steps added and removed).

## Not checked
- **No GUI.** Nothing was run in the real GUI (the human is at the machine): the native "Use an image…" sheet, the pad's painted image and dark theme, the pad's focus order, real mouse drags or handles on an image signature, Cmd/Ctrl+Shift+J through the real menu, or the F4 race.
- **Stamp rendering.** mupdf rendering cost of a 16384² stamp on every page render: I measured only one placement (1.43 s).
- **Not run:** packaging, the shot script, and the acceptance script's fixture-hash comparison (no fixture was written).
