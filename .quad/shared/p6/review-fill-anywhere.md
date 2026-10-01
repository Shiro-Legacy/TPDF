# fill-anywhere review (refute-first, 2026-09-30)

Diff read in full against `.quad/backup/pre-fill-anywhere-20260930.tgz` (extracted under `.quad/scratch/fill-anywhere-review/before/`). Changed files match the build note; nothing changed outside §1 ownership; fixtures untouched. Probes: `.quad/scratch/fill-anywhere-review/{probe,probe2}.mjs` (headless), `attack.jsonl`, `attack2.jsonl` (drive).

## Findings
1. **should-fix, FIXED** `src/renderer/engine.ts:5010` — `annot-set fit` re-fitted every FreeText, not only Helv ones (D1). A foreign FreeText (TiRo 20, rect [100,100,300,160]) was shrunk to [100,100,235.9,114.4]: its 20 pt text wrapped to 2 lines in a 14.4 pt box and was clipped. Non-WinAnsi contents were also refused (`unsupported-encoding`) on a font that is not ours. Fix: `fit` now applies only when the DA is Helv 12. Any other font takes today's path. This is stricter than the contract's "Helv", because `freeTextFit` measures at 12 pt only. Pinned by the new test `(a) review: annot-set fit leaves a FreeText that is not Helv 12 alone` (fails on the pre-fix build: `annot-set: unsupported WinAnsi characters`).
2. **should-fix, FIXED** `src/renderer/renderer.ts:4001` — `fill-text` did nothing and set no `error` for two inputs: empty or whitespace-only `text`, and a document that forbids annotations (certified-p2 gave `error ""`, revision 0). Both are now refused: "fill-text: text is empty" and "fill-text: this document does not permit annotations". Pinned by drive `fill-anywhere.jsonl` steps 29 and 34 (red on the pre-fix build).
3. **nit, FIXED** `src/renderer/renderer.ts:4099` — re-editing a box and adding only a trailing blank line counted as a change. The commit then sent `annot-set` with `addDraft(value)`, which equals the old contents: a journal op with no change, and the document became dirty (pre-fix: revision 1, dirty). The unchanged check now compares `addDraft` of both sides. Pinned by drive steps 30–32 (red pre-fix).
4. **should-fix, LEFT (orchestrator)** `engine.ts:1833` `freeTextFit` — a line wider than the page is only partly visible in the saved PDF. The height counts typed lines, not wrapped lines, so every wrapped line after the first is clipped. Measured with the 45-char sentence ×3/×6/×9/×18 (134/269/404/809 chars): the rect is [0,292.8,612,307.2] and the AP draws 2/3/4/8 Tj, so 1/2/3/7 lines are invisible. The box also jumps to x = 0. A word with no spaces (`"W"×80`, 4096 × "x") does not wrap: 1 Tj, clipped at the rect's right edge. A fix needs mupdf's word-wrap rule (scope). The build note admits the ceiling, but Help does not state it.
5. **should-fix, LEFT (data-loss edge, security-adjacent)** `src/main.ts:689-696` + `renderer.ts:9952` — if the "(filled)" copy is written but the rebind is skipped (`inspectRegularFile(dest)` fails, e.g. the copy is deleted or replaced by something else right after the write), main still returns `ok` with the new name. The renderer then sets `fillSave = "copied"` while `openPaths` still points at the original, so the next ⌘S is a plain in-place save that **overwrites the original**. The Untitled route has the same shape, but there it only rewrites a temp file. Proposed fix: return the rebind result, and set "copied" only when it rebound. Not reproduced: it needs an external race.
6. **nit, LEFT** a drive `mode` (or other) step taken while a text-box editor is open commits in the background. STATE shows `revision 0`, no box on that step, and settles 1–2 steps later (`attack2.jsonl` steps 4–7). This is the same pattern as the Edit-text add editor, so not new.
7. **nit, LEFT** tests: (a) pins "Jane Doe" width as `< 60` rather than the exact 51.86. (b) compares `flat.pdf` by byte length, not hash. The p1-save flip dropped a form-specific mode-preservation check; in-place mode is still pinned at `p1-save.test.mjs:389/442`. `annot-move` on a foreign FreeText regenerates its `/AP` with mupdf's writer (D2 as written), so a rich or bordered appearance is lost.

## Held (tried, no defect)
- Save-as-copy: `save-in-place path` = own path is refused `same-file` (doc stays dirty, a.pdf unchanged). Other cases that held:
  - fill → undo → save: "No changes", still saveAsCopy.
  - Undo then a non-fill edit → save is cancelled.
  - `save-to` keeps the flag; the next `save-in-place` is still cancelled.
  - A file named `z (FILLED).pdf` keeps plain Save (case-insensitive).
  - Untitled + fill: cancelled without a path; with a path it rebinds and `saveAsCopy false`.
  - `new`/`open` resets `filled`.
  - `filled` with `inPlace:false` takes the normal Save As route; a non-boolean `filled` → `unreadable`.
  - Stale `saveToArg`: every drive `save-in-place` reassigns it, and smoke forbids `--save-to` with `--save-in-place`, so it cannot leak.
  - The flag is set by `commitField` (panel rows, on-page editor, checkbox/radio, `form`) and by drive `fields fill`. A failed `fields fill` does not set it. `edit-batch` and paste have no `field-set`.
- Engine: 60 strings (kerning pairs, W/m/i runs, trailing and leading spaces, blank lines, ligature letters, Latin-1, €, …) each drew exactly one Tj per typed line.
  - `\t`, `\r`, Ω, CJK and emoji are refused before `mutate` (dirty false, revision 0). 4097 chars are refused.
  - Edge points and a 10×10 / 50×20 page stay inside the page box; off-page points are refused.
  - `annot-move`: same point → `noop`, revision unchanged; clamped at every edge; refused on a note or a bad id/page/NaN; undo and redo are exact.
  - Encrypted no-annotate and certified documents: `read-only`, not dirty.
  - The saved FreeText has no `/T`. Without `fit` the path is unchanged (160×40).
- Renderer: fields still win over boxes; an Escape'd editor writes nothing; a mode change commits once; undo/redo keep `fill.boxes` in step. STATE `fill.boxes` carries `chars` only. The p2-shell op-coverage and `OP_PERMISSION` entries are correct. Help text stays in the existing cell. The red-before log shows 5/5 `not ok` for the right reasons.

## Not verified
The GUI (real click, ⌘S sheet with "(filled).pdf" as the default) was not run: the human is at the machine, and the shot script was only read. Also not verified: the `#fill-textbox-add` panel button (drive cannot press DOM buttons), a real-mouse drag's click suppression (read in code only: `swallowNextClick`), and the Enter-then-blur double commit (read in code only).

## Checks after fixes
Build ok. `tests/fill-anywhere.test.mjs` 6/6. Drives ×3: fill-anywhere 34 steps / 0 red, fill-anywhere-form 14 / 0 red. Both tsc clean. `fixtures.sha256` 9 OK. The 55-line hash snapshot matches `fill-anywhere-hash-before.txt`. Full `npm test` (last run): `# tests 607 / # pass 606 / # fail 0 / # skipped 1`. Two earlier full runs failed only on `tests/p2-annots2.test.mjs:293`, the wall-clock Ink budget: 2191 and 2075 ms against a 2000 ms budget. That file passes alone in 274 ms, the Ink path is untouched, and the load average was 8.8. It is a new flake candidate for progress.md §0.
