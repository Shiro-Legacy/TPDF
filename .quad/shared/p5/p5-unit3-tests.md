# P5 U3 tests seat - evidence (opus55, 2026-09-24)

Files: tests/drive/sign-field.jsonl (new, 32 steps), tests/drive/direct-edit.jsonl (line 32 expect only, per 9.12; still 75 steps),
tests/p5-drive-field.test.mjs (new, 2 tests), .quad/shared/p5/checks/shot-sign-u3.mjs (new, not run), .quad/shared/p3/design-agent-drive.md (appended "## P5 U3 drive additions").
scripts/drive.mjs unchanged.

Step count: 32, not 9.29's 31 - one added `key Escape` after the refused signs (closes the dialog, expect sign.dialog null; 9.12/9.22). The test pins 32.
Every sign step has a top-level `to`; the two refused signs write refused-signed.pdf / refused-nope.pdf, and the test checks neither file exists.

Run under the lock (13:0x): npm run build (exit 0); node scripts/drive.mjs tests/drive/sign-field.jsonl -> exit 1, 31/32 green;
node scripts/drive.mjs tests/drive/direct-edit.jsonl -> exit 0 (75 steps); node --test tests/p5-drive-field.test.mjs -> 1 pass (pyHanko), 1 fail (the red step below).
Only red step: #20 `sign {field:'Signature3'}` on field-signed2.pdf expects sign.last.revisions 3; saw
{op:'sign', ok:true, name:'field-signed3.pdf', signer:'P0B Signer', revisions:2, field:'Signature3'}, dialog null, dirty false, revision 0.
Cause: src/renderer/sign-prepare.ts:128 `revisions: sourceLength ? 2 : 1` (pre-U1 semantics: any incremental reports 2). The file itself is right:
field-signed3.pdf has 3 %%EOF, pyHanko lists 3 signatures. Not weakened; returned as a request to the engine seat / fable51.

Checked by hand on that run's report: drawn rect Signature4 [72,300,292,360] (drag step and reopen equal); 6 sign steps, all contents "[redacted]",
no "password" key, no "pdf4all" JSON value.
pyHanko (verify-sign.py --ca fixtures/p0b/.ca/ca.pem): field-signed 1 sig; field-signed2: Signature4 INTACT ENTIRE_REVISION trusted docmdp-ok,
Signature1 INTACT ENTIRE_FILE trusted docmdp-ok; field-signed3: Signature4, Signature1 ENTIRE_REVISION + Signature3 ENTIRE_FILE, all INTACT trusted docmdp-ok;
hello-field: S1 INTACT ENTIRE_FILE trusted docmdp-ok.
Hashes: shasum -c p6 fixtures.sha256 all OK; fixtures/**/*.pdf|png vs u3-hash-before.txt: only fixtures/p5/empty-sigfield.pdf added.
pgrep -fl -- '--drive=' empty. User trust store ~/Library/Application Support/pdf4all-p0/trusted-certs.pem absent before and after.

Not done: shot-sign-u3.mjs was not run (sign-field.jsonl not fully green; the brief allows the GUI only on green). Syntax-checked with node --check only.

## Fixes after review
- S1 reproduced (probe review-u3-tests-probe.jsonl re-run: handle grip rev 2 rect [92,82,292,142], se rev 3, ArrowRight rev 4 +1 pt, Read-mode click opens S1 dialog, signed Signature4 click → direct.status "Verify signatures", status valid; no such step in sign-field.jsonl). Fixed in tests/drive/sign-field.jsonl (+11 steps, 32 → 43):
  after the field-signed.pdf reopen: click (182,330) → direct.status "Verify signatures", dialog null, [empty,empty,valid]; key Escape.
  hello block after the delete-undo: handle grip (172,102) dx20 dy10 → S1 [92,82,292,142] rev 7; undo → rev 8; click → S1 dialog; Escape; ArrowRight → S1 [73,72,273,132] rev 9; undo → rev 10.
  before the S1 sign: mode read; click (172,102) → S1 dialog; Escape. Later revisions re-pinned 6 → 10 (fill/add refusals, S1 sign).
- S3 reproduced (1 px drag step had no error expect; STATE error ''). Added "error":"" to that drag's expect.
- tests/p5-drive-field.test.mjs: steps.length pinned 32 → 43. Header comment updated.
- Ran under the lock: tsc ok; npm run build; drive sign-field.jsonl exit 0 (43 steps, 0 red); drive direct-edit.jsonl exit 0; node --test tests/p5-drive-field.test.mjs 2/2 pass (incl. pyHanko).
- Hashes: p6 fixtures.sha256 all OK; fixtures vs u3-hash-before.txt: only fixtures/p5/empty-sigfield.pdf added.
- Not killed: a --drive= Electron under review-renderer/ (another reviewer's probe, not mine).
