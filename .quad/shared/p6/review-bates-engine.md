# Bates review — ENGINE lens (Opus 5.5, 2026-09-24)
Verdict: REVISE (engine behaviour correct on every probe; two §9.5 behaviours have no test that bites). Probes: .quad/scratch/bates/review-engine/{probe,refuse,edge}.mjs (+ .out), mutants mut/M0-M5 (mutants.out, one lock hold).

## Findings
- **E-R1 (should) tests/p2-decorate.test.mjs:428-560, §9.5 "{bates} first"** — mutant M1 (build/engine.js stampTextFor moves `.replaceAll("{bates}")` after `{label}`) keeps bates (a)-(f) 6/6 green. The probe shows the break: labels-set prefix `L{bates}-` then header `[{label}] {bates}` start 9 pad 2 → M1 gives `[L09-2] 09` (label expanded), the real build gives `[L{bates}-2] 09`.
  Fix: in (b) or (e) add `op(labels-set {from:0, style:'D', prefix:'L{bates}-'})`, stamp `'[{label}] {bates}'`, then `assert.match(pageText, /\[L\{bates\}-2\] 09/)`.
- **E-R2 (should) tests/p2-decorate.test.mjs:519-529, §9.5 watermark width from `expanded`** — mutant M4 (`template.length` back in the watermark width) stays 6/6 green. On nav p0 `{bates}` pad 15 start 1: real bbox centre (323.5, 384.5); M4 gives (391.5, 316.5), about 96 pt off the page centre (306, 396). Triple `{bates}` loses 8 digits off the page under M4.
  Fix: in (e) stamp `'{bates}{bates}{bates}'` pad 15 on p0 and assert the stext line centre is within 30 pt of (306, 396). Real build: (306, 404).
- **E-R3 (nit) tests/p2-decorate.test.mjs:503,513, §9.6** — the real right-edge error is 0.00 pt. The last glyph quad x1 is exactly 576.00 on nav (Tm 510.42 + 65.58), and it is 0 on R=0/90/180/270. The test's stext `bbox.x + bbox.w` reads −1 only because asJSON() floors to integers (asJSON(100) gives 51042 + 6557). Mutant M3 (the 0.556 estimate, +4.42 pt) stays green under the 10 pt tolerance, which §9.1/§9.6 keep on purpose.
  Fix, if wanted: use `asJSON(100)`/100 and a 1 pt tolerance; otherwise leave as the contract says.
- **E-R4 (nit) tests/p2-decorate.test.mjs:466-475, §9.2-9.4** — (c) omits pad 1.5, pad '6', start NaN/null and align 'RIGHT' and null. All of them refuse correctly in probes (refuse.mjs, 34/34 OK). Add them to the table for coverage.

## checkedFine
- Diff vs pre-bates-20260924.tgz: engine.ts changes only StampRequest, the MAX_BATES/MAX_BATES_PAD literals (:144-146), stampTextFor, stampPage (expanded/width/align) and stampPages (checks + loop + reply). mutate, checkStampText and winAnsiBytes are untouched, engine-core.ts is identical, and p2-decorate is additive only (408a409,550). tsc clean.
- Read-only signed doc with `{bates}` without start / start −1 / align 'centre' / pad 99 → reason `read-only` (requireMutable runs first at :5825).
- refuse.mjs, on a fresh doc and after one edit (rev 1, dirty, canUndo): 17 cases each, including −1, 1.5, '5', NaN, null, 1e15, the end cap 1e15−2 × 3 pages, pad 0/16/1.5/'6'/null, align centre/RIGHT, a watermark with align centre, and plain text with start −1 or the end cap. All give ok false, reason invalid, `^stamp: <token>:`, and an unchanged {revision, dirty, canUndo, canRedo}; the first undo then returns false/true as expected. The checks sit after checkPages and before mutate. The order is align → pad → start → start-required.
- End cap: start 1e15−3 on 3 pages is ok, to = 999999999999999 (exact).
- Expansion: `{bates}{page}|{pages}` start 1234567 pad 3 pages [2,0] → p0 `12345671|3`, p2 `12345683|3` (no truncation, stamped order). `X{bates}Y{bates}` start 0 pad 1 → `X0Y0`. `{BATES}` is literal and has no bates key. `(a\{bates})` is escaped correctly. Plain text with batesStart has no `bates` key (test (b)).
- Mutants M2 (no end cap) → (f) red; M5 (page number instead of index) → (b) and (e) red.
- Save → validate → reopen: right-aligned footer on nav survives on all 3 pages (ACME-000100..102, p2 edge on the margin); one undo afterwards removes all three (`add footer`), canUndo false.
- Long right-aligned text clamps vx at 36 and runs off the page like left-aligned text does (pre-existing behaviour, not a Bates regression).
- fixtures.sha256 9 OK; pdf/png hash list identical to bates-hash-before.txt (55).
