# fill-anywhere build note (contract-fill-anywhere.md)

**Files changed:** `src/renderer/engine.ts` (D1 `fit` on `annot-add`/`annot-set`, `freeTextFit` using `editAdvance` + the base-14 Font map, `checkFreeTextEncoding` using `winAnsiByte`; D2 `annot-move` + `AnnotMoveRequest`), `src/renderer/engine-core.ts` (`OP_PERMISSION["annot-move"]="annotate"`), `src/renderer/renderer.ts` (D3–D7), `src/main.ts` (`save-file` `filled`, `asCopy`, "(filled).pdf" dialog default, rebind), `src/preload.cts` (option type), `src/renderer/index.html` (`#fill-textbox-add`, Help "Direct editing" cell), `src/renderer/styles.css` (`.textbox-target`), `docs/agent.md` (Fill recipe, `fill-text` in Ops), `.quad/shared/p3/design-agent-drive.md` ("## fill-anywhere"), `tests/p2-shell.test.mjs` (`fill-textbox-add` in REQUIRED_IDS), `tests/p1-save.test.mjs` (flip). New: `tests/fill-anywhere.test.mjs` (5 tests), `tests/drive/fill-anywhere.jsonl` (26 steps, 3 clicks), `tests/drive/fill-anywhere-form.jsonl` (14 steps, 2 clicks), `.quad/shared/p6/checks/shot-fill-anywhere.mjs` (written, **not run**).

**Probe (mupdf 1.28.1, Helv 12, border 0, `.quad/scratch/fill-anywhere/probe.mjs`):** padding 0 (the clip rect is the full rect); line height 14.4 (1.2 × size); first baseline 9.6 below the top; a line wraps only when its advance is greater than the width (width = advance → 1 Tj; advance − 0.5 → 2 Tj). Advances: "Jane Doe" 51.36, "The quick brown fox" 107.376; "a\nb" → 2 lines. Rect = widest advance + 0.5 pt slack (float rounding) × lines·14.4, top = y − 7.2, clamped in the page box; a line wider than the page gets width = page width.

**Tests:** red-before `acceptance/fill-anywhere-red-before.log`: `# pass 0`, `# fail 5`. After: `tests/fill-anywhere.test.mjs` `# tests 5 # pass 5 # fail 0`. Full `npm test` (under lock): `# tests 606`, `# pass 605`, `# fail 0`, `# skipped 1` (600 + 5). Drives ×3: fill-anywhere 26 steps / 0 red, fill-anywhere-form 14 steps / 0 red, every run exit 0. Both tsc clean. `shasum -c fixtures.sha256`: 9 OK; the 55-file snapshot (`bugfix1-hash-after.txt` list) is identical.

**Flipped pin:** `tests/p1-save.test.mjs` ex-:530 `smoke --edit=form --save-in-place rewrites a form2 scratch` → `… is cancelled on a filled form2 scratch (Save writes a copy)`: `report.ok false`, error matches `P1_SAVE_ERROR .*cancelled`, scratch bytes unchanged, nothing else written. Its field/appearance assertions (`FORM_FIELDS`, `verifyFields`, `inside_frac > 0.01`) moved into fill-anywhere (c) on "form (filled).pdf"; the now-unused helpers were removed from p1-save. No other fill-then-save-in-place pin exists (grep: `save-in-place.jsonl` uses notes, `bugfix1.jsonl` uses Untitled + text-add). No existing drive script changed.

**Trigger file** (headless, output only under `.quad/scratch/fill-anywhere/out/`; the file in Downloads was only read and its sha256 is unchanged): 2 pages, rotate 0, 0 fields, 0 FreeText → `fill-text` → FreeText 1, revision 1, error "", `fill {filled:true, saveAsCopy:true, boxes:1, last:"add"}` → `save-to` ok, validated, 821770 bytes; reopened with mupdf: 2 pages, 1 FreeText.

**Deviations / choices:**
- Smoke never ended on a cancelled save, because the renderer's cancel path logged nothing. It now also logs `P1_SAVE_ERROR {"reason":"cancelled"}`. That line is read only by smoke `--save-to`/`--save-in-place`; STATE is unchanged.
- `fill.last` is built by the helper `fillOp()`. Written as an object literal, its `op:"add"` would be read as a request op by the p2-shell A1 scan.
- Delete on a focused box goes through the shared `commitTextBox` (`annot-delete`, "Deleted text box"), not `deleteNote`, so that `fill.last` is recorded as well.
- Tab on a text-box editor commits and stays put, as the add/link editors do. It does not jump to the next field.
- `annot-set fit` re-fits from the box's current top-left. `annot-move` refuses a non-FreeText with a plain Error, with no reason token.

**Not done / ceilings:**
- Text boxes are not painted as targets on rotated pages: no move or edit there, and placement is refused.
- Text boxes are not selection members (no Shift, clipboard or menu).
- A line wider than the page wraps, but the box height still counts only the typed lines.
- The GUI shot script was not run (the human is at the machine).
