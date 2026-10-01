# P6 U5 adversarial review — ENGINE lens (Opus 5.5, 2026-09-24)
Verdict: ACCEPT. One `should` (it predates U5 in U1b `editPrivatise`, and U5's new writers go through it) and two test nits.
No U5 engine claim was refuted: every duplicate and `edit-objects` write survived save (garbage=yes) → reopen → edit-targets,
and every refusal left the journal, page bytes and scan byte-identical.
Probes (read-only, built engine): `.quad/scratch/p6u5/review-engine/probe.mjs` (43 PASS; the 2 FAILs were errors in my own
expectations, see below), `probe2.mjs` / `probe3.mjs` (a shared indirect /Contents array).

## Ran
- Locked: `npm run build` ok; `node --test tests/p6-objects5 p6-direct3 p3-objects p3-targets p3-edit p2-shell` → 98 tests, 98 pass, 0 fail.
- `npx tsc --noEmit -p tsconfig.json` clean. `shasum -c checks/fixtures.sha256` 8/8 OK. fixtures *.pdf|png: 54 lines, and the diff
  against `u5-hash-before.txt` is empty.

## Findings
**E-1 (should, engine; U1b defect that U5's new writers inherit) — an edit through a shared *indirect /Contents array* also lands on the other page.**
src/renderer/content-edit.ts:1037 (`if (contents.isArray()) contents.put(elementIndex, owned)`). It is reached from engine.ts:4940-4943 (`edit-objects`, where
`contents` is also read once before the per-element loop) and engine.ts:4757 (`edit-object`). Clause: §1.2 "privatise … byte-identical copy
so every record's spans hold", §0 "Proven or refused". The cow exists to keep the other page unchanged.
Repro (`probe2.mjs`): take direct3, split p0's stream in two, put ONE indirect array `[a b]` on both p0 and p1 with the same /Resources.
`edit-objects` on p0 (duplicate Im1 → [82,610,182,660], move Im2) → ok. p1's scan then lists 4 objects (it had 3), and after reopen p1 still has the copy.
`edit-object duplicate` on p0 behaves the same. `probe3.mjs`: a pre-U5 `edit-object move` also writes `82 610` into p1's c0, so this
predates U5, but the duplicate and compound writers make it more reachable. The scan counts each element as shared
(content-edit.ts:1698-1700), but `editPrivatise` replaces the element inside the shared array rather than giving the page its own array.
Fix: in `editPrivatise`, when `contents` is an indirect array used by more than one page, first `pageObject.put("Contents", copy)`,
where `copy` is a direct copy of the array, then `put` the owned stream into that copy. In `edit-objects`, re-read `at(pageObject,"Contents")` for each
element. Test: the probe2 recipe; p1's content bytes are identical before and after, and one undo restores both pages.

**E-2 (nit, engine) — tests/p6-objects5.test.mjs:247: the §9.3 "cap first, before any shape check" order is not pinned.**
The over-cap case sends 513 well-formed fake ids, so a cap check moved after the shape check would still pass. Probe: 512 fake ids plus one
`{bogus:1}` → `too-large` (the build is correct). Fix: make the last of the 513 items malformed.

**E-3 (nit, engine) — the tests pin weaker facts than the ones proven here.** `nothingWritten` (p6-objects5:83-91) compares scan items and
the journal, not page bytes. The two-element delete + duplicate case (:318-326) has no undo and no reopen. The cow case (:282) compares page-0 *scan
items*, not page-0 bytes. The probe proved all three (below). Fix: add `contents(s)` before/after equality to `nothingWritten`, add one
`undo` → `contents` equality in :326, and compare `contents(s, 0)` in :282.

## Checked fine (reproduced)
- (1) direct3 p0 compound [dup Im1, move Im2, delete Fm1] → count 3, rev 1. Reopen: boxes exact, every object a candidate. One undo gives the
  page bytes back exactly, and redo works. edit-objects.pdf p0 [move Im1, dup SMask Im2, dup Fm1, dup /Matrix Fm2] → count 4. The reopen scan
  (boxes and ops, 10 objects) deep-equals the live scan.
- Cow p1 (§9.8): page 0 contents and /Resources bytes are identical after the op. Reopen shows 3 objects. Undo gives p1's bytes back and `cowable:true` on all objects.
  A single cow duplicate of Fm1 leaves p0's bytes unchanged. A refused cow op writes nothing on either page.
- Two elements: [dup Im1 (c0), delete Im2 + dup Fm1 (c1)] → count 3. Reopen is correct, and one undo restores the bytes. `/Contents [a a]`
  (one stream twice) privatises each slot correctly.
- (2) Refusals: journal, bytes of p0–p2 and scan are unchanged, and `targetId` is top-level. Tokens: non-array/null item/number id/missing
  items/[] → invalid-selection; single → invalid-selection; repeated id → invalid-selection + id; 513 → too-large; 512 fakes → stale-target + `f0`;
  stale revision/generation → stale-target; text id → not-candidate + id; later out-of-bounds item → its id; move without box → out-of-bounds; a
  20000 pt side → `too-large` (the correct `editObjectBox` token; I had expected the wrong one); clipped Im1 → unsupported-transform + id;
  rotated Fm1 → not-candidate + id. Partial write: a valid c0 duplicate followed by an out-of-bounds c1 move writes nothing.
- (3) All same-cm moves → `noop:true` with the journal unchanged. A same-cm move beside a duplicate → count 1. A duplicate at its own box (single and in
  a compound op) lands and is never a no-op.
- (4) Copy bytes: `…/Im1 Do Q\nq 100 0 0 50 82 610 cm /Im1 Do Q\nq 60 0 0 40 …`, inserted after the original's Q. The original id is
  unchanged (`p0.c0.212…`). Later ids shift (Im2 245→278). The copy is a candidate with the same ops. Two placements of Im2 (dup one,
  move the other) in one op land and survive reopen.
- (5) objectOps pins: p6-direct3:93, the p3-objects table (133/134/136 +duplicate, unsupported rows and :177 unchanged), p3-targets:187/195.
- (6) `SessionRequest` includes `EditObjectsRequest` (engine.ts:560). `edit-objects` is only in `DEFAULT_EDIT` (p2-shell:173) and is green.
  encrypted-noedit → `read-only` for `edit-objects` and for a single duplicate.
- (7) §9.4: the 511-record page tests pass (two duplicates → too-large, nothing written; one → 512; the next → too-large).
- (8) Each p6-objects5 assertion would fail if its behaviour were missing. The regex pins "after Q", `revision +1` pins one op, `ink`
  0→>0 pins the paint, and refusals assert tokens and ids. I found no vacuous case beyond E-2/E-3.
