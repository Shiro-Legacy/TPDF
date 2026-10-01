# P5 U3 integration (integrator, 2026-09-24)

## Applied fixes
- src/renderer/sign-prepare.ts:128 scanSignature: `revisions: sourceLength ? hits("/ByteRange").length : 1` (was `sourceLength ? 2 : 1`). The unsigned branch stays 1.
  - Why /ByteRange and not the intended `hits("%%EOF").length`: I tried %%EOF first and it turned tests/p5-sign.test.mjs:88 red (`3 !== 2`). fixtures/p0b/signed-intact.pdf already has 2 %%EOF from pyHanko's own increment but only 1 signature. Changing that assertion to 3 was blocked by the permission classifier. The /ByteRange count is one per signature dict (the earlier ones plus the new sentinel). It gives 2 there, 2 for sign.jsonl signed-twice and 3 for field-signed3, so no existing expect changes.
  - Ceiling: a /ByteRange inside a stream, or an earlier signature dict rewritten in an increment, would raise the count. No fixture does either.
  - fable51 decision: `revisions` currently means "signatures after this one". A literal revision count (%%EOF) would need tests/p5-sign.test.mjs:88 changed to 3 by its owner.
- tests/p5-sign-field.test.mjs: module-level `signed2`, plus a new test "a third signature on a twice-signed source reports revisions 3". It signs Signature3 (page 1) on signed2 via patchSignature+cmsDetached and asserts `revisions === 3` and reopen `info.signatures 3`. It pushes signed3 into pyhankoCases (`--expect-signatures 3`, Signature3 INTACT ENTIRE_FILE trusted docmdp-ok). The pyHanko count pin is now 6 (was 5).
- Accepted: sign-field.jsonl has 32 steps (the extra `key Escape` per §9.12/§9.22); tests/p5-drive-field.test.mjs:80 pins 32. Accepted: `field-add` returns `name` for every kind (additive).

## Suite
- `npx tsc --noEmit` clean. `npm run build` exit 0.
- `npm test`: 450 tests / 449 pass / 0 fail / 1 skip. U4 baseline was 436/435/0/1; the suite gained 14 tests.
- First run with the %%EOF variant: 1 fail, tests/p5-sign.test.mjs:88 `3 !== 2`. The /ByteRange variant fixed it.

## Drive (all under the lock)
- sign-field.jsonl ×3 (run-1..3): exit 0/0/0, 32/32 green each time.
- sign 0, verify 0, field-design 0, direct-edit 0 (75 steps), direct-edit5 0, help-fields 0. No red steps.
- Reports: .quad/scratch/p5u3/integ/*.json

## Redaction
- Every `sign` step's contents is `[redacted]`: 6 per sign-field run, 3 in sign.jsonl, 1 in verify.jsonl. No report contains a `"password"` key or a quoted `"pdf4all"` value.

## pyHanko (run-3 outputs, --ca fixtures/p0b/.ca/ca.pem)
- field-signed.pdf (1): Signature4 INTACT ENTIRE_FILE trusted docmdp-ok. Exit 0.
- field-signed2.pdf (2): Signature4 INTACT ENTIRE_REVISION trusted docmdp-ok; Signature1 INTACT ENTIRE_FILE trusted docmdp-ok. Exit 0.
- field-signed3.pdf (3): Signature4 and Signature1 INTACT ENTIRE_REVISION trusted docmdp-ok; Signature3 INTACT ENTIRE_FILE trusted docmdp-ok. Exit 0.
- hello-field.pdf (1): S1 INTACT ENTIRE_FILE trusted docmdp-ok. Exit 0.

## Hashes
- `shasum -c .quad/shared/p6/checks/fixtures.sha256`: 7/7 OK.
- Baseline diff against u3-hash-before.txt: 52 files before, 53 after. The only addition is fixtures/p5/empty-sigfield.pdf (07e46851…a206).

## GUI (shot-sign-u3.mjs, exit 0, "PASS all")
- u3-a-drawn-field.png: Fill forms mode. The drawn Signature4 is a dashed box with a "Sign" caption, and the status bar reads "Added signature field Signature4".
- u3-b-sign-dialog.png: Sign with certificate dialog showing "Signing field Signature1 on page 1". The page and appearance inputs are hidden.
- u3-c-signed-badges.png: field-signed2.pdf opened read-only. A signed field shows "Digitally signed by P0B Signer … Reason: drive field" with a "Signed" caption.
- u3-d-verify-dialog.png: Verify dialog. Signature1 and Signature4 are valid/untrusted; Signature2 and Signature3 are empty.
- u3-d-valid-badges.png: the "✓ Valid, untrusted" caption is visible above the signed field.
- Observation (not a defect): an empty /Sig field without /AP renders mupdf's orange "SIGN" tag (probe C2).

## Hygiene
- `pgrep -fl -- '--drive='` is empty.
- User trust store is absent before and after.
- No trusted-certs.pem under .quad/shared/p5/checks/review-out/.

## Open defects
- None blocking.
- Pending decision (fable51): which meaning `revisions` should have; see Applied fixes.
