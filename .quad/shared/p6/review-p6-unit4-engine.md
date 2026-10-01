# P6 U4 adversarial review — ENGINE + FIXTURES lens (Opus 5.5, 2026-09-24)
Verdict: ACCEPT with one `should` and one `nit` (neither blocks: both refusals are safe, nothing dirties).
Probe: `.quad/shared/p6/checks/review-u4-engine-probe.mjs` (run under the lock after `npm run build`), plus
`.quad/scratch/review-u4-engine/pad.mjs`; generators re-run from a scratch copy in `.quad/scratch/review-u4-engine/fx/`.

## Ran
- Locked: `npm run build` exit 0; `node --test tests/p6-links tests/p6-tabs tests/p2-nav tests/p2-shell tests/p2-forms tests/p3-forms tests/fixtures tests/p1-worker` → 102 tests, 102 pass, 0 fail.
- Probe: 37 PASS lines, 0 FAIL (details below). Fixture hashes afterwards: fixtures.sha256 6/6 OK, u4-hash-before.txt 50/50 OK, only links.pdf and tabs.pdf new.

## Findings
**E-1 (should, engine) — a whitespace-padded URI passes the allowlist, then mupdf refuses it with no token.**
src/renderer/engine.ts:2502 (`schemeOf` trims) + 4187-4196 (link-set passes the untrimmed `target.uri` to `setURI`).
Clause: §0 "refused with a fixed token and detail"; §1.2/§9.3 `scheme` refusal set.
Repro: links.pdf, `{op:'link-set', page:0, annotId:10, target:{uri:'  https://example.org/pad '}}` →
`{ok:false, error:'unsupported link URI type'}` with no `reason`, no `detail` (rev 0, dirty false — safe, abandoned).
Reachable from the drive: `link {action:'set', index:0, uri:' https://…'}` (renderer.ts:1192 does not trim) →
renderer.ts:5327 records `nav.last.detail 'invalid'` (not a contract token) and shows mupdf's raw text in `#error`.
link-add (engine.ts:4103) has the same hole (pre-existing). Fix: test the scheme on the untrimmed string
(`/^([A-Za-z][A-Za-z0-9+.-]*):/.exec(uri)` without `.trim()`), so padding reads as
`link-set: a URI without a scheme is not a link this app opens` (`scheme`); or trim once before both check and write.

**E-2 (nit, engine) — the depth-64 bound counts each `/K` array as a level.** engine.ts:2594-2606.
Probe: StructElems nested through `/K << … >>` keep `structIndex` to 63 levels; nested through `/K [ … ]`
(the usual tagged-PDF form) lose it at 32 levels (probe "nested dict-in-array depth 32" → null for all).
§9.11 does call arrays nodes, so this is within the letter; a real tagged form 32+ elements deep falls back to reading order.
Fix (optional): increment depth only when entering a dict, or state "64 /K steps (arrays count)" in the Help fallback ceiling.

## Checked fine (claims that held)
- §1.1/§9.1 annotId: links.pdf p0 [10,11,13] (L5=12 skipped), p1 [14] rect [700,72,720,200]; interleaved Widget/Link/Widget/Link page → [4,6,8]; the §9.1 scratch (inline, reversed /Rect, /AA-only, JS) → [0,4,5,7] (test).
- §1.2 order/tokens: rect (off-page, zero area, 3 numbers), no-such-annot (0, 999, 'x', L5=12, L4 on the wrong page, obj 9 not in /Annots), not-link (tabs.pdf a1=13; scratch widget 5), target ({}, null, page 9), scheme (javascript:, file:, ftp:, '', 2049 chars) — each dirty false, revision 0, `undo` refused.
- No partial write: valid rect + refused target (5 variants) on L2 → raw `/Annots` dump identical, journal empty.
- No-op ±0.01 (rect+uri, same page) → `noop:true`, journalState, revision 0; two consecutive identical link-sets on p0 and on /Rotate 90 p1 → the second is noop (the first write is seen; page handle destroyed in `finally`, engine.ts:4221).
- P1 space: page-space rect in == rect out after save(garbage=yes) → reopen on p0, /Rotate 90 p1, a /Rotate 270 page and an offset MediaBox [100 100 712 892].
- P2: /Dest L2 → uri survives reopen; a /Dest + /A URI link → uri survives; /A /GoTo link → uri survives; uri → page 2 and page → uri survive; annotIds stable across all writes.
- Undo restores rect AND target (p0 test; p1 probe with rect+page target) read from a fresh `links`; redo re-applies; step 'edit link'.
- link-add → link-set by the new link's annotId works; link-delete of the middle keeps [10,13] after reopen (test); p2-nav additive test goes through the engine.
- §2.1: tabs C/A/S/S on tabs.pdf, structIndex s2 0 < s3 1 < s1 2, none on page 3, form.pdf null; /Tabs /R → 'R', /W → 'W', /X and a string (S) → null; 50 000-node bound exact (50 000 nodes keep indices, 50 001 drop all); a cyclic /K (with /P back-pointer) terminates with the right order.
- §5 fixtures: both generators run twice from a scratch copy → identical to fixtures/p6 (pdf and provenance); shas equal PINNED_SHA256, checks/fixtures.sha256 and the evidence; provenance rects (y-up, y-down, L4 page space) equal the engine's; tabs.pdf structure matches §9.17 (no /StructParents, /MarkInfo, /K [E2 E3 E1], /ParentTree /Nums, /Annots orders).
- §9.4/§0: OP_PERMISSION['link-set'] = 'annotate' (engine-core.ts:250); LinkSetRequest in SessionRequest (engine.ts:544); p2-shell op scan and p2-nav refusal regexes green.
