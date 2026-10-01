# P6 U8 review — ENGINE lens (Opus 5.5, 2026-09-24) — verdict ACCEPT (0 blocking, 0 should, 2 nits)

Ran (under .quad/run.lock): `npm run build` + `node --test` p6-text8 p6-objects7 p3-targets p3-edit p3-replace p6-direct3 p6-paragraph p3b-content → 140/140 pass. tsc clean. fixtures.sha256 9 OK; u8-hash-before diff empty. No Electron launched.
Probes: `.quad/scratch/p6u8/review-engine/probe.mjs`, `probe-dupstream.mjs`; mutants `mut1..mut5/` (build copy + tests copy + fixtures symlink).

## Findings
- N1 (nit, engine) content-edit.ts:1269 and :2943 — the `depth` argument at editMeasureRun and editPlanParagraph is unpinned: mutants passing `0` there (mut3, mut4) pass all 121 tests of p6-text8/p6-objects7/p3-targets/p3-replace/p6-paragraph/p3-edit. Near-equivalent in practice (for a cowable Form run both branches return the same glyphs); §9.3's required param + tsc is the real guard. Fix: none required; optionally a comment at :1269 saying so.
- N2 (nit, engine) tests/p6-text8.test.mjs:61-86 — (a) does not save → reopen, and undo is checked by scan only (not bytes). Probe shows both hold (edit A → save garbage=yes → reopen: "Twix" at A's bounds, "Twin" B id/bounds unchanged; undo → /Contents bytes identical). Fix: append `reopen` + `contents()` equality to (a).

## checkedFine
- Diff vs .quad/backup/pre-p6u8-20260924.tgz content-edit.ts: exactly §1.1 (:1475 accept line), §9.3 (required 7th param, no defaults on anyOfEqual/kerned/origin either — only caller set is the three sites), §1.2 (:1269 record.depth, :2362 walk depth, :2943 first.depth), §1.3 comment. `!origin` early-outs, limit, overflow, anyOfEqual paths untouched. `let depth` at :2247 is block-local, not what :2362 sees (walk param :1928).
- (1) twins: ids p0.c1.89.f22530a3 / p0.c2.89.f22530a3, bounds [300,387.1,325.33,403.59]/[300,287.1,325.33,303.59], second text-add reply bounds match; edit A → B id/bounds/text unchanged; survives save+reopen; undo restores bytes; edit B after undo leaves A.
- (2) same origin: both unsupported/ambiguous-text-association, bounds null; edit-text refuses not-candidate, bytes + revision unchanged. edit-shared.pdf Form ALPHA ×2: candidate, cowable, bounds null (§9.2 — NOT ambiguous; brief's "edit refuses" is superseded by §9.2: edit privatises one placement, the other ALPHA stays).
- (3) p6-objects7:450-466 per §9.5: both candidate, pasted bounds = source+10 ±0.5 from scan, targetIds = [pasted id].
- (4) "Move me" add → both candidates (test (b)); paragraph "Twin\nsecond line here" + single "Twin" elsewhere: both Twin candidates, edit-paragraph ok and rewraps, the other Twin untouched.
- (5) scoped replace (test (e)) rewrites only B; unscoped preview matches per occurrence (Twiz matched, Twin rejected overflow) — truthful.
- One stream referenced twice in /Contents with a cm between (probe-dupstream): pre = both cowable bounds null; post = both cowable with own bounds; edit one privatises its element, the other copy unchanged in stext.
- (7) bite: mut1 (revert rule) fails (a),(b),(e), p6-objects7 paste, and the 4 p3-targets flips; mut2 (drop depth at :2362) fails (d) and p3-targets edit-shared; mut5 (near.length >= 1) fails (c). Red-before log: 3 not ok (a,b,e), 2 ok.
