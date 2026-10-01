# Bates build record (implementer, Opus 5.5, lean, 2026-09-24)
Contract: contract-bates.md incl. §9. Scratch logs: .quad/scratch/bates/impl/.
## Changed
- src/renderer/engine.ts: StampRequest +batesStart/batesPad/align (~:385); MAX_BATES 1e15−1 / MAX_BATES_PAD 15 literals beside MAX_STAMP_CHARS (:144-145); stampTextFor replaces {bates} first; stampPage expands once (`expanded`), watermark width from expanded.length, right align measured with font.advanceGlyph(font.encodeCharacter(cp)) (advanceGlyph exists on 1.28.1 — the 0.556 fallback was NOT needed); stampPages checks align/pad/start(+end cap)/start-required via failReason("invalid","stamp: <token>: …") after checkPages, before mutate; reply `bates {from,to}` only when the text has {bates}. mutate, checkStampText, winAnsiBytes untouched.
- src/renderer/renderer.ts: #stamp-align/#stamp-bates-start/#stamp-bates-pad refs; module batesNext/batesPad/batesLast/stampBates (never reset); `last` type + decorateLast param gain bates?; driveStamp writes batesStart/batesPad verbatim, align left/right else "Align must be left or right"; openStampDialog prefills; syncStampHeading disables align for watermark; saveStampDialog §9.11 regexes + showError (U+2013); addStamp sends options, sets counter, decorateLast (ok/fail), announce "(Bates 000500–000502)" in #page-status; STATE emit adds decorate.bates.
- src/renderer/index.html: dialog fields + hint "{bates}"; <h3>Headers, footers and Bates numbers</h3><ul id="help-bates"> after #help-ceilings.
- tests/p2-decorate.test.mjs: +6 tests bates (a)-(f) appended (end of file).
- tests/p2-shell.test.mjs re-pins: :51 REQUIRED_IDS += help-bates, stamp-align, stamp-bates-start, stamp-bates-pad; :95 hint assert (new); :140 "undo does not move it back" assert (new).
- tests/drive/bates.jsonl (new, 9 steps, 0 click lines), tests/p6-bates.test.mjs (new, 1 test), .quad/shared/p6/checks/shot-bates.mjs (new; node --check only, NOT run), .quad/shared/p3/design-agent-drive.md (appended "## Bates drive additions").
## Red before fix (acceptance/bates-red-before.log, old build/, 8 not ok)
- bates (a): deepStrictEqual reply.bates undefined vs {from:100,to:102}
- bates (b): deepStrictEqual reply.bates undefined vs {from:100,to:101}
- bates (c): {"text":"{bates}"} `true !== false` (reply.ok true, no refusal)
- bates (d): 'R=0: the Bates line is found' (text stayed literal {bates})
- bates (e): reply.bates undefined vs {from:7,to:7}
- bates (f): /999999999999999/ did not match 'one\n\n{bates}'
- p2-shell required IDs: missing [help-bates, stamp-align, stamp-bates-start, stamp-bates-pad]
- p2-shell help dialog: 'Help names the Bates undo ceiling' false
## Deviation from the verbatim template (§9.18)
- bates.jsonl dirty values: save-to is a copy and keeps dirty true (existing behaviour, cf. direct-edit3.jsonl:64), so S3/S6 dirty false → true, S7 (undo) dirty true → false, S8 dirty true → false (journal back at the never-saved position 0). The template ledger assumed save-to clears dirty. All other values verbatim.
## Checks (one lock hold each)
- npx tsc --noEmit clean; npm run build ok.
- node --test p2-decorate p2-shell p3-drive p6-bates: 46/46 pass, 0 fail.
- Drives exit 0 / 0 red: bates 9, dialogs 11, refusals 29, help-fields 21, direct-edit8-chords 19, replace-page 7, direct-edit 75, -2 50, -3 79, -4 71, -5 35, -6 59, -7 66, -7-save 19, -8 115, -8-cut 51, -8-save 42, -9 43, object-edit 11, content-edit 4, debt1 22.
- npm test (once): tests 551, pass 550 (= debt2 543 + 7 new), fail 0, skipped 1.
- fixtures.sha256 9 OK; bates-hash-before.txt (55) identical after; nothing added under fixtures/; no --drive= stragglers.
## Not done
- shot-bates.mjs not run (fable51 runs it in acceptance; ⌥⌘F real-keystroke collision still UNCHECKED). Drives ×3 and npm run package left to acceptance.
## Fixes after review
- T-R1 (shot-bates.mjs:190): Apply gate now waits for `decorate.last` (absent after the open per §9.13, set only after applyEdit's awaits) and then asserts `revision 1`, `last.ok true`, `bates.next 503`; `node --check` only, not run.
- T-R2: new tests/drive/bates-pad.jsonl (5 steps, 0 click lines): nav footer start 1 `batesPad "4"` → bates {next 4, pad 4}; header with no start/pad → from 4 to 6, pad 4; align "centre" → "Align must be left or right", revision 2, counter unchanged; save-to. Second test() in tests/p6-bates.test.mjs deep-equals decorate.bates, pins the refusal, and reads ACME-0001..0003 / 0004..0006 from the saved file. Fails if renderer.ts `batesPad = pad` is removed (pad stays 6). bates.jsonl untouched.
- T-R4: `assert.equal(line.target, 576)` for bates-b p0.
- design-agent-drive.md Bates section names bates-pad.jsonl.
- Checks (one lock hold): tsc clean, build ok, p6-bates 2/2 pass, bates-pad drive exit 0, shot-bates node --check ok; fixtures.sha256 9 OK; bates-hash-before.txt list identical.

## Fixes after review
- E-R1 reproduced (probe: real '[L{bates}-2] 09', M1 '[L09-2] 09'; M1 6/6 green). E-R2 reproduced (triple {bates} pad 15 watermark centre: real (306,404), M4 (376.5,332.5); M4 6/6 green).
- tests/p2-decorate.test.mjs only: lineMatching also returns cx/cy (additive); bates (e) gains a '{bates}{bates}{bates}' pad-15 watermark on p0 asserted within 30 pt of (306,396), then labels-set prefix 'L{bates}-' + header '[{label}] {bates}' start 9 pad 2 on p1 asserted /\[L\{bates\}-2\] 09/.
- Kill check (.quad/scratch/bates/fix-engine/mut): M0 6/6 green; M1 and M4 now fail bates (e). No engine change (no defect).
- tsc --noEmit clean; npm run build; node --test tests/p2-decorate.test.mjs 14/14 pass. fixtures.sha256 9 OK; bates-hash-before.txt identical.

## Fixes after review (renderer)
- B-R1/B-R2/B-R3 all reproduced pre-fix with checks/review-bates-renderer-mouse.mjs (scratch/bates/fix-renderer/pre.log): stale Bates error after a later Apply; decorate.last still the 500–502 success after a GUI refusal; "Added header to 1 page (Bates 000503–000503)".
- renderer.ts saveStampDialog: clearError() first; the Bates refusal calls decorateLast({ action: "stamp", kind, ok: false }) before return false (driveStamp still overwrites with pages, so bates.jsonl step 8 is unchanged). addStamp: one-page range announces "(Bates 000503)", 2+ pages keep the en dash range.
- tests/p2-shell.test.mjs: new test pins the three edits; checked to fail on the pre-fix source text (false/false/false) and pass after.
- Post-fix probe: stale-error gate ok, single page "Added header to 1 page (Bates 000503)", decorate.last {action:"stamp",kind:"header",ok:false}; the probe's own "(stale?)" gate now reports FAIL as intended (it asserted the bug). Its "undo: revision 0" gate fails pre- and post-fix (probe assumption, not a finding).
- tsc clean; build; p2-shell + p3-drive 32/32; drive bates.jsonl exit 0; fixtures.sha256 9 OK; bates-hash-before.txt identical.
