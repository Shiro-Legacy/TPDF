# A2 — agent-driveability: STATE evidence, comment/markup/link ops, parameterizable dialogs

2026-09-17 15:10, fable51. Human approved "A1 then A2" (`post-p3-audit-plan.md` §A2). Source: `driveability-grok.md` ranked gaps 1, 3, 4 (three dialogs), 5, 6. Reviewer **opus5** (objections ≤ 20 min, then review); ds2 = renderer/engine/tests (starts only after A1 is accepted — same files); grok seat = drive scripts + independent verification. Baseline = A1 acceptance count. Drive conventions: `design-agent-drive.md`, `p3-agent-drive.md`; ops enter through `runEditRequest(op, contents)` in `renderer.ts`, `contents` is JSON for parameterized ops (like `fields`/`content`/`encrypt`).

## 1. Rules
- Every new op reuses the handler the menu/panel already calls (`addMarkup`, `annot-set`/`annot-delete` requests, `openStampDialog` apply path, the insert/labels/link/bookmark dialog save paths). No second code path, no new engine op unless listed here.
- STATE stays a subset-matched JSON; new keys only, existing keys unchanged (all `tests/drive/*.jsonl` expectations stay valid).
- Refusals land in STATE (`error`/the op's `last.detail`), never throw out of `driven()`.
- One script per closed gap under `tests/drive/`, added to `tests/p3-drive.test.mjs`. Each script proves the result through STATE, not through a log line.
- No new dependencies. Dialog HTML unchanged except ids needed for prefill.

## 2. STATE additions (ds2, `stateSummary`)
- `page: number` (0-based shown page = `wantedPage`), `zoom: { mode: "fit-width"|"fit-page"|"fixed", scale: number }`, `search: { query, count, index }` (from `searchQuery`/`searchHits.length`/`searchIndex`; `index` −1 when none), `tab: string` (active sidebar tab id) — present whenever a document is open.
- `pages.ids: number[]` — page object numbers in current order, so Move/Delete/Insert are observable. Gathered in the renderer's `pages` refresh from a new read-only engine op `page-ids` (`pdf.findPage(i)` object numbers, `attempt` per page, PDF only; ≤ 2 ms on `large.pdf`, recorded). `pageState` itself is untouched (A1 don't-touch list).
- New drive cmds (renderer, via `edit-request`): `goto` `{page}` (0-based, clamps + refuses out of range in `error`), `zoom` `{mode}` or `{scale}` (one of `ZOOM_STEPS`), `find` `{query, dir?: "next"|"prev"}` (runs the toolbar search; `search` in STATE shows the result), `tab` `{id}`.

## 3. Comment and markup ops (ds2)
- `annot` cmd: `{"action":"set","index":n,"contents":"…"}` and `{"action":"delete","index":n}` where `index` is the row in the current Comments list (`annots` reply order); resolved to `annotId` exactly as the list buttons do. STATE `annots` (existing) proves count/contents; add `annots.last: { action, index, detail? }`.
- `underline` and `strikeout` get a `runEditRequest` branch calling `addMarkup` on the current search match, mirroring `smokeHighlight(contents)` (same `contents` semantics). `OP_MODE` entries stay.

## 4. Parameterizable dialogs (ds2) — three only
- `insert` with `contents` JSON `{ "from": "<path>", "after": n, "range": "1-3" }` runs the insert dialog's save path with those values (the smoke `assemble` sequence stays for its fixed test). Refusals (bad range, missing file) → `error` + `assemble.last.detail`.
- `watermark` / `header-footer` with `contents` JSON `{ "kind": "watermark"|"header"|"footer", "text": "…", "pages": "1-3|all" }` runs `openStampDialog(kind)`'s apply path. STATE `decorate.stamps` (existing) proves it.
- `labels` with `contents` JSON `{ "from": n, "style": "D|r|R|a|A|", "prefix": "…", "start": n }` or `{ "clear": true, "from": n }` runs the labels dialog's save/clear path. STATE `decorate.labels` proves it.
- Ceilings (recorded, not built): extract range, export dpi, icon-stamp choice, signature appearance, security dialog, note/bookmark/link dialogs' own prompts stay smoke-fixed.

## 5. Links, bookmarks, save (ds2)
- `link` cmd gains `contents` `{ "kind": "page", "page": n }` or `{ "kind": "uri", "uri": "https://…" }` (allowlisted schemes as the dialog enforces); `{ "action": "delete", "index": n }` deletes a row of the Links list.
- `bookmark` cmd: `{ "action": "add", "title": "…" }` (current page), `{ "action": "rename", "index": n, "title": "…" }`, `{ "action": "delete", "index": n }`, `{ "action": "go", "index": n }` (then `page` in STATE proves it). Uses the existing `outline-add/set/delete` engine ops.
- `save-in-place` line added to `tests/drive/note-undo-save.jsonl` (on a copy the script first `save-to`s), expecting `save.ok` and `dirty:false`.

## 6. Scripts (grok seat) and tests
- `tests/drive/nav-zoom-find.jsonl` (goto, zoom modes and one fixed scale, find next/prev with `search.count/index`, tab switch, move-page proving `pages.ids` reorder, delete-page proving the id vanishes).
- `tests/drive/annot-edit.jsonl` (note add → set → delete; underline + strikeout on a match; undo/redo; save; reopen counts).
- `tests/drive/dialogs.jsonl` (insert with range after page 1, watermark + header with page ranges, labels set + clear; refusal cases: bad range, unknown style).
- `tests/drive/links-bookmarks.jsonl` (page link, uri link, link delete; bookmark add/rename/go/delete; save/reopen).
- Each added to `tests/p3-drive.test.mjs` (ds2 wires, grok writes). Grok drafts the scripts from this contract before ds2 starts (expectations may be adjusted once the ops land; record every adjustment).
- Keystroke walk at 900×600 (the P3 waived gap): Fields designer, Help dialog, one parameterized dialog — only when the human is away; otherwise a recorded waiver.

## 7. Acceptance
All four scripts pass in the real app (`scripts/drive.mjs`), `npm test` green with the real count, `npm run smoke` and `gui-shell-check.mjs` PASS, every existing `tests/drive/*.jsonl` unchanged and passing, `measure.mjs` within budgets (the `page-ids` cost recorded), refusals never dirty. Evidence `a2/ds2.md`, `a2/grok.md`; review `review-a2.md`.

## 8. Amendments 2026-09-17 (fable51, from `objections-a2-opus5.md` — all 16 accepted; override §2–§6 where they differ)
1. **Dialog-backed ops fill the dialog's own inputs, then call its own save function** (`saveInsertDialog`, `saveStampDialog`, `saveLabelsDialog`/`clearLabels`, `saveLinkDialog`, `saveBookmarkDialog`); never the inner helper. If the dialog is still open after the save call (refused), `close()` it and leave the refusal in `error`. Dialog numbering applies: `insert.after` 0..n, `labels.from`/`link.page`/stamp `pages` 1-based; only `goto`/`page`/`index` are 0-based. Table recorded in `p3-agent-drive.md`.
2. **`insert`:** `{"cmd":"insert","from":"<path>","contents":{"after":1,"range":"1-3"}}` — `from` is the top-level line key `--drive` already answers for `pickInsertSource`; branch = `requestInsert()`'s sequence with the two inputs filled. Missing file surfaces through `pickInsertSource`'s own `showError`.
3. **`decorate` STATE:** the branches set `decorateSummary.last = { action: "stamp"|"labels-set"|"labels-clear", kind?, pages?, ok, detail? }` (creating the summary when null) and refresh `decorate.labels` from `labels` after the labels save/clear. A stamp's presence is proved through an engine read (`text-all` substring via the existing `content`/export-txt path) or recorded as a ceiling.
4. **`comments` STATE:** new top-level key `comments: [{ index, type, contents (≤ 200 chars), editable }]` = the current page's `notes` in `renderNotes` order, plus top-level `annotLast: { action, index, detail? }`. `annots` stays counts only.
5. **`annot` index:** position in `notes` as rendered (including non-editable rows); `set`/`delete` on a non-editable row → `not-editable`, never dirty; the branch `await refreshNotes()` before resolving; id = `notes[index].id`; `set` → `editNote(id, text.slice(0, MAX_NOTE_CHARS))`, `delete` → `deleteNote(id)`.
6. **`goto`:** refuse `!Number.isInteger(page) || page < 0 || page >= pageCount` before calling `goTo` (which clamps silently); after `goTo` the branch awaits `Promise.all([refreshNotes(), refreshFields(), refreshLinks()])`.
7. **`page-ids`:** requested only on the drive path, awaited once in `driven()` before `logState` when a document is open; never from the UI refresh; `pages.ids` omitted when not fetched; `pdf4allState` returns the last fetched ids. Engine op: read-only, PDF-only (`[]` otherwise). Cost recorded on `large.pdf` at open and after a move.
8. **`save-in-place`:** new script `tests/drive/save-in-place.jsonl`: `save-to ${OUT}/x.pdf` → `open ${OUT}/x.pdf` → edit → `save-in-place` → `save.ok`, `dirty:false` → reopen and check. Existing scripts untouched. Review checks the fixture sha256 is unchanged.
9. **Unknown `style`/`kind`:** checked against the select's options before assigning; else `invalid-style`/`invalid-kind`, never dirty.
10. **Stamp `pages`:** omitted or `""` = all pages; no `all` keyword.
11. **`link` add** needs a prior `find` hit; with none → `no-anchor` in `nav.last.detail`. URI allowlist is the engine's `LINK_SCHEMES`; a bad URI reaches `error` through `addLink`'s `showError`, never dirties; scripts expect that text. `link delete` / `bookmark rename|delete|go` resolve `index` against the rendered `links`/`outline` after awaiting their refresh.
12. **Menu meaning kept:** `insert`, `watermark`, `header-footer`, `labels`, `bookmark`, `link` with `contents === null` still open the dialog; only non-null `contents` takes the parameterized branch. `underline`/`strikeout` require a prior `find` (no implicit query) and refuse `no-anchor`; `highlight` unchanged.
13. **`tab.id`** ∈ the inspector tab ids; unknown → `error`; STATE `tab` = selected inspector tab id.
14. **`find`** uses `startSearch()` and the toolbar next/prev handlers, awaited; STATE `search: { query, count, index }` only.
15. New top-level keys (`page`, `zoom`, `search`, `tab`, `comments`, `annotLast`) do not collide; `assembleSummary` is created with only `last` on a refusal.
16. **`zoom.scale`** = `zoomNow(wantedPage)` rounded to 3 decimals; scripts expect `zoom.mode` for fit modes and `scale` only for `fixed`.
