# P6 U5 — engine seat evidence (Opus 5.5, 2026-09-24)

Files changed (edited in place):
- src/renderer/engine.ts — `EditObjectAction` + "duplicate"; `EditObjectDetail` + "invalid-selection"; new `EditObjectsRequest`; `SessionRequest` union entry; `edit-object` duplicate branch (editObjectBox like move, never a no-op, §9.4 cap, insert at qEnd inside the one mutate after privatise); new `case "edit-objects"` (cap → shape/single/distinct → per item resolve/candidate/ops/placement+element/box; same-cm moves dropped; all dropped → noop:true; §9.4 cap; ONE mutate "edit objects", grouped by c<n>, privatise once per element, ONE editSplice per element; reply {ok, page, count, ...editState}; per-item refusals carry targetId).
- src/renderer/content-edit.ts — the three objectOps literals gain "duplicate" last; new exported helper `editDuplicateEdit(bytes, placement, box)` (byte-exact "\n" + [qStart,cmStart) + cm' + [cmEnd,qEnd) at start=end=qEnd).
- tests/p6-objects5.test.mjs (new, 11 tests).
- Pins in place: tests/p6-direct3.test.mjs:93; tests/p3-objects.test.mjs table rows 129/132/133/135 (move rows +duplicate; unsupported rows and :177 stay); tests/p3-targets.test.mjs:187 and :195.
- limits.ts, engine-core.ts, worker.ts: no change.

Proof (under the lock): `npm run build && node --test tests/p6-objects5 p6-direct3 p3-objects p3-targets p3-edit p2-shell` → 98 tests, 97 pass, 1 fail.
- The one failure is p2-shell "shell HTML has required control IDs": index.html already has the four #object-menu ids, p2-shell.test.mjs:34's inventory does not yet (renderer seat's edit). Not an engine file. p2-shell's DEFAULT_EDIT coverage test is green (the renderer's 'edit-objects' token has landed).
- Engine-only: 11/11 p6-objects5, 79/79 p6-direct3+p3-objects+p3-targets+p3-edit.

p6-objects5 cases: ops pins (Im1/Im2/Fm1, page-2 Bg incl. a Bg duplicate that succeeds, §9.6); duplicate Im2 → 4 objects, two Im2 with distinct ids, copy after the old Q, reopen keeps it, ink in strip [462,43,469,81] 0 → >0 (§9.7), undo → dirty false; single refusals out-of-bounds ×3, edit-objects.pdf p0 clipped Im1 → unsupported-transform, rotated Fm1 → not-candidate, each with journal+scan deep-equal; compound move/delete/duplicate (§9.9 boxes) each one revision and one undo; refusals before any write: replace/resize/unknown/empty id/non-array → invalid-selection, out-of-bounds with Im2's id, duplicate id with its id, single item, MAX+1 distinct fake ids → too-large, stale revision → stale-target, both own boxes → noop:true, one own box → count 1, edit-objects.pdf second Im1 → unsupported-transform with its id; cow p1 (§9.8) duplicate Im1 + move Fm1 → count 2, p0 scan deep-equal, reopen 3 objects, undo → cowable:true; two-element /Contents scratch (Im1:c0, Im2:c1, Fm1:c1) move+move one revision, reopen, undo; delete Im2 + duplicate Fm1 same element; §9.4 scratch page at MAX-1 records: two duplicates → too-large, one → ok (next scan = 512), another → too-large; encrypted-noedit → read-only (detail undefined); hello.pdf → stale-target with the item id; sha pins before/after for direct3 (e3ae5a43…) and edit-objects.pdf (b22a001f…).

journalState: no read op exists, so the test compares session.revision/savedPosition + pdf.canUndo/canRedo and the scan items.
§9.12: in-mutate rollback not separately tested (no inject hook), as the contract says.
Fixtures: shasum -c fixtures.sha256 8 OK; nothing added under fixtures/. Scratch: .quad/scratch/p6u5/engine/.

## Fixes after review

- E-1 (reproduced with review-engine/probe2.mjs + probe3.mjs: p1 listed 4 objects after an edit-objects/edit-object duplicate on p0; a move wrote into p1's c0). Fix in src/renderer/engine.ts only: new `directContents(pdf, pageObject)` (next to `directAnnots`) replaces an indirect `/Contents` array with a direct copy before a U1b privatise; edit-object and edit-objects call it when the record is cow, and edit-objects now re-reads `/Contents` per element instead of once before the loop.
- Test: tests/p6-objects5.test.mjs "U5 review E-1 …" builds the probe2 recipe in memory and, for edit-objects, edit-object duplicate and edit-object move, asserts p1's content bytes are unchanged, p1 still has 3 objects after save → reopen, and one undo restores both pages. It fails with directContents neutralised in the build output (checked) and passes with the fix.
- Not fixed here (outside my lines): the edit-text path (content-edit.ts:920 → editPrivatise:1036) still does `contents.put(elementIndex, owned)` on a shared indirect array. Suggested edit at content-edit.ts:1036: when `contents.isArray() && contents.isIndirect()`, copy it into `pdf.newArray()`, `pageObject.put("Contents", copy)` and put `owned` into the copy.
- Ran under the lock: tsc clean; npm run build; node --test p6-objects5, p6-direct3, p3-objects, p3-targets, p3-edit, p2-shell = 99/99 pass. Fixture checks: 8 OK, and the 54-line u5-hash-before list is identical.
