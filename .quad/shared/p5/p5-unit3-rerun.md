# P5 U3 rerun, after the review fixes (2026-09-24, Opus 5.5)

Nothing in src/ or tests/ was changed. Scratch files are in .quad/scratch/p5u3/rerun/.

## Results
- `npx tsc --noEmit -p tsconfig.json`: clean.
- `npm run build && npm test`: exit 0. 453 tests: 452 pass, 0 fail, 1 skipped. The skip is "legacy p12 parses when rc2-40-cbc exists", which is environmental.
- `scripts/drive.mjs`, every run exit 0 and 0 red steps:
  - sign-field.jsonl ran 3 times (run-1, run-2, run-3), 43 steps each.
  - sign 6, verify 17, field-design 23, direct-edit 75, direct-edit5 35, help-fields 21.
- pyHanko on the run-3 outputs, every run exit 0:
  - field-signed.pdf (expected 1): Signature4 INTACT ENTIRE_FILE trusted docmdp-ok
  - field-signed2.pdf (expected 2): Signature4 ENTIRE_REVISION and Signature1 ENTIRE_FILE, all INTACT, trusted, docmdp-ok
  - field-signed3.pdf (expected 3): Signature4 and Signature1 ENTIRE_REVISION, Signature3 ENTIRE_FILE, all INTACT, trusted, docmdp-ok
  - hello-field.pdf (expected 1): S1 INTACT ENTIRE_FILE trusted docmdp-ok
- Fixtures:
  - `shasum -c` against p6 fixtures.sha256: every file OK.
  - fixtures/**/*.pdf|png compared with u3-hash-before.txt: nothing changed. The only new file is fixtures/p5/empty-sigfield.pdf.
- shot-sign-u3.mjs: ran once, "PASS all".
  - Shots in checks/review-out/: u3-a-drawn-field, u3-b-sign-dialog, u3-c-signed-badges, u3-d-valid-badges, u3-d-verify-dialog (.png).
  - The user's trusted-certs.pem did not exist before the run and still does not exist after it.
- Electron hygiene: `pgrep -fl -- '--drive='` found nothing. The earlier straggler (pid 68639) was already gone.
- Secrets: no key or password material appears in any log or report I kept. The password string does occur in the repo path and the package name, but those are the only matches.

## Still open (neither one is red)
1. The S2 drive expect was never applied. tests/drive/sign-field.jsonl has no step that presses Enter on the S1 .sig-target, and tests/p5-drive-field.test.mjs:80 still pins 43 steps.
   - The renderer fixer's exact edit is still pending: insert the Enter and Escape steps after the ArrowRight step at rev 9, then pin the count to 45.
   - The static selector test in p2-shell covers the fix itself. It does not test the behaviour end to end.
2. Renderer probe check f4 is still BAD, as it was before the fixes. The check is that fields.count includes signature fields while the defs/values do not. No review finding covered it, so it was never triaged.

## Verdict
Everything is green. The engine fixes (E-R1, E-S1), the renderer fixes (R-U3-1, S2) and the tests fixes (S1, S3) all hold. The two gaps above are coverage and triage items only.
