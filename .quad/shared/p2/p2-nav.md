# P2 unit 5 — bookmarks and links (ds2): gate numbers, shapes, tests, limits

Scratch: `/tmp/nav-gate.pdf`, `/tmp/nav2.pdf`, `/tmp/nav3.pdf`, `/tmp/nav4.pdf`, `/tmp/nav5.pdf`
(pypdf 6.18.1) and `/tmp/form-gate.pdf` (pypdf, 2 pages, 3 text fields), measured with
`node_modules/mupdf@1.28.1` in Node (`.stage/gate-u5*.mjs`). `fixtures/nav.pdf` had not landed
when the gate ran; the shapes below are the contract's fixture shape, built independently.

## Gate — journal coverage (contract A.1). STOPPED ONLY IF A PRIMITIVE IS NOT JOURNALED; nothing was.

| Primitive | Journaled? | Step name | Undo / redo |
|---|---|---|---|
| `OutlineIterator.insert` | yes | `Insert outline item` | restores |
| `OutlineIterator.update` | yes | `Update outline item` | restores |
| `OutlineIterator.delete` | yes | `Delete outline item` | restores the whole subtree |
| `PDFPage.createLink` | yes | `Create Link` | restores |
| `PDFPage.deleteLink` | yes | `Delete Link` | restores (redo deletes again) |
| `/AcroForm /Fields` array `delete` inside one `beginOperation` | yes | outer op name | restores the removed field |
| page-tree `/Annots` array edit (fallback) | yes | outer op name | restores; needs a page reload to show in `getLinks()` |

Journal step names are ours: `mutate()` opens the operation and `deletePage`/`createLink` begin
their own inside it, which mupdf folds into the single outer record (unit 3 measured the same).

## Gate — the traps that shaped the code

1. **`item()` reports no children.** `it.item()` returns `{title, uri, open}` only; there is no
   `down`/`hasKids`. `down()` on a leaf returns `ITERATOR_AT_EMPTY` (1) *and moves the cursor to
   empty* (`item()` → null; `next()`/`up()` then return `ITERATOR_DID_NOT_MOVE`). So a blind
   preorder walk with `down()` is impossible. `Document.loadOutline()` returns the whole tree
   (`title`, `uri`, `open`, `page` 0-based, `down` children) and is the **guide**: the `outline`
   listing comes from it, and `outline-set`/`outline-delete` seek with the iterator only where
   the guide says a child exists.
2. **`insert()` inserts before the cursor and does not move it.** A fresh iterator sits on the
   first root item; `insert()` there prepends. To append a top-level entry, walk `next()` to
   `ITERATOR_AT_EMPTY` (one call past the last root) and insert there — measured: `A, B, C`
   appends in order, `insert` returns 1 and the item lands last.
3. **`update()` keeps siblings and children** (measured: renaming a parent left its child in
   place) and **`update({uri: undefined})` clears the destination** (the entry reports no uri
   afterwards). `delete()` takes the subtree with it (parent + child both gone; one undo
   restores both).
4. **`resolveLink` retargets, it does not dangle.** `createLink(rect, "#page=3&view=Fit")` writes
   `/A << /S /GoTo /D [pageref /Fit] >>`; deleting the page whose slot that object held makes
   `resolveLink` report the page that **took the index over** (measured: p3's GoTo → page 1
   became page 0 after deleting page 1; the raw `/Dest` still names the deleted page object).
   A numeric `/Dest [2 /Fit]` on a 2-page file resolves to the stale index 2, not -1. So
   **every reference must be measured before `deletePage` runs**, which is the order the repair
   uses (links, then outline dests, then `deletePage`, then the phantom fields).
5. **`file:` is not a plain external link.** `createLink(rect, "file:///etc/hosts")` writes
   `/A << /S /GoToR /F <filespec> /D [0 /XYZ null null null] >>` — a remote-file action. The
   `link-add` allowlist (`http:`, `https:`, `mailto:`) refuses it before the mutation.
6. **stderr proof.** Reproduced: a surviving link naming the **last** page, that page deleted,
   then the next page-tree load → `format error: cannot find page 3 in page tree` on stderr
   (`/tmp/nav5.pdf`). Deleting a middle page is silent. So the stderr test deletes the last page;
   a middle-page delete cannot prove the repair.
7. `getLinks()` order is `/Annots` order and stable across calls and reloads in one session.
   `getBounds()` is y-down page points, the same space as search quads and `setRect`
   (PDF y-up `[72 680 200 730]` reads back as `[72 62 200 112]`, and a `createLink` rect passed
   in that space saves to the same PDF rect). `isExternal()` is a scheme regex, not an allowlist.
   One more quirk: `deleteLink` of a link created **on the same live `Page` object** records no new
   journal step (measured: create → position 1, delete from that object → still 1, steps `["Create Link"]`);
   deleting from a reloaded page records `"Delete Link"` (position 2). The engine always loads the
   page fresh in `link-delete` and in the repair, so it is never in the same-object case; the gate test
   reloads the page to measure what the engine actually does.
8. The mupdf **default export has no `LinkDestination` class** (`import mupdf from "mupdf"`;
   only the namespace import has it) — but `doc.formatLinkURI()` accepts a plain object with the
   same fields, so the engine passes `{type:'Fit', page, chapter:0, x:0, y:0, width:0, height:0, zoom:0}`.
9. `/AcroForm /Fields` phantom, measured on `/tmp/form-gate.pdf`: deleting the only page of
   field `only` leaves field 8 in `/Fields` with no widget anywhere (`keep` and `both` survive;
   `both` keeps its surviving widget). Field ids 6/7/8, widget ids 9..12 before, 9/10 after.

## Shapes (as landed)

`schema`: no new dependencies, no PDF JavaScript, read-only rule unchanged.

- `{op:'outline'}` → `{ok, items:[{id, title, depth, page, uri?}]}`, preorder, `id` = preorder
  index; `page` is `resolveLink(item.uri)` when it is a valid page index, else `null`, and `uri`
  is given only when the entry has a destination that is not a page.
- `{op:'outline-add', title, page}` (append top level, `Fit` dest) / `{op:'outline-set',
  outlineId, title}` / `{op:'outline-delete', outlineId}` — one journal operation each
  (`add bookmark`, `rename bookmark`, `delete bookmark`).
- `{op:'links', page}` → `{ok, page, items:[{id, rect, kind, page?, uri?}]}`; `id` = index in
  `getLinks()` order; `kind:'page'` when resolution yields a page index, `'uri'` only for
  `http:`/`https:`/`mailto:`, `'blocked'` otherwise with the raw `uri` (so the panel can name the
  scheme).
- `{op:'link-add', page, rect, target:{page}|{uri}}` / `{op:'link-delete', page, linkId}` — one
  journal operation each (`add link`, `delete link`).
- **Deviation, forced by the seam:** the entry/link identifier cannot be `id` on the request,
  because `id` is the worker request id on every op (the renderer sets it). So `outline-set`/
  `outline-delete` take `outlineId` and `link-delete` takes `linkId`, the same convention unit 2
  (`annotId`) and unit 4 (`widgetId`) already use. The reply keeps `id` = the entry/link id.

## Checks run

- `npm test` → **122/122**. Baseline 106 + 13 `tests/p2-nav.test.mjs` + grok's 3 (`fixtures/nav.pdf`
  `--outline`/`--links`, `--edit=nav --save-in-place`, `--edit=nav` + undo in `tests/p1-save.test.mjs`).
- The 13 engine tests cover: the journal gate (three primitives, undo/redo); the nav.pdf listing
  (4 entries preorder + depths + pages, then 5 after an append) and `outline-set`/`outline-delete`
  with undo/redo (rename keeps the child, delete takes the subtree); an external bookmark listing its
  `uri` with `page:null` (and a JavaScript action listing neither, never followed); links listing (kinds, rects,
  blocked `file:`); `link-add` page + uri with undo/redo and `link-delete`; 22 refusals that leave the
  document clean; the `page-delete` repair (dangling dests → `page:null`, links gone on both surviving
  pages, one undo restores every destination and both links); the phantom-field repair on a
  hand-built 2-page/3-field scratch (removed on delete, restored by undo, removed by redo); the stderr
  proof (a fresh process loads the page tree: the unrepaired scratch *warns* `cannot find page 3 in page
  tree`, the repaired one prints nothing); save → validate → reopen keeps the smoke's outline and links;
  hello.pdf lists nothing and the five mutations refuse a signed/certified/empty-password read-only
  document; large.pdf `outline`/`links`/repaired `page-delete` cost.
- `U5_LARGE {"outlineMs":0,"linksMs":0,"deleteMs":3.3}` on every run (500 pages; the repaired delete
  adds one page walk to the unit-3 cost of 3-4 ms).
- grok's Electron smoke drives the renderer end to end and passed with this code: `P2_EDIT.nav` satisfies
  `main`'s `navEditOk`, the saved file passes pypdf `--outline`/`--links`, the mode is preserved, and
  `P2_LINK {"allowed":false, "uri":"file:..."}` appears for the blocked link.

## Renderer notes (for the reviewer)

- One `#nav-status` (`role="status"`) line carries all five announcements from the contract; a bookmark
  announcement is visible in the Links panel, which is where the contract lists the line.
- The outline is document-wide, so it is refreshed on open/unlock, every edit and undo/redo, and every
  page op (`applyEdit`/`applyPageEdit`, awaited, so the smoke's `P2_EDIT.nav` is never stale). `goTo`
  refreshes the page-scoped panels (links, notes, fields); it does not re-fetch the outline, which
  navigation cannot change.
- A blocked link has **no** action button; the one place `openLink` is called with a blocked URI is the
  `--edit=nav` smoke hook (`smokeNav`), which the contract asks to activate it for the `P2_LINK`
  `allowed:false` line. Main re-checks the allowlist, so nothing opens.
- `link-add` refuses `file:` before the mutation because mupdf would otherwise write a remote
  `/GoToR` action with a file specification (`createLink(rect, "file:///etc/hosts")` → `/S /GoToR`),
  which is exactly the action class this app never follows.
- Deviations, both stated in "Shapes" above: the request keys are `outlineId`/`linkId` (the seam owns
  `id`), and a blocked link carries its raw `uri` so the row can name the scheme.

## Gate scripts (scratch, `.stage/` is gitignored)

`.stage/gate-u5.mjs` … `gate-u5o.mjs` (journal coverage, cursor semantics, `down()` on a leaf,
`createLink` shapes, `resolveLink` across a delete), `.stage/scenario.mjs`/`scenario2.mjs` (the stderr
reproduction), `.stage/make-nav-gate.py`/`make-form-gate.py` (the pypdf scratches),
`.stage/try-nav.mjs`, `try-form.mjs`, `try-large.mjs`, `try-smoke-nav.mjs`, `try-numeric2.mjs`
(engine end-to-end before the tests existed). `.stage/dump.mjs` prints an outline tree and a page's links.
