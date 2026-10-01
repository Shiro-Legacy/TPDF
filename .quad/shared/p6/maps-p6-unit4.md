# P6 U4 surface maps (Opus 5.5 readers, 2026-09-24) — concatenation of the three lens maps

# P6 U4 surface map — ENGINE (Opus 5.5 reader, 2026-09-24, read-only)

**Summary.** Links today have three ops, `links`/`link-add`/`link-delete`. A link is named by its positional `getLinks()` index. No link op carries an object number, and `page.getAnnotations()` never returns Link annots (measured), so `findAnnotation` cannot find a link. Fields list in `/Annots` widget order (measured on form/form2). They carry no page `/Tabs` and no per-widget order key. The worker is a pass-through to `engine.dispatch`.

I measured four facts with plain node against build/renderer/engine.js and mupdf 1.28.1. The scripts are in the session scratchpad, `probe-u4-{engine,link,dict,skip,dest,order}.mjs`:
- **M1 `Link.setBounds` is not persistent.** It writes the y-down display rect into `/Rect` without the inverse page CTM. Live `getLinks()` shows the new rect, but after save and reopen the link is mirrored. On a Rotate 0 page, setBounds([82,72,210,122]) reopens as [82,670,210,720]. On a Rotate 90 page it reopens transposed. A test without save → reopen would pass wrongly.
- **M2 a direct `/Rect` rewrite works.** Build the new `/Rect` from `mupdf.Rect.transform(rect, mupdf.Matrix.invert(page.getTransform()))` and write it into the Link dict inside one `beginOperation`. It is exact live, after `garbage=yes` save and reopen, and after undo, at Rotate 0 and 90. `setURI` also journals (undo restores it). With a `formatLinkURI(fitDest(p))` string it writes `/A <</S/GoTo/D[obj /Fit]>>`.
- **M3 `setURI` loses to `/Dest` on reload.** On a link that carries `/Dest`, `setURI` writes `/A` but leaves `/Dest` in place. MuPDF reads `/Dest` first, so after reopen the old target is back and the change is lost with no error. link-set must `delete("Dest")` in the same mutate.
- **M4 `getLinks()` skips some `/Link` annots.** A `/Link` with no `/A` and no `/Dest`, with a JavaScript `/A`, or with no `/Rect` is left out. So getLinks index ≠ position among the /Link entries in `/Annots`, and an `annotId` needs its own walk.

## api (today)
- `LinksRequest {op:'links', page}` → `LinksReply {id, ok, page, items: LinkEntry[]}` (engine.ts:351, :785). It is read-only, with no `requireMutable`. It refuses no-document or locked with a plain Error (:3973).
- `LinkAddRequest {page, rect:number[4], target:{page}|{uri}}` (:352) and `LinkDeleteRequest {page, linkId}` (:356), in the `SessionRequest` union at :531/:538.
- `LinkEntry {id: getLinks index, rect (y-down, display space incl. /Rotate), kind:'page'|'uri'|'blocked', page?, uri?}` (:615).
- `LINK_SCHEMES` http/https/mailto (:133) · `MAX_URI_CHARS` 2048 (:127) · `schemeOf` (:2487) · `fitDest(page)` (:2468) is a plain Fit literal for `doc.formatLinkURI`.
- `checkLinkRect(box, rect, op)` (:2519) takes 4 finite numbers and normalizes them. It **refuses** an off-page rect (`onPage` :1551 on both corners) and never clamps. `pageBox` (:1542) is `page.getBounds()`: the display box, rotated, y-down.
- Link errors are **plain `Error` strings** with no `reason`/`detail` (:3985-4000, :4020). link-add and link-delete reply `editState` (journal + pageCount/pages/rotations, :1487).
- `AnnotItem {id: object number, type, rect, contents, subject, editable, search?}` (:584). `listAnnotations` (:2359) walks `page.getAnnotations()` and skips Widget/Popup. **Link is never listed**: MuPDF keeps links out of that list (measured: nav.pdf gives `annots` []).
- `annotId(annot)` = `getObject().asIndirect()` (:1669). `findAnnotation(page, id, op)` (:1673) throws a plain `no annotation N`.
- `FieldsRequest {op:'fields', page}` (:308) → `FieldsReply {id, ok, page, items: FieldItem[]}` (:784, handler :3172). `FieldItem` (:555) has id, name, label, kind, value, readOnly, required, scripted, rect, plus optional tu/default/on/options/edit/maxLen/multiline. It has **no tabs, no /Annots index and no struct key**.
- `failRedact(detail: 'rect'|'no-such-annot'|'not-redact')` (engine-core.ts:143) and `failField(FieldDetail)` (:135) set `reason:'invalid'` plus `detail`. `failReason(SaveReason)` (:151) sets `reason` only. dispatch's catch (engine.ts:4757) spreads `reason`, `detail`, `extra` and `journalState` into every refusal.

## redact-set: the precedent for link-set (engine.ts:3755-3782)
The order is fixed: `requireMutable` → `checkPage` → rect validation, with the failure re-tagged by `failRedact('rect')` → `findAnnotation`, re-tagged by `'no-such-annot'` → a type check giving `'not-redact'` → a **no-op check** against `getRect()` (±0.01, returning `{ok, noop:true, ...journalState}` with no journal op) → `mutate(session, doc, 'move redaction mark', …)`. The reply is **journalState only**, while the link ops reply editState. `redactRect` (redact.ts:79) **clamps** to the page and requires at least 1×1 pt. `setRedactRect` (redact.ts:102) does setRect, clearQuadPoints, deletes /PDF4AllSearch, then update. The renderer rounds the rect to 1e-4 (renderer.ts:2486). Tests: tests/p4-redact.test.mjs:173-223.

## constraints
- `mutate` (engine.ts:1697) is the only journal path: one gesture = one op = revision+1. Validate before calling it. A throw abandons the op. **A refusal or no-op never dirties.**
- Every new op joins `OP_PERMISSION` (engine-core.ts:236). Links there are `'annotate'`. tests/p2-shell.test.mjs:158-177 regex-scans renderer.ts for `op: "x"` and fails if x is not in OP_PERMISSION, READ_ONLY (which already has `fields`, `annots`, `links`) or DEFAULT_EDIT. `link-set` → `'annotate'`. A read-only `fields` extension needs no entry.
- The link scheme allowlist and the 2048 bound apply to any target change. A page target goes through `doc.formatLinkURI(fitDest(checkPage(...)))`.
- `SAVE_OPTIONS = "incremental=no,garbage=yes"` (:162). `saveOptions` (:1162) appends `encrypt=keep|none|aes-256…`, and `serializeDocument` (:1176) is used by `save` (:4741). The save round-trip test pattern is `op({op:'save'})` → reopen the bytes with `mupdf.Document.openDocument` → `getLinks()`/`links`.
- Rotation: `/Rotate` is read via `pdf.findPage(i).get('Rotate')` (`pageRotation` :1306), and `rotations[]` rides every editState. Link, annot and widget rects are all in display space (y-down, after /Rotate and CropBox), the same space as `placeBox`. Only a raw `/Rect` write needs `Matrix.invert(page.getTransform())` (M2). `rawPageBox` (:2869) reads the unrotated Crop/MediaBox for the stamp only. text-add refuses rotated or UserUnit pages (:4291-4294).
- The worker (worker.ts:31-33) does `engine.dispatch(data, session)` with no routing table. A new op needs only a type in the union, a `case` in `dispatch`, an OP_PERMISSION entry, a renderer `request({op})` call and tests.
- Fields read the page dict as `page.getObject()` (a PDFPage; :4291 does this) or `pdf.findPage(i)`, then `at(dict,'Tabs')`. `at`/`attempt` are at engine-core.ts:71/:99. `/Tabs` is page-level only and not inheritable.

## extensionPoints
- **link-set (≈45 lines engine.ts + 1 in engine-core.ts).** Add `LinkSetRequest {op:'link-set', page, annotId, rect?, target?: {page}|{uri}}` to the union, and `failLink(detail: 'rect'|'no-such-annot'|'not-link'|'scheme')` next to failRedact.
  - Lookup: walk `page.getObject().get('Annots')` for the indirect entry whose `asIndirect() === annotId`. If none, refuse `no-such-annot`. If its `/Subtype` ≠ Link, refuse `not-link`.
  - Rect: validate with `checkLinkRect`, re-tagged `'rect'` (or clamp like redactRect; the contract decides). Write `/Rect` as `Rect.transform(rect, Matrix.invert(page.getTransform()))`, never `link.setBounds` (M1).
  - Target: allowlist check, then write `/A` (via the matched `Link.setURI` or by dict) and `delete('Dest')` (M3). Refuse a blocked-scheme URI with `'scheme'`.
  - No-op: compare the current display rect and target against the request (±0.01) → `{ok, noop:true}`. Destroy the page before `mutate(…'edit link'…)` so the next `loadPage` re-reads links (measured: live `getLinks` is then correct).
- **`listLinks` gains `annotId`** (:2497). Walk `/Annots` /Link entries in order. Match each `getLinks()` item to the next /Link whose transformed `/Rect` equals `getBounds()` ±0.01 (M4 skips), or compute the entries from the dict directly. `LinkEntry` gets an `annotId: number` (additive; keep `id` for link-delete).
- **fields metadata (≈10 lines).** Add `FieldsReply.tabs?: 'R'|'C'|'S'|'A'|'W'|null` from `at(pageDict,'Tabs')?.asName()`, allowlisted. Items already come in `/Annots` widget order, so A and W need no new key. For S, a per-widget `/StructParent` (singular) value, or a struct-tree DFS index. `editStructElement`/`editParentTreeNode` (content-edit.ts:1328/:1343) resolve **page MCIDs**; they treat a dict value as a number-tree node, so they are not reusable for a widget's OBJR entry. A DFS needs the `walkObject` cap (redact.ts:166, 50k nodes).
- **Tests.** tests/p2-nav.test.mjs:366-440 holds the link list/add/delete/refusal cases; add link-set there or in a new p6 file. Cover: link-set → save → reopen → `links` rect + target, on the rotated page too; undo restores both; refusals leave `dirty:false`; a /Dest link retargeted to a uri survives reopen.

## risks
- **M1/M3 are silent data bugs.** Only a save → reopen proves a link edit. The contract must require it (§0 "proven or refused").
- **`/TI` is not a tab index.** ISO 32000-1 Table 231 defines `/TI` as the *top index* of a scrollable list box. No per-widget tab-order key exists. The file's only order data is `/Tabs` plus the `/Annots` array order (PDF 2.0 `A`/`W`, and Acrobat's "Unspecified"). Handoff A2's "/TI sorts within the page" should be dropped or restated as `/Annots` order.
- `S` needs the widget's `/StructParent` and a StructTreeRoot walk. Page `/StructParents` is the MCID key and is the wrong test.
- The link id space is mixed: link-delete stays positional while link-set is by annotId. After an add or delete, positional ids shift. A renderer that holds `id` across refreshes deletes the wrong link.
- `checkLinkRect` refuses off-page rects, but handoff A1 says "clamped". A drag past the edge then refuses instead of snapping. Pick one, and state the token.
- Reply shape: redact-set replies journalState, the link ops editState. Pick one for link-set; it is additive either way.
- Blocked links (file:, javascript:) are listed, but JS-action links are invisible entirely (M4). A rect-only move of a blocked link is safe via the dict path (no createLink).

## openQuestions
- Does link-set clamp or refuse off-page rects? Does a rect-only edit of a `blocked` link succeed, or refuse `scheme`?
- Should Tab order use `A`/`W` = /Annots order (cheap, already the `fields` order) instead of the handoff's "treat as R"? And with no `/Tabs`: keep the current row rule, or use /Annots order like Acrobat's Unspecified?
- For S: engine DFS index per field, or fallback only (STATE `fields.tabsOrder:'fallback'`)?

## anchors
- src/renderer/engine.ts:127/:133 — MAX_URI_CHARS / LINK_SCHEMES
- src/renderer/engine.ts:162 — SAVE_OPTIONS "incremental=no,garbage=yes"; :1162 saveOptions; :1176 serializeDocument; :4741 save case
- src/renderer/engine.ts:300 — RedactSetRequest (template for LinkSetRequest)
- src/renderer/engine.ts:308 — FieldsRequest; :784 FieldsReply (add `tabs`)
- src/renderer/engine.ts:351-356 — LinksRequest / LinkAddRequest / LinkDeleteRequest
- src/renderer/engine.ts:531-541 — SessionRequest union (link ops at :538)
- src/renderer/engine.ts:555 — FieldItem (no tabs/order key)
- src/renderer/engine.ts:584 — AnnotItem; :615 LinkEntry (add annotId)
- src/renderer/engine.ts:1296 — journalState; :1306 pageRotation; :1324 pageState; :1487 editState
- src/renderer/engine.ts:1496 — requireMutable (OP_PERMISSION ?? 'edit'); :1510 checkPage (plain Error)
- src/renderer/engine.ts:1542/:1551 — pageBox (getBounds, display space) / onPage
- src/renderer/engine.ts:1669/:1673 — annotId / findAnnotation (getAnnotations: no Link, no Widget)
- src/renderer/engine.ts:1697 — mutate: one journal op, abandon on throw
- src/renderer/engine.ts:1738 — annotRect
- src/renderer/engine.ts:1756/:1790 — fieldContext / widgetScripted
- src/renderer/engine.ts:1848 — listFields: getWidgets() order == /Annots widget order (measured)
- src/renderer/engine.ts:1897 — findWidget (failField 'stale-target'); :2062 fieldOwnDict
- src/renderer/engine.ts:2359 — listAnnotations (Link absent)
- src/renderer/engine.ts:2468/:2487/:2497/:2519/:2537 — fitDest / schemeOf / listLinks / checkLinkRect / linksToPage
- src/renderer/engine.ts:2869 — rawPageBox (unrotated Crop/MediaBox)
- src/renderer/engine.ts:3027 — dispatch; :3172 fields case; :3729 annot-delete; :3755 redact-set; :3973 links; :3980 link-add; :4012 link-delete
- src/renderer/engine.ts:4291 — page.getObject() + getTransform inverse math (text-add)
- src/renderer/engine.ts:4634 — page-rotate; :4659 page-delete link repair (positional)
- src/renderer/engine.ts:4757 — refusal reply shaping (reason/detail/extra + journalState)
- src/renderer/engine-core.ts:71/:99 — at / attempt; :135 failField; :143 failRedact (add failLink); :151 failReason
- src/renderer/engine-core.ts:236 — OP_PERMISSION (add 'link-set': 'annotate'); :250 allowed
- src/renderer/redact.ts:79/:102/:166 — redactRect (clamp) / setRedactRect / walkObject (struct cap)
- src/renderer/content-edit.ts:1328/:1343 — editStructElement / editParentTreeNode (MCID only)
- src/renderer/worker.ts:31 — onmessage → engine.dispatch (no per-op routing)
- src/renderer/renderer.ts:2486/:2505/:4800/:5081/:5093 — renderer calls redact-set / fields / links / link-add / link-delete
- tests/p2-shell.test.mjs:158-177 — OP_PERMISSION / READ_ONLY / DEFAULT_EDIT pin
- tests/p2-nav.test.mjs:366-440 — link engine tests; tests/p4-redact.test.mjs:173-223 — redact-set tests
- node_modules/mupdf/dist/mupdf.d.ts:16/:24/:427/:566 — Matrix.invert / Rect.transform / Link (no getObject) / PDFPage.getObject,getTransform

---

# P6 U4 surface map — renderer (Opus 5.5 reader, 2026-09-24)
Scope: src/renderer/renderer.ts (7653 lines), index.html, styles.css, as of build/ today. Anchors are file:line. Read-only survey; nothing was edited.

## renderer-u4

**Summary.** No link can be clicked on the page today. Links exist only in the Inspector Links tab. `refreshLinks` (renderer.ts:4789) asks the worker for the shown page's `links` and fills `links: LinkEntry[]` (621) and `navLinks[page]` (627), then `renderLinks` (4892) builds the rows. A row's Go/Open button calls `followLink` (4951): a page link calls `goTo`, a web link calls `api.openLink`, which goes over the `open-link` IPC (main.ts:657). In drive/smoke mode that IPC only logs `P2_LINK` and sets `lastP2Link`; otherwise it shows a confirm box and then calls `shell.openExternal`. A link can only be added from a search hit, with `openLinkDialog` → `saveLinkDialog` → `addLink` (`link-add`, rect = `hitRect(currentHit())`). A link is deleted by its row index (`deleteLink`, `link-delete {linkId}`).

`paintHits` (2262-2313) never paints links. There are no click handlers on `#canvas` or `svg#text-layer` at all. The `#page` click handler (6947) handles only click-to-add text (Edit mode), the content-add baseline, and the text-box and note tools. So in Read mode a page click does nothing unless it lands on a field target: fields are painted and fillable in Read, Annotate and Forms.

Tab order is `readingOrder(fields)` (2008). It works on the current page's `FieldsReply.items` and ignores `/Tabs` and `/TI`; the reply carries neither.

U3 built the direct-manipulation kit and U4 reuses it: `startRectGesture` (2155), `gestureRect` (2046), `nudgeRect` (2386), `addFieldHandles` (2127), `placeBox` (2088), `markSelected` (2334), `restoreDirectFocus` (2346), `runGestureWork` (2377), and the `directClickWork`/`directGestureWork` promises that the drive awaits.

Plain arrows on a focused target are not handled. The `#page` keydown handler (7169) returns early for anything inside `#hit-layer`, and the object, mark and field handlers only act on ⌥-arrows. So the browser default runs and the `#viewport` scrolls.

Redact mode draws an area only while "Mark area" is armed. An unarmed click only drops the focus from the selected mark. An unarmed drag starts a native text selection.

### (a) Links — exact current behaviour
- `LinkEntry = {id, rect, kind:'page'|'uri'|'blocked', page?, uri?}` (engine.ts:615). `id` is the index in `page.getLinks()` order, not an annotId. `rect` = `link.getBounds()` (engine.ts:2502), y-down page pt, the same space as `FieldItem.rect` and the hit layer. So `placeBox(button, item.rect, shownZoom)` works unchanged.
- `refreshLinks` (4789-4808) is seq/generation/page guarded. It sets `links` and `navLinks[page] = linkSummary(items)` and calls `renderLinks()`. **It never calls `paintHits`**, unlike `refreshFields` (4277) and `refreshNotes` (4235 `repaint()`). Callers: 989 (open), 1530, 1729, 1814 (`goTo`, not awaited), 4009, 4047, and `applyEdit`.
- `linkSummary` (4811) maps to `{kind,page}` / `{kind,uri}`, with no rect and no id. `loadAllLinks` (4818) is used only by the nav smoke. STATE `nav.links` = `navLinks` (6320), keyed by page string.
- `linkLabel` (4880): "Page N", the URI, or "Blocked link (scheme:)". `followLink` (4951-4955): page → `goTo(page)`; allowed uri → `api?.openLink?.(uri)`; blocked → `navStatusEl` text. It sets no STATE (`navLast` is untouched).
- `renderLinks` (4892-4930): one `<li>` per link holding `<span>label</span>`, `button[data-action=go|open][data-link-id]` (none for a blocked link), and `button[data-action=delete][data-link-id]` (disabled when `!canAnnotate()`). The rows carry no aria-current and no selected state. The click delegation is at 6936-6946.
- `openLinkDialog(mode)` (5034) needs a `currentHit()` rect, otherwise it announces "Find something first". `saveLinkDialog` (5052) checks a page in 1..pageCount and a non-empty uri of ≤ MAX_URI_CHARS. `addLink(page, rect, target)` (5079) sends `link-add` and then `applyEdit`. `deleteLink(linkId)` (5091) sends `link-delete {page: wantedPage, linkId}`.
- `driveLink` (1153-1186) handles `link {action:'delete', index}` or `{kind:'page'|'uri', page|uri}`, the latter needing a prior find hit. It sets `navLast = {action:'link-add'|'link-delete', index?, detail?}` (703). `link` is not in `OP_MODE` (563), so it runs in whatever mode is current.
- DOM: `#tab-links` (index.html:229) and `#panel-links` (354-365). The panel holds `h2#links-heading` ("Links on page N"), `#link-page-add` (⇧⌘K), `#link-uri-add`, `p#links-empty` and `ul#link-list`. `#link-dialog` is at 727-734 (`#link-target` maxlength 2048). `MODE_TAB.read = 'tab-links'` (478), so Read mode shows the Links panel.
- The only mechanism that marks a row current is `syncNoteCurrent` (2339-2344). It looks for a `#note-list li` that contains a `[data-annot-id=<directFocusedMark.annotId>]` and sets `aria-current="true"` on it, removing it from the others. It is called from `dropDirectFocus` (2345), `restoreDirectFocus` (2375), `focusObject` (2420) and the mark click. Fields have no current-row mirror. Thumbnails use `aria-current="page"` (2911).

### (b) Fields Tab order
- `readingOrder(items)` (2008-2022) is exported and unit-tested (tests/p6-direct.test.mjs:37,43). It drops signatures, sorts by y then x, and groups items into rows when their vertical overlap is ≥ 50 % of the smaller height. Rows are ordered top to bottom and items left to right. A radio group appears once, as its checked kid if there is one, else the first kid.
- It has 6 call sites, and all of them must see the same order: 2270 (the radio roving tabIndex in `paintHits`), 2499, 2506 (reads a *foreign* page's `reply.items`), 2511, 2527, and 2543 (`directFieldIndex` in `activateField`).
- `focusNextField(reverse, fromPage, fromIndex)` (2497-2517) skips readOnly and signature fields. Within the page it moves by index. Past either end it scans the pages with wrap: for each page it sends `request({op:'fields', page})`, then `goTo`, waits for the paint, calls `refreshFields`, and activates the first or last field. It is called from the Tab key on a field target (2281), from an inline editor (`setupInlineEvents`, 2572), from `driveDirectKey` Tab (1269) and from `#field-next` (2111).
- `activateField(item, commit)` (2526-2544) sets `directFieldIndex`. For a checkbox or radio it toggles when `commit` is true and only focuses when it is false. Otherwise it opens `input|textarea|select.inline-editor` and sets `directActive = {kind:'field', ...}`.
- `FieldsReply = {id, ok, page, items: FieldItem[]}` (engine.ts:784). `refreshFields` (4257-4278) keeps only `fields = reply.items` and `fieldsPage = page`, so a new `tabs` key would be dropped unless it is stored, for example in `fieldsTabs`. At the end it calls `paintHits`.
- STATE `fields` = `fieldSummary()` (4466-4478): `{count, scripted, values, defs, last?}`. `defs` comes from `fieldDefs` (4482). It covers the current page only. `order` and `tabsOrder` can be added safely, since expects are subset matches.
- `#field-bar` (index.html:162-166) holds `#field-count`, `input#field-highlight` ("Highlight fields") and `#field-next`. `paintHits` sets `fieldBar.hidden = !doc || fields.length === 0` (2269). **`fields` holds the current page only**, so the bar disappears on pages without fields, and "This document has N form fields" actually counts the page's fields.

### (c) Direct-manipulation kit (reuse as is)
- `paintHits(width, height, page)` (2262-2313) removes every `.hit-target` and `.para-outline` and toggles `#hit-layer.live = layerLive()`. The targets it paints depend on `uiMode`:
  - read/annotate/forms: field buttons (2271-2283: `.hit-target[.highlighted][.required][.readonly][data-field-id]`, handles if `editable()&&permitted('edit')`, click → `activateField`), then signatures (2284-2291).
  - edit, when `contentScan.page === page`: `.para-outline`, then `paintObjectTarget` for each `objectTargetItems` (2317), then `.text-target` (2297-2310).
  - redact: `paintMarkTarget` for every `Redact` note (2312).
  - organize: nothing.
  - It ends with `restoreDirectFocus(refocus)` (2313).
  - **A link loop would slot in here, and DOM order decides which target is on top.**
- `layerLive()` (2112) is true in read, annotate, forms, edit and redact, as long as no tool is armed (note, freetext, ink, redactArea, contentAdd in edit), no `redactDrag` and no `inkStroke`. A new Link arm button must join this check and the tool map in `driveArm` (1357).
- `startRectGesture(event, {rect, id, handle, min, lockAspect, can, commit})` (2155-2186). Page pt come from the canvas box ratio. It uses a 3 px threshold and `gestureRect` for the rect, and listens on window for pointermove, pointerup and Esc. It sets `directGesture` and `directGestureWork` and restores `initial` when `commit` returns false or throws. `addFieldHandles(button, start)` (2127) adds a `.hit-grip` and 8 sides. Body drag is `pointerdown` on the button → `start(e, 'grip')`, as in 2400 and 2464.
- `placeBox(el, rect, zoom)` (2088) positions through `editorBox` (2023): CSS px = pt × zoom. `cssBox(el)` (6226) reads back `{left, top, width, height}` for STATE.
- Focus model: `directFocusedObject` (2098) and `directFocusedMark` (2099) are cleared by `clearDirectFocus` (2104), which `goTo` (1807) and `undoEdit`/`redoEdit` (5494) call. `markSelected(btn)` (2334) sets `.selected`; never use `.active`, which carries pointer-events:none. `dropDirectFocus` (2345). `restoreDirectFocus` (2346-2375) re-finds the object by kind+name+nearest rect, or the mark by `data-annot-id`, then re-selects and refocuses it. **A link focus (for example `directFocusedLink {annotId}`) needs its own branch here, in `clearDirectFocus`, and in a `syncLinkCurrent` modelled on 2339.**
- `paintObjectTarget` (2391-2417) and `paintMarkTarget` (2455-2481) are the templates for a `.link-target`: button, aria-label, title, placeBox, then (if permitted) handles + body drag. Click → focus + select + mirror the row + `directClickWork = Promise.resolve()`. Keydown: Esc → drop focus; ⌥-arrow → `nudgeRect` (⌥⌘↑/↓ return, since that chord is Move page); Delete/Backspace → delete; Enter → action. Each keydown runs through `runGestureWork` guarded by `!directGestureWork`. The commit mirrors `commitMarkRect` (2483): a single request, `noop:true` skips `applyEdit`, and a refusal shows `reply.error` in `#error`.
- Drive: `driveDirectClick` (1189-1227) does `goTo`, then waits for `shownPage`, `inFlight` and **`fieldsPage === page`** (1195); it never waits for links. It scrolls, hit-tests with `elementsFromPoint(x,y)[0]`, and dispatches pointerdown, pointerup and click before awaiting `directClickWork`. `driveDirectGesture` (1228-1261) with `op:'drag'` targets **only a `#hit-layer` node under the point and returns silently when there is none** (1236-1238). It fires pointermove and pointerup on `window`, then awaits `directGestureWork`. `handle` looks up `[data-handle]` inside the `.hit-target`. `driveDirectKey` (1262-1296) forwards keys for a focused `.object-target, .redact-target` (1272), a list a `.link-target` must join. It handles Alt-arrows for fields (1278-1281) and Tab (1269).
- STATE: `directStateActive()` (6229-6252) returns null when `!layerLive()`. Its order is add, object, redact, then the directActive editor, then the focused field. `editSummary().direct` (6307-6316) is `{mode, active, gesture, highlight, targets, objects, marks, status}`. `targets` counts fields in read/annotate/forms (tests/drive/direct-edit.jsonl:36,39 and direct-edit3.jsonl:46,51 pin it), so links need a separate `direct.links` count.

### (d) Arrow keys today
- `#page` keydown (7169-7179): ignores any target inside `#hit-layer` (7171). PageUp/← → previous page, PageDown/→ → next page, Home/End → first/last page. ↑/↓ are not handled (native scroll).
- Object keydown (2404-2416) and mark keydown (2468-2478): only `event.altKey` arrows act (1 pt, ⇧ 10 pt, ⌥⌘←/→ = width). A plain arrow reaches `else if (key !== ' ') return;` with no preventDefault, so the browser scrolls `#viewport`.
- Field button keydown (2281): ⌥-arrows → `nudgeField` (2143, `field-props`, 8 pt minimum). Radio ←/→ → `moveRadio` (2518), which conflicts with a plain-arrow nudge on radios. Tab → `focusNextField`. Space toggles. Other plain arrows scroll natively.
- Inline editors (`setupInlineEvents` 2572-2579): plain arrows move the caret; ⌥-arrows nudge fields only.
- Drive `key` with a plain arrow on a focused object or mark sends a synthetic keydown (no default action), so nothing happens today. On a focused radio it calls `moveRadio` (1270).
- `document` keydown (7180) handles ⌘/Ctrl chords only.

### (e) Redact mode
- DOM: `#context-redact` (index.html:149-156) holds `#redact-area[aria-pressed]` ("Mark area"), `#redact-search-text`, `#redact-search-go`, `output#redact-count` and `#redact-apply-open`.
- `redactAreaButton` click (6822-6832): keyboard activation (`detail 0`) → `addRedactArea(wantedPage, shapeBox())` (5255: the search-hit box, else a box at the page centre). A pointer click toggles aria-pressed, announces, and calls `paintHits`, which takes the layer out of live because `layerLive` is false while armed. `cancelPendingTools` (3955-3964) disarms it and clears `redactDrag` on any mode change.
- Armed drag: `#page` pointerdown (6971-6979) sets `redactDrag = inkPoint(e)` (clamped, 5275) and calls `setPointerCapture`. `finishInk` on `#page` pointerup/pointercancel (6999-7007) disarms and calls `addRedactArea(page, [start, end])` (3779). There is **no threshold and no rubber-band preview**. A plain click sends a degenerate rect, and the engine refuses it with `invalid` (redact.ts:91, clamped <1×1 pt), which shows `#error` and sets `redactLast.ok=false`.
- Not armed: pointerdown only sets `addBlocked`. The click reaches `clickToAdd` (2645), which calls `dropDirectFocus()` when a mark or object is focused (2648) and returns false because the mode is not edit. Nothing else happens. A drag on empty space gets native SVG text selection (`.text-layer text {cursor:text}`, styles.css:546).
- Drive: `driveRedact` (3875) takes `redact {page, rect}` or `{search}`. `arm {tool:'redact'}` presses the button (1357). A drive `drag` on empty space is a no-op (see (c)). The `#page` pointerup listener never sees drive's window-dispatched pointerup, so **the unarmed drag must listen on window (like `startRectGesture`), and `driveDirectGesture` must fall back to the `#page` element under the point**.

### (f) Help
- `helpRows()` (1440) = `#help-matrix tbody` row count. STATE `help: {open, rows}` (6278). **Rows = 11 is pinned** (tests/drive/help-fields.jsonl:4-5, tests/p3-drive.test.mjs:162, tests/p2-shell.test.mjs:126), so U4 text goes into existing cells, never a new `<tr>`.
- "Direct editing" row (index.html:543-547). Its can-do cell includes "⌥-arrow nudges focused fields" and the U3 object/add-text/mark sentences. Its ceiling cell ends with "links are not yet editable on the page". "Direct editing ceilings" `<ul>` (index.html:555-556) starts with "Tab order follows the visible reading order; Tab continues onto the next page with fields and wraps at the end." `#context-edit` hint (index.html:146-148): "Click a supported text run to edit it in place."

### (g) Toggles and CSS
- `highlightFields` (2108-2110) reads `localStorage['field-highlight']` inside try/catch (default true). On change it writes the value and repaints, and the value is reported as STATE `direct.highlight`. Only field buttons get `.highlighted` (2273). An outlines toggle would copy this pattern with its own key, for example `edit-outlines`.
- styles.css: `#field-bar` (516-526, flex, `[hidden]` wins through the `!important` at 33). `#hit-layer` (552-557, pointer-events none, z-index 1). `#hit-layer:not(.live) .hit-target {pointer-events:none}` (558). `.hit-target` (559-566, transparent 1 px border, `cursor:text`). Hover and focus-visible outline `#6558d8` (567). `.active` (573). `.object-target`/`.redact-target` `cursor:move` (576). Redact red outline on hover, focus or `.selected` (577). `.selected` (578-579, shows handles). Handles (580-592, `display:none` until hover, focus-visible or `.selected`).
- An always-on outline fits in one rule plus one class toggle: in `paintHits` add `hitLayer.classList.toggle('outlines', uiMode === 'edit' && showOutlines)`, then `#hit-layer.outlines .object-target, #hit-layer.outlines .text-target:not(.unsupported) { outline: 1px dashed #6558d8; }`. The `.selected` and `:hover` rules come later in the file and keep their solid outline. A link could use `.link-target { cursor: pointer }` in Read mode (the default `cursor:text` is wrong for a link).

### (h) Modes
- `UiMode` = read | annotate | organize | forms | edit | redact (renderer.ts:161; radios at index.html:57-68; `modeInputs` 446; `DIRECT_MODES` 1337). `setMode` (3970-3986) drops the editor, sets the radios, shows `#context-<mode>`, calls `cancelPendingTools`, selects the `MODE_TAB` tab (475-482: read→links, annotate→notes, organize→text, forms→fields, edit→text, redact→notes), refreshes content targets in edit, then calls `paintHits`.
- What the hit layer shows per mode:
  - Read: fields (fillable); context is only a hint (index.html:73-75).
  - Annotate: fields plus markup and note tools (77-108).
  - Organize: no targets.
  - Forms: fields plus `#cert-sign-open`.
  - Edit text: objects, text and paragraphs.
  - Redact: marks.

### constraints
- STATE shape pins: `nav.links` items are **deepEqual'd** as `{kind,page}` / `{kind,uri}` (tests/p3-drive.test.mjs:315-327). Adding `rect` or `annotId` to `linkSummary` breaks them, so put the new data in a separate `links[]` or `direct.links` slice. Expect is a subset match with exact array lengths (main.ts:1658-1663). Do not change `direct.targets`.
- Help rows stay 11 (see (f)). Help and #error wording must match the refusal tokens (§0).
- No new renderer .js file (policy.ts asset map). `readingOrder`, `gestureRect` and any new pure helper (`tabsOrder`, `snap`) stay exported from renderer.ts and are tested in a child process (tests/p6-direct.test.mjs:32).
- Every new click or gesture path must set `directClickWork` or `directGestureWork`. `driveDirectClick` must also wait for a `linksPage === page` guard, as it does for `fieldsPage`, or a click right after `goto` misses the link targets.
- Link permission is `canAnnotate()` (the Delete button at 4924, and syncControls 1906-1909). Field and object gestures use `canEdit()`.

### risks
- Read mode already paints fillable field targets. Link targets added in Read (follow) and Annotate (edit) would overlap fields where a link covers a widget, so DOM order decides which one wins. A link over text also blocks text selection there. Link-following in Read must not open `api.openLink` while the user is dragging, so use the 3 px rule.
- `refreshLinks` does not repaint and runs unawaited from `goTo`. Links must trigger `paintHits` and keep the seq guard, or the targets stay stale after undo, page moves or `link-set`.
- LinkEntry `id` is an index into getLinks(). Rows and `deleteLink` key on it. After `link-set` or `link-add` the order can shift, so focus restore and the row `aria-current` need a stable `annotId` from the engine.
- A plain-arrow nudge must not take ←/→ from radios (`moveRadio`) or caret keys from editors. It needs `preventDefault`, or the viewport also scrolls. Decide whether fields nudge with plain arrows in Read/Forms (fill) modes at all.
- The outlines toggle cannot live in `#field-bar` unchanged: that bar is hidden on any page without fields (2269). Either decouple its `hidden` in Edit mode or put the checkbox in `#context-edit`.
- Unarmed redact drag starts on the text layer, which also selects text. It needs `preventDefault` on pointerdown after the threshold and the window listeners. A plain click must stay deselect-only, with no `invalid` refusal.
- Drive `drag` with no hit-layer node silently does nothing (1236-1238). Any "drag on empty space" expect needs a harness change in the renderer's `driveDirectGesture` (renderer owner) and an expect that fails on a miss (§9.12).
- `followLink` sets no STATE. A Read-mode click needs a truthful slice, for example `navLast = {action:'follow', kind, page|uri}`. `lastP2Link` lives only in main's final report.

---

# P6 U4 surface map: drive, tests, fixtures (Opus 5.5 reader, 2026-09-24)
Read-only map for the U4 contract (handoff-20260924-next.md §2 A1–A4). Anchors are file:line on the current tree; renderer lines moved since maps-p6-unit3.md. I checked fixture facts with a mupdf scan of 52 PDFs (5 encrypted ones skipped) and by overlapping every drive gesture with the engine's `links` reply.

## (a) Drive runner
- `scripts/drive.mjs:15-28` packDrive rewrites only lines whose cmd is in `click|type|key|drag|handle` (:22) and that have no `contents` (:21). It moves `page x y text key alt dx dy handle escape shift` into `contents` (:25). Other keys (`target`, `uri`, `tool`) must be written inside `contents`. `alt` is already moved on `drag`, but the renderer reads only `shift` (renderer.ts:1246), so handoff A3's "add alt to the drag whitelist" is a renderer change (U5), not a drive.mjs change.
- `scripts/drive.mjs:44-48` spawns the real Electron binary: `--user-data-dir=<tmp>/udata . --drive=<packed> --drive-out --report [--gui]`. It prints the report JSON and passes through the exit code.
- `src/main.ts:1658` `matches()` is a subset match: arrays need exact length, primitives use `===`. `:1665` driveSecrets covers password and encrypt*. `:1679` startDrive: `${OUT}` is replaced (:1683), `#` and blank lines are dropped (:1684), a secret in STATE aborts (:1698), `nextState` has a 60 s timeout (:1703-1706), and the run waits for hello.pdf P1_RESULT first (:1709).
- `src/main.ts:1719-1721` `to`, `from` and `images` persist into later steps. `:1723-1731`: `open` → deliverOpen, then `save-to`, `save-in-place` (no fixtures/ guard), `print-to`, and `wait` (sends op `state`). Anything else is sent as `edit-request {op, contents: JSON string}`. `:1732` step timeout, `:1735-1738` expect, `:1742` finish.
- `src/main.ts:657-664` under `--drive`, `open-link` only logs `P2_LINK {uri, allowed}` and never opens a browser. STATE has no trace of it.

## (b) Renderer drive registry (src/renderer/renderer.ts)
- `OP_MODE` (563-575) switches the UI mode before the op runs (931-939): `fields`/`field-*`/`form` → forms, `redact*` → redact, `content*`/`text-add-open`/`content-edit` → annotate. `link`, `bookmark`, `click`, `key`, `drag`, `handle` and `goto` have no entry and keep the current mode. `driven()` (956) runs requests one at a time and emits one STATE line per request.
- `runEditRequest` 1363-1423. Relevant entries: `fields` 1387 → smokeFieldDesign 4141 (add/props/delete/fill by name); `link` 1412 → driveLink 1153; `redact` 1417 → driveRedact 3875 (`{page, rect y-down}` or `{search}`); `mode` → driveMode 1340; `arm` → driveArm 1353.
- driveLink 1153-1187 has two shapes. `{action:'delete', index}` deletes by index into the `links` reply. `{kind:'page'|'uri', page|uri}` puts the new link on the rect of the current find hit and refuses with `no-anchor` when there is no hit. The result goes to `navLast`. There is **no `set` action and no rect or annotId input**. driveArm's tool map (1357) has note, freetext, ink, redact, text and off, and **no `link`**.
- driveDirectClick 1189-1227 targets `elementsFromPoint[0]` (1206), the top-most element, and fires synthetic pointer events and a click. A press outside the hit layer focuses `#page`, which commits any open editor.
- driveDirectGesture 1228-1250 picks a hit-layer element at the point, else a `.hit-target` whose rect contains it (1235-1236); `handle` then looks for `[data-handle]` inside it (1237). **On a miss it returns silently** (1238). Moves and pointerup fire on `window`. startRectGesture (2155-2186) listens on window with a 3 px threshold.
- driveDirectKey 1262-1296: Modifier prefixes must come in the order Alt+, Meta+, Shift+ (1265).; Plain ArrowLeft/Right on a focused radio → moveRadio (1271).; Tab commits the editor, then focusNextField (1272).; Keys on a focused `#hit-layer .object-target, #hit-layer .redact-target` are sent as keydown on that target (1274-1278).; Alt+Arrow → nudgeField (1280).; **Enter with nothing active is swallowed** (1286-1287).
- Real keyboard handlers: Field buttons 2281 handle Tab, radio arrows and Alt+arrows.; Object (2403-2412) and mark (2468-2477) handlers return early on a plain arrow.; `#page` (7169-7178) flips pages on arrow keys, PageUp/PageDown and Home/End, except when the event target is inside the hit layer.
- readingOrder 2008 (exported) orders fields by row, then x. A radio group collapses to its checked kid; signature fields are dropped. focusNextField 2497-2517 skips readOnly fields and wraps across pages by calling `{op:'fields', page}`.
- STATE is built by stateSummary 6254 and editSummary 6292.; `fields` comes from fieldSummary 4466 and holds count, scripted, values, defs and last.; `direct` holds mode, active (6229), gesture, targets, objects, marks and status.; `nav.links` maps the shown page to `[{kind, page|uri}]` (linkSummary 4811).; `redact.rects` is a list of `{id, rect, search}`.

## (c) Test harnesses
- `tests/p6-drive3.test.mjs`: Setup: drive() 18, savedBytes 31, at/pick 35-45 (first step matching cmd + dotted keys), DIRECT3_FIXTURES 47, near 54, object() 58. The test runs with `{skip: CI_NO_ELECTRON, timeout: 180_000}` and drive(…, 150_000).; Pins: 79 steps (64), 15 clicks (65), 0 red steps (66-67).; Index-based lookups: nudge1/nudge2 are the first two Alt+ArrowRight (104). markNudge is the **third** Alt+ArrowRight in the whole run (136). The second pass starts at findLastIndex of the reopen (152). Fixture hashes are checked before and after (169-171).
- `tests/p3-drive.test.mjs` has its own drive() at 17.; Index-based: field-design byStep(1..22) (129-155), redact steps[3]/[7] (184-190), help-fields steps[1] (162).; Count-pinned: direct-edit2 50 steps and steps[2..49] (383-422); direct-edit 75 steps and 19 clicks (424-441).; deepEqual on whole values: help-fields defs/values (166, 179-181); links-bookmarks nav.links (315, 317, 319, 327) and nav.outline (320-323). DIRECT_FIXTURES is at 378.
- `tests/p6-direct.test.mjs` runs renderer.js in a child process with `P6_HELPERS=1`.; The DOM is a Proxy (10-31): window = globalThis, Worker and ResizeObserver are stubbed, localStorage returns null. It imports named exports (32, 51).; readingOrder cases are at 33-45, gestureRect at 63-70.; It is one node:test (74-79). A new top-level browser API such as matchMedia crashes the child.
- `tests/p2-nav.test.mjs`: Harness: createEngine 25, buildPdf 32, open(id) = `fixtures/<id>.pdf` 47-55, op 57, linksOf 66, reopen 72.; links: `id` is the index into getLinks, rects are y-down (366-385).; link-add: revision goes 1 then 2, undo is 'add link'/'delete link', delete takes `linkId` (387-415).; Refusals are matched by regex (417-459): /off the page/ /positive area/ /4 finite numbers/ /0\.\.2/ /file:/ /javascript:/ /scheme/ /2048/ /target must be/ /no link 9/.
- `tests/p2-forms.test.mjs` has its harness at 20-58 and richFormPdf at 86. `tests/p3-forms.test.mjs` has its harness at 22-77, rawFacts 79, buildPdf 163 and hierarchyPdf 177. Neither deepEquals whole engine items, so new keys on items are safe.
- Other pins: `tests/fixtures.test.mjs:7-12` PINNED_SHA256 lists redact, redact-shared-only, edit-tj and p6/direct3; each is read as `fixtures/${name}` (53-56).; `tests/p1-worker.test.mjs:40` checks provenance with a **non-recursive** readdir of fixtures/.; `tests/p2-shell.test.mjs:157-175`: every renderer `op: "x"` literal must be in OP_PERMISSION (engine-core.ts:236, links at 242), READ_ONLY or DEFAULT_EDIT.; `tests/p2-shell.test.mjs:126` pins **11 help rows**.

## (d) Fixtures
- `fixtures/minipdf.py`: stream 10; pdf_bytes 15 (1-indexed objects, classic xref, no /ID or date); emit(path, objects) → (len, sha256) 34; sha256_file 41.
- `fixtures/make-nav.py`: link(parent, rect, action) 28 writes `/Subtype /Link /P /Rect /Border [0 0 0] /F 4 /A`; GoTo `/D [4 0 R /Fit]` 35; URIs 36-37.; Needle asserts 65-75. The provenance JSON (77-125) gives link rects **y-up**.
- `fixtures/p6/make-direct3.py`: HERE and sys.path set-up 17-19; PLACEMENTS 25; box() 48-56 and run() 59-62 give y-up, y-down and click points.; The provenance record 65-107 is the template for new fixtures: blocks, generator{path, helper, sha256, deterministic, reproduce}, provenance, license, generated_on.; It asserts a single %%EOF and no /Prev (133). Page 1 is `/Rotate 90` (119), the **only /Rotate page among all fixtures**.
- `fixtures/rotated.pdf` is **not** a /Rotate page: make-rotated.py:18-22 rotates text with Tm on an upright page.
- form.pdf is bundled (policy.ts:46, build.mjs:12) and its provenance is hand-kept, so do not regenerate it. form2's `/Annots` order (make-form2.py:53) equals its row order.
- **No fixture has /Tabs or /TI.** StructTreeRoot appears only in p0b/edit-tagged, redact and p0b/redact.
- Existing links: nav.pdf page 0 has a page link at [72,62,200,112] and a uri link at [72,162,300,192] (y-down). Page 2 has the same two rects, with the uri one blocked (file:).

## (e) Drive JSONL conventions
- Each script opens with a `#` header giving the fixture, a sha prefix, y-down centres (y_down = 792 − y_up) and the exact-expect rule (direct-edit3.jsonl:1-7).
- Keys sit at the top level beside `expect`. For a second pass, reopen the repo fixture, which resets revision to 0 (direct-edit3.jsonl:71-72).
- An area mark is `{"cmd":"redact","contents":{"page":0,"rect":[300,600,400,650]}}` (y-down), then targeted at its centre.
- `links-bookmarks.jsonl` (15 steps) runs on hello.pdf: a no-anchor refusal (7), find (8), link add (10-11), delete index 0 (13), bookmarks (14-18), save-to, reopen and wait (19-21), then nav.pdf bookmark go (23-24).
- `field-design.jsonl` is index-pinned through step 22.

## (f) GUI check `.quad/shared/p6/checks/shot-direct3.mjs`
- Setup: root from import.meta.url (15); udata at review-out/u3-udata, removed first (18, 95); electron resolved via createRequire (19). Helpers: freePort 24, Cdp 41-73, attach via /json/list 74-85.
- Probe: open, redact, then hold(40, 5000) (98-102). Launch flags: `--remote-debugging-port --remote-allow-origins=* --user-data-dir . --drive --gui --drive-out --report` (104-105). The window is set to 1280×900 (110).
- Input: a drag is a dispatchMouseEvent press, 8 moves, then release (117-130). **pagePoint assumes a 612 pt page width** (134), which is wrong on a /Rotate 90 page. It hovers before using handles (143-149). Mode chips are pressed with eval (155, 195). Meta+Enter is modifiers 4 (185).
- Teardown: finally → SIGTERM and a hash check (210-215). `checks/fixtures.sha256` covers form2, p0b/edit-transformed, p6/paragraph and p6/direct3. u4-hash-before.txt (50 files) equals u3-hash-after.txt.

## Steps that break if Tab order, arrow keys or on-page link targets change
- Tab order: direct-edit.jsonl:26-28 `{"cmd": "key", "key": "Tab"}` goes name→notes→agree→size on direct-form.pdf; :29 `"Shift+Tab"` goes back to agree.; :53 `Tab` goes from edge (nav p0) to next (p1). :88 `Tab` wraps to edge (page 0, revision 2).; direct-edit3.jsonl:91 `Tab` commits the add editor (active null, revision 4).
- Plain keys inside an open editor: direct-edit.jsonl:83 `"ArrowRight"` and :84 `"End"` expect page 0 and edge's draft to stay `P0`. These must not flip the page or nudge.
- Alt nudges: direct-edit2.jsonl:11 `Alt+ArrowRight`, checked via steps[9] = [83,80,273,104].; direct-edit3.jsonl:31-32 `Alt+ArrowRight`, :33 `Alt+Meta+ArrowRight`, :61 `Alt+ArrowRight` on a mark, :98 `Alt+Shift+ArrowRight`.; Plain arrows on a radio stay moveRadio (1271, 2281).
- Link overlap: seven Forms-mode steps on nav.pdf fall inside its GoTo link [72,62,200,112]: direct-edit.jsonl:51 click (162,84), :81 click (200,84), :87 click (200,84);; direct-edit2.jsonl:4 handle (160,84), :7 handle (172,92), :10 click (177,92), :15 click (178,92).

## Risks the contract must decide or guard
1. Empty-space drags cannot be driven. driveDirectGesture silently misses (1238), and the armed area drag (6971-7006) uses #page listeners plus setPointerCapture, which a synthetic pointer cannot satisfy. Drawing a new link or an unarmed redact rect needs a #page fallback with window-listener gestures, or a new `draw` command.
2. `.link-target` must carry `.hit-target` (1236) and join the key selector at 1274. Otherwise Enter, Delete and arrows on a focused link are swallowed (1286-1287). `arm` also needs a `link` tool (1357).
3. Link targets must not cover field targets in Forms mode (the seven overlap steps). Decide the z-order and which modes paint link targets.
4. `link` has no OP_MODE entry. Adding `link: 'annotate'` changes the mode inside links-bookmarks.jsonl, which has no mode expects. Decide this explicitly.
5. STATE shapes compared with deepEqual are frozen: nav.links entries, fields.defs, fields.values and fields.last. Put annotId, rect, tabs, ti and order in **new** keys, for example `links[]`, `fields.order` and `fields.tabsOrder`.
6. The p2-nav refusal regexes must still match any reworded link-add errors.
7. A link's `id` is its getLinks index, which shifts after add or delete. Define `annotId` as the stable key.
8. Help rows are pinned at 11 in p2-shell:126, p3-drive:162 and help-fields.jsonl:4-5. Put U4 help in prose, or change all three under one owner.
9. A new op such as `link-set` must be added to OP_PERMISSION, or p2-shell:173 fails.
10. Scripts and tests pinned by step index or step count break on any inserted line. Add direct-edit4.jsonl and a p6-drive4.test.mjs with its own copied helpers.
11. form2's /Annots order equals its row order, so a /Tabs fixture must use an /Annots order that differs from both the row and column orders. New fixtures under fixtures/p6/ must be added to PINNED_SHA256 and fixtures.sha256, and need their own provenance test, because p1-worker's scan is non-recursive.
12. A drive miss and a uri follow both leave no trace in STATE. Mirror them in `navLast`, and give every handle step an expect that fails on a miss.
13. shot-direct4 must scale by the page's real width, not 612, for rotated pages.
14. The p6-direct child stubs the browser, so no new import-time browser APIs. localStorage is null in the child and udata is fresh in each drive run, so any outlines toggle needs a deterministic default.

## Open questions
- Which modes paint link targets, and should they sit above or below field targets?
- Should drawing on empty space extend `drag`, or be a new `draw {page, rect}` command?
- For /Tabs /S, should tabs.pdf carry a StructTreeRoot with /StructParent on its widgets, or should the contract only prove the fallback?
