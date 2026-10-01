# P6 U6 review — TESTS / DRIVE / EVIDENCE lens (Opus 5.5, 2026-09-24)
Verdict: **ACCEPT** (no blocking; 1 should, 3 nits). Every run held .quad/run.lock; scratch .quad/scratch/p6u6/review-tests/.
Probes written: checks/review-u6-tests-save.jsonl (crop + ne-to-full + flip + group rotate of a cropped member → save-to → reopen),
checks/review-u6-tests-keys.mjs (real CDP Escape / Enter in crop mode, real ⌥⌘→ on a group).

## Ran
- `npm run build` 0; `node --test tests/p6-drive7.test.mjs tests/p3-drive.test.mjs tests/p6-drive3.test.mjs tests/p6-drive6.test.mjs` → 24/24 pass (p6-drive7 = ok 24).
- `node scripts/drive.mjs tests/drive/direct-edit7.jsonl` ×3 → exit 0, 60 steps, 0 red each. Deep diff of the three reports: the ONLY differing key is
  `state.status` render time ("rendered in 19 ms" vs 20 ms) → deterministic.
- review-u6-tests-save.jsonl → exit 0; content.objects before save and after reopen are identical (cropped+group-rotated Im1 view [161,53,211,113]
  full [161,53,211,153] {true,1,-1}; Im3 [161,181,211,281] {true,1,1}; Im2 flip-h {true,-1,-1}; Im4 clip removed, full absent).
- review-u6-tests-keys.mjs → real Escape in crop: crop null, selection Im1, rev unchanged; real Enter in crop: crop null, no Replace dialog,
  rev unchanged; real ⌥⌘→ (modifiers 5) on Im1+Im3 → rev+1, edited {objects,resize,Im1,Im3,2}.
- shot-direct7.mjs PASS all; shot-direct6.mjs PASS all; review-u5-renderer-probe.mjs (default ONLY) ALL CHECKS OK.
- `shasum -c checks/fixtures.sha256` 9 OK after drives and after the GUI scripts; fixtures/**/*.pdf|png vs u6-hash-before.txt: exactly one added
  line `ca0a5988…2ab8 fixtures/p6/objects6.pdf`, nothing changed. `pgrep -fl -- --drive=` empty after every run.
- Backup diff (pre-p6u6-20260924.tgz): scripts/drive.mjs differs only at :25 (`'meta'` appended); p3-drive, p6-drive3, p6-drive6 tests and every
  existing tests/drive/*.jsonl byte-identical; design-agent-drive.md only gains the appended section.
- PNGs viewed: u6-a (one frame, 8 handles, members none), u6-b (vertical guide at Im2's left edge + horizontal at y 142, outline snapped),
  u6-c-cropped (Im1 60×50 visible, pale dashed full), u5-b (Fm1 menu, 8 items, Replace dimmed). Claims hold.

## Findings
| id | sev | owner | file:line | clause | claim / reproduction | fix |
|---|---|---|---|---|---|---|
| TS1 | should | tests | tests/drive/direct-edit7.jsonl:47 | §2.1 enable rules, §9.22, §0 parity | No drive step reads `direct.menu.items` on a group or on a non-fitting rotate: the only items expect (:15) is the single-Im1 all-true case. A build that leaves Crop enabled for a 2-member group, or rotate enabled when the rotated box would leave the page, passes all 60 steps (grep `"crop":false` / `"rotateCw":false` over tests/drive → none). | direct-edit7 is new in this unit, so add after :46 one `menu {page:0,x:122,y:167}` (no item) → `direct.menu {count:2, items {…, crop:false}}` and bump the header/test to 61 steps; cover the rotate-fit rule in p6-drive7 or shot-direct7 with a union wider than the page height. |
| TS2 | nit | tests | .quad/shared/p6/checks/shot-direct7.mjs:277 | §2.6 `direct.gesture.kind` gains 'crop' | The crop drag passes no `during`, so `gesture.kind === 'crop'` is never observed anywhere (drive STATE follows `end`). | `drag(…, 0, async () => (await state()).direct.gesture?.kind)` and deq 'crop'. |
| TS3 | nit | tests | .quad/shared/p6/checks/shot-direct7.mjs:124,181 | §9 numbering | Comment cites nonexistent "§9.34"; 'fixtures before' hashes logged twice (:181 and :197). | "§9.19"; drop :181. |
| TS4 | nit | tests | tests/drive/direct-edit7.jsonl (tail) | §0 "write survives save → reopen" | No shipped script saves a crop / swapped cm / group transform and reopens; only my review probe proves it (green). | Keep checks/review-u6-tests-save.jsonl as the acceptance probe or add a save-to + open tail. |

## checkedFine
- §4.1/§9.31: 60 steps / 17 click lines pinned literally and from the header (p6-drive7:95-102); ledger 0..28 then 0..2 absolute on every mutate
  and every undo; step 20 no-op with rev 8 and error "" (STATE's house empty value; `null` would never match, documented deviation); step 21 `ne dx 40 dy -10`;
  meta drag [398,…] vs snapped [400,…]; pass-through expect `{selection:[],active:{kind:'add',page:0}}`; band (74,600) at 131 % = 2.6 px inside the ring;
  "Over" (115,537); second crop exit `key Enter`; direct3 tail rev 0→1→2 with `mode edit`. Every menu line carries contents, every handle page/x/y.
- Non-vacuity: removing crop mode turns :16/:17 into move/resize (action ≠ 'crop'); removing the group box makes :28 resize Im3 alone (kind 'image');
  removing the driveDirectKey crop branch clears the selection at :17; removing pass-through selects Fm1 at (200,600); `crop:null` expects fail on a missing key (main.ts:1659 strict ===).
- p6-drive7: picks by view, all() ±0.5, every orient/box of the "Recomputed geometry" section deep-equalled incl. Im3 {true,1,1}, ⌥⌘→ 172.4386/200.5614,
  group flip-h swap; pristine after every undo; both fixture shas before/after.
- shot-direct7: CDP Input.dispatchMouseEvent with modifiers on press/move/release, guides read during the drag, member handles asserted hidden,
  scale from real canvas width, §9.30 scroll rule, SIGTERM + shasum in finally. shot-direct6 asserts one shared scrollTop. Probe split per §9.32 (8 checks, 5 sessions).
- design-agent-drive.md section matches shipped item names, STATE keys and the §4.4/§9.13 traps (grip on a group is a no-op: renderer.ts driveDirectGesture resolves the owner, no grip handle on .group-box).
- Not reproduced here: full `npm test` 492/491/0/1 (integration evidence) — not in this lens's run list.
