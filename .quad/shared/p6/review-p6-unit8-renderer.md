# P6 U8 review — renderer + human UX, real CDP mouse (Opus 5.5, 2026-09-24)
Verdict: REVISE (1 should on Help truth; 2 should carried pre-U8 engine gaps that still reproduce the human's report; 1 nit).
Probes (under .quad/run.lock, fresh build): checks/review-u8-renderer-probe.mjs (real Input.dispatchMouseEvent clicks, Input.insertText,
CDP Meta+Enter, real click on #undo), checks/review-u8-renderer-overprint.mjs (Text panel add ×3 by real clicks),
checks/review-u8-renderer-save.jsonl (save → reopen + edit-shared COW). Records: review-out/review-u8-renderer-probe.json, -overprint.json.

## Findings
R8-1 (should, renderer) index.html:569 + tests/p2-shell.test.mjs:217, §9.2 vs §9.8 — new Help item says "text inside a form drawn more
than once, cannot be edited." False: a run in a Form drawn twice scans candidate/cowable and the Text panel labels it "Editable (this
placement only)" (renderer.ts:4463); cow-edit.jsonl (edit-shared.pdf ALPHA→BRAV, refused []) and my save.jsonl step 20 edit it, exit 0.
It is only unreachable ON THE PAGE (bounds null). Fix: "Text placed exactly on top of the same text cannot be edited; text inside a form
drawn more than once has no on-page target (edit it from the Text panel, this placement only)." and re-pin p2-shell:217.
R8-2 (should, engine; pre-U8, §9.14 defers) — a GUI-added run cannot grow: B "Twin" → "Twin 2" by real click + insertText + Meta+Enter is
refused `overflow`, status "The replacement exceeds the available free space." with ~280 pt free to the right (probe commitTwin2;
review-u8-r4-after-twin2.png). This is the human's complaint in another form ("text I add is not editable"). Fix: text-add runs take
the page text right margin as `limit`, not their own right edge.
R8-3 (should, engine+renderer; pre-U8, §9.14 defers) — add → undo ×N → add via Text panel: revision +1, "Added text", but
content.lastAdd {targetId "", bounds null}, direct.targets unchanged (5), run drawn in a fallback serif (review-u8-r8-overprint.png).
Renderer stays silent because renderer.ts:3958 requires targetId !== "". Smallest renderer fix: announce when targetId === "" too;
engine fix: the dangling /PDF4AllF1 after undo (integration defect 2).
R8-4 (nit, renderer) renderer.ts:3958 — the overprint announce is reachable and truthful (Text panel "Add text" twice at the same point:
status "Added text is not selectable on the page (it lies exactly on top of the same text)", targets 6→5), but it omits that the
FIRST run also lost its target (both now ambiguous). Fix: "…; neither copy can be edited until one moves".

## checkedFine
- (a) real mouse, direct3 p0: add Twin (300,400) rev1 targets 6; (300,300) rev2 targets 7; both content.bounds entries present with
  own boxes; click (312,404) → text/Twin/p0.c1.95.f22530a3, editor box [299.88,396.54,325.21,413.06] = rect; click (312,304) →
  text/Twin/p0.c2.94.f22530a3, editor over B (r2/r3 shots). B → "Pair" rev3; A still "Twin"; B id rehashed p0.c2.94.51f72021.
- (b) add "Move me" (300,200): targets 8, lastAdd p0.c3.98.6c51e7e2 (the new run); original opens p0.c0.314.6c51e7e2 at its box, copy
  opens its own box (r5/r6). Real #undo ×4 → targets 7/7/6/5, dirty false (r7).
- (c) #help-dialog: U7 pasted-run sentence and "exactly repeats" gone; no other sentence claims duplicates unselectable ('whole line',
  'repeats an existing line' 0 hits; 'cannot be told apart' only as the reason label at index.html:514, required by p2-shell:147-151).
- (d) announce reachable via Text panel overprint, wording per §9.9; (e) direct/content key sets identical before/after; no new keys.
- Save → reopen: twins.pdf reopens targets 7, A "Twin", B "Pair" each open (save.jsonl 20 steps, 0 red).
- (f) drives exit 0: direct-edit, direct-edit3, direct-edit8, direct-edit8-cut (flipped per §9.14), direct-edit9, content-edit, debt1,
  object-edit, help-fields, cow-edit. node --test p2-shell + p6-drive9: 10 pass 0 fail.
- fixtures.sha256 9 OK; fixtures hash list = u8-hash-before.txt; no --drive= / review-out stragglers.
## Screenshots (checks/review-out/)
r1-twins-added: both Twins outlined as targets. r2-editorA / r3-editorB: editor+handles over the clicked twin only. r4-after-twin2:
overflow refusal, B editor still open with "Twin". r5-orig-moveme / r6-copy-moveme: each "Move me" opens its own editor. r7-after-undos:
page back to original. r8-overprint: post-undo "Solo" in fallback font, no target (R8-3). r9-panel-overprint: overprinted "Solo", no outline.
