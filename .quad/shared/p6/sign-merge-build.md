# sign-merge — build note

Files (new): tests/sign-merge.test.mjs (5 tests) · tests/drive/sign-merge.jsonl (18 steps, 0 clicks; needs ${OUT}/signature.png staged) · acceptance/sign-merge-red-before.log (5/5 not ok) · .quad/scratch/sign-merge/probe.mjs (mupdf: setRect on an image stamp keeps its image appearance).
Files (edited): src/sign/drawn.ts (D2 `ImageSignature`, `SavedSignature`, `checkSavedSignature`, 8 MiB read cap) · src/renderer/engine.ts (D3 annot-add Stamp signature, annot-move on SIGNATURE_SUBJECT stamps; request-type doc comments) · src/renderer/renderer.ts (D1 pad image, D4 placement/targets, D5 old dialog removed, D6 drive/STATE) · src/renderer/index.html (#sign-dialog removed, #sigpad-image, #sign-add title, Help Fill-forms cell) · src/main.ts (menu: one "Place Signature" Shift+CmdOrCtrl+J → sign-place; pick-images headless guard; comment) · docs/agent.md · .quad/shared/p3/design-agent-drive.md (## sign-merge) · tests/p2-shell.test.mjs · tests/draw-signature.test.mjs · .quad/shared/p6/checks/shot-draw-signature.mjs (D image route, written NOT run).
Not touched: preload.cts (the `signatureSet(value: unknown)` type already fits), styles.css (the old dialog had no rules; the new button needs none), safe-write/policy/trust, scripts/*, fixtures, p2-annots2, p1-save.

Numbers pinned: image 1..4 MiB bytes, PNG 89 50 4E 47 0D 0A 1A 0A or JPEG FF D8 FF, canonical base64, width/height integers 1..16384; store read cap 8 MiB; placement fit inside 200×60 pt (img-rgb 300×200 → 90×60), width clamp [24, page], noop 0.01; pad image fitted, aspect kept.

Existing test values flipped (in place, no line added/removed):
1. tests/draw-signature.test.mjs :239 — `fill.signature` deep-equal gains `kind: 'drawn'` (D6 adds the key; the pin was a full deepEqual).
2. tests/draw-signature.test.mjs :367 (review drive) — "sign-draw: give strokes or clear, not both" → "sign-draw: give one of strokes, image or clear" (D6).
3. tests/p2-shell.test.mjs :18 — REQUIRED_IDS += 'sigpad-image' (sign-add stays).

Deviations:
1. src/main.ts `pick-images`: under --smoke/--drive with no `images` pre-answer it now answers `cancelled` (as pick-folder) instead of opening a native sheet. Needed so `sign-draw {image:true}` without `images` is an `error` ("sign-draw: no image was given") and not a 60 s step timeout. --edit=shapes always passes --images, so the smoke is unchanged.
2. OP_MODE.sign is unchanged ("annotate"); instead the edit-request mode lookup sends a bare `sign` (no contents) to "forms". A certificate `sign` (contents) keeps its mode; sign*.jsonl green.
3. annot-add refusal sentence for the Stamp cases: "annot-add: a Stamp signature takes an image and no icon or sign name" (also covers `signature:false`/non-boolean on a Stamp). Other types keep "annot-add: signature applies to Ink only and must be true or false" verbatim, as D3 says, though Stamp now also takes it.
4. Store: an extra key beside kind "image" (e.g. `strokes`, `name`) is ignored and dropped (pinned in test b). A `kind` other than absent/"image" → null.
5. Drive refusal texts: "sign-draw: " + the pad's message — "A signature uses one image; pick a single file.", "That image is larger than this app will read.", "That image could not be read.", "That image is not a PNG or JPEG this app can read." (magic check, then createImageBitmap); cancelled/no pre-answer → "sign-draw: no image was given".
6. "#sign-dialog gone / #sign-add present" is asserted statically in (d), not by a drive step (no DOM probe exists); the drive's `mode annotate` step stays.
7. STATE `fill.signatures[].kind` is "image" for every SIGNATURE_SUBJECT Stamp, including old Sign… appearances in existing documents (they are now targets too, per D3/D4).
8. Pad heading stays "Draw your signature" (contract names no new text).

Results: tsc (both) clean · tests/sign-merge.test.mjs 5/5 (with draw-signature.test.mjs 16/16) · sign-merge.jsonl ×3: 18 steps, 0 red, exit 0 · regression drives draw-signature, draw-signature-rotated, sign, sign-field, dialogs, debt1, direct-edit: exit 0 · npm test 642 tests / 641 pass / 0 fail / 1 skipped (baseline 637/636/0/1 + 5) · no --drive= stragglers · lock released.
Not done / unverified: GUI (real "Use an image…" sheet, the pad's painted image, real click/handles on an image signature, Cmd/Ctrl+Shift+J through the real menu) — shot script appended, not run. Fixture hashes not re-run by me (nothing under fixtures/ written; acceptance script checks them).
