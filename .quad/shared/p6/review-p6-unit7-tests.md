# P6 U7 review — tests / drive / evidence lens (Opus 5.5, 2026-09-24)

**Verdict: ACCEPT.** No blocking findings. Three should-level gaps: in each, the behaviour works when probed, but no drive line or test pins it.
Scratch: `.quad/scratch/p6u7/review-tests/` (`run1.sh`, `run2.sh`, `save.jsonl`, `cut.jsonl`, `d8-{1,2,3}.json`, `save.json`, `cut.json`, `shot8.log`, `npmtest.log`).

## Ran (three lock holds)
- Hold 1: `fixtures.sha256` 9 OK and the list identical to `u7-hash-before.txt` (55), before and after. `npm run build` 0.
  `node --test` p6-drive8, p6-drive7, p3-drive, p6-drive3, p6-drive6: 26 pass, 0 fail.
- `direct-edit8` ×3: exit 0, 104 steps, 0 red each; wall 4 s, 3 s, 3 s. The reports are byte-identical once elapsedMs, out,
  render-time status and memory are stripped (no nondeterminism).
- Hold 2: probes `save.jsonl` (paste, save-to, reopen for images on objects6 p0/p1, a run on direct3 p2, the radio `color`, a link)
  and `cut.jsonl` (Cut of 2 links / a run alone / run + Im1 / the radio, each then ⌘V; Shift-click on a paragraph; contextmenu on an
  unselected field; page menu in Redact). Both exit 0. `U7_SKIP_D=1 shot-direct8.mjs` exit 0, "PASS all", 21 s.
  Case D not run: it writes the human's OS clipboard (§9.41).
- Hold 3: `npm test` 523 / 522 pass / 0 fail / 1 skipped, matching the integration file.
- After every hold: 9 OK, list identical, no `--drive=` or `u7-udata` process. The `mutate` diff (engine.ts:1790 vs backup :1732)
  is only the §9.2 line.

## Findings
**T-1 (should, tests) §10.8 / §10.21 / §10.42 — no Cut of a non-object selection is driven or pinned.**
direct-edit8 has one Cut: S11, an image (direct-edit8.jsonl:24). §10.21 was a blocking objection, yet the edit-batch delete half, the
per-kid radio delete, the runs-only Cut (no journal op) and the §10.42 announcements have no test. `grep -rl "text runs cannot be cut" tests/`
finds only the p2-shell Help check. `cut.jsonl` shows it works today: 2 links → "Cut 2 items", rev 1, ⌘V → 3 links; a run alone →
"Copied 1 text run; text runs cannot be cut", rev 0; run + Im1 → objects 2; the radio → fields 5→3, ⌘V restores "color" (5).
A regression in any of these stays green. Fix: a new drive script with these four cuts, exact `announcement`/`revision`/counts/`content.edited`, pinned by step index.

**T-2 (should, tests) §0 / §9.53 — no pasted kind is proven through save → reopen in a drive.**
direct-edit8 never saves; p6-objects7 reopens only image, form and text (:229, :265, :303). `save.jsonl` shows every kind survives
(objects6 7 on p0 and 2 on p1; direct3 p2 2 runs; field-defs 7 fields incl. `color copy`; links.pdf 4 links), but nothing pins it.
Fix: `tests/drive/direct-edit8-save.jsonl` on the direct-edit7-save pattern, pinning the counts and the S43/S62/S80/S100 rects after reopen.

**T-3 (should, tests) §9.22 / §10.38 — paragraph refusal and member menu outside Edit text are not pinned.**
A Shift-click on a paragraph announces "Paragraphs cannot join a selection" (`cut.jsonl` S11), untested. `direct.menu` is never read on a
field, link or mark member: S61, S79 and S99 all carry `item`, so the menu closes first. The contextmenu-selects-alone rule has no line
(probe: an unselected nom gives menu `{name:'nom', kind:'field'}`). Fix: one `menu {page,x,y}` line with no `item` per mode, exact `direct.menu` and `selection`.

**T-4 (nit, tests)** design-agent-drive.md:61 advertises `key Meta+c|Meta+x|Meta+v`; direct-edit8 uses only Meta+v (S45). Fix: one Meta+c and one Meta+x line.
**T-5 (nit, tests)** p6-drive8.test.mjs:134 lacks the `.6c51e7e2` targetId-suffix assert for the ⌘V run (S45) that S43 has (§10.36);
:168 does not recheck mark 2 after the two undos. Fix: two asserts.
**T-6 (nit, tests)** shot-direct8.mjs:274/:337 save and restore the OS clipboard as plain text only (pbpaste/pbcopy), so an image or rich
clipboard becomes "" after case D (as §9.41 wrote it). Fix: log a warning when pbpaste returns "".
**T-7 (nit, renderer)** the §10.42 announcement reads "1 text run stay (they cannot be deleted)" (`cut.jsonl` S16). Fix: "1 text run stays (it cannot be deleted)".

## Checked fine
- Header 104 steps / 23 click lines, re-counted and pinned. Every mutate, undo and copy pins an absolute revision; every copy expects
  all six `clipboard` kinds; every `menu` has `contents`; every `goto` expects its page.
- Template lines present: S11 Cut pins `content.edited`; S5/S7 disabled Paste with `canUndo:false`; S20 Fm1 paste → objects 7; S30
  `delete:true`; S37 the §10.42 error text; S23 and S84 prove D1 through `direct.clipboard` after `open`; Escape before every reselect after an undo.
- §9.46: the `active?.draft` check catches an open editor or focused field (renderer.ts:7782-7816). A literal `active:null` cannot hold,
  since S29 and S68 legitimately keep an object or link active.
- p6-drive8 pins every §4.1/§10.10 box by step index at ±0.5 and the 5 shas before and after. Re-derived by hand: "plain copy"
  [82,257,202,282] (make-field-defs.py:78 y-up [72,520,192,545] → y-down [72,247,192,272], +10); "Move me" width 57.57 (Helvetica 4112/1000 × 14).
- p3-drive, p6-drive3/6, `scripts/drive.mjs` and every existing `tests/drive/*.jsonl` are byte-identical to pre-p6u7-20260924.tgz; tests green.
- `direct-edit7-save.jsonl` = the probe + header; p6-drive7's second test pins 19/5, exit 0, 0 red, the steps 16/19 deep-equal,
  Im1 [161,53,211,113] with orient, and the sha. shot-direct7 TS2/TS3 are in; contract-p6-unit6 §3 reads (74,600).
- shot-direct8: CDP rawKeyDown with modifiers 4 and `commands`; `--inspect` + main-process `wc.copy()/paste()`; `defaultPrevented`
  true for (a), false for (d) (a missing event leaves null and fails); pbpaste restored in `finally`; 612-pt canvas scaling (all three
  fixtures are 612 wide); SIGTERM in `finally`.
- PNGs opened with Read: a = PA1 with handles at +10,+10; b = 11-item menu, only Paste enabled (last item clipped at the viewport
  bottom, cosmetic); c = one group box around "Move me" and Im1; e = "plain copy" selected and listed, status "Pasted 1 item".
- design-agent-drive.md section (6 lines) matches the shipped commands and keys (name = first 12 chars, renderer.ts:2768) and all four §4.4 traps.
