# P6 U4 rerun (after fixes E-1, R1, R2, S1), 2026-09-24

Status: ALL GREEN. Nothing in src/ was changed.

1. npx tsc --noEmit -p tsconfig.json: clean (exit 0).
2. npm run build: exit 0. npm test: 436 tests, 435 passed, 0 failed, 1 skipped (duration 36.7 s).
3. Drive scripts (node scripts/drive.mjs), all exit 0 with no red steps:
   - direct-edit4.jsonl, 3 runs: 0/0/0
   - direct-edit5.jsonl, 3 runs: 0/0/0 (includes the new S1 steps)
   - direct-edit, direct-edit2, direct-edit3, links-bookmarks, field-design, help-fields: 0 each
   - Logs are in .quad/scratch/rr-drive-*.log, rr-test.log and rr-build.log.
4. Fixtures:
   - shasum -a 256 -c checks/fixtures.sha256: every entry OK.
   - fixtures/**/*.pdf and *.png compared with checks/u4-hash-before.txt: no hash changed. The only new files are fixtures/p6/links.pdf and fixtures/p6/tabs.pdf, which are allowed.
   - After the shot run, fixtures.sha256 was still all OK.
5. node .quad/shared/p6/checks/shot-direct4.mjs: PASS all (exit 0). PNGs in .quad/shared/p6/checks/review-out:
   u4-a-link-handles, u4-b-link-moved, u4-c-link-editor, u4-c-link-retargeted,
   u4-d-tab-b1, u4-e-edit-outlines, u4-f-im2-nudged, u4-g-redact-drag (.png)
6. Electron hygiene: pgrep -fl -- '--drive=' and pgrep -fl review-out both returned nothing, so no stragglers were left and none had to be killed.

Open defects: none found by this rerun. Outside this rerun, the renderer fixer noted probe check h1 (Help rows) was already BAD before their edits. That probe is not part of this rerun, so it has not been re-checked here.
