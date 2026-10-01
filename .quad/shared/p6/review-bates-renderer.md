# Bates review — renderer + human UX, real CDP pointer (Opus 5.5, 2026-09-24)
Verdict: REVISE (1 should, 2 nits; no blocking). Probes: .quad/shared/p6/checks/review-bates-renderer-mouse.mjs (33/35 gates; the 2 red are B-R1 and a wrong probe expectation, see below), review-bates-renderer-shot.mjs (canvas pixel scan). Log: .quad/scratch/bates/review-renderer/mouse.log.

## Findings
- **B-R1 (should) renderer.ts:7634 saveStampDialog, §9.11/§9.15, STATE truthfulness.** A human's Bates refusal leaves `#error` (and STATE `error`) set after a later successful Apply: nav, start "abc" + {bates} → refused (ok); then text "Page {page} of {pages}" → Apply succeeds, revision 1→2, but STATE.error still "Bates start must be a whole number and digits 1–15" and the red bar stays (shot c). The drive never sees it because driveStamp calls clearError() first. Same latent gap for the old pages refusal, but Bates adds 5 new ways to hit it. Fix: `clearError();` as the first line of saveStampDialog.
- **B-R2 (nit) renderer.ts:7650-7653, §9.8.** The GUI refusal (bad start/pad, before addStamp) does not call decorateLast, so STATE decorate.last still reads the previous `{…ok:true,bates:{from:500,to:502}}` while the dialog shows a refusal; the drive path records ok:false. Fix: `decorateLast({ action:"stamp", kind, ok:false })` before the `return false` (kind computed one line earlier than today).
- **B-R3 (nit) renderer.ts:7679.** A one-page Bates stamp announces "Added header to 1 page (Bates 000503–000503)"; print one number when from === to.

## checkedFine (real pointer unless noted)
- Fresh nav: no `decorate` key. Dialog opened by #stamp-header .click() (the button sits under "Inspector" at 1280 px — elementFromPoint misses it, pre-existing toolbar overflow, not Bates); prefill start "1", pad "6".
- #stamp-kind footer + change → heading "Add footer"; ⌘A+insertText into #stamp-text/#stamp-bates-start; align right; real mouse press/release on #stamp-save → dialog closed, decorate.last `{action:stamp,kind:footer,pages:[0,1,2],ok:true,bates:{from:500,to:502}}`, decorate.bates `{next:503,pad:6,last:{from:500,to:502,pages:[0,1,2]}}`, #page-status exactly "Added footer to 3 pages (Bates 000500–000502)" (U+2013), revision 1, error "".
- Reopen: start "503", pad "6", align reset "left". Watermark → #stamp-align.disabled true, heading "Watermark"; back to header → enabled.
- Refusals with {bates}: "abc", "" (never 0), pad 16, pad 0, 16-digit start, "0x10", "1e3" → dialog open, exact U+2013 message, revision unchanged.
- No {bates} + start "abc" → applied (§2.2/§9.11: fields parsed only with the token); last has no `bates` key; next stays 503; "Added header to 3 pages".
- Drive `open hello` → `decorate` absent; dialog prefill "503" carried; header "DOC {bates}" → last.bates {503,503}, next 504. Real click on #undo → revision 2 (monotonic journal revision, as bates.jsonl S7), next stays 504, reopen prefill 504 (D2 ceiling); Help sentence says "undo does not move it back".
- Help: #help-ceilings nextElementSibling is `<h3>Headers, footers and Bates numbers</h3>`, then ul#help-bates, nothing between. All new controls have ids and exactly one label (Align, Bates start, Digits).
- Right alignment (shot probe, canvas backing-store scan, nav 612 pt, 2.614 px/pt): footer ACME-000500 right edge ≈575.4 pt vs 576 target; left header "Page 1 of 3" starts ≈36.7 pt — margins match within 1 pt.
- Backup diff (index.html, renderer.ts, p2-shell.test.mjs): only contract-allowed hunks; p2-shell pins (REQUIRED_IDS +4, hint regex, undo-ceiling assert) additive; node --test p2-shell 10/0.
- Drives under lock, exit 0 / 0 red: bates 9, dialogs 11, refusals 29, help-fields 21, direct-edit8-chords 19, replace-page 7, direct-edit3 79, direct-edit8-cut 51, object-edit 11, content-edit 4, debt1 22. tsc clean; npm run build 0.
- fixtures.sha256 9 OK; pdf/png list identical to bates-hash-before.txt (55); no stragglers.

## Screenshots (.quad/shared/p6/checks/review-out/)
- review-bates-0-organize.png — organize toolbar at 1280 px; Header/footer… clipped behind Inspector.
- review-bates-a-dialog-filled.png — dialog filled: footer, ACME-{bates}, Right, start 500.
- review-bates-b-refusal-abc.png — dialog open, start "abc", refusal in the dimmed #error bar behind the modal.
- review-bates-c-nav-fitpage.png — after the successful plain header: stale Bates error bar still shown (B-R1); Text panel lists ACME-000500.
- review-bates-d-hello-after-undo.png — hello after undo.
- review-bates-e-nav-p1-canvas.png — canvas clip (bottom not captured; the pixel scan is the measurement).
