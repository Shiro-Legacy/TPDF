# P6 U7 objections — engine lens (Opus 5.5, 2026-09-24)
Scope: contract-p6-unit7.md D1, D2, D5 (engine half), §0 permission entries, §1.1–§1.5, §7. Read-only on src/tests/fixtures. Probes in `.quad/scratch/p6u7/objections/engine/`: `graft.mjs` (→ `graft.out`: a scratch `new mupdf.PDFDocument()` clipboard; objects6 Im1/Im2/Im4/Fm1 and edit-objects Fm2/Im2 grafted in, both sources closed, grafted back into edit-objects p1 as `PA1..PA6`, one new `/Contents` stream of U6 blocks, rescan, ink, save→reopen, undo; plus a page with a leftover top-level `cm`), `batch.mjs` (→ `batch.out`: targetId survival across writes in one element), `perm.mjs` (copy bit of every encrypted fixture), `dump.mjs`/`raw.mjs` (objects, resources, raw content), `d5.mjs` (→ `d5.out`: built app under the lock, udata in the same dir, real CDP key events). `shasum -a 256 -c checks/fixtures.sha256`: 9/9 OK after the Electron run.

**E1 (blocking) §1.3** — "dispatch each item to the existing case bodies" inside one op cannot work for two content items in one `/Contents` element. A targetId includes the operator's byte offset (`editTargetId(page, path, opStart, …)`, content-edit.ts:1321), and the scan cache is keyed on `revision` (engine.ts:1392-1398), which the batch holds still. `batch.out` on direct3 (run at byte 24, `/Im1 Do` at 207, one element): after the run moves 1 pt, Im1's id `p0.c0.212…` is gone (`stale-target`). With the frozen revision the cached record would instead splice at stale offsets. §1.4's own case ("text run + image move on direct3") hits this.
Amendment: "9.x `edit-batch` plans every content item against the one pre-batch scan, as `edit-objects` does (engine.ts:4865-4976). An `edit-text {move}` item → `resolveEditTarget` + `editPlanText(doc, record, record.text, null, undefined, {move})`. An `edit-object` move → `editPlanPlacement(placement,'move',box,undefined,clip)`, and a delete → the `[qStart,qEnd)` span. Items are grouped by `elementIndex`, and inside ONE `mutate(session, doc, "edit batch", …)` each element is written once through `editWriteElement(pdf, pageView, elementIndex, textPlans, privatised, objectSpans)` (content-edit.ts:1073; `EditSpanEdit` spans, the mixed writer that paragraphs use at :3055). The annotation items (`field-props {rect}`, `field-delete`, `link-set {rect}`, `link-delete`, `redact-set`, `annot-delete`) run after the content writes in the same `work()`, by recursive `dispatch` of a synthetic request with `session.batch = true` (reset in `finally`). A reply with `ok:false` is rethrown with its `detail` and `{index, id}`, so the outer op abandons. The only `mutate` change is its first line `if (session.batch) { work(); return journalState(session); }`."

**E2 (blocking) §1.3** — `link-delete` has no `annotId`. It takes `linkId` = an index into `page.getLinks()` (engine.ts:4219-4225; the renderer sends `linkId`, renderer.ts:6074). Two index deletes in one batch shift the second index, so the wrong link is deleted.
Amendment: "9.x A batch `link-delete` item carries `annotId` (the `/Annots` object number, as `link-set` does, engine.ts:4247-4262). It is resolved to its pair through `linkPairs(owner).find(p => p.annotId === id)` before any write, and that pair's link is deleted. The single `link-delete` op is unchanged."

**E3 (blocking) §1.2, D2** — `editObjectBox` never clamps. It refuses `out-of-bounds` whenever a box leaves MediaBox ∩ CropBox (content-edit.ts:559-561), so "clamped into the page by `editObjectBox`" builds a refusal, and §1.4's "`at` placement and its clamp" fails. The per-item offset and `at` also break a multi-item paste: each item's top-left goes to `at`, so they stack, and an edge item flips to −10 alone. The renderer computes the U5 offset once on the union (`menuDuplicateRect`, renderer.ts:2791-2795).
Amendment: "9.x Paste moves the clipboard as one group. Let U be the union of the items' `visible` boxes and C = `editPageClipBox`. The translation d is `(at[0]−U[0], at[1]−U[3])` when `at` is given. Otherwise, when on the source page, d = (U[2]+10 ≤ C[2] ? 10 : −10, U[1]−10 ≥ C[1] ? −10 : 10), and else (0,0). Then d is shifted so that U+d lies inside C: `out-of-bounds` when U is wider or taller than C. Each target = visible + d, then `editObjectBox` (which now cannot fail on position) and `editCheckFull`."

**E4 (blocking) §1.4, §4.1, D3** — objects6 Fm1 has `/Resources<<>>` and paints `0.9 g 0 0 300 200 re f` (raw.mjs, obj 13). Im5 is a separate top-level `q 50 0 0 50 300 120 cm /Im5 Do Q` (page 0 content; dump: Im5 candidate on the page). So "Fm1: its `/Resources` and Im5 travel" is false, and pasting Fm1 gives objects 7, not "8 with an Im5-sized child". The forms with nested resources are edit-objects.pdf Fm1/Fm2 (`/Resources<</Font<</F1 8 0 R>>>>`, Fm2 also `/Matrix [1 0 0 1 10 10]`).
Amendment: "9.x §1.4's form case copies edit-objects.pdf p0 Fm2 → pastes into objects6 p0: the pasted form's `/Resources/Font/F1` is present, its stream bytes equal the source's, ink > 0 inside its box (graft.out PA5: 4800). §4.1's Fm1 paste expects `objects 7` and a form `PA1` at Fm1's box + the offset, with no child."

**E5 (should) §1.2 resources** — the "text-add copy" is not a helper that fits. text-add always replaces `/Resources` with a fresh shallow direct copy and copies only `/Font` (engine.ts:4606-4618). A direct `/XObject` subdict is then still the parent's dict, the U4 bug that `editPrivatise` guards against (content-edit.ts:1177-1184, which copies only when inherited, :1174). edit-objects p0's own `/Resources` is indirect obj 5 = `/Pages /Resources`, inherited by p1 (probe), so writing `PA1` there lands on both pages.
Amendment: "9.x Paste always gives the page a fresh direct `/Resources` (shallow copy of its own or inherited one) holding a fresh direct `/XObject` copy, in the op, and picks `PA<n>` against that copy (graft.mjs does this; `/Pages` XObject keys unchanged after the paste)."

**E6 (should) §1.2 contents** — text-add does not append to the last element. It rebuilds a direct `/Contents` array and pushes a NEW stream (engine.ts:4611-4616). That route needs no privatise. graft.out shows a new element of U6 blocks scanning back as candidates with `orient`/`full` intact. The leftover-CTM case also needs a gate: on a page whose content is `2 0 0 2 0 0 cm`, the pasted block scans `candidate`/`unsupported-transform` with no placement.
Amendment: "9.x Paste appends one new stream element holding all blocks (text-add's `/Contents` code, engine.ts:4611-4616), never edits an existing element. It refuses `unbalanced-content` when `scan.balancedContent` is false (as text-add, :4591). Inside `work()` it rescans and requires every `PA<n>` to be a candidate whose `placement.box` equals its target visible box ±0.01, else `failEditObject('unsupported-transform')` → abandon."

**E7 (should) §1.1** — a clipboard item has no `rect`, and `editPlacementMatrix(full, rect, orient)` needs it (content-edit.ts:572). For a form, `rect` is `/BBox` under `/Matrix` (`formRect`, content-edit.ts:2476-2485, not exported).
Amendment: "9.x Clipboard items gain `rect: number[]` (the source `placement.rect`), and a clip is written only when the source had `full`."

**E8 (should) §1.1, §1.3, §1.4** — `Session` has no `info` (engine.ts:723-765), so `allowed(session.info, …)` does not compile. No fixture clears the copy bit: perm.mjs shows copy:true on all five encrypted fixtures.
Amendment: "9.x `object-copy` checks `allowed(detect(doc, session.locked), "copy")` (the read side is `doc.hasPermission("copy")` in edit-replace-preview, engine.ts:4637) and refuses reason `read-only`. Batch items check permission with `requireMutable(session, item.op)` (engine.ts:1531-1543). The copy-denied and annotate-only documents are both built in the test with `security-set {permissions:{print:true, annotate:true, copy:false, edit:false, form:false}}` → save → open → `authenticate` (tests/p2-encrypt.test.mjs:125-160)."

**E9 (should) §1.1 budget** — `charge()` is a closure inside `editPrivatise` that measures one stream against `MAX_EDIT_SCAN_BYTES` = 2 MiB (content-edit.ts:35, 1164-1171). A graft pulls a form's whole subtree, and a 2 MiB cap would refuse ordinary photos.
Amendment: "9.x After grafting into the fresh scratch, the sum of `readRawStream().getLength()` over every stream object in the scratch is compared with `MAX_IMAGE_BYTES` (engine.ts:139): above it → `too-large`, the scratch is destroyed and the old clipboard kept."

**E10 (should) §1.2 objectOps** — a pasted masked image scans `unsupported-image` with no `replace` (graft.out PA6 = edit-objects Im2 with `/SMask`; content-edit.ts:2533-2536). Amendment: "9.x A pasted block has the same `objectOps` and `reason` as its source."

**E11 (should) D5, §4.3, §6** — d5.out, focus on the selected Im1 button and on BODY:
- `keyDown` meta+c with `commands:['copy']` → `copy` event fires (cut/paste the same).
- `rawKeyDown`/`keyDown` meta+c without `commands` → keydown only, no `copy`.
- `document.execCommand('copy')` → `copy`.

A CDP key never reaches the menu role: it has no NSEvent, so the accelerator path is untested. The DOM listener of D5 does work.
Amendment: "9.x Every 'real CDP ⌘C/⌘X/⌘V' in §4.3 and §6 is `Input.dispatchKeyEvent {type:'keyDown', key:'c', code:'KeyC', windowsVirtualKeyCode:67, modifiers:4, commands:['copy']}` + `keyUp` (cut 88/`['cut']`, paste 86/`['paste']`). The Edit-menu role path is proven once, by fable51, with an AppleScript keystroke. D5 stands."

**E12 (should) §1.3 tokens** — `annot-delete` and `link-delete` refuse a missing id with a plain `Error` (engine.ts:1713, 4222), which carries no token. Refusals put the token in `detail` (`reason` is `"invalid"`; engine-core.ts:117-123). Amendment: "9.x A batch refusal carries `detail` = the item's token (a plain-Error item → `stale-target`) and `extra {index, targetId|widgetId|annotId}`."

**E13 (should) §1.3** — `BatchItem edit-text` allows `text` and `size`, but D4/§2.3 allow only a move. Amendment: "9.x A batch `edit-text` item is `{op, targetId, move}`. The text is the run's own (engine.ts:4509-4511). A `text` or `size` key → `invalid-selection`."

**E14 (should) §1.3** — `mutate` is on the refactor don't-touch list (progress.md §5). Amendment: "9.x The E1 one-line branch is the only permitted `mutate` change. The reviewer diffs the rest of `mutate` byte-for-byte."

**E15 (nit) D2** — `session.name` is the display name (renderer.ts:1654), so two files with the same name count as one. Amendment: "`source: {generation, page}`; the offset applies when `session.generation === source.generation`."

**E16 (nit) D1** — a worker restart (`workerFailed` → `startWorker`, renderer.ts:834-850) drops the clipboard. Amendment: "the renderer clears `clipboardInfo` in `workerFailed`; the engine answers `empty-clipboard` meanwhile."

**E17 (nit) §1.2 tokens** — `invalid` is not an `EditObjectDetail` (engine.ts:441-443). Amendment: "a malformed `at` → `failEditObject('out-of-bounds')` (as `editObjectBox` does for non-finite boxes, content-edit.ts:551-553). `EditObjectDetail` gains `'empty-clipboard'`."

**E18 (nit) §7** — an image with `/OC` (`image.oc`) grafts an OCG that the target's `/OCProperties` does not list. The render outcome is UNCHECKED. Amendment: "copy refuses `not-candidate` for an item with `image.oc` (Help ceilings unchanged)."

## checkedFine
- D1: one `Session` per worker for its life (worker.ts:9-10). `open`/`close` reset only their named fields (engine.ts:3188-3232, 3312-3328), so a `clipboard` field survives. `createSession` (engine.ts:5215) must add `clipboard: null`.
- `new mupdf.PDFDocument()` exists (node_modules/mupdf/dist/mupdf.d.ts:460) and is already used by replace (engine.ts:4817-4822).
- Graft across two documents through a scratch, with both sources closed (graft.out):
  - raw stream bytes are equal for PA1, PA5 and PA6;
  - Fm2's `/Resources/Font` travels, and PA6's SMask is grafted;
  - save → reopen keeps all six candidates, ink > 0 in each;
  - undo → 0 `PA*`.
- The U6 writer is reusable for a fresh block: the `editPlacementMatrix` + `editClipOperands` bytes `q\n72 250 60 40 re W n\n100 0 0 50 72 250 cm\n/PA3 Do\nQ` scan back as PA3 `full [72,250,172,300]`, and PA2 as `orient {swap:true,sx:1,sy:-1}`.
- `mutate`'s pre-op reads (engine.ts:1738-1739) and `journalState` (:1331-1339) are wrapped in `attempt`, so nested case replies cannot throw.
- `requireMutable`'s token is `read-only` for locked, signed and permission-denied documents (engine.ts:1533-1541).
- `edit-text` takes a move with the run's own text (engine.ts:4509-4511).
- `field-props` with `rect` only works (engine.ts:3602, 3615, 3679).
- Widget and annot ids are object numbers (engine.ts:1704-1714, 1963-1969).
- tests/p2-shell.test.mjs:172-180 passes with the three `OP_PERMISSION` keys and no `DEFAULT_EDIT` change. `allowed(…,'any')` = annotate|edit|form (engine-core.ts:260), and E8's per-item `requireMutable` refuses a `field-props` item on the annotate-only document.
- U5 §9.25 = contract-p6-unit5.md:85, implemented at renderer.ts:2791-2795.
- §1.5 lines are real:
  - tests/p6-objects6.test.mjs:348-355: `im3` is `undefined` when `refuses()` receives it;
  - :233-234: both `invalid-transform` cases send no box.

## buildRisks
- `session.batch` must be reset in `finally`, and `editScanCache` must be invalidated after a batch or a paste (the revision bump does it).
- Destroy the old scratch only after the new one has been built and charged, and never keep `PDFObject` handles of a destroyed scratch.
- Use one graft map per paste into the target (one per copy into the scratch) so a shared SMask lands once.
- The target doc's `addStream` takes a string; blocks must use `editNumber` output only.
- Renderer: the page-menu `at` is converted from view space with the page transform (MediaBox origin, `/Rotate`) before sending.
- Cut = copy then delete: if the delete refuses, the clipboard has already been replaced (state this in Help or accept it).
