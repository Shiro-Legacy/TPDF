# P6 U8 objections — tests lens (Opus 5.5, 2026-09-24)
Probes: `.quad/scratch/p6u8/objections/tests/` — probe1/probe2 (engine on build/), `sim/` = copy of build/ with the §1.1 patch at all three call sites (sim-run.out, sim-full.out). fixtures.sha256 9 OK; hashes = u7-hash-after (55 lines) after every run.

**T-1 (blocking) §1.4/§1.5/§4** — Four P3 tests flip, not one: with the §1.1 rule, `tests/p3-targets.test.mjs` tests at :199 (pin :237/:239 rtl.pdf " PDF4All"), :365 (pin :386), :533 (pin :547) and :631 (pin :636, targets-syntax.pdf ALPHA ×4) fail, plus p6-objects7:457 **and** :466 (`targetIds` [] becomes two); evidence sim-run.out `not ok 78/83/88/91/127`. p3-targets is owned by nobody (§4). Amendment: "§9.x: the rule flips p3-targets.test.mjs:237-239, :386, :547, :636 (now `support 'candidate'`, distinct targetIds, bounds at their own origins) and p6-objects7.test.mjs:455-466 (both candidates, `reply.targetIds.length === 1` = the pasted run, test renamed); the engine seat owns tests/p3-targets.test.mjs for those four pins only, and each rewritten pin keeps its distinct-identity assert."
**T-2 (blocking) §1.4/§1.5(d)/D1** — A repeated Form never reports `ambiguous-text-association`: fixtures/p0b/edit-shared.pdf p0 (`/Fm1 Do` twice) gives both ALPHA runs `candidate, cowable true, reason undefined, bounds null` before and after the patch (p3-targets.test.mjs:188-191; probe2.out); direct3.pdf p2 holds only "On top" at depth 0 (probe2.out). Case (d) as written cannot pass. Amendment: "§9.x: case (d) opens fixtures/p0b/edit-shared.pdf page 0 and asserts both 'ALPHA' items `support 'candidate'`, `reason undefined`, `bounds null` (unchanged by U8; not a red-before-fix case); D1's 'Forms keep the P3 rule' means Form runs keep no on-page bounds."
**T-3 (blocking) §5** — §5 has no GUI proof, but the run requires a real osascript ⌘-keystroke in accept-u8.sh; accept-u7.sh:17-19 is the shot precedent (CDP only; no osascript script exists under .quad/shared/p6/checks — grep "System Events" finds none). Amendment: "§9.x: accept-u8.sh runs `node checks/shot-direct9.mjs > u8-shot.log` (fail=1 on non-zero) and gates `u8-a-twins.png` exists and 9 fixtures OK after it; the script is specified in 'GUI proof' of objections-p6-unit8-tests.md."
**T-4 (should) §3.1** — "the original 'Move me' run at (82,376.95…)" clicks the run's top edge (bounds [72,376.95,129.57,396.19], direct-edit8.jsonl:8-9; probe1.out). Amendment: "§9.x: the original run is clicked at its centre (100.78, 386.57)."
**T-5 (should) §3.1** — "targetId B ≠ A" holds before the fix too: each text-add appends a stream, ids are `p0.c1.89.f22530a3` / `p0.c2.89.f22530a3` on the pre-unit build (probe2.out); the real gate is `active.kind "text"` (pre-fix the clicks at (305,397)/(305,297) hit no target and open `kind "add"`). A JSONL cannot compare steps. Amendment: "§9.x: tests/p6-drive9.test.mjs pins by index: S(9).…targetId ≠ S(11).…targetId; S(16).targetId === S(9).targetId (A untouched by B's edit); S(14).targetId ≠ S(11).targetId (B rewritten); S(24).targetId === 'p0.c0.314.6c51e7e2' ≠ S(26).targetId; `direct.active.rect` near [300,387.1,325.33,403.59] (S9, S16) and [300,287.1,325.33,303.59] (S11) ±0.5."
**T-6 (should) §5** — accept-u7.sh:8 gates only `# fail 0`; the §5 count and "red-before-fix shown in the engine report" are not gates. Amendment: "§9.x: accept-u8.sh fails unless `# fail 0`, `# skipped 1` and `# pass` ≥ 529 + new tests; the engine seat copies build/ to .quad/scratch/p6u8/prebuild before its first edit and runs p6-text8 there (tests/ copy + fixtures symlink, as objections/tests/sim) into acceptance/u8-red-before.log, gated `grep -c '^not ok' ≥ 3` naming (a), (b), (e)."
**T-7 (should) §1.2** — content-edit.ts:2941 is inside `editPlanParagraph` (:2909, edit-paragraph), not find-and-replace; case (e) (`edit-replace-preview`/`-apply` with `targetId` of run A) rewrites only A in the sim (probe2.out). Amendment: "§9.x: :2941 is the edit-paragraph plan (`first.depth`); find-and-replace reaches the rule through the scan (:2360) and editMeasureRun (:1269)."
**T-8 (should) §5** — the drive list drops seven U7 regressions (accept-u7.sh:12) incl. help-fields while Help text changes. Amendment: "§9.x: accept-u8.sh runs the accept-u7.sh:12 list unchanged plus direct-edit8 and direct-edit9 ×3."
**T-9 (nit) D2** — the Help pin is tests/p2-shell.test.mjs:217, not :205.

## Exact line templates (tests/drive/direct-edit9.jsonl, 27 steps, 9 clicks)
```
# P6 U8 duplicate text stays editable (contract-p6-unit8.md §3.1). fixtures/p6/direct3.pdf p0, y-down pt. Twins land at [300,387.1,325.33,403.59] and
# [300,287.1,325.33,303.59]; "Move me" copy [300,187.1,349.34,203.59]; original (100.78,386.57). expect exact, revision absolute; ids/rects by index in the test.
# 27 steps, 9 click lines (pinned in tests/p6-drive9.test.mjs). Ledger: 0 open, 1-2 adds, 3 edit B, 4-6 undo, 7 add "Move me".
{"cmd":"open","path":"fixtures/p6/direct3.pdf","expect":{"doc":{"name":"direct3.pdf","pageCount":3},"dirty":false,"revision":0}}
{"cmd":"mode","contents":{"mode":"edit"},"expect":{"mode":"edit","direct":{"mode":"edit","objects":3,"targets":5,"active":null}}}
{"cmd":"click","page":0,"x":300,"y":400,"expect":{"direct":{"active":{"kind":"add","page":0,"fontSize":12,"draft":""}},"revision":0}}
{"cmd":"type","text":"Twin","expect":{"direct":{"active":{"kind":"add","draft":"Twin"}}}}
{"cmd":"key","key":"Meta+Enter","expect":{"direct":{"active":null,"targets":6},"dirty":true,"revision":1,"error":""}}
{"cmd":"click","page":0,"x":300,"y":300,"expect":{"direct":{"active":{"kind":"add","page":0,"draft":""}},"revision":1}}
{"cmd":"type","text":"Twin","expect":{"direct":{"active":{"kind":"add","draft":"Twin"}}}}
{"cmd":"key","key":"Meta+Enter","expect":{"direct":{"active":null,"targets":7},"revision":2,"error":""}}
{"cmd":"click","page":0,"x":305,"y":397,"expect":{"direct":{"active":{"kind":"text","page":0,"value":"Twin"}},"revision":2}}
{"cmd":"key","key":"Escape","expect":{"direct":{"active":null},"revision":2}}
{"cmd":"click","page":0,"x":305,"y":297,"expect":{"direct":{"active":{"kind":"text","page":0,"value":"Twin"}},"revision":2}}
{"cmd":"type","text":"Twin 2","expect":{"direct":{"active":{"draft":"Twin 2","value":"Twin"}}}}
{"cmd":"key","key":"Meta+Enter","expect":{"content":{"edited":{"from":"Twin","to":"Twin 2"}},"direct":{"active":null,"targets":7},"revision":3}}
{"cmd":"click","page":0,"x":305,"y":297,"expect":{"direct":{"active":{"kind":"text","value":"Twin 2"}},"revision":3}}
{"cmd":"key","key":"Escape","expect":{"direct":{"active":null},"revision":3}}
{"cmd":"click","page":0,"x":305,"y":397,"expect":{"direct":{"active":{"kind":"text","value":"Twin"}},"revision":3}}
{"cmd":"key","key":"Escape","expect":{"direct":{"active":null},"revision":3}}
{"cmd":"undo","expect":{"revision":4,"canRedo":true,"direct":{"targets":7}}}
{"cmd":"undo","expect":{"revision":5,"direct":{"targets":6}}}
{"cmd":"undo","expect":{"revision":6,"dirty":false,"direct":{"targets":5}}}
{"cmd":"click","page":0,"x":300,"y":200,"expect":{"direct":{"active":{"kind":"add","page":0,"draft":""}},"revision":6}}
{"cmd":"type","text":"Move me","expect":{"direct":{"active":{"kind":"add","draft":"Move me"}}}}
{"cmd":"key","key":"Meta+Enter","expect":{"direct":{"active":null,"targets":6},"dirty":true,"revision":7,"error":""}}
{"cmd":"click","page":0,"x":100.78,"y":386.57,"expect":{"direct":{"active":{"kind":"text","value":"Move me"}},"revision":7}}
{"cmd":"key","key":"Escape","expect":{"direct":{"active":null},"revision":7}}
{"cmd":"click","page":0,"x":305,"y":197,"expect":{"direct":{"active":{"kind":"text","value":"Move me"}},"revision":7}}
{"cmd":"key","key":"Escape","expect":{"direct":{"active":null},"revision":7}}
```
Revisions/targets/bounds/ids derived from the engine sim (probe1-sim.out, probe2.out); `content.edited` `from/to` shape from direct-edit.jsonl:60; GUI rounding of clicks UNCHECKED (±0.5 pt, p6-drive3.test.mjs:111). No save steps: the script writes nothing. Optional +4 (31 steps, 10 clicks): `save-to "${OUT}/direct9.pdf"` (rev 7, dirty true), `open "${OUT}/direct9.pdf"` (revision 0), `mode edit` (targets 6), click (305,197) → text "Move me" — proves the copy stays editable after reopen.
**Engine shapes (§1.5, harness = p6-objects6.test.mjs:12-37):** `{op:'text-add',page:0,point:[300,400],text:'Twin',font:'Helvetica',size:12,color:[0,0,0]}`; `{op:'edit-targets',page:0}`; `{op:'edit-text',page,targetId,generation,revision,text:'Twin 2'}`; `{op:'undo'}`; `{op:'edit-replace-preview',page:0,generation,revision,find:'Twin',replace:'Pair',targetId}` then `{op:'edit-replace-apply',…,previewToken,targetIds:preview.matches.map(m=>m.targetId),targetId}` (p3-replace.test.mjs:48-64). (c) = two text-adds at the same point → both `ambiguous-text-association`, bounds null (probe2.out). (d) per T-2.
**GUI proof (checks/shot-direct9.mjs, template shot-direct8.mjs:103-160 without `--inspect`):** spawn the electron binary with `--remote-debugging-port`, a fresh `--user-data-dir` under checks/review-out, `--drive=<hold probe opening fixtures/p6/direct3.pdf> --gui`; mode edit; CDP click at pagePoint(300,400); `osascript -e 'tell application "System Events" to tell (first process whose unix id is <child.pid>) to set frontmost to true'`; each keystroke is ONE osascript call `tell (first process whose unix id is PID) … if frontmost then keystroke "Twin" … else error "not frontmost"` (same for `keystroke return using command down`); on "not frontmost" or any other app frontmost, send nothing, log `blocked: frontmost <name>` and exit 1 (never retry into another app). Gates: after each ⌘Return `direct.active` null and revision +1 (1, then 2 after the second add at (300,300)); CDP clicks at (305,397) and (305,297) each give `kind "text"`, value "Twin", distinct targetIds; shot `u8-a-twins.png`; SIGTERM child; fixtures.sha256 9 OK.
## checkedFine
- direct3 p0 has 5 text targets with bounds (`direct.targets` = text items with 4 bounds, renderer.ts:7884; probe1.out); 7 after two adds post-fix, 5 pre-fix (sim vs build).
- Add point is the baseline: "Twin" at (300,400) → [300,387.1,325.33,403.59]; clicks (305,397)/(305,297)/(305,197) fall inside (probe1/probe1-sim).
- Opening a run editor selects all (renderer.ts:3840), so `type` replaces (renderer.ts:1392); a non-paragraph run commits on Enter/⌘Enter (renderer.ts:3843); the add editor needs ⌘Enter (direct-edit3.jsonl:90).
- Red-before-fix: (a) twins, (b) "Move me" copy (both ambiguous, 4 targets) and (e) fail on build/; they pass on the sim (probe1.out, probe2.out).
- p3-replace "ALPHA four" (ActualText mismatch) and replace-page.jsonl stay ambiguous (p3-replace passes in sim-run.out).

## buildRisks
- renderer.ts:3958 still announces "…(it repeats an existing line)…" for any bounds-less add; after U8 that fires only for overprints: reword it or it misleads.
- The sim-full.out extra failures (p2-shell, p1-save, p5-verify-crypto, p0b table) are artefacts of the sim copy lacking src/ (ENOENT); causes of p1-save and p0b-table UNCHECKED; run the real suite.
- targetIds end in a text hash (`…6c51e7e2` for both "Move me"); never pin a twin's id by suffix alone.
