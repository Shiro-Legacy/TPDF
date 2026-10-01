# Debt sweep 2 review — TESTS / DRIVE / EVIDENCE lens (Opus 5.5, 2026-09-24)
Verdict: ACCEPT (no blocking; 1 should, 2 nits). Scratch: .quad/scratch/debt2/review-tests/ (c1..c3, cut, npmtest.log).

## Ran (one lock hold)
- chords ×3 via scripts/drive.mjs: exit 0/0/0, 19 steps, 5 clicks, 0 red each; the three step+state dumps differ only in "rendered in N ms" (normalised diff 0 lines).
- direct-edit8-cut: exit 0, 51/11, 0 red. npm test (with build): 542 / 541 pass / 0 fail / 1 skip — matches debt2-integration.md; the six debt2 tests and "drive debt2 …" are ok.
- fixtures.sha256 9 OK after; pdf/png list byte-identical to debt2-hash-before.txt (55). No secret material in npmtest.log.
- Vacuity probe on c1: no step's expect is already satisfied by the previous step's state (every chords expect bites).
- node --check shot-debt2.mjs, shot-direct8.mjs, review-u7-renderer-probe.mjs: OK. shot-debt2 NOT run.

## Findings
- S-1 (should) §9.9/§6.3 — the only test of §9.8 (blocking R-1: unchanged Apply stays a no-op) and §2.2 rounding is shot-debt2 case B, which has never run, and its required pre-fix bite ("shown to bite once against the pre-fix build") is absent; debt2-red-before.log has 6 reds, none for rounding/passthrough. Repro: grep -c content-box acceptance/debt2-red-before.log → 0; debt2-build.md "Not done". Fix: acceptance runs shot-debt2 once on the pre-debt2 build (from the backup tgz) and logs the TWO-regex FAIL, then PASS on the current build.
- N-1 (nit) §5 — build and integration defer shot-debt2, direct-edit8-save/7/6 and package to accept-debt2.sh, which does not exist yet (find .quad -name 'accept-debt2*' → empty). Fix: fable51 writes it before accepting.
- N-2 (nit) §6.3 — red-before evidence for T-7 (chords S17 / cut.jsonl:31) and the p2-shell Help pins lives only in the objections probe, not acceptance/debt2-red-before.log. Fix: cite objections-debt2-tests.md probe S17 in the acceptance log.

## checkedFine
- Backup diff (all tests/*, tests/drive/*, p6/checks/*, design-agent-drive.md): only p6-direct, p2-shell, p6-objects6, p6-objects7, p6-drive8, direct-edit8-cut.jsonl, shot-direct8.mjs, review-u7-renderer-probe.mjs, design-agent-drive.md changed; each diff is what §3/§9 allow. No line inserted into an existing drive script; cut.jsonl:31 is a value-only change (announcement string).
- chords.jsonl = objections template verbatim (only the §9 → §9.11 comment differs); header 19/5 = file counts = counted() pin; revisions absolute on every non-mode line (mode lines match house style in cut.jsonl/direct-edit8.jsonl). ⌘C S4 pins clipboard {count:1,image:1} after S2 pins null; S8 flips kinds to form:1 so S10's ⌘X image:1 proves Cut wrote; S17 pins content.edited {kind:selection, action:delete, name:Im1, count:1} and clipboard count 2.
- p6-drive8 fourth test: FIXTURES sha before/after, one/all/has pins from the template, the no-draft Shift-click check (report steps carry contents.shift — verified S15/S16).
- §9.12 T-5 re-derived on impl/d-direct-edit8/stdout.json: S45 match p2.c1.111.6c51e7e2 (page-2 c0 run at x 72 not matched), flipped suffix false; S96 [300,600,400,630] 1 hit, flipped [301,600,401,630] 0 hits.
- red-before log: 6 not-ok, 0 pass — cutAnnouncement (§9.10), N1 name/page/link-set (§9.1-9.2), N2 Tr 1/Tr 2 (§9.3-9.4).
- shot-debt2.mjs read fully: every osascript call targets `first process whose unix id is <child.pid>` with the frontmost check in the same script, blocked → exit 1 no retry, -1712/1002 → OSA-UNAVAILABLE exit 2; SIGTERM + shasum in finally, 9-OK gate at end; A gates on exact #edit-status, objects 2, clipboard.count 2, revision, debt2-a-cut.png, ⌘Z objects 3/dirty false; B drags with META on every CDP event (meta disables snap, renderer.ts:2453), "no float to round" precondition, TWO regex + round2 equality, real click on #content-apply-object → revision/dirty unchanged, ⌘Z restores [72,600,172,650].
- design-agent-drive.md new EOF section matches engine.ts:3819-3829 (ALLOW, message, idOf index+id, pass before repeat check and dispatch) and engine.ts:3592 (`${tr} Tr 0 g 0 G 1 w 0 J 0 j [] 0 d`), and cutAnnouncement's singular text.
- Other reviewer's Electron (review-out/debt2-review-udata-r, --drive=) was live throughout; not mine, left alone.
