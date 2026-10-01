# Bugfix 1 — build note (implementer, 2026-09-29)

**B1 root cause (probe `.quad/scratch/bugfix1/probe-b1.mjs`, pre-fix build):** as expected. mupdf's simple-font resource cache (keyed by font digest) outlives the journal undo: the first `text-add` makes `14 0 R`; after `undo` the second `addSimpleFont` returns `14 0 R` again, which no longer exists ("cannot find object in xref (14 0 R)", resolves to null). Same for stamp → undo → stamp (`/PDF4AllF` → `14 0 R`, null). Good dictionary (identical for every Base-14 name, both `addSimpleFont(font)` and `(font,"Latin")`): `<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>`. Fix: `addBase14Font(pdf, name)` in `engine.ts` (next to `stampPage`), used by both call sites; if the returned object's `/Type` is not the name `Font`, `pdf.addObject` of that exact dictionary. After the fix the probe shows `16 0 R` / `21 0 R` live dicts and the scan says `candidate`. mupdf still prints the "cannot find object" warnings to stderr on the fallback path (cosmetic).

**B6 was real** (probe `.quad/scratch/bugfix1/probe-b6.mjs`): with `/Rotate 90` only on `/Pages`, both images and the form scanned `candidate` with a `placement`; with a page-own `/Rotate 90` they are `unsupported-transform`. Fixed by reading `/Rotate` through `getInheritable` in `scanEditTargets`. Text runs stay `candidate`/`moveable` on a rotated page in both cases (pre-existing, not in scope).

**Changed paths**
- `src/renderer/engine.ts`: `addBase14Font` helper; stamp site and `text-add` site call it (D1).
- `src/renderer/content-edit.ts`: `editAttachGrowthLimits` app-written (`/^PDF4AllF\d*$/`) runs → `obstacleLimit ?? page.getBounds()[2]` (D2); inherited `/Rotate` (D6).
- `src/main.ts`: `untitledPaths` set filled by `newBlank`; `save-file` in-place on an untitled path takes the copy route (dialog default `Untitled.pdf`; smoke/drive: `saveToArg`, else `cancelled`) and on success rebinds `openPaths` (new file id, same url, new name) and drops the set entry (D3); drive `save-in-place` sets/clears `saveToArg = pathOf(args.path)` (D3b).
- `src/renderer/renderer.ts`: `saveInPlace` adopts `result.name` as `doc.name` + title; `undoEdit`/`redoEdit` `clearError()` after the canUndo/canRedo check (D5). Also: the `cancelled` branch of `saveInPlace` now records `lastSave = {ok:false, reason:"cancelled"}` — without it STATE `save` never showed the D3b cancel (the old early `return` skipped `lastSave`). Same shape as every other failed save; no new key.
- `scripts/pdf4all-mcp.mjs`: relative `pdf4all_save` → `mkdir -p dirname` after the session starts; a relative path whose normalised first segment is `..` is refused before any step (D4); tool description updated.
- `docs/agent.md` (pdf4all_save line, Create note on `save-in-place` `path`, escape refusal), `src/renderer/index.html` :595 (sentence inside the existing `<li>`), `.quad/shared/p3/design-agent-drive.md` ("## Bugfix 1").
- New: `tests/bugfix1.test.mjs` (5 tests), `tests/drive/bugfix1.jsonl` (11 steps, 0 clicks), probes under `.quad/scratch/bugfix1/`, `.quad/shared/p6/acceptance/bugfix1-red-before.log`.

**Flipped pins: none.** Grepped every `tests/drive/*.jsonl` undo/redo step and every test for a non-empty `error` pin after undo/redo (compare.jsonl step 8 pins no `error`) and for an `overflow` pin on an app-written run (text-add-grow, direct-edit9, p6-text8: none). `tests/p2-shell.test.mjs` does not quote the old Help sentence. No drive script lines changed.

**Numbers (all run under the lock after a fresh build):**
- red-before (pre-fix build): `# tests 5 / # pass 0 / # fail 5` — B1 (a), B2 (b), B6 (c), B4 (d), B3 + B5 (e: 6 red steps = the 3 save steps + undo/redo/state error).
- new file after fix: 5/5 pass. Affected set (p6-text8, p6-drive9, p6-bates, p2-decorate, p2-shell, agent-session, drive-guard, p3-drive, p6-compare): 74/74 pass.
- full `npm test`: `# tests 600 / # pass 599 / # fail 0 / # skipped 1` (594 + 5); no flake hit.
- `node scripts/drive.mjs tests/drive/bugfix1.jsonl` ×3: 11 steps, 0 red, exit 0 each.
- `shasum -c fixtures.sha256`: 9 OK; hash-after = `bugfix1-hash-before.txt` (55 lines, empty diff).

**Not done / unverified:** GUI ⌘S on Untitled (the real save sheet, defaultPath `Untitled.pdf`) was not run (human may be at the machine) — only the drive/smoke half is proven. A cancelled GUI dialog path is reasoned, not run. Temp `pdf4all-new-*` folders are still left behind (out of scope). The acceptance script `accept-unit-nogui.sh` and `npm run package` were not run by me.
