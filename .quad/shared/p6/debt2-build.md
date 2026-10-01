# Debt sweep 2 — build evidence (implementer, Opus 5.5, lean mode, 2026-09-24)
Contract `.quad/shared/p6/contract-debt2.md` (§9 folded). One implementer, all files. No fixture change, `mutate`/drive.mjs/main.ts untouched.

## Red before fix (current build/ 22:36, no rebuild in that hold) — `acceptance/debt2-red-before.log`, 6/6 red
- readingOrder and editorBox match §8 (p6-direct child): `TypeError: cutAnnouncement is not a function`
- debt2 N1 name on field-props: `still` journalState deep-equal — revision 1 / canUndo true (the batch wrote: field renamed)
- debt2 N1 page key (links.pdf): nothingWritten journalState deep-equal — revision 3 → 4 (page-1 mark moved)
- debt2 N1 link-set target: nothingWritten journalState deep-equal — revision 0 → 1 (link 10 retargeted)
- debt2 N2 Tr 1 / Tr 2: `assert.match` / ` 0 Ts 1 Tr 0 g 0 G 1 w /` vs block `… 0 Ts 0 Tr 0 g 1 0 0 1 82 390 Tm …`

## Source
- engine.ts:3815-3829 D1 per-op ALLOW pass after `raw.every(shaped)`, before the repeat check → `invalid-selection`, "edit-batch: unexpected key <k> on <op>", `idOf` (index + id).
- engine.ts:812 ClipItem text `tfState.tr: 0|1|2`; :3398 from `record.renderMode`; paste block (~:3600) `${tr} Tr 0 g 0 G 1 w 0 J 0 j [] 0 d` inside its q…Q.
- content-edit.ts:194 `renderMode?: number` on the record; :2408 set from the scan state (worker-only, no reply key).
- renderer.ts:3081 `export cutAnnouncement(items, runs)`; :3062 uses it. :4670-4673 Inspector shows round2; :4780-4789 a field equal to String(round2(exact)) passes the unrounded value through, unchanged edges are the placement's own (unchanged Apply = placement.box exactly); 0.05 move/resize tolerance kept. (Lazier than §9.8's "remember the shown strings": compares against round2 of the current box — same effect, no state.)
- index.html Direct editing <ul>: N2 li replaced by §9.4 sentence; clip li (U+2026) appended after the crop li. D4 second sentence dropped (§9.7).

## Re-pins (file:line old → new)
- p2-shell.test.mjs:213 'Pasted text keeps its font and size but is black; kerning is dropped.' → §9.4 sentence; :222 clip sentence added; :232 old N2 string into `gone`.
- p6-objects7.test.mjs:411 bare batch → `still(f, () => batch(...))`; :645 `paste(s, 2)` → `nothingWritten(s, 2, () => paste(s, 2))`.
- p6-objects6.test.mjs:232 `transform(s, sc, 'Im1', 'spin')` → `transform(s, sc, 'Im1', 'spin', [90, 0, 110, 200])`.
- direct-edit8-cut.jsonl:31 "Cut 1 item; 1 text run stay (they cannot be deleted)" → "… stays (it cannot be deleted)" (value only; 51/11 unchanged).
- checks/review-u7-renderer-probe.mjs:304 `/Cut 1 item; 1 text run stay/` → `/Cut 1 item; 1 text run stays \(it cannot be deleted\)$/`.
- checks/shot-direct8.mjs:11 header + D3 ceiling line.

## New
- p6-objects7.test.mjs:759-822 five tests (3 N1, N2 Tr 1/2 via objects7 `scratch` + `scanEditTargets`, last /Contents element only).
- p6-direct.test.mjs:149-153 cutAnnouncement(1,1)/(2,2)/(3,0). p6-drive8.test.mjs:135-137 + :172 T-5 asserts; :268 fourth test (chords, 19/5).
- tests/drive/direct-edit8-chords.jsonl (19 steps, 5 click lines, template verbatim). design-agent-drive.md "## Debt sweep 2 drive notes" at EOF.
- checks/shot-debt2.mjs (node --check OK; NOT run): A Shift-click run+Im1, osascript ⌘X (5 s FAIL), exact #edit-status, clipboard 2, debt2-a-cut.png, ⌘Z; B CDP ⌘-held drag (0,13 px), "no float to round" precondition, TWO regex + round2 equality, real click on #content-apply-object → revision/dirty unchanged, ⌘Z restores [72,600,172,650]; 9 OK at end.
- T-5 bite: `scratch/debt2/impl/t5-bite.txt` (flipped asserts against the saved direct-edit8 report: suffix false, flipped mark 0 hits).

## Results (one lock hold after the last source edit: npm run build OK)
- node --test p6-objects7 p6-objects6 p2-shell p6-direct p6-drive8 p3-drive: 85 tests, 85 pass, 0 fail, 0 skip.
- drives exit 0 / red 0: chords 19, direct-edit8-cut 51, direct-edit8 115, direct-edit9 43, direct-edit3 79, object-edit 11, content-edit 4, help-fields 21.
- tsc clean; fixtures.sha256 9 OK; fixture list = debt2-hash-before.txt (55, diff empty); 0 --drive= stragglers.
- Expected full suite: U8 536 + 6 new tests = 542 (not run; full npm test left to acceptance).
## Not done
- §9.8 drive step: no drive path for the object-form Apply exists, so the passthrough is covered by the GUI proof (shot-debt2 B) alone; its pre-fix bite is not shown (script not run here).
- shot-debt2.mjs not run (sends real keystrokes; fable51 runs it). Full `npm test`, drives ×3, `npm run package` left to accept-debt2.sh.
## Fixes after review
- R-S1 (tests, shot-debt2.mjs case B only): reproduced by reading — B had no field edit, and its "no float to round" precondition read `content.objects[].box`, which STATE rounds to 2 decimals (renderer.ts contentSummary), so it could never see a float.
- Precondition now reads the unrounded `direct.active.placement` after the Im1 click.
- Before the unchanged Apply: `#content-object-form.checkValidity()` gate (exit 1 naming each invalid field).
- After it: real CDP click on `#content-box-w`, ⌘A (CDP key, selectAll command), `Input.insertText('120')`, value asserted '120', real click Apply → revision +1 and `direct.active.placement` ≈ [p0,p1,p0+120,p3] ±2e-4; osascript ⌘Z → pre-edit placement ±2e-4 (re-clicks Im1 if undo dropped the selection); a second ⌘Z restores the drag ([72,600,172,650], revision before+3).
- tsc clean; build OK; `node --check` OK; not run (sends real keystrokes). fixtures.sha256 9 OK; PDF list = debt2-hash-before.txt.

## Fixes after review (engine fixer, E-R1..E-R3)
- E-R1 (engine.ts runAnnotation): reproduced (`...item` at the dispatch). Now `dispatch({ id, page, op: item.op, ...picked })`, picked = `ALLOW[item.op]` keys only. Bite shown in scratch (fix-engine/probe.mjs, ALLOW pass disabled in a build copy): pre-fix field 5 renamed to RENAMED, post-fix keeps "nom". No in-suite test can fail pre-fix: the ALLOW pass refuses first.
- E-R2 (p6-objects7): reproduced (only 3 ops pinned). New test loops all 8 ops with a stray `page` key: invalid-selection, `unexpected key page on <op>`, index 0, nothingWritten. Bite: widening field-delete's ALLOW in a build copy turns it red (stale-target reply).
- E-R3 (p6-objects7): reproduced (no reopen). N2 Tr 1/2 cases now also assert renderMode after save+reopen. Test-strength only; behaviour already held.
- tsc clean; build OK; p6-objects7 + p6-objects6: 47/47 pass. fixtures.sha256 9 OK; debt2-hash-before.txt all OK.

## Fixes after review
- R-B1 (renderer, reproduced): review-debt2-renderer-probe.mjs pre-fix → `#content-box-y` validity stepMismatch (594.64 vs step 0.1); real Apply click blocked by the validation bubble, W=120 never wrote (log .quad/scratch/debt2/fix-renderer/probe-pre.out).
- Fix: `step="any"` on `#content-box-x/-y/-w/-h` (src/renderer/index.html:410,411,414,415). Nothing else touched.
- Pin: tests/p2-shell.test.mjs new test "debt2 R-B1 … step=\"any\"" asserts each of the four inputs; fails on the pre-fix source (step="0.1").
- Post-fix probe (fix-renderer/probe-post.out, probe-reopen.out = same probe reopening the form after the unchanged Apply closes it): validity all valid; unchanged real-click Apply submits, form closes, revision/dirty 7/true unchanged; W=120 real-click Apply → revision 8, "Resized image to 120×50".
- Probe remaining FAILs "box exact"/"W edit box" compare STATE `content.objects[].box` (rounded to 2 dp at renderer.ts:4966) against the unrounded placement — probe artifact, not a defect; `direct.active.placement` stays exact. Upstream probe also needs a reselect of Im1 before the W edit now that the unchanged Apply succeeds (tests owner).
- Ran under lock: tsc --noEmit OK; npm run build; node --test p2-shell + p6-direct 15/15 pass; fixtures 9 OK.
