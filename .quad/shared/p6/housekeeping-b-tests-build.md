# housekeeping-b — TESTS implementer build note (2026-09-25)

Scope: contract-housekeeping-b.md T1–T3 only (tests/**, scripts/drive.mjs, scripts/session.mjs). No src/ edits.

## Files changed

- `tests/tmp-cleanup.test.mjs` (new, T3, 2 tests):
  (1) runs `scripts/drive.mjs` on a 1-line script (`open fixtures/hello.pdf`, expect revision 0) with `TMPDIR=<fresh mkdtemp>`, no `--out`; asserts exit 0, 1 step, and no `pdf4all-*` entry left in that dir; the fresh dir is removed by `t.after`.
  (2) static: no `tests/*.test.mjs` passes `'-CAcreateserial'` without `'-CAserial'`.
- `scripts/drive.mjs` (T1): file mode wrapped in `try { … } finally { rm(dir, {recursive, force, maxRetries: 5}) }`; the finally runs after the child's `close` (Electron exited). SIGINT/SIGTERM/SIGHUP are forwarded to Electron, so a killed drive.mjs (tests' timeout `child.kill()`) no longer orphans Electron and still cleans up. `--session` mode: same signals kill the session child, then `session.close()` and exit 1. Header comment says: without `--out`, `${OUT}` files are temporary (they live in the removed dir).
- `scripts/session.mjs` (T1): the child's `close` handler reads the report text, then `rm`s the `pdf4all-session-*` dir (every way a session ends: close(), stdin EOF, crash). `close()` returns the in-memory report (throws `no report at …` if none). Given `out`/`report` paths outside the dir are untouched (the MCP server passes its own `--out`).
- `tests/p5-verify-crypto.test.mjs:193` (T2): `-CAcreateserial` → `-set_serial 0x5eed` (CA serial is 0A84…, so no clash). No serial file is read or written.
- 19 files with async `mkdtemp` — agent-session, debt1-drive, drive-guard, p1-open, p1-save, p2-export, p3-coded-drive, p3-drive, p3-smoke2, p5-drive-field, p6-batch, p6-bates, p6-compare, p6-drive3/4/6/7/8/9: import `mkdtemp as makeTempDir` + `rm`, `import test, { after }`, and a 4-line block after `const root` (p2-export: after `const engine`): a local `mkdtemp` that records each dir, and a file-level `after()` that rm -rf's only those dirs. Call sites unchanged; removal runs after all the file's tests (never before an assert).
- `tests/p5-sign-field.test.mjs` (pyHanko test): `p5u3-field-*` mkdtempSync now in try/finally `rmSync` (63 of these were left in $TMPDIR).
- `tests/p2-nav.test.mjs`: `scratchPath()` records its `pdf4all-p2u5-<pid>-*.pdf` files; file-level `after()` removes them.

## Red before

`.quad/shared/p6/acceptance/housekeeping-b-red-before.log`: `not ok 1` (leftover `pdf4all-drive-Xo4qz6`), `not ok 2` (offender p5-verify-crypto.test.mjs). # pass 0 / # fail 2.

## Results (after `npm run build`, exit 0)

- tmp-cleanup: 2/2 ok.
- Batches (`node --test …`), each diffed on `find $TMPDIR -maxdepth 1 \( -name 'pdf4all-*' -o -name 'p5*' \)` before/after:
  - drive-guard, p3-coded-drive, p6-drive9, p5-drive-field, agent-session: 13/13; new entries: 3 × `pdf4all-new-*` (src leak, below).
  - p1-open, p1-save, p3-smoke2, p2-export, p2-nav, p5-sign-field, p5-verify-crypto: 108/108, 0 skipped; nothing new.
  - p3-drive, p6-batch, p6-compare, p6-bates, debt1-drive: 48/48; nothing new.
  - p6-drive3/4/6/7/8 + tmp-cleanup: 12/12; nothing new.
- SIGTERM to drive.mjs mid-run (private TMPDIR): Electron gone, temp dir removed, exit 1. `drive.mjs --session` with private TMPDIR: exit 0, dir empty afterwards.
- `fixtures/p0b/.ca/ca.srl` sha256 `eca06ff8…fd814` unchanged (mtime still 10:16) after p5-verify-crypto ran.

## Leftover leaks (not in my scope)

- `src/main.ts:432` — `newBlank` makes `mkdtemp(<temp>/pdf4all-new-)` for `Untitled.pdf` and never removes it (by design per agent-session contract: GUI ⌘S writes there). Every drive/session `new` step leaves one; agent-session.test.mjs leaves 3 per run (from tests (c) file drive + session, and (d) MCP).
- `scripts/pdf4all-mcp.mjs:27` — without `--out`, the MCP server makes `pdf4all-mcp-*` as the out dir and keeps it (intended: user's saved files). Tests always pass `--out`.
- `.quad/shared/p6/scripts/accept-unit*.sh` `stage()` makes `/tmp/pdf4all-accept-*` for batch drives (gate staging, excepted by the contract).
