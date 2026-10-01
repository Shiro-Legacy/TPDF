# P6 U4 adversarial review — lens TESTS / DRIVE / EVIDENCE (Opus 5.5, 2026-09-24)
Verdict: ACCEPT (no blocking; 1 should, 3 nits). Every claim in p6-unit4-tests.md and the drive/GUI parts of p6-unit4-integration.md reproduced.

## Ran (all under .quad/run.lock, one hold)
- npm run build → 0; node --test tests/p6-drive4.test.mjs tests/p3-drive.test.mjs tests/p6-drive3.test.mjs tests/fixtures.test.mjs → 30 tests, 30 pass, 0 fail.
- node scripts/drive.mjs direct-edit4.jsonl ×3 → exit 0/0/0, 65 steps, 0 red; direct-edit5.jsonl ×3 → exit 0/0/0, 27 steps, 0 red.
- STATE diff run1 vs run2 and run1 vs run3 (scratch/rv-tests/cmp.mjs), ignoring status "rendered in N ms" and save.saveMs: 0 differing steps in both scripts.
- Probe checks/review-u4-tests-probe.jsonl (25 steps, no expects): plain → on a focused redact mark rev 1→2 rect +1 pt; armed Mark area dx 1 → no mark; armed Mark area 100×30 → mark [300,300,400,330];
  direct.links 0 in Forms/Edit/Redact, 3 in Annotate; link {action:'set', index:7} → nav.last detail no-such-link + "link: no link 7 on this page"; link {action:'set', index:0, page:2} → links[0].page 1;
  editor "9" → "Enter a page number between 1 and 3", editor stays open, rev unchanged. All behave as the contract says.
- node .quad/shared/p6/checks/shot-direct4.mjs → PASS all; same numbers as evidence ([102,112.01,230,132.01], Im2 [401,700,461,740], mark [300,500.14,400,530.15]).
- After all runs: shasum -c checks/fixtures.sha256 6/6 OK; u4-hash-before.txt 50/50 OK; pgrep --drive= / u4-udata empty.
- Backup diff (pre-p6u4-20260924.tgz): tests/p3-drive.test.mjs, tests/p6-drive3.test.mjs, scripts/drive.mjs identical; tests/fixtures.test.mjs +2 PINNED lines only.
- PNGs viewed: u4-a (grip + 8 handles on L1, L3 dotted), u4-b (L1 moved), u4-c-link-editor (editor shows https://example.org/gui), u4-e (dashed outlines on all objects, checkbox on), u4-g (one red mark + Comments row).

## Findings
S1 (should, tests) tests/drive/direct-edit5.jsonl:33-36 — §0 "every on-page interaction … has a drive path", §3.1 (plain arrow on a focused `.redact-target`), §3.3 (armed "Mark area" rewritten onto startAreaDrag "gets the 3 px threshold … and becomes drivable").
  Neither has a drive step in any tests/drive/*.jsonl (grep '"tool":"redact"' → none; no plain arrow on a mark). My probe proves both work today, so a regression would pass every gate.
  Fix: after line 33 add `click {page:0,x:350,y:515}` → direct.active.kind 'redact'; `key ArrowRight` → revision 2, rects [{search:false}] (delta +1 via pick()); `arm {tool:'redact'}`, `drag (300,300) dx 1 dy 0` → marks 1;
  `arm {tool:'redact'}`, `drag (300,300) dx 100 dy 30` → marks 2, revision 3; adjust the undo tail and the step count pin in tests/p6-drive4.test.mjs:132.
N1 (nit, tests) tests/drive/direct-edit4.jsonl:72 and :60 — §1.4 "other modes: none", §9.10 missing-row and 1-based `page`, §1.5 out-of-range page: none is expected in a drive (mode forms expect lacks `direct.links 0`; only the uri form of link set runs).
  Probe shows all correct. Fix: add `"links":0` to line 72's direct expect; add one `link {action:'set', index:9, …}` → nav.last {detail:'no-such-link'} and one editor "9" → error "Enter a page number between 1 and 3".
N2 (nit, tests) .quad/shared/p6/checks/shot-direct4.mjs:192 — §6.3 shots "show what they claim": u4-c-link-retargeted.png is byte-identical to u4-b-link-moved.png (sha1 6f5557c7…), so the retarget shot shows nothing; the claim rests on the nav.links read (which holds).
  Fix: click the Links tab (or hover L1 so its title shows) before shot('u4-c-link-retargeted.png').
N3 (nit, renderer) src/renderer/renderer.ts:5103, 5305 (announceNav → navStatusEl) with STATE announcement at 6617 = editStatusEl — §1.4 blocked-follow message and "Deleted link" are not observable in STATE
  (direct-edit4 step 5 and 27: announcement "" / "Link now goes to page 1"), so no drive can pin them. Fix: STATE `nav.status` = navStatusEl.textContent (new key, additive).

## Checked fine
- Every §6.2 gesture has an expect that fails without the feature: follows (page/nav.last), focus (direct.active.kind link), handles/nudges (absolute revision 1/3/4/5), editor (editing/draft), ftp refusal exact text + detail scheme + revision 6 + dirty true, Delete/undo counts, arm (direct.links 0), draw (linkDraft {page:0}), dx 1 (links 5 proves disarm), 3×2 (error "Draw a bigger link"), rotated handle rev 13.
- Every key step follows a click (lines 25, 33, 39, 44; Tabs 74, 80, 88; edit5 12, 20, 25). Arrays match exact length (main.ts:1660), so length claims are real.
- Reopen resets revision (line 67 revision 0, dirty false, page 0); page-1 rotated move persisted checked by pick() ±0.01 (p6-drive4:119); tabs.pdf C/A/S/fallback orders pinned per page incl. cross-page t2 → s2.
- No expect carries a gesture-produced rect (§9.8); p6-drive4 checks grip/undo/se/→/⇧↓/drafts/drawn/rotated/persisted/Im2/agree/radio/mark via pick()/near(); step counts 65/27, zero red, exit 0, 4 fixture hashes before/after, savedBytes == save.bytes (2264).
- shot-direct4 uses CDP Input.dispatchMouseEvent/dispatchKeyEvent/insertText, fresh udata per fixture, SIGTERM + shasum in finally; scales by #canvas (inside #page) width / widthPt — admitted in evidence, same box.
- design-agent-drive.md "## P6 U4 drive additions" (11 lines) matches shipped commands and STATE keys (driveArm link/off at renderer.ts:1410-1418, cancelPendingTools clears draft at 4095-4096).
- Not reproduced: full npm test 436/435/0/1 (integrator's number; outside this lens's run list).
