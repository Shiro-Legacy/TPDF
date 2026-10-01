# Acceptance — debt sweep 1 (2026-09-24, fable51, lean mode)

Scope: `.quad/shared/debt/scope-debt-1.md` (25 rows). Build: one Opus 5.5 workflow `wf_ddee6057-301` (engine / renderer / tests parallel → integrator), 4 agents, 0 errors, no requests between roles, no integrator fixes needed. No review workflow (human's instruction: build + gate).

## Gate (`accept-u3-debt1.sh`, one lock hold; log `gate.log`)
- tsc clean; build ok; `npm test` 466 / 465 / 0 / 1 (baseline 453 / 452 / 0 / 1 → +13 tests).
- sign-field.jsonl ×3: exit 0, 45 steps, 0 red each. sign 6/0, verify 17/0, field-design 23/0, direct-edit 75/0, direct-edit5 35/0, help-fields 21/0.
- No secrets in reports. pyHanko on run 3: field-signed 1, field-signed2 2, field-signed3 3, hello-field 1 signatures, all INTACT trusted.
- fixtures.sha256 all OK (incl. new `fixtures/p3/field-defs.pdf`); vs `debt1-hash-before.txt` (53 files): changed none, added `p3/field-defs.pdf` only (the script's second diff line pointed at a stale path; run by hand, then the script was fixed with `sed g`).
- Trust store absent; no `--drive=` Electron left. Gate exit 0.
- `npm run package` exit 0; asar 2026-09-24 13:57 (12 279 731 B).

## Integrator (debt1-integration.md)
- debt1.jsonl (22 steps), refusals.jsonl (29), nonascii-label.jsonl (4): ×3 each, all exit 0; direct-edit3/4/5, links-bookmarks, field-design, help-fields, sign-field once each, exit 0.
- shot-debt1.mjs PASS 10/10; shot-sign-u3.mjs PASS (real CDP mode click). Electron clean.

## Screenshots viewed by the orchestrator (11)
- debt1-a: page before selection (no handles visible; the handle check is covered by b and c). debt1-b: grey rect selected with 8 handles + grip after the 6 px drag. debt1-c: gradient image selected after double-click replace. debt1-d: add editor grown to "Hi there, a much longer line of text". debt1-e/f: "This page has 6 form fields." / "This page has 3 form fields." on tabs.pdf pages 1 and 2. Toolbar reads "Save As…" in every shot.
- u3-a: drawn Signature4 with the mupdf orange SIGN tag (§10 cosmetic). u3-b: sign dialog "Signing field Signature1 on page 1". u3-c: "Signed" badge; u3-d: "✓ Valid, untrusted"; u3-d-verify: Signature1 valid whole file, Signature4 valid partial coverage, Signature2/3 empty. One badge in view at fit-width, the second field is below the fold.

## Rows not proven by a failing-then-passing check (by design)
5 (housekeeping), 9 (deferred), 17 (skipped), 20 (code-only, unreachable from a drive), 2 and 16a (pin existing behaviour), 8/16d/21/22/23 (docs, grep).

## Left for §10
direct-edit3 click-before-key; orange SIGN tag; `contentReplaced`/`contentLastAdd`/`contentLastShift` carry over after open; render/thumbnail page errors and `checkAt` without reason token; journal `.tmp` path shared (saves serialised).
