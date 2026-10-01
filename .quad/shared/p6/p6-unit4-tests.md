# P6 U4 tests/drive/GUI/docs evidence (Opus 5.5 tests seat, 2026-09-24)
Files (all new unless noted): tests/drive/direct-edit4.jsonl (65 steps), tests/drive/direct-edit5.jsonl (27 steps),
tests/p6-drive4.test.mjs (2 tests), .quad/shared/p6/checks/shot-direct4.mjs, .quad/shared/p3/design-agent-drive.md (appended
"## P6 U4 drive additions", 11 prose lines). scripts/drive.mjs untouched (no new packed key needed).

## Scripts
- direct-edit4: links.pdf read-mode follows (page/uri/blocked), annotate focus, grip move + undo, se resize, plain → / ⇧↓ nudge,
  Enter editor + type + Enter, ftp refusal (exact "link-set: ftp: is not a link this app opens", detail scheme, revision unchanged),
  Escape, L2 page retarget "1", Delete + undo, two drawn links (uri, page 3), <3 px drag (no draft, disarmed), 3x2 pt (Draw a bigger
  link), panel link {action:'set'}, rotated page-1 grip move, save-to → reopen → page 0/1 links; tabs.pdf C/A/S/fallback orders with
  Tab/Shift+Tab incl. cross-page t2 → s2. Fixture annotIds 10/11/13/14 in expects; drawn links omit annotId; absolute revisions.
- direct-edit5: outlines default on/off/on, Im2 plain → and ⇧↓ + two undos, form2 checkbox ↑ nudge (agree) + undo, radio size → M,
  unarmed redact drag 100x30 → 1 mark, 1 px drag and click → no mark, undo.
- p6-drive4: step counts 65/27, zero red, exit 0, 4 fixture hashes pinned before/after; pick()/near(): L1 grip +30/+40, undo, se
  [72,72,220,102], nudges +1/+10 ±0.05, refusal wrote nothing, drafts ±0.5, drawn links = drafts, rotated L4 [710,72,730,200],
  persisted rects ±0.01, saved bytes; Im2 +1 / −10 (y-up), agree [160,183,178,201], radio kid rects unchanged, mark [300,500,400,530] ±0.5.

## Runs (under .quad/run.lock)
- npm run build → 0; node scripts/drive.mjs direct-edit4 → exit 0 (65 steps, 0 red); direct-edit5 → exit 0 (27, 0 red).
- node --test tests/p6-drive4.test.mjs → 2 pass, 0 fail.
- node .quad/shared/p6/checks/shot-direct4.mjs → PASS all: handles visible on hover, real body drag L1 → [102,112.01,230,132.01],
  Enter+insertText+Enter → nav.links uri https://example.org/gui, real Tab a1→a2→a3→b1, Im2 plain → [401,700,461,740], outlines
  visible (u4-e-edit-outlines.png, viewed), real Redact drag → [300,500.14,400,530.15]. Shots checks/review-out/u4-*.png (8).
- fixtures.sha256 6/6 OK, u4-hash-before.txt all OK after every run; no --drive= / u4-udata stragglers.

## Not done / notes
- Full npm test not run (integrator's). pagePoint scales by #canvas (inside #page) width / widthPt.

## Fixes after review
- S1 (tests-drive) reproduced: review-u4-tests-probe.jsonl steps 1-9 → mark nudge rev 1→2 rect [301,500,401,530]; armed 1 px drag marks 1; armed 100×30 marks 2 [300,300,400,330]. It worked, but no script covered it.
- tests/drive/direct-edit5.jsonl: after the unarmed click (line 35) and before the undo tail: click (350,515) → active.kind redact; ArrowRight → rev 2;
  arm redact + drag 1 px → marks 1 rev 2; arm redact + drag 100×30 → marks 2 rev 3; undo tail is now 3 undos (rev 4, 5, 6 → marks 0, clean).
  Added after line 35, not 33, so the existing `next(mark)` (a 1 px drag moves nothing) check keeps its step.
  While a tool is armed, STATE direct.marks reads 0 (targets hidden), so the arm steps expect redact.marks 1.
- tests/p6-drive4.test.mjs: steps 27→35; pick() checks: mark → +1 pt x, both drags follow an arm, armed 1 px leaves the rects unchanged,
  armed area [300,300,400,330] ±0.5, the nudge is undone after two undos.
- Ran under the lock: tsc OK, build OK, direct-edit4 exit 0, direct-edit5 exit 0, node --test tests/p6-drive4.test.mjs 2/2 pass.
  fixtures.sha256 OK; u4-hash-before.txt unchanged; new files are only links.pdf and tabs.pdf; no stragglers.

## R1 drive coverage
- tests/drive/direct-edit4.jsonl: 6 steps inserted after the L2 retarget (report indices 26-31, 0-based): click L1, key Enter, type ftp://nope, mode read, wait 500 (error "", nav.last {link-set,0,scheme}, L1 uri https://example.org/changed, dirty true, revision 7, active null), mode annotate. The wait step exists because dropDirect's commit is async and the mode STATE can arrive first.
- tests/p6-drive4.test.mjs: direct-edit4 pinned count 65 → 71; pick('wait', {nav.last.detail:'scheme'}) asserts prev is mode, error '', L1 rect unchanged.
- Bite: built renderer.js dropDirect patched to commitLinkEditor(active, false) without a rebuild → drive exit 1, red step 30 (wait) and 31 (mode annotate) with STATE.error "link-set: ftp: is not a link this app opens". Rebuilt → green.
- Final: npm run build 0; node --test tests/p6-drive4.test.mjs pass 2/2; drive direct-edit4 exit 0, 0, 0 (71 steps, 0 red). fixtures.sha256: 6/6 OK.
