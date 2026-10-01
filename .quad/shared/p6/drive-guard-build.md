# drive-guard — build note

Implementer: Opus 5.5 (single implementer, lean mode). Contract: `.quad/shared/p6/contract-drive-guard.md`.

## Files
- New: `tests/drive/drive-guard.jsonl` (8 steps, 0 click lines), `tests/drive-guard.test.mjs` (2 tests, both `skip: CI_NO_ELECTRON`), this note, `.quad/shared/p6/acceptance/drive-guard-red-before.log`.
- Edited (additive): `src/renderer/renderer.ts` (9 added lines, 0 removed), `docs/agent.md` (D6), `.quad/shared/p3/design-agent-drive.md` (one bullet under "## Agent session").
- Untouched: main.ts, scripts/, index.html, README.md, progress.md, package.json, every existing test and drive script, every fixture.

## Renderer diff (D1, D2)
```diff
--- src/renderer/renderer.ts (before)	2026-09-25 09:52:59
+++ src/renderer/renderer.ts	2026-09-25 09:53:13
@@ -1681,6 +1681,8 @@
   else if (op === "redact-search") { redactSearchText.focus(); redactSearchText.select(); }
   else if (op === "redact-apply-open") return openRedactDialog();
   else if (PAGE_OPS.has(op)) return runPageOp(op);
+  // A typo must not read as a green step (the chain above IS the list of renderer ops).
+  else showError(`Unknown command "${op}"`);
 }
 
 /**
@@ -5109,6 +5111,13 @@
       const parsed = JSON.parse(contents) as { password?: string; target?: string; targetId?: string; text?: string; action?: string; box?: number[]; image?: string; find?: string; replace?: string; preview?: boolean; page?: number };
       if (parsed.password) await smokePassword(parsed.password);
       wanted = parsed;
+      // An object with none of the keys never runs the self-test below (that stays the null/password path).
+      const has = (key: "text" | "find" | "action" | "password") => Object.prototype.hasOwnProperty.call(parsed, key) && typeof parsed[key] === "string";
+      if (!has("text") && !has("find") && !has("action") && !has("password")) {
+        showError("content: give text, find or action");
+        logContentSmoke();
+        return;
+      }
     } catch (err) {
       showError(`content: ${messageOf(err)}`);
     }
```
D2 sits inside the `try`, right after `wanted = parsed`, so a `{`-prefixed string that fails `JSON.parse` keeps today's `content: <parse error>` path, `contents === null` (the `--edit=content` smoke / menu) never enters the block, and a `{password}`-only object passes the guard (today's unlock-then-self-test). `contentRefusals/contentEdited/contentReplaced` are already reset at the top of `smokeContent`; the refusal calls `logContentSmoke()` and returns before `refreshContentTargets`, mirroring the `find`/`action`/`text` early returns.

## Docs diff (D6)
```diff
--- docs/agent.md (before)	2026-09-25 09:53:30
+++ docs/agent.md	2026-09-25 09:53:30
@@ -38,7 +38,7 @@
 Every tool returns the step record `{cmd, …, state}` as JSON text; `isError` is true when the
 step has `error` or `state.error` is non-empty. `pdf4all_do` refuses `close`.
 In a raw session the same steps are JSON lines: `{"cmd":"new","contents":{"size":"a4"}}`.
-An unknown `cmd` does nothing and reports no error: check that `revision` moved. `open` and `new`
+An unknown `cmd` sets `error` `Unknown command "…"` and changes nothing. `open` and `new`
 discard unsaved edits without asking; save a copy first.
 
 STATE keys worth reading after each step:
@@ -84,8 +84,8 @@
 drew, often a line or part of one). If you do not know the runs, `find` the words first
 (`{"cmd":"find","contents":{"query":"Total"}}` → `search.count`, and the page it lands on), or
 use `find`/`replace`, which matches inside runs. A `target` that matches no run changes nothing and
-sets no `error` (only `announcement` "No text target …"): check `content.edited`. Always send `text`
-(or `find`): a `content` step with neither runs the built-in self-test edit on the page's first run.
+sets no `error` (only `announcement` "No text target …"): check `content.edited`. A `content`
+object with none of `text`, `find` or `action` is refused (`error` "content: give text, find or action").
 Other edits: notes (`{"cmd":"note","contents":"text"}`),
 redaction (`{"cmd":"redact","contents":{"search":"secret"}}` then
 `{"cmd":"redact-apply","contents":{},"to":"/abs/out.pdf"}`), page ops on the shown page
@@ -127,6 +127,7 @@
 ### Refusals you will meet
 
 A refused step changes nothing (revision and dirty stay) and says why:
+- drive: `error` `Unknown command "<cmd>"` (a typo'd `cmd`) or "content: give text, find or action" (a `content` object with none of them).
 - `content.refused[].detail` / `content.replaced.rejected[].detail`: `unsupported-encoding`
   (character not drawable in that font; `missing` lists them), `subset-font`, `overflow` and
   `tail-would-move` (new text too wide for the free space), `ambiguous-text-association` (the same
--- .quad/shared/p3/design-agent-drive.md (before)	2026-09-25 09:53:30
+++ .quad/shared/p3/design-agent-drive.md	2026-09-25 09:53:30
@@ -95,4 +95,5 @@
 - `{"cmd":"close"}` (or stdin EOF, or the end of a file script) finishes as before: `--report=` gets the whole report, exit 0 iff every expect matched. `close` is never a step, so a file script may end with it harmlessly.
 - `{"cmd":"new","contents":{"size":"letter"|"a4"|[w,h]}}` (contents optional, letter default; each side 72–14400 pt) writes a blank one-page PDF as `Untitled.pdf` in a fresh temp folder and opens it (`doc.name "Untitled.pdf"`, revision 0); a bad size is the `unreadable` open-error ("Could not read Untitled.pdf."). File → New… (⌘N) does the same. `text-add` and `fields add` work on it.
 - `scripts/session.mjs` `createSession({out?, gui?, report?})` → `{step(line), close()}` (packDrive applied per line, one step in flight); `node scripts/drive.mjs --session` pipes stdin through it. `scripts/pdf4all-mcp.mjs` is the MCP server (tools `pdf4all_open`, `pdf4all_new`, `pdf4all_do`, `pdf4all_state`, `pdf4all_save`, `pdf4all_help`); agent reference `docs/agent.md`.
+- Drive guard (contract-drive-guard.md): an unknown `cmd` sets `error` `Unknown command "<cmd>"` and changes nothing, and a `content` object with none of `text`/`find`/`action`/`password` is refused (`error` "content: give text, find or action") instead of running the `--edit=content` self-test, which stays on the null-contents and `{password}` smoke path; `error` stays until the next step that clears it (a bare `state` step does not). Script: tests/drive/drive-guard.jsonl.
 Script: tests/drive/agent-session.jsonl (new A4, text-add, field add + fill, save-to, reopen, find, close), run in both modes by tests/agent-session.test.mjs.
```

## Menu-op verification (before D1)
`grep -o "op: '[a-z-]*'" src/main.ts | sort -u` → 41 ops, each checked against the `runEditRequest` chain:
batch, bookmark, cert-sign-open, circle, compare, content, content-edit, encrypt, encrypt-keep, export, extract, freetext, header-footer, help, highlight, ink, insert, labels, line, link, new-from-images, note, pages, redact-apply-open, redact-area, redact-search, redo, security, sign, square, stamp, state, text-add-open, undo, verify-open, watermark — each has its own `op === "…"` branch; delete-page, move-down, move-up, rotate-left, rotate-right — covered by `PAGE_OPS.has(op)`. No menu op falls through to the new `else`.
Other sends: `smokeEditPayload()` (main.ts:197) sends `editArg`, which main.ts:470 restricts to a whitelist that is on the chain; the drive dispatcher (main.ts:1886) forwards any other `cmd` — that is the path the guard is for.

## D3 finding (error lifetime)
- A bare `state` step does NOT clear `error` (runEditRequest returns at once for `state`): step 2 of drive-guard.jsonl pins `error "Unknown command \"nosuch\""` after it. Existing behaviour, unchanged.
- `find` clears it (`driveFind` calls `clearError()` first): step 6 pins `error ""`.
- A successful `content` edit does NOT clear a stale error. Probe (scratch script, not committed: open edit-transformed → `nosuch` → `content {target:"ALPHA",text:"BETA"}`): the last step reads `error "Unknown command \"nosuch\""`, `revision 1`, `content.edited {from:"ALPHA",to:"BETA"}`. `smokeContent`/`applyContentEdit` never call `clearError()`, so this is the same as after any existing refusal (e.g. `content: <parse error>`, "The drive contents must be a JSON object"). Not changed (contract D3). Consequence for MCP: after a refusal, a following successful `content` step still returns `isError true` until some clearing step (e.g. `find`) runs. Flagged for the orchestrator; the script's order (refusals → find → content) matches the contract.

## Deviations
1. drive-guard.jsonl has one extra step beyond the D4 list: a bare `{"cmd":"state"}` after `nosuch`, pinning the D3 finding in the script's expects (as D3 asks). 8 steps total.
2. `tests/drive-guard.test.mjs` test (b) waits (condition poll, no budget) until the runner child `node tests/agent-session.test.mjs` is gone before spawning its MCP session. Reason: the first `npm test` run failed `agent session (b)` with `'92847' !== ''` — its global `pgrep -f -- --drive=-` saw this file's MCP `--drive=-` Electron running in parallel. `node --test tests/*.test.mjs` queues files in name order, so agent-session always starts before drive-guard and the wait cannot miss it; standalone the wait is a no-op. Verified: `node --test tests/agent-session.test.mjs tests/drive-guard.test.mjs` (the two in parallel) → 8/8 ok.
3. Test (b) opens `fixtures/p0b/edit-transformed.pdf` via `pdf4all_open` first, so pre-fix `content {}` would really run the self-test (the red-before shows it came back `isError false`). Never saved.

## Commands and results (all under the run lock)
- Red before (pre-source-change build): `npm run build && node --test tests/drive-guard.test.mjs` → `.quad/shared/p6/acceptance/drive-guard-red-before.log`: `not ok 1` (drive: 7 red steps), `not ok 2` (MCP `nosuch` isError false); `# tests 2 / pass 0 / fail 2`.
- `npx tsc --noEmit -p tsconfig.json` (no lock) → clean.
- After fix: `npm run build && node --test tests/drive-guard.test.mjs` → `# tests 2 / pass 2 / fail 0 / skipped 0`.
- `node scripts/drive.mjs tests/drive/drive-guard.jsonl` ×3 → 8 steps, 0 red each time.
- `node scripts/drive.mjs tests/drive/<x>.jsonl` once each: content-edit 4 steps 0 red; cow-edit 5/0; subset-edit 6/0; object-edit 11/0; replace-page 7/0; text-add-grow 5/0; coded-edit 8/0; agent-session 7/0.
- `node --test tests/p3-smoke2.test.mjs` (D5 c, the `--edit=content` smoke) → `# tests 4 / pass 4 / fail 0 / skipped 0` (ok 1 edit-transformed ALPHA→BETA, BRAVO overflows; ok 2 edit-tagged keeps MCID and link rect; ok 3 edit-tail growth; ok 4 encrypted-noedit read-only).
- `npm test` (once, before deviation 2) → `# tests 593 / pass 590 / fail 2 / skipped 1`. Failures: `agent session (b)` (the cross-file `--drive=-` race, fixed by deviation 2) and `large.pdf: 500-page page ops stay inside the reply budget` (tests/p2-pages.test.mjs, worker-only, `page-move 620.5 ms` > 500 ms under full-suite parallel load; untouched by this unit). `npm test` was not rerun (brief: at most once). Targeted reruns after deviation 2: `node --test tests/agent-session.test.mjs tests/drive-guard.test.mjs` → `# tests 8 / pass 8 / fail 0`; `node --test tests/p2-pages.test.mjs` → `# tests 13 / pass 13 / fail 0` (page-move 29.7 ms). Expected on the acceptance rerun: tests 593, pass 592, fail 0, skipped 1.
- `pgrep -fl -- '--drive='` empty after every run. Fixture hashes: fresh `find fixtures … | shasum -a 256` is byte-identical to `.quad/shared/p6/checks/drive-guard-hash-before.txt` (56 lines).
- D7: no GUI/keystroke run (shot-agent.mjs is fable51's).
