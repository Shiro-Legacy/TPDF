# Review — housekeeping-b (2026-09-25)

**Verdict: ACCEPT-WITH-FIXES.** One real regression in the new signal forwarding in `scripts/drive.mjs` (a SIGTERM'd run exited 0), fixed here. One acceptance clause cannot pass as written because of a source leak outside this unit's scope (open issue 1).

Diff taken against `.quad/backup/pre-housekeeping-b-20260925.tgz`, extracted to `$TMPDIR/hkb-review` and deleted afterwards. The only changed files are src/ (9 files), both tsconfigs, scripts/drive.mjs, scripts/session.mjs, 22 test files plus the new `tests/tmp-cleanup.test.mjs`, README.md, docs/agent.md and docs/gotchas.md (from the orchestrator). Nothing under fixtures/, package*.json or progress.md changed. **Nothing outside the contract's scope.**

## Verification

- `npx tsc --noEmit -p tsconfig.json`: clean. `-p tsconfig.preload.json`: clean. `npm run build`: ok.
- `node --test` in batches of 8:
  - tmp-cleanup, agent-session, debt1-drive, drive-guard, p3-coded-drive, p5-drive-field, p6-drive9, p5-verify-crypto: 28/28.
  - p1-open, p1-save, p3-smoke2, p2-export, p2-nav, p5-sign-field, p3-drive, p6-batch: 131/131.
  - p6-compare, p6-bates, p6-drive3/4/6/7/8, p2-decorate: 36/36.
  - p1-worker, p5-sign, p5-verify, p3b-content, p2-shell, p6-text8, p3-edit, p5-sign-crypto: 111 pass, 1 skip (the usual skip).
  - After my fix: tmp-cleanup, p3-drive, drive-guard and agent-session again: 31/31.
- `find $TMPDIR -maxdepth 1 -name 'pdf4all-*'`: 9 before. The only new entries after the batches were `pdf4all-new-*`, 3 for each agent-session run (see open issue 1). I deleted the 6 that my runs created, so the count is back to 9. The 9 that were already there are 6 `pdf4all-drive[-test]-*` dirs with mtime 11:55, before the tests/drive.mjs edits, and 3 `pdf4all-new-*`. I left them alone.
- `fixtures/p0b/.ca/ca.srl` sha256 is `eca06ff8…fd814` before and after, unchanged.
- No `--drive=` process was left behind. I killed no Electron I did not start.

## Refutation attempts

### 1. Is SRC behaviour-neutral?
- **WinAnsi (S1): holds.** The engine-core `WIN_ANSI_EXTRA` has the same 27 entries as the deleted copy. `winAnsiByte` and `winAnsiBytes` are the same logic, including `?` for characters outside WinAnsi and escaping of `(`, `)` and `\`. `checkStampText(text, op, max)` gives the same error strings, and the call site passes `MAX_STAMP_CHARS` (200, engine.ts:143) at engine.ts:3227. tsc would catch any leftover 2-argument caller, because `max` is required. The closure copies had shadowed the imports, so the imports are now what engine.ts:3148/5637/5669 already used.
- **Removed symbols: holds.** `confirmDiscard`, `confirm-discard`, `trustList`, `trust-list`, `editIsAscii`, `LabelStyle`, `ClipCopyReply`, `ClipPasteReply`, `fieldPath`, `contentApplyButton`, `contentApplyObjectButton` and `WIN_ANSI_EXTRA` (outside engine-core) have no hits in src/, tests/, scripts/, .quad/shared/p6/scripts or README. The only doc hit is docs/gotchas.md:72, which is already reworded to "`trust-list` removed 2026-09-25".
- **Tests that read main.ts source: holds.** Only two do. p6-batch.test.mjs:268 checks only `pick-folder` and `batch-io`. p2-shell checks menu and HTML ids, and `content-apply` / `content-apply-object` stay in index.html. No test lists the preload API.
- **Replaced helpers: holds.** `normaliseLine` (engine.ts:6339) has the same body as the deleted renderer copy. It comes from `./engine.js`, which renderer.ts already imports values from. `copyBytes` in engine-core has the same body; only an `as ArrayBuffer` type cast differs. `engine-core.js` is in the policy.ts:36 asset map. The module-level `round2` (renderer.ts:6023) has the same body as the inner copy it replaces.
- **S5: holds.** The comment targets are correct: `MAX_SAVE_BYTES` is in limits.ts:13 and `MAX_NOTE_CHARS` in engine.ts:125.
- **S6: holds.** `noUnusedLocals` is set in both tsconfigs, and both tsc runs and `npm run build` pass.

### 2. Is the TESTS cleanup safe?
- **`after()` placement: holds.** All 19 wrapped files use a file-level `after()` that removes only the dirs that file made. None uses `describe` or `concurrency`, and `npm test` runs files with plain `node --test`. The local `const mkdtemp` is declared before its first use in every file (checked with a line-number scan). `t.after(ending)` in agent-session runs before the file-level `after`. The p1-save `chmod` calls only set files back to 0644, so `rm` never hits an unreadable dir.
- **drive.mjs file mode: 2 defects found, both fixed (see Fixes).**
  - `rm` runs in `finally` after the child's `close`, with `maxRetries: 5`. That part holds.
  - **Refuted: exit code.** A SIGTERM'd drive.mjs exited **0**, with an empty stdout and "no report written". I reproduced it with a private TMPDIR, a `wait 8000` script and SIGTERM after 3 s. Electron quits cleanly with code 0 when it gets SIGTERM, so `process.exitCode = code ?? 1` went green. Before this unit, the same kill ended node by the signal (143). Tests and the accept `red()` still failed on the empty report, but the exit code broke "exits 0 iff every expect matched". The build note's "exit 1" was not reproducible.
  - An `rm` that still failed after its retries would reject in `finally` and turn a green run red. session.mjs already swallows the same failure.
- **drive.mjs --session and session.mjs: holds.** I tested with a node harness, a private TMPDIR and one `state` step. After SIGTERM: exit 1, Electron gone, `pdf4all-session-*` removed. After stdin EOF: exit 0, dir removed.
  - A `close()` on a session whose Electron crashed now throws "no report at …" instead of ENOENT. No caller checks that text: pdf4all-mcp.mjs:144 catches everything.
  - `session.dir` is still used by `sessionDrives()` only while the session runs (pgrep), so it is fine that the dir is gone afterwards.
- **Callers that read `${OUT}` without `--out`: holds.**
  - Every test that runs drive.mjs passes `--out=<own mkdtemp>` and `--report`.
  - agent-session (a), (b) and (f) use `createSession()` without `out`, but never read saved files. (c) passes `out`.
  - The accept scripts read only stdout. Batch drives get `--out` from `stage()`.
  - **pdf4all-mcp.mjs:27 always passes `out`** (its own `pdf4all-mcp-*` or `--out`), so the user's saved PDFs survive.
  - docs/agent.md and README now say that `${OUT}` is temporary without `--out`.
- **Is tmp-cleanup.test.mjs isolated, and does it catch regressions?**
  - It catches a T1 regression in drive.mjs: the red-before log shows `pdf4all-drive-Xo4qz6` left behind. It catches a T2 regression through the static check (`not ok 2` names p5-verify-crypto).
  - **Isolation is partial.** TMPDIR reaches node's `tmpdir()`, so drive.mjs's own dir lands in the private dir. It does **not** reach Electron's `app.getPath('temp')`: a `new` step under `TMPDIR=<private>` still created `pdf4all-new-*` in the real `$TMPDIR`. The test only runs `open`, so its assertion is sound. It does not cover session.mjs cleanup (not required by T3).
- **T2 certificate: holds.** `-set_serial 0x5eed` does not clash with the CA serial `0A842C…`, and the test asserts only `valid`/`expired` and the SKI `signer-id`, never the serial. It passes, and `ca.srl` is unchanged. Note that the static check accepts any `-CAserial`, even one pointing into fixtures/. That is weak, but matches the contract wording.

### 3. Orchestrator files (light review)
- `.gitignore`: fine.
- `accept-unit{,-nogui}.sh`: `rm -rf /tmp/pdf4all-accept-*` runs before the result line. In zsh, an empty glob would print "no matches found" but not fail, because the scripts use `set -u` only. Batch drives always stage, so the glob always matches.
- accept-unit-nogui.sh still runs `npm run package` unconditionally. The contract says to package only when the human's packaged app is closed, so the orchestrator must take care when running the gate.

## Fixes made
- `scripts/drive.mjs:62-64,69`: `let signalled = false`, set in the signal handler, and `process.exitCode = signalled ? 1 : code ?? 1`. Re-run with SIGTERM: exit 1, Electron gone, dir removed.
- `scripts/drive.mjs:71-72`: the `finally` `rm(...)` now has `.catch(err => console.error(\`temp dir not removed: ${err.message}\`))`, so a leftover dir can't flip a green run red. tmp-cleanup.test.mjs still catches a real leak.

## Open issues
1. **The acceptance clause "nothing new in $TMPDIR from the suite" will fail: +3 `pdf4all-new-*` per suite run.** They come from `src/main.ts:432` `newBlank`, via agent-session (c) file and session and (d) MCP. The leak is deliberate: it carries a `ponytail:` comment and is by design per agent-session D4. It is not in S1–S6, and a private TMPDIR can't redirect it, because `app.getPath('temp')` ignores TMPDIR. Choose one: accept it as known in the gate, or schedule a follow-up (for example, remove the dir when the drive/session quits, or give newBlank a `--drive-out`-relative dir in drive mode).
2. tmp-cleanup.test.mjs does not cover session.mjs cleanup or the SIGTERM path. I checked both by hand (above). Optional follow-up test.
3. The `-CAserial` static check would accept a serial file under fixtures/. That meets the contract as written.
