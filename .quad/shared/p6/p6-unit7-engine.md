# P6 U7 engine evidence (2026-09-24)

Files: src/renderer/engine.ts, src/renderer/engine-core.ts, tests/p6-objects7.test.mjs (new, 664 lines),
tests/p6-objects6.test.mjs (§1.5 only). Not touched: content-edit.ts, worker.ts, limits.ts.

- engine-core.ts OP_PERMISSION: clip-copy copy, clip-paste any, edit-batch any (per-member checks inside).
- engine.ts: clipCopy / clipPaste / editBatch / addField (field-add body factored; field-add passes mutate).
  Session gains clipboard {doc, items} (survives open/close) and batch. Dispatch cases before "stamp".
- mutate: the ONLY change is the first line `if (session.batch) { work(); return journalState(session); }`
  (diffed against .quad/backup/pre-p6u7-20260924.tgz: one added line). Smaller-diff rule chosen: branch in mutate;
  batch annotation items dispatch recursively to their own case bodies; content items plan against one scan and
  write through editWriteElement per /Contents element.
- clip-copy reply {count, kinds:{image,form,text,field,link,mark}, revision, dirty}; clip-paste reply
  {page, count, names, targetIds, fields, fieldIds, linkIds, markIds, skipped, ...editState}.
- Spaces per §10.13 (view space union/occupancy/delta, clamp inside editPageClipBox; out-of-bounds only when
  the union is larger than the page or `at` is malformed).

Re-pins (tests/p6-objects6.test.mjs, §1.5 N1/N2, in place):
- :350 `const im3 = id(scan(s, 0), 'Im3');` hoisted before both refusals (N1).
- :234 the invalid-transform case now sends box `'junk'` (N2).

Tests (one lock run, after npm run build, build=0):
node --test tests/p6-objects7 p6-objects6 p6-objects5 p6-direct3 p3-objects p3-targets p3-edit p3-forms
p2-encrypt p2-shell fixtures p6-links p2-nav p4-redact → 216 tests, 216 pass, 0 fail.
p6-objects7: 26 tests, 15 named `D4 ` (gate needs >= 12).
Fixtures: shasum -c .quad/shared/p6/checks/fixtures.sha256 → 9 OK; fixtures/ pdf+png count 55 (unchanged).

Pinned truths: same-page paste of "Move me" on direct3 p0 is ok, both runs then scan ambiguous-text-association
with null bounds (bounds + d read from stext, ±1 because asJSON rounds), targetIds []; undo restores the source.

Not done / ceilings:
- a batch edit-text inside a Form refuses not-candidate (privatising would split the element's one splice).
- the "page points" too-large branch of clip-paste is not tested; the 512-target cap is.
- batch annotation-item checks run inside the op (refusal abandons; journal and bytes proven unchanged).
- no Electron drive run by this seat.

## Fixes after review
- E1 (reproduced: batch2.out, batch.out B3/B4, canRedo true→false). `edit-batch` now runs a "plan" pass before the op: each field-props/field-delete/link-set/redact-set/annot-delete item goes through its own single-op case with `session.batch = "plan"`, and `mutate`'s one batch line skips `work()` in plan mode (`if (session.batch) { if (session.batch === true) work(); return journalState(session); }`). Every single-op check (findWidget/findAnnotation, checkFieldRect, checkLinkRect, redactRect, not-redact) refuses through `member()` with its index before `beginOperation`, so the redo branch survives. `Session.batch` is now `boolean | "plan"`. The in-op run is unchanged. This supersedes the ceiling above ("batch annotation-item checks run inside the op").
- E2 (reproduced: shapes.out has no detail). clip-paste `unbalanced-content` and the mark cap `too-many` now go through `failDetail`, so the reply is `{reason:'invalid', detail, index, kind:'mark'}`.
- E3: new tests in tests/p6-objects7.test.mjs. (1) Big-image test: a 36,000,000-byte raw image built in the test, copy refuses `too-large` and writes nothing, then paste still lands `PA1` from the old clipboard. (2) Mixed batch of five kinds (run, Im1, field, link, mark): revision +1, one undo restores contents, the page dict, fields, links and marks, and redo and reopen hold. Also new: an E1 redo-survives test (field-defs, plus the mixed direct3 B3/B4 cases) and an unbalanced-content detail test. The too-many assert changed from `reason ?? detail` to `refused(capped,'too-many')`.
- §6.3: with the fixes reverted in build/renderer/engine.js only, three tests fail (the too-many, unbalanced and redo-survives tests). Restored afterwards, 30/30 pass. The final suite run is 130/130. Fixture hashes: 9 OK, and the 55-line list is identical.
- Side note: a test that leaves a `loadPage` handle undestroyed during a batch makes a link rewrite come back with annotId 0 and a y-flipped rect. The worker never leaks page handles; the new test destroys its view.
