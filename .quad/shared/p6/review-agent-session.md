# Review — agent-session (reviewer, Opus 5.5, 2026-09-25)

Verdict: **accept with the fixes made below.** No blocking defect found. Four fixes applied (MCP help, docs honesty, test cleanup, one new test). The remaining items are listed with proposed fixes.

Probe scripts were in the reviewer scratchpad (not in the repo). The encrypted fixture's password was read at runtime from tests/drive/password-pages.jsonl and is not written anywhere here.

## Findings

1. **Low: fixed.** `pdf4all_help` built a RegExp from an unvalidated `topic`. Evidence: `tools/call pdf4all_help {"topic":"("}` returned `{"id":3,"error":{"code":-32603,"message":"Invalid regular expression: /^## (\\b/im: Unterminated group"}}`. Fix: in scripts/pdf4all-mcp.mjs `help()`, a topic outside the schema enum now returns the whole file (one line).
2. **Medium: fixed (docs).** docs/agent.md left out traps that an agent meets right away. Evidence, all over MCP or `--session`:
   - `content {target:"no such text anywhere", text:"x"}` → `isError false`, `error ""`, `refused []`, only `announcement "No text target …"`.
   - `pdf4all_do {cmd:"txt-add"}` → `isError false`, `error ""`.
   - `new` on a dirty doc → `dirty false, revision 0`, and the edits are silently gone (drive mode is `smoke`, so there is no discard box).
   - After a step timeout the MCP server restarts on hello.pdf.
   - `content` without `text`/`find`/`action` falls into the legacy self-test edit (renderer.ts smokeContent: first run → "BETA", then "BRAVO"×120).
   Fix: 6 lines added to docs/agent.md (now 161 lines) in Start, Tools and Modify.
3. **Medium: fixed (tests).** Tests (a), (b) and (c) did not clean up when an assertion failed. A failed assert left the `--drive=-` Electron running with its stdin pipe open, which holds the test process open (a hang, not a failure). Fix: added `ending(session)` (ends stdin when the session is still running) via `t.after` in (a), (b) and (c). (d) already had `finally`.
4. **Medium: fixed (tests).** Nothing tested the stdin-mode FIFO (concurrent `step()`) or redaction in STEP lines. Test (a)'s `times[0][1] <= times[1][0]` is tautological, because the test awaits each step before sending the next (left in place; harmless). Fix: new test (f):
   - three `step()` calls go out at once (open encrypted.pdf, password, state) → cmds come back in order.
   - The password STEP has `contents "[redacted]"` and `needsPassword false`.
   - No STEP contains the password.
   - A `find` for the password → the step rejects `/session ended/`, and the report has `ok false, error "password leaked in STATE"`, without the password.
   Mutations it catches: emitting the raw line instead of `step`, and a broken waiter/queue.
5. **Medium: not fixed (renderer.ts is outside ownership).** A `content` step with contents that carry none of `text`/`find`/`action` runs a destructive self-test edit. An agent that sends a wrong key (e.g. `{"target":"A","replace":"B"}`) rewrites the page's first run. Proposed fix: in `smokeContent`, when `contents` parsed to an object with none of `text`, `find`, `action` or `password`, call `showError("content: send text, find or action")` and return. Keep the legacy path only for `contents === null` and the existing smoke payloads (grep tests/drive first).
6. **Low: not fixed.** An unknown `cmd` is a silent no-op (pre-existing renderer behaviour), and MCP reports `isError false`. Now documented. Proposed: main or the MCP server could refuse a cmd that is not in a known-op list with `error "unknown cmd"`.
7. **Low: not fixed.** A STATE that leaks a secret ends the session with no STEP line. The client sees only `session ended (exit 1)`; the reason is only in the report. This is secret-safe. Proposed: `emit({error:'password leaked in STATE'})` before `finish` in the console handler.
8. **Low: not fixed (cosmetic).** `session.step({cmd:"close"})` rejects `session ended (exit 0)`. `drive.mjs --session` with a `close` line prints `{"error":"session ended (exit 0)"}` before the (correct, ok) report. Proposed: in session.mjs, treat a `close` line as `close()`.
9. **Low: not fixed (UX, documented ceiling).** GUI Save (⌘S) on Untitled.pdf silently writes into the temp folder. For Acrobat parity, Save on an untitled document should act as Save As. Proposed: in main's save-in-place path, treat a path under a `pdf4all-new-` temp dir as Save a copy.
10. **Low: not fixed.** `pdf4all_save {path:"sub/filled.pdf"}` into a missing folder → `save {ok:false, reason:"write-failed"}`, `isError true`. It is correct, but the docs are silent. Proposed: mkdir the parent for relative paths under `--out`, or add one doc line.
11. **Info.** A bad `new` size shows "Could not read Untitled.pdf." while the previous document stays open. This is contract-mandated (no new reason token).
12. **Info (pre-existing, not this unit).** Every full `npm test` bumps `fixtures/p0b/.ca/ca.srl` (tarball `…650C` → builder run `…650D` → my run `…650E`), via tests/p5-verify-crypto.test.mjs. The file is not in either hash list.
13. **Info.** `shot-agent.mjs` was reviewed by reading only (not run):
   - It sends ⌘N by `unix id` with the frontmost check in the same osascript call.
   - It exits 2 with `OSA-UNAVAILABLE`, and exits 1 with `FAIL … blocked: frontmost X`.
   - It SIGTERMs only its own spawned child (the electron binary path, not the shim).
   - It writes `review-out/agent-new.png` and prints `PASS all`.
   - It asserts `doc.name/pageCount/pages.count/dirty/error`.
   Its fixed settle waits (`delay 0.4`, `sleep(800)`) are copied from shot-coded.mjs; they are not budgets.

## Checks (commands and result lines)

A. Session protocol. Everything below ran under the lock after `npm run build` (build=0).
- `node scripts/drive.mjs tests/drive/coded-edit.jsonl` → `exit=0 steps 8 red 0 ok true` (script has 8 lines; matches the builder).
- `node scripts/drive.mjs tests/drive/direct-edit8-save.jsonl` → `exit=0 steps 42 red 0 ok true` (42 lines).
- `node scripts/drive.mjs tests/drive/agent-session.jsonl` → `exit=0 steps 7 red 0 ok true`.
- probe-session (createSession, three steps fired at once) → `open form.pdf | state form.pdf | goto`, in order.
- Bad line `{nope` → `{"error":"bad line: Expected property name or '}' in JSON at position 1 …"}`; array `[1,2]` → `bad line: not a JSON object`; the session went on answering.
- Bad line holding a would-be secret → the V8 message has only a position, and `leaks= false`.
- text-add with `\n` in the text → the STEP parsed (one line each).
- `{"cmd":"save-to","path":"${OUT}/probe-out.pdf"}` → path substituted, file exists.
- `drive.mjs --session < timeout.jsonl` (`wait 70000`) → `{"cmd":"wait","ms":70000,"error":"step timeout"}`, `{"error":"session ended (exit 1)"}`, report `error "step timeout"`, exit 1.
- `drive.mjs --session < recipes.jsonl` (no close; last line expects a wrong name) → 23 result lines, report printed at EOF, `recipes-exit=1`.
- packDrive: moved byte-identical (diff shows only the removal from drive.mjs and the export in session.mjs), and applied per line in session.step.

B. Secrets (stdin mode):
- password step → `contents "[redacted]"`, `needsPassword false`, `leak in STEP false`.
- encrypt step → `"[redacted]"`, `leak false`.
- `find {query:<password>}` → the step rejected `session ended (exit 1)`; report `ok false, error "password leaked in STATE"`, `report leaks pw false`.
- Now pinned by test (f).

C. Blank document:
- Default `new` → `Untitled.pdf 1 "" false 0`.
- Refused sizes → `Could not read Untitled.pdf.`, previous doc kept: `"legal"`, `[1e9,1]`, `null`, `[10,10]`, `[71.9,100]`, `["612","792"]`, `[612,792,1]`, `"A4"`, `{}`, contents `'"legal"'`.
- Accepted: `[72,14400]` and contents `{}` (letter).
- `new` on a dirty doc → no hang, `dirty false revision 0`.
- `temp dirs made 4` for 4 accepted `new`s (one fresh dir each).
- save-in-place on Untitled → `save {ok:true,inPlace:true}`; the temp file changed 431→746 bytes, and the dir holds only `Untitled.pdf`.
- STATE/STEP contains `pdf4all-new-` or tmpdir → `false false`.
- mupdf: `wasRepaired fn function false pages 1 bounds [0,0,595.276,841.89]`. A corrupted xref offset gives `wasRepaired true`, so test (e)'s repair check is live.
- Accelerators (`grep -n accelerator src/main.ts`): `CmdOrCtrl+N` appears once (:1074), next to `Alt+CmdOrCtrl+N` and `Shift+CmdOrCtrl+N`; no collision.
- The menu item calls `newBlank(window.webContents)`, the same function as the drive cmd.

D. MCP (raw pipes, probe-mcp):
- `tools/call` before `initialize` answered (lenient).
- `initialize` without params, id 0 → `{"id":0,…"protocolVersion":"2025-06-18",…"serverInfo":{"name":"pdf4all","version":"0.0.0"}}`. String id `"x-1"` echoed `"2025-03-26"`.
- `replies to 3 notifications 0` (initialized, id-less ping, id-less unknown method).
- Six tool schemas are all `type:"object"`, with valid JSON Schema (`oneOf`, `enum`, `type` arrays).
- `tools/call` without params → `-32602`; `[1]` → `-32600 id null`; `{garbage` → `-32700 id null`, then `ping` → `{}`.
- `pdf4all_open {}` → `isError true, "path is required"`.
- Back-to-back open+state → `open form.pdf false | state form.pdf`, reply order `[10, 11]`. Fill with the name read from `state.fields.defs` → `Ada`.
- `pdf4all_save {path:"filled.pdf"}` → under `--out`, `true`.
- `pdf4all_do {cmd:"close"}` → `isError true`; `pdf4all_new {size:"legal"}` → `isError true`.
- `non-JSON stdout lines 0`; stdin EOF → `exit 0`, `drive procs left: ""`.

E. Tests:
- `node --test tests/agent-session.test.mjs` (after my edits) → `# tests 6 / # pass 6 / # fail 0 / # skipped 0`.
- Every Electron test has `skip: CI_NO_ELECTRON`. The only timer is the copied `drive()` kill-guard.
- Red log `.quad/shared/p6/acceptance/agent-session-red-before.log` has 5 `not ok` lines.

F. Docs:
- The recipe lines ran through `node scripts/drive.mjs --session` (recipes.jsonl, 23 steps) and all landed: new a4, text-add (`lastAdd`), fields add text/checkbox, save-to, open, goto, content target → `edited {from:"Invoice 42",to:"Invoice 43"}`, find/replace (honestly refused `overflow`), undo/redo `rev +1 each`, header-footer footer and `{bates}`, note, find, fill text/checkbox, redact search + redact-apply `to` → `verified`.
- STATE keys named in the doc all exist: `doc{name,pageCount,readOnly,needsPassword}`, `page`, `pages.count`, `revision`, `dirty`, `canUndo`, `canRedo`, `error`, `announcement`, `content.*`, `fields.values/defs/last`, `save`, `search`.
- The required ceilings are present: single-run, repertoire gate, no OCR, points with a top-left origin, 0-based pages.
- The README (+3 lines) and the design doc (`## Agent session`, 6 lines) are accurate. The design doc's "its STEP carries `error`, then the app exits" is verified above.

G. Ownership and rules:
- `find fixtures -type f | wc -l` → 200 (tarball 200).
- `shasum -a 256 -c agent-session-hash-before.txt` → 0 non-OK of 56; `fixtures.sha256` → 0 non-OK.
- `diff -q package.json` against the tarball → same.
- Every existing tests/drive/*.jsonl `cmp` equal to the tarball. `diff -rq tests` shows only agent-session.test.mjs and agent-session.jsonl.
- `npx tsc --noEmit -p tsconfig.json` → 0.
- No `--drive=` process after any run (`pgrep` exit 1).
- The declared departures are justified:
  - Help list, not a table row (row count pinned in 3 files).
  - `doc.name`/`doc.pageCount` (the real STATE keys).
  - `find` instead of a content listing (`content` has no text).
  - No `mode` line (text-add has no OP_MODE).
  - `close` is not a step (keeps report = steps sent).
  - The MCP extras are sane.

Suite (once, at the end, after the edits): `npm test` → `# tests 591 / # pass 590 / # fail 0 / # skipped 1` (590 + my test (f)).

## Files changed by the reviewer
- scripts/pdf4all-mcp.mjs: `help()` topic whitelist (1 line).
- docs/agent.md: +6 lines (Start, Tools, Modify).
- tests/agent-session.test.mjs: `ending()` helper, `t.after` in (a)/(b)/(c), and new test (f).
