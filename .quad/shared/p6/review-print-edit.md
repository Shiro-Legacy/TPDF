# print-edit review (refute-first, 2026-09-30)

**Verdict: accept with fixes**. There is one blocker, a quadratic regex. It is a one-token fix plus one test. There is also one major finding, the double-space lines.

How this was checked: the diff against `.quad/backup/pre-print-edit-20260930.tgz`, extracted to `.quad/scratch/print-edit-review/`. Outside the listed files, only `.DS_Store` files differ under `src/ tests/ scripts/ fixtures/ docs/`. Probes are in `.quad/scratch/print-edit-review/probes/r1..r7`. All ran headless under the lock against the current `build/`. No GUI was used and no product file was touched. On the human's file I read counts only.

## Findings

### 1. BLOCKER: the glyph-block regex takes quadratic time, so a small hostile page hangs the worker
- Where: `src/renderer/content-edit.ts:1985`, `GLYPH_NUM = [+-]?(?:\d+\.?\d*|\.\d+)`. `editGlyphBlocks` uses it for the six `cm` numbers, the six `Tm` numbers, the clip and the colour.
- Input: a depth-0 stream holding any single-byte candidate `Tj`, plus a comment (or a string, or any bytes) of the form `% q q 1111…1` with N digits. The regex runs over the raw stream bytes. It tries a match at every `q` preceded by a delimiter, and inside a digit run `\d+\.?\d*` can split the run N ways. Each failed attempt therefore backtracks O(N²).
- Observed (probe `r1-redos.mjs`):
  - N = 5k: 123 ms
  - N = 10k: 443 ms
  - N = 20k: 1717 ms
  - N = 80k: **27.3 s**
  - The same page with a 2-byte `Tj`, which never reaches `editGlyphBlocks`, scans in 2–6 ms.
  - At the 2 MB scan-byte cap the extrapolation is hours. `edit-targets` runs synchronously in the engine, so the app's edit mode freezes.
- Smallest fix: make the number unambiguous, `GLYPH_NUM = [+-]?(?:\d+(?:\.\d*)?|\.\d+)`. Add a test: a 100 k-digit comment beside a single-glyph `Tj` scans in well under 1 s.

### 2. MAJOR: a line with two consecutive 0x20 space blocks has non-normalised text, so a move re-lays out the line and an identical edit dirties the document
- Where: `content-edit.ts:2128`. Each space member adds `" "`, so two adjacent 0x20 blocks give `"ab  cd"`. `editPlanGlyphLine` then compares at `:1065` (`same = next === record.text`) using the normalised `next`.
- Input: a D11 line `a b <sp> <sp> c d` (probe `r5-attacks.mjs` case 9, Helvetica WinAnsi with `/Widths`).
- Observed:
  - The item text is `"ab  cd"`.
  - `move {dx: 10}` gives x `[82, 87.56, 95.9, 100.9]`, but the exact shift would be `[82, 87.56, 98.68, 103.68]`. The second half of the line slides left by one space width, and the rescan text becomes `"ab cd"`. So a pure move changes the text, which violates D4.3 ("original spacing kept exactly").
  - `edit-batch` moves take the same path (`engine.ts:4079` passes `record.text`).
  - An edit with the identical text returns `dirty=true, revision=1`, which violates D6/§9.15.
  - This also breaks D4.1's premise that `record.text` is already normalised.
- Not seen on the human's file: 0 double-space lines on both pages, and 0 of 56 / 68 identity edits dirtied (`r6-human.mjs`).
- Smallest fix: in the collapse, a space member adds `" "` only when the previous member is not a space member, i.e. a run of 0x20 blocks is one word gap. Mirror that in the targetId key. Add a fixture-free test: two 0x20 blocks, then move +10 keeps every x + 10 and the text is unchanged.

### 3. minor: raising the raw ceiling makes ordinary over-cap pages about 60× slower to refuse
- Where: `content-edit.ts:2333` (raw cap 4096) together with the existing O(records × stext runs) `editMatchRuns` (`:1524`). A profile shows `normalise` + `editMatchRuns` take about 80 % of the time.
- Input: 4096 plain `(aN) Tj` runs. Before the unit this was refused at run 513.
- Observed:
  - 513 plain runs: 67 ms. 4096 plain runs: 4.1–4.4 s, then the same `too-large` refusal.
  - 4096 real glyph blocks in 64 lines: accepted, but in 4.0 s.
  - Human file: 499 / 470 ms per page, within the ≤ 1000 ms acceptance limit.
- This is not a hang, and the contract puts scan speed out of scope. It is still an ordinary-page behaviour cost: any dense page with more than 512 runs now pays seconds on every scan.
- Smallest fix, if wanted: count records that cannot join a glyph line (not `kind text` with a 1-byte payload) inside `emit`, and refuse at 513 of those.

### 4. minor: Find/Replace apply with a no-op replacement rewrites the line unclipped
- `buildReplacePreview` passes `next` even when it equals `text` (for example find = replace, or a whitespace-only difference). Apply writes `plan.operand` unconditionally (`editWritePlans`). The glyph line's plan re-emits it without clips even though `rewrite` is false.
- An ordinary run in the same case also takes a journal op but gets identical bytes. Here the clips are dropped (D5: only rewritten lines should lose them).
- Found by reading; not run.
- Fix: skip plans with `record.glyphLine && !plan.rewrite` in apply, or drop such matches in the preview.

### 5. minor: the chain tolerances compare each block only with the previous one, so they can drift along a line
- `content-edit.ts:2172`: |ΔY| < 0.005 and |Δscale| ≤ 1e-3·scale are checked pairwise. A long chain can therefore drift, for example 0.5 pt of baseline over 100 glyphs.
- The rewrite writes `first.block.y` and `first.block.scale` for every glyph (`:2144`), so an edit would flatten the drift.
- Theoretical: Quartz writes one exact Y per line, and nothing like this was seen on the human's file.
- Fix: compare against `chain[0]`.

### 6. note: test gaps (`tests/print-edit.test.mjs`)
- (h) at `:392` says "4097 glyph blocks", but its blocks are not glyph blocks. The outer `q 1 0 0 1 0 0 cm q …` fails the D1 head, and `addSimpleFont` Helvetica has no `/Widths`. The test only proves the walker's raw cap. That is acceptable, but the title is wrong.
- No test covers any of these:
  - D1 negatives: outer cm, Form XObject, non-uniform scale, `Tf` ≠ 1
  - a chain split by a stream boundary, a comment or a colour change
  - the D12 scale-tolerance boundary
  - a Coded-font 0x20 block
  - two 0x20 blocks (finding 2)
  - regex performance (finding 1)
  - an `edit-batch` mixing a glyph line and an ordinary run in one stream
- Probe r5 exercised these and they behave correctly (see "held" below), so the missing tests are a coverage gap only.
- Every other test has assertions that can fail. The red-before log shows 10 of 11 `not ok` plus the D11 `not ok`; only the fixture-shape test passed before.

### 7. note: fixture content versus the human's file
- Checked by counts only (`r7-copied.mjs`). Of the 25 fixture strings, the only ones longer than 12 characters found verbatim in the human's file are #0 and #7: the title "Business Licence Application Form" (also the file's name) and "Office use only". The contract (D8/D11) prescribes both literally.
- None of the 14 body sentences occur in the human's file.
- The generator is deterministic: two runs in the scratch copy produced sha256 937d55fe…, which equals the pin.

## Implementer deviations (judged)
1. `editCollapseGlyphLines(state, resources)` takes `resources`: fine, needed to re-resolve the font.
2. `cow` comes from the lender, not the first member: fine. All members share one depth-0 stream and so one element cow, and an ambiguous record carries none.
3. `rewrite` means "text changed, or geometry changed after rounding": correct. The literal "bytes differ" rule would make every zero move write. But see finding 4.
4. A chain with no bounded member stays uncollapsed: fine, and it is the conservative choice. The same happens to a chain with no candidate: a lone "zz" stays two ambiguous items (probe case 5), as D2 requires.
5. `textLinear[3]` is checked against −d: correct under D12.
6. Two 0x20 blocks give two spaces: rejected. This is finding 2; it conflicts with D4.1, D4.3 and D6.
7. Leading and trailing space blocks stay ordinary `" "` items: as D12 says (probe case 14). The human's file has 0 such items.
8. Replace "Suite" → "Unit" on a fresh page and "Post code" in the drive: consistent with the page-scoped repertoire and growth rules.

## Held (attack, then result)
- **Diff scope:** nothing outside the listed files differs (only `.DS_Store`).
- **`editPlanWidth` extraction:**
  - Checked line by line against the backup: `placed.prefix !== ""` equals the old truthiness, the offset, ratio and clip are the same values, and the order of refusals is unchanged.
  - The only path change for ordinary runs is the operand being built after `elementIndex`. That code cannot throw, so behaviour is identical.
- **D1 grammar:** each of these stays per-glyph:
  - blocks under an outer non-identity `cm`
  - blocks in a Form XObject
  - `d` off by more than 1e-3·a (10 vs 10.02)
  - a comment between blocks (splits the line)
- **D1 scale tolerance:** 10 vs 10.009 joins (D12).
- **Chaining (D2):**
  - blocks in two content streams give two lines
  - a colour change splits the line
  - a gap of exactly 1.0 em joins, 1.001 em splits
- **Outer clip:** a line inside an outer `q … re W n … Q` collapses and edits. After the rewrite q/Q stay balanced (depth 0, never negative), the neighbour line's bytes are untouched, and undo restores exact bytes.
- **Hex and escapes:** a hex operand `<61>` and escaped `(\()` `(\))` operands are recognised. An edit writing `)` and `(` is emitted as hex and rescans to the same text.
- **Repertoire, same font object:** two resource names pointing at one font object share the drawn set, which is correct because the glyphs are the same.
- **Repertoire across pages:** where `/F1` is a different object on each page, page 1 cannot write page 0's characters (`unsupported-encoding` with `missing ["a","b","c"]`).
- **D11 text check:** a record whose decoded text ≠ its code (e.g. a `/ToUnicode` override) is rejected by `record.text !== String.fromCharCode(code)`. `/Differences` encodings and direct font dictionaries are excluded.
- **`edit-batch`:** a glyph-line move together with an ordinary-run move in the same stream is one op, rev 1, balanced.
- **Size and save:** `size: 20`, then save, then reopen, then rescan gives the same line as one glyph line.
- **Rotation and UserUnit:** rotated or UserUnit pages are `geometryBlocked`, so the collapse is skipped. The page-view to user-space move mapping is only used on identity-scale pages.
- **D6:**
  - the no-op test uses the normalised `plan.next`
  - `edit-batch` passes `|| plan.rewrite`
  - `clip-copy` refuses with `not-candidate`
  - `editParagraphLine` excludes the line through `cidWrite`
- **D3:**
  - the raw cap message is "more than 4096 targets"
  - the 512 check runs after the collapse with today's exact message
  - tokens are raised to 200,000, and the `p3-targets` pin was flipped accordingly
- **Collapse cost:** the collapse itself is linear (block maps cached per stream, fonts cached), so there is no quadratic step in the new pass apart from finding 1's regex.
- **Human file (counts only):**
  - page 0: 58 items, 56 glyph lines, 499–503 ms
  - page 1: 70 items, 68 glyph lines, 470 ms
  - on both pages: 0 whitespace-only items, 0 double-space lines, and 0 identity edits that dirtied the document
