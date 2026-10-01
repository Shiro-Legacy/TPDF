# P6 U4 engine evidence (Opus 5.5 engine seat, 2026-09-24)
Files: src/renderer/engine.ts, src/renderer/engine-core.ts (edited in place); tests/p6-links.test.mjs, tests/p6-tabs.test.mjs (new); tests/p2-nav.test.mjs (+1 test appended). worker.ts, redact.ts untouched (worker is a pass-through; the union lives in engine.ts).

## engine-core.ts
- `failLink(detail: 'rect'|'no-such-annot'|'not-link'|'target'|'scheme', message): never` next to failRedact (reason `invalid`, token in `detail`).
- `OP_PERMISSION['link-set'] = 'annotate'`.

## engine.ts
- Types: `LinkSetRequest` (in `SessionRequest`), `LinkEntry.annotId: number`, `FieldItem.structIndex?: number`, `TabsName`, `FieldsReply.tabs: TabsName | null`, `MupdfLink` alias.
- `linkPairs(page)` (§9.1): /Annots walk keeping /Link + /Rect + (/Dest|/A|/AA) − JavaScript /A; key rect = Rect.transform(normalised raw /Rect, page.getTransform()); cursor pairing ±0.01; unpaired → annotId 0; any throw → all 0. Used by `listLinks` and `link-set`.
- `listLinks`: every item gains `annotId`; `id`/rect/kind/page/uri unchanged.
- `link-set` (§1.2/§9.2/§9.3): requireMutable → checkPage (plain Error) → `target` (neither rect nor target) → `rect` (checkLinkRect re-tagged) → `no-such-annot` (non-positive/non-integer id, or no indirect /Annots entry with that number) → `not-link` → `no-such-annot` (unpaired /Link, e.g. L5 or a JS link) → `target` (shape / page range, checkPage text) → `scheme` (allowlist, then 2048). Exact §9.3 messages.
- No-op: rect ±0.01 of getBounds and same target (same resolved page, or same URI on a non-page link) → `{ok, noop:true, ...journalState}`, nothing journaled.
- Write: ONE `mutate(…'edit link'…)`: `link.setBounds(Rect.transform(rect, Matrix.invert(page.getTransform())))` and/or `entry.delete('Dest')` + `link.setURI(uri)` (page target = formatLinkURI(fitDest(p))). Page handle destroyed in `finally` before the reply. Reply `{ok, ...editState}`.
- `fields` reply: `tabs` from the page dict /Tabs (R/C/S/A/W else null); on `S`, `structObjectOrder` (/K-only DFS, OBJR /Obj numbers, first wins, visited set, 50 000 nodes / depth 64 → null) sets `structIndex` by widget id.

## Proof (under the lock)
`npm run build && node --test tests/p6-links.test.mjs tests/p6-tabs.test.mjs tests/p2-nav.test.mjs tests/p2-shell.test.mjs tests/p2-forms.test.mjs tests/p3-forms.test.mjs tests/fixtures.test.mjs` → 77 tests, 77 pass, 0 fail.
- p6-links (10 tests): shas pinned; OP_PERMISSION pin; annotIds p0 [10,11,13] (L5=12 skipped), p1 [14] rect [700,72,720,200]; rect move → save → reopen ±0.01; uri→uri→page→uri and /Dest L2→uri each save → reopen; 18 refusals (token, exact message, dirty false, revision 0); no-op (rect + uri, and same page) noop:true revision 0; blocked link rect-only move; undo restores rect+target, redo re-applies (step 'edit link'); rotated page 1 rect round-trip; link-delete middle keeps [10,13]; §9.1 scratch (inline, reversed rect, /AA-only, JS, last) → annotIds [0,4,5,7].
- p6-tabs (3 tests): C/A/S/S tabs; page 2 structIndex s2 0, s3 1, s1 2; page 3 none; form.pdf tabs null; a1 item deep-equals the existing key shape.
- p2-nav +1: nav.pdf annotIds positive/unique; link-delete keeps the others after save → reopen via the engine.
- Fixtures: `shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256` 6/6 OK.

## Not done / notes
- One interim build failed on renderer.ts (renderer seat mid-edit); after the tree typechecked clean, a final locked `npm run build` (exit 0) + the same 7 files → 77/77 pass; fixtures 6/6 OK.
- Nothing from §1.1-§1.3/§2.1 left undone. tests/p2-shell.test.mjs not edited (§9.4/§9.22).

## Fixes after review

- E-1 (reproduced with .quad/scratch/review-u4-engine/pad.mjs: padded `'  https://example.org/pad '` gave `{ok:false, error:'unsupported link URI type'}`, no reason/detail, on both link-set and link-add). Fix: `schemeOf` (engine.ts ≈2502) no longer trims, so leading whitespace means "no scheme" and the allowlist refuses before any write: link-set now replies `{ok:false, reason:'invalid', detail:'scheme', error:'link-set: a URI without a scheme is not a link this app opens'}`, revision 0, dirty false; link-add replies `link-add: a URI without a scheme is not a link this app opens`. `listLinks` reports a stored URI with leading whitespace as `blocked` (consistent with what the allowlist would accept). The renderer panel path (renderer.ts:1192, untrimmed) now receives the `scheme` token, so no renderer edit is needed.
- Tests: p6-links refusal table gains the padded URI (`scheme`, exact message); p2-nav link-add refusal table gains it (`/without a scheme/`).
- Ran under the lock: tsc clean; build; the 6 suites 71/71 pass. Fixture hashes unchanged (only fixtures/p6/links.pdf, tabs.pdf new vs u4-hash-before).
