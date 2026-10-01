# P6 U7 tests seat evidence (2026-09-24)

Files: tests/drive/direct-edit8.jsonl (new), tests/p6-drive8.test.mjs (new), .quad/shared/p6/checks/shot-direct8.mjs (new),
tests/drive/direct-edit7-save.jsonl (new; review-u6-tests-save.jsonl + 3 header lines), tests/p6-drive7.test.mjs (one appended test()),
.quad/shared/p6/checks/shot-direct7.mjs (§4.5), .quad/shared/p3/design-agent-drive.md (appended "## P6 U7 drive additions", 6 lines),
.quad/shared/p6/contract-p6-unit6.md §3 (one number). scripts/drive.mjs untouched (no new packDrive key needed).

direct-edit8.jsonl: 104 steps, 23 click lines (header re-counted from the file). = §9.53 template (76) + D4 template (+26)
+ 2 lines for the §10.35/§10.36 chord path (step 45 `key Meta+v` on direct3 p2, step 46 undo). Ledger: objects6 0..8,
direct3 0..8, field-defs 0..6, links 0..6, hello 0..12. Six-key `kinds` everywhere (§10.14d); line 30 menu cut/copy/paste/delete true
(§10.37); Delete refusal text "…deselect them first or use Cut" (§10.42); plain click on link (step 67) / mark (step 91) selects (§10.31).
p6-drive8: counts, exit 0, 0 red, Shift-click never has direct.active.draft, 5 fixture shas before/after, geometry by step index.
Pinned from the first green report (all matched the lens numbers ±0.5, none re-pinned):
- S32 "Move me" [73,376.95,130.57,396.19]; S43 page-menu text paste [300,500,357.57,519.24], targetId p2.c1.92.6c51e7e2;
  S45 ⌘V text paste [72,376.95,129.57,396.19]; S62 "plain copy" [82,257,202,282]; S80 links [82,82,210,102],[82,132,210,152];
  S86 hello link [300,400,428,420], announcement "Pasted 1 item; 1 link skipped (its page is not in this document)";
  S90 marks [300,500,400,530],[300,600,400,630] (redact.rects is view y-down); S100 pasted mark [310,510,410,540].

Re-pins (file:line old → new):
- shot-direct7.mjs:124 "§9.34: modifiers" → "§9.19: modifiers" (TS3); :181 duplicate `log('fixtures before…')` removed (TS3);
  :277 crop drag `drag(…, -40 * s, 0)` → passes `during` and asserts direct.gesture.kind === 'crop' (TS2).
- contract-p6-unit6.md:41 (§3) "Fm1 band (75,600)" → "Fm1 band (74,600)". (§4.1 :44 still says (75,600); §9.24 amends it; not §3, left.)

Runs (one lock hold, build 0): node scripts/drive.mjs direct-edit8 → exit 0; direct-edit7-save → exit 0;
node --test tests/p6-drive8.test.mjs tests/p6-drive7.test.mjs → 3 pass, 0 fail (direct-edit8 wall 3817 ms, far under 240 s).
Fixtures: shasum -c fixtures.sha256 9 OK; fixture list cmp-identical to u7-hash-before.txt (55); no --drive= stragglers.

shot-direct8.mjs (run once under the lock with U7_SKIP_D=1, exit 0, "PASS all"): A real CDP ⌘C/⌘V (commands) → clipboard 1,
copy defaultPrevented true, PA1 [82,152,182,202]; A' main-process wc.copy() on Fm1 → kinds.form 1, wc.paste() → PA1 form
[82,502,382,702]; B real right-click empty → page menu Paste alone; C Shift-click run + Im1 → 1 .group-box, 0 handles shown,
ArrowRight both +1; E Fill forms Shift-click plain, ⌘C/⌘V → fields.count 6, plain copy [82,257,202,282].
PNGs viewed with Read: u7-a-pasted.png (PA1 outlined with handles at +10,+10 over Im1); u7-b-page-menu.png (11-item menu, only
Paste enabled); u7-c-mixed-group.png (one box around "Move me" and Im1, no handles); u7-e-field-paste.png ("plain copy" selected
under plain, Fields tab lists it, status "Pasted 1 item").
NOT RUN: case D (⌘C in the Inspector W field) — it writes the OS clipboard (pbpaste saved / pbcopy restored in finally, text only).
Skipped via the added U7_SKIP_D env switch; §9.41 says fable51 runs it while the human is away (run without U7_SKIP_D).
Not done: full npm test not run by this seat.

## Fixes after review
- F1 (tests half): the renderer seat appended the mixed-selection tail to direct-edit8.jsonl (S105-S115: Shift+ArrowRight, run drag, Delete refusal). p6-drive8 pins move to 115 steps / 26 clicks. S109/S110 pin Im1 and "Move me" at +10 and +40, S109-S112 keep [text, image], S114 is back at source. On the pre-fix build the review-u7-renderer-mixed probe failed at Shift+ArrowRight (selection [image]).
- T-1 / T-3: new tests/drive/direct-edit8-cut.jsonl (51 steps, 11 clicks), third test() in p6-drive8. It covers:
  - Cut of L1+L2 ("Cut 2 items", then ⌘V back at the source boxes) and of L1 alone ("Cut 1 item").
  - A runs-only Cut ("Copied 1 text run; text runs cannot be cut", rev 0, no edited).
  - Run+Im1 ("Cut 1 item; 1 text run stay (they cannot be deleted)", edited {selection,delete,Im1,1}).
  - Radio color (fields 5→3→5, undo restores the defs exactly).
  - The paragraph Shift-click refusal.
  - `menu {page,x,y}` with no item on an unselected link, field and mark: exact direct.menu, and the selection becomes that member alone.
- T-2: new tests/drive/direct-edit8-save.jsonl (42 steps, 5 clicks), second test() in p6-drive8. It deep-equals the objects (p0/p1), the run bounds (p2 at drive8 S43's box), the field defs ("plain copy" at S62's box) and the links (S80's boxes) before save-to and after reopen. Marks are left out because they are not document content until Apply.
- These are coverage gaps, not bugs: the new T-1/T-2/T-3 lines pass on the current code (the behaviour already worked).
- Found and not pinned (renderer/engine owners):
  - After Cut of [run, Im1], ⌘V on the same page lists neither "Move me" in content.bounds (candidates 8→7, targets 9), and the pasted run is not selected.
  - Radio Cut→⌘V gives defs {color.B:[100..], color:[100..], color.A:[72..]} against the original {color.A:[100..], color:[72..]}.
  - In Fill forms, a right-click on nom while plain's editor is open (plain click) opens no menu: the hitLayer contextmenu returns when `directActive` is set, renderer.ts ~8619.
- Ran under the lock: build, direct-edit8 / direct-edit7-save / direct-edit8-cut / direct-edit8-save drives (exit 0), node --test p6-drive8 + p6-drive7 (5/5). Fixtures: 9 OK, 55-line list identical.
