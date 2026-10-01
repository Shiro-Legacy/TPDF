# Debt sweep 1 — RENDERER evidence (2026-09-24, Opus 5.5)

Files: src/renderer/renderer.ts, src/renderer/index.html, tests/p2-shell.test.mjs (one additive test). styles.css, p6-direct.test.mjs untouched.

| Row | Change | Proof / bite |
|---|---|---|
| 1 | announce() clears #nav-status, announceNav() clears #edit-status; the 3 direct navStatusEl writes (reset, bookmark no-dest, blocked bookmark) route through announceNav; STATE announcement = edit || nav || ""; a blocked link follow announces linkLabel() ("Blocked link (ftp:)", same words as the panel row/bookmark) | debt1.jsonl steps 2, 6; reverting the STATE line in built JS fails steps 2,6 |
| 3 | "This page has N form field(s)." (renderer paintHits + index.html initial text) | T's shot-debt1.mjs (#field-count) |
| 13 | .sig-target pointerdown in Forms (movable = editable && edit && !signed, button 0, not a handle) calls fieldGesture(item)(event,"grip") = startRectGesture (3 px threshold, field-props commit); the click whose point is >= 3 px from the press is swallowed; plain click still activates | debt1 steps 9-10; revert fails steps 9-16 |
| 14 | announce("") in the sigfield (and link) draw onEnd; driveArm now announces the same arm prompt as the toolbar button (DRAW_SIGFIELD_PROMPT / DRAW_LINK_PROMPT consts) | debt1 step 13; revert fails step 13 |
| 15 | clearError() on the successful add in addDesignedField, smokeFieldDesign (drive `fields add`) and addSignatureField | debt1 step 16; revert fails step 16 |
| 18 | #save-copy button text "Save As…" (id kept); "source-changed" message now says "Use Save As instead." | p2-shell 'debt1 row 18' test; fails with the old text (shown by reverting index.html once) |
| 19 | resetEditState() (every open, via resetDocument) resets contentEdited/contentRefusals | debt1 step 21; revert fails step 21 |
| 20 | refreshPageIds catch sets pageIds = null | code-only; tsc + drives green |
| 23 | renderer.ts:280/323 comments cite "contract-p3-unitN.md §3, archived in .quad/backup/history-20260924.tgz" | grep |

Bite method: .quad/scratch/bite.sh — build, debt1 green, then per row a perl revert in build/renderer/renderer.js, rerun debt1, restore.

Runs (under .quad/run.lock): npx tsc --noEmit clean; npm run build; node --test tests/p6-direct.test.mjs tests/p2-shell.test.mjs tests/p3-drive.test.mjs 30/30 pass;
drive.mjs exit 0: debt1, sign-field, direct-edit3, direct-edit4, direct-edit5, links-bookmarks, field-design. fixtures.sha256 all OK. No straggler Electron.

Not done: contentReplaced / contentLastAdd / contentLastShift also survive an open (not in row 19's scope).
