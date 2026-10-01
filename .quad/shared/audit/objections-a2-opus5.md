# A2 contract objections: opus5, 2026-09-17 (first pass)

Read against `renderer.ts`/`main.ts` after A1 (`runEditRequest` `858–912`, `stateSummary` `4702`, `editSummary` `4724`, and the dialog functions named below).

## General rule for §4/§5 (applies to every dialog-backed op)
1. **Parameterize by filling the dialog's own inputs, then call its own save function.** Don't call the inner request helper directly. The DOM validation lives in the save functions, not in the helpers:
   - `saveInsertDialog` `4159`: `at` range check
   - `saveStampDialog` `4559`: `parsePageRange`
   - `saveLabelsDialog`/`clearLabels` `4608/4629`: `labelsFrom` range, `prefix.slice(0, 32)`, `Number(start) || 1`
   - `saveLinkDialog` `3584`: page 1..n, empty URI
   - `saveBookmarkDialog` `3518`: empty title

   Calling `insertPages`/`addStamp`/`addLink`/`addBookmark` directly would skip that validation and be a second path.
   Rule for each branch: `open*Dialog()` → set the input values from `contents` → `await save*Dialog()`. If the dialog is **still open** afterwards (the save function refused and returned early), `close()` it and leave the refusal in `error`. Otherwise the modal dialog stays open and blocks every later drive line.
   Numbers are typed into the inputs exactly as given, so dialog-backed fields use the dialog's numbering:
   - `insert.after` is 0 (before the first page) to n
   - `labels.from`, `link.page` and the stamp `pages` range are 1-based
   - only `goto`/`page`/`index` are 0-based

   Record that table in `p3-agent-drive.md`.

## Blocking
2. **`insert` `contents.from` can't work.** The renderer is sandboxed and can't read a path. The file only arrives through `api.pickInsertSource()` (`renderer.ts:4119`), which `--drive` already answers from the **top-level** line key `"from"` (`main.ts:1519`, `insertFromArg`). A `contents.from` would be ignored or be a second file path.
   Rule: `{"cmd":"insert","from":"fixtures/…pdf","contents":{"after":1,"range":"1-3"}}`. The branch is `requestInsert()`'s sequence: `pickInsertSource` → `openInsertDialog` → fill `insertAfterInput`/`insertRangeInput` → `saveInsertDialog()`. A missing file surfaces through `pickInsertSource`'s own `showError`.
3. **`decorate.stamps`/`decorate.labels` are not live state.** `decorateSummary` is written only by `smokeDecorate` (`4671`). Its `stamps` are counts the smoke computed itself, and `refreshDecorateSummary` only copies `labels`/`layers` over an existing summary. A dialog-path watermark or labels change leaves STATE untouched or `null`, so §4's "STATE `decorate.stamps` proves it" can't hold.
   Rule: the new branches set `decorateSummary.last = { action: "stamp"|"labels-set"|"labels-clear", kind?, pages?: number[]|"all", ok, detail? }` after the save function returns, creating the summary when it is null (with the existing `labels`/`layers` fields). `decorate.labels` must also be refreshed from `labels` after `saveLabelsDialog`/`clearLabels`. Proving that a stamp landed needs an engine read, not a counter the renderer bumps: expect a `text-all` substring through the existing `content`/`export-txt` path, or record that ceiling.
4. **`annot set` contents can't be proven from STATE.** `annots` holds only per-type **counts** (`editSummary` `4725`), and `notes` is a count. Nothing in STATE carries comment text.
   Rule: add a new top-level key `comments: [{ index, type, contents, editable }]`: the current page's `notes` array in render order, contents cut to 200 chars like other STATE text. Don't put `items`/`last` inside `annots`, whose keys are annotation type names. Make it `comments.last: { action, index, detail? }`, or `annotLast` if `comments` must stay an array. Pick one and record it.
5. **`annot` index resolution.**
   - `index` is the position in `notes` exactly as `renderNotes` (`3669`) iterates it. That includes the rows it shows as "not editable in this version" (`annotName` falsy) and the rows with `editable:false`.
   - `set`/`delete` on such a row → refusal `not-editable` in `comments.last.detail`, never dirty.
   - `notes` is refreshed asynchronously (`refreshNotes` with `notesSeq`, and `goTo` fires it with `void`), so the branch must `await refreshNotes()` **before** resolving the index. Otherwise after undo/redo/goto the index hits the previous list.
   - The id comes from `notes[index].id`, the same id the row buttons carry in `data-annotId`. `set` calls `editNote(id, text.slice(0, MAX_NOTE_CHARS))` and `delete` calls `deleteNote(id)` (`4001/4010`), the same functions the buttons use.
6. **`goto` must wait for what it triggers.** `goTo` (`1275`) fires `refreshNotes`/`refreshFields`/`refreshLinks`/`refreshContentTargets`/`drain` with `void`. `driven()` then logs STATE before they finish, so `page` changes but `comments`/`fields`/`nav.links` still describe the old page.
   Rule: the drive branch (not `goTo` itself) awaits `Promise.all([refreshNotes(), refreshFields(), refreshLinks()])` after `goTo`. Out-of-range values are refused **before** `goTo`, because `goTo` clamps silently: refuse when `!Number.isInteger(page) || page < 0 || page >= pageCount`, reporting `error` and nothing else.
7. **`page-ids` cost lands on every UI page refresh.** §2 puts the new engine op inside the renderer's `pages` refresh, which runs after every rotate/move/delete/insert/undo, not only under drive. That is an extra worker round-trip on user actions, and `pdf.findPage(i)` for each `i` is one tree lookup per page (O(n log n) or worse on unbalanced trees).
   Rule: request `page-ids` only on the drive path, awaited once in `driven()` before `logState` when a document is open. Never from the UI refresh. STATE `pages.ids` is omitted when not fetched. `pdf4allState` (CDP, synchronous) returns the last fetched ids. Cost is recorded on `large.pdf` at open and after a move. The engine op validates nothing, writes nothing, and is PDF-only (`[]` for non-PDF).
8. **`save-in-place` in `note-undo-save.jsonl` risks overwriting a repo fixture, and edits an existing script.** Plain save writes back to the path the document was **opened** from (`openPaths`, `main.ts:585`). "A copy the script first `save-to`s" is safe only if the script then **`open`s** that `${OUT}` copy. Otherwise the in-place write targets `fixtures/…`.
   Rule: `save-to ${OUT}/x.pdf` → `open ${OUT}/x.pdf` → an edit → `save-in-place` → expect `save.ok` and `dirty:false` → reopen, then check the edit. Put it in a new script (`save-in-place.jsonl`) so §1's "existing jsonl unchanged" holds. The review checks that the fixture's sha256 is unchanged.

## Should fix
9. **`labels` unknown style silently becomes "no numbering".** Assigning a value that isn't an option to `labelsStyleSelect.value` makes it `""`, and `""` is a valid style (prefix only). The "unknown style" refusal in §6 would pass through as a successful change.
   Rule: the branch checks `[...labelsStyleSelect.options].some(o => o.value === style)` before assigning, else refuses `invalid-style`, never dirty. The same applies to `stampKindSelect`: `kind` must be one of its options.
10. **Stamp `pages: "all"` isn't accepted.** `parsePageRange` treats **empty** as every page (`engine.ts` doc comment on `parsePageRange`); `"all"` returns null and `saveStampDialog` shows the range error.
    Rule: `pages` omitted or `""` = all. There is no `all` keyword, unless the branch maps `"all"` → `""` explicitly and says so.
11. **The `link` anchor is the current search match.** `openLinkDialog` (`3566`) needs `currentHit()`, else it announces "Find something first" and returns with no dialog. The contract's `link` contents have no anchor.
    Rule: `link` add requires a prior `find`. With no hit, refuse `no-anchor` (`nav.last.detail`), which is what `nav-zoom-find.jsonl`/`links-bookmarks.jsonl` must do first. URI schemes: the dialog checks only for an empty value. The allowlist is the engine's `LINK_SCHEMES = ["http:", "https:", "mailto:"]` (`engine.ts:128`), refused in `link-add`. So a bad URI reaches `error` through `addLink`'s `showError` and never dirties. The script expects that text, not a renderer token. `link delete` / `bookmark rename|delete|go` resolve `index` against `links` (the rendered Links list for the current page) and `outline` (render order) after awaiting their refresh, the same rule as #5.
12. **Existing op names keep their menu meaning.** `insert`, `watermark`, `header-footer`, `labels`, `bookmark` and `link` already exist in `runEditRequest`. With `contents === null` (the menu), they must keep opening the dialog for a human. Only a non-null `contents` takes the parameterized branch. `link` without contents today opens "page" mode, which must stay. `underline`/`strikeout` are new branches mirroring `smokeHighlight`. Note that `smokeHighlight` searches `"PDF4All"` when there is no hit and `contents` is non-null, which is a hidden query. For the new ops, require a prior `find` (no implicit query) and refuse `no-anchor`. `highlight` is left as it is.
13. **`tab` needs a defined namespace.** `selectTabById(inspectorTabs, inspectorPanels, id)` covers only the inspector tabs, and `setMode` (`2532`) also moves them. Rule: `tab.id` ∈ the `inspectorTabs` ids; unknown → `error`. STATE `tab` = the selected inspector tab id. A later mode switch can change it, which is expected.
14. **`find` must await the search and use the existing step functions.** `startSearch()` (`1524`) plus the handler behind the toolbar next/prev buttons; no new search loop. STATE `search: { query, count, index }` only, never the hits array.

## Small
15. New STATE keys `page`, `zoom`, `search`, `tab`, `comments` don't collide with existing top-level keys (`doc`, `mode`, `status`, `error`, `announcement`, `canRedo`, `help`, `revision`, `dirty`, `canUndo`, `notes`, `annots`, `pages`, `fields`, `nav`, `assemble`, `extract`, `export`, `decorate`, `sign`, `security`, `content`, `save`, `print`). `pages.ids` and `assemble.last` add keys inside existing objects, which is safe under the subset matcher (`main.ts:1457`). `assembleSummary` is `null` until a successful insert, so a refusal must create it with only `last`.
16. `zoom.scale` = `zoomNow(wantedPage)` rounded to 3 decimals. In fit modes it depends on the window size, so scripts should expect `zoom.mode` for fit modes and `scale` only for `fixed`.
