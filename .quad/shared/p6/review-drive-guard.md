# drive-guard — review (Opus 5.5 reviewer, refute-first)

Verdict: **accept with reviewer fixes** (fixes 1–4 applied and proven; 5–6 fixed under contract amendment §4; 7–8 informational; 9 proposed).

Renderer diff vs the tarball: 11 added lines, 0 removed (builder's 9 + reviewer's 2: comment + `clearError()`).

## Findings

1. **High (test design) — FIXED.** `tests/drive-guard.test.mjs` (b) polled pgrep for the `agent-session.test.mjs` runner before starting (`agentSessionRunning()`, 500 ms poll), because `tests/agent-session.test.mjs` (a)(b)(c)(d)(f) asserted that NO `--drive=-` Electron existed machine-wide. Cross-file coupling, name-order assumption, a sleep used as synchronisation. Root fix:
   - agent-session.test.mjs: `stdinDrives()` → `sessionDrives(session)` = `pgrep -f -- "<session.dir>/udata .*--drive=-"` (the session's unique mkdtemp user-data-dir; matches only that session's main Electron). (b) also asserts the pattern FINDS the live session before `close()`, so the check is not vacuous. (d) records `stdinDrivesOf(serverPid)` = `pgrep -P <serverPid> -f -- --drive=-` after the first tool call (asserts exactly 1), then after exit asserts none of those pids is alive (`process.kill(pid,0)`), since orphans would be reparented and `-P` after exit would see nothing.
   - drive-guard.test.mjs: same pid-scoped check for (b); `agentSessionRunning()` and its wait deleted; no sleeps remain.
   - Evidence: under the lock, `node --test tests/agent-session.test.mjs tests/drive-guard.test.mjs` ×2 → each `# tests 8 / # pass 8 / # fail 0 / # skipped 0`, `pgrep -fl -- '--drive='` empty after. Full `npm test` (files in parallel) → agent session (a)–(f) and drive guard (a)(b) all ok.

2. **Medium (D3, agent-visible) — FIXED.** A successful `content` step did not clear a stale `error`, so over MCP a good edit after any refusal came back `isError true`. Fix in `smokeContent`: `clearError()` right after `JSON.parse` succeeds, BEFORE `smokePassword` (so a wrong-password error set by that call is not wiped; the guard's refusal re-sets the error when it fires). Safety check: `grep -n '"error": \?"[^"]' <every jsonl with a content step>` → only drive-guard.jsonl has non-empty error expects; no .test.mjs sends a content step other than drive-guard.test.mjs and the `--edit=content` smoke (contents null / `{password}`), which re-sets its own errors later. Pinned: drive-guard.jsonl reordered (still 8 steps) so the real edit follows the `{"page":0}` refusal directly and expects `error ""`; test (a) asserts `steps[7].state.error === ''` and the refusal indexes moved to 4 and 6, find to 5. Mutation: without the `clearError()` step 8 goes red.
   - Remaining (not fixed, informational): other ops (e.g. `fields`, `text-add`) may still carry a stale refusal. docs/agent.md now says `error` can outlive its step (bare `state` keeps it; `find` and a `content` object clear it; check `revision`).

3. **Medium (docs regression) — FIXED.** The builder removed the docs warning that a `content` step with neither `text` nor `find` runs the self-test, but that is still true for a `content` step with NO `contents` and for a non-object string. Probe (scratch script, under the lock) on edit-transformed.pdf:
   - `{"cmd":"content"}` → `error ""`, `revision 1`, `dirty true`, `edited {ALPHA→BETA}` (self-test ran, silent).
   - `{"cmd":"content","contents":"{nope"}` → `error "content: Expected property name or '}' in JSON at position 1 (line 1 column 2)"` AND `revision 1`, `dirty true`, `edited {ALPHA→BETA}` (the self-test still ran after the parse error).
   docs/agent.md now adds: "but a `content` step with no `contents` at all (or a string that is not a JSON object) still runs the built-in self-test edit on the page's first run: always send an object."

4. **Low (design doc placement) — FIXED.** The Drive guard bullet was inserted above the existing trailing `Script: tests/drive/agent-session.jsonl …` line, making that line read as belonging to the guard. Swapped so the guard bullet is last; its D3 clause now also says `find` and a well-formed `content` object clear `error`.

5. **Medium — FIXED under amendment §4.1 (see below); original text:** Bare `{"cmd":"content"}` from drive/stdin/MCP still runs the destructive self-test: main.ts:1886 omits `contents` when the line has none, which the renderer cannot tell from the `--edit=content` smoke (main.ts:209, also undefined). Proposed: in the drive dispatcher (main.ts ~1886) send `contents: '{}'` when `cmd === 'content'` and the body is undefined; the D2 guard then refuses it. One line, main.ts only.

6. **Low — FIXED under amendment §4.2 (see below); original text:** A `{`-prefixed string that fails `JSON.parse` sets the error and then falls through to the self-test (evidence in 3). Proposed: `logContentSmoke(); return;` at the end of the `catch` in `smokeContent`. Left because contract D2 explicitly keeps non-JSON behaviour unchanged.

7. **Info.** `fixtures/p0b/.ca/ca.srl` differs from the tarball (serial ...650F → ...6510, mtime 09:54:51 = the builder's `npm test`); it is bumped by the signing tests on every full run (not a pdf/png, so outside the 56-line hash set). Pre-existing side effect, not this unit.

8. **Info (G).** The 500-page flake passed in my single full run: `ok 208 - large.pdf: 500-page page ops stay inside the reply budget`.

## Checks

- A (D1): `grep -o "op: '[a-z-]*'" src/main.ts | sort -u` → 41 ops; `comm -23` against the `op === "…"` branches of `runEditRequest` + PAGE_OPS → empty. Drive cmds from `cat tests/drive/*.jsonl | grep -oE '"cmd": ?"[a-z0-9-]+"'` missing a branch: `close feilds new nosuch open print-to save-in-place save-to wait` = main-side cmds + the two deliberate typos. `--smoke` `editArg` whitelist (main.ts:470: note highlight freetext pages form nav assemble extract export-png/jpeg/txt decorate shapes encrypt encrypt-keep content) all on the chain; runEditRequest has one caller (renderer.ts:1069). Drives under the lock, all exit 0 / red 0: help-fields 21, dialogs 11, refusals 29, direct-edit8-chords 19, bates 9, batch 7 (staged batch-in/batch-c in a scratch --out), content-edit 4, cow-edit 5, subset-edit 6, object-edit 11, replace-page 7, text-add-grow 5, coded-edit 8, agent-session 7; drive-guard ×3 → 8 steps red 0 each.
- B (D2): probe → `{"target":"ALPHA"}`, `{"text":5}`, `{"find":7}` each `error "content: give text, find or action"`, `revision 0`, `dirty false`, `edited null`; `{"password":"x"}` on the unlocked doc → self-test ran (`revision 1`, edited ALPHA→BETA, error ""): the documented `{password}` dev path, still works; `{}` and `{"page":0}` refused in drive-guard.jsonl. Error text exact.
- C (D3): see finding 2.
- D (tests): mutating either guard turns (a) red (step 2 / step 5 expects) and (b) fails (`isError`/`state.error`); (b) cleans up in `finally` (stdin EOF); both `skip: CI_NO_ELECTRON`; no sleeps as sync after fix 1 (drive()'s kill timer is a safety net only).
- E (docs): D6 strings match the code exactly (`Unknown command "${op}"`, "content: give text, find or action"); Refusals line present; design bullet accurate after fix 4.
- F: `shasum -a 256 -c .quad/shared/p6/checks/drive-guard-hash-before.txt` → 56 OK (after the full run too); `find fixtures -type f | wc -l` 200 = tarball 200; `diff -rq` tarball vs tree: only renderer.ts, tests/agent-session.test.mjs (reviewer fix), new drive-guard.test.mjs / drive-guard.jsonl, and ca.srl (finding 7); package.json, main.ts, scripts/, README.md, progress.md identical; `npx tsc --noEmit -p tsconfig.json` clean.
- `node --test tests/p3-smoke2.test.mjs` → 4/4 ok (the `--edit=content` smoke survives the new `clearError()`).
- Full `npm test` (once, after all edits): `# tests 593 / # pass 592 / # fail 0 / # skipped 1`; `pgrep -fl -- '--drive='` empty after.

## Amendment §4 fixes (findings 5 and 6)

```diff
--- src/main.ts (startDrive)
@@ -1885 +1885 @@
-        const body = args.contents ?? (cmd === 'batch' ? '' : undefined);
+        const body = args.contents ?? (cmd === 'batch' ? '' : cmd === 'content' ? '{}' : undefined);
--- src/renderer/renderer.ts (smokeContent catch)
@@ -5122,6 +5122,8 @@
     } catch (err) {
       showError(`content: ${messageOf(err)}`);
+      logContentSmoke();
+      return;
     }
```
- `--edit=content` smoke (main.ts:209, `smokeEditPayload`) is a separate path and still sends contents undefined → self-test (p3-smoke2 4/4).
- Pins: drive-guard.jsonl now 10 steps: `{"cmd":"content"}` → error "content: give text, find or action", revision 0, edited null; `{"cmd":"content","contents":"{nope"}` → revision 0, dirty false, edited null (the error text is V8's, so the jsonl has no error expect; test (a) asserts `/^content: /` and that it is not the guard's text). Both placed before `find`, so `search.count 1` still proves ALPHA intact. Test (a): count 10, EMPTY_CONTENT at steps[4,5,8], parse refusal steps[6], find steps[7], final edit steps[9] error "". MCP test (b): `pdf4all_do {cmd:"content"}` → isError true, state.error EMPTY_CONTENT, revision 0.
- Docs: agent.md "always send an object" warning dropped; now: no-contents is refused like an empty object, a non-JSON string is refused with "content: <parse error>", and the `{"password":…}`-only object is named as the smoke's dev path that runs the self-test. Refusals line mentions "or no `contents`". Design doc bullet updated.
- Verification (one locked call): `npx tsc --noEmit` → TSC-OK; build ok; `node --test tests/agent-session.test.mjs tests/drive-guard.test.mjs` → `# tests 8 / pass 8 / fail 0 / skipped 0`; `node --test tests/p3-smoke2.test.mjs` → `# tests 4 / pass 4 / fail 0`; drive-guard.jsonl ×3 → steps 10 red 0 each; content-edit 4/0, cow-edit 5/0, subset-edit 6/0, object-edit 11/0, replace-page 7/0, text-add-grow 5/0, coded-edit 8/0, agent-session 7/0; `pgrep -fl -- '--drive='` empty; fixture hash check 56 OK. Full `npm test` not rerun (gate does it).

9. **Low (proposed) — NOT FIXED.** A `content` string that does not start with `{` (e.g. `"contents":"hello"`) skips the JSON branch and still runs the self-test. Proposed: in `smokeContent`, treat any non-null `contents` that does not start with `{` like the parse-error case (`showError("content: give text, find or action"); logContentSmoke(); return;`). Not in amendment §4, so left.

**Finding 9 — FIXED by fable51 (contract §4.3).** `smokeContent` refuses any non-null `contents` that does not start with `{` before the JSON branch (same error text); pin: 11th step `{"cmd":"content","contents":"hello"}` after the real edit → error "content: give text, find or action", revision 1, edited null; test (a) expects 11 steps. Verified under the lock: `node --test tests/drive-guard.test.mjs` 2/2, `tests/agent-session.test.mjs` 6/6, `p3-smoke2` 4/4, content-edit 4/0 and cow-edit 5/0, drive-guard report ok with 11 steps, no stragglers.
