# agent-session — build note (implementer, Opus 5.5)

## What changed
- `src/main.ts` (additive):
  - :8-9 imports `createInterface` (node:readline), `blankPdf`/`pageSizeOf`; `mkdtemp` added to the fs/promises import (:4).
  - :425 `newBlank(contents, size?)`: `pageSizeOf(size)` → bad size sends `open-error {reason:'unreadable', name:'Untitled.pdf'}` (existing token); else writes `blankPdf(w,h)` to `mkdtemp(<temp>/pdf4all-new-)/Untitled.pdf` and calls `deliverOpen` (a dirty GUI doc gets the existing discard box).
  - :1073 File → `New…` `CmdOrCtrl+N`, above New from Images… (⌥⌘N unchanged).
  - :1805 `startDrive`: `live = scriptPath === '-'`; a file script is still read and parsed up front (fail-fast unchanged); :1841 `emit()` writes `STEP <json>\n` to stdout in live mode only; :1844 `for await` over `Iterable<DriveLine>` (file) or a readline over `process.stdin` (made after `fixtureOpened`, so no line is read early); a live line that is not a JSON object → steps gets `{error:"bad line: …"}` + STEP, loop continues; :1860 `close` breaks the loop in both modes (never a step); :1873 `new` step; :1891 a timed-out step is also emitted before `finish`; :1902 `emit(step)` after each `steps.push`. The per-line body (secrets, pre-answers, send, await STATE, expect) is unchanged.
- `src/blank-pdf.ts` (new, pure): `PAGE_SIZES`, `pageSizeOf`, `blankPdf(w,h)` (`%PDF-1.7`, catalog/pages/page/empty content stream, `/Resources << >>`, computed xref). tsconfig already includes `src/**/*.ts` → `build/blank-pdf.js`; no tsconfig change.
- `scripts/session.mjs` (new): `packDrive` (moved byte-for-byte from drive.mjs, now exported), `createSession({out?, gui?, report?})` → `{step, close, out, dir, child, ended}`; spawn args as drive.mjs plus `--drive=-`, stdio `['pipe','pipe','inherit']`; steps queue (one in flight); non-STEP stdout ignored; child exit rejects a pending step.
- `scripts/drive.mjs`: packDrive removed and imported from session.mjs; `--session` branch (:22-40) runs before the script check; the script-file path is untouched.
- `scripts/pdf4all-mcp.mjs` (new, 146 lines, builtins only): initialize/notifications/ping/tools/list/tools/call; -32601 unknown method, -32602 unknown tool, -32700 `id:null` parse error, -32600 non-object; six tools; session started lazily on first step, and started again if the app has exited; closed at stdin EOF, exit 0.
- `docs/agent.md` (new, 155 lines): start, tools, STATE keys, Create/Modify/Fill/Ops (+ refusals, prefer semantic ops, ceilings). `pdf4all_help {topic}` returns the `## <Topic>` section.
- `README.md` "## Driving the app": +3 lines (:41-43). `.quad/shared/p3/design-agent-drive.md`: "## Agent session" (:93, 6 lines).
- `src/renderer/index.html`: a `<h3>New document</h3><ul><li>…</li></ul>` after `#help-compare` (:595). No renderer.ts change.
- `tests/agent-session.test.mjs` (5 tests), `tests/drive/agent-session.jsonl` (8 lines, 7 steps), `.quad/shared/p6/checks/shot-agent.mjs` (written, NOT run).

## Deviations from the contract (and why)
1. **Help "row" → an id-less list after `#help-compare`, not a table row.** The matrix row count is pinned at 11 in tests/p2-shell.test.mjs, tests/p3-drive.test.mjs and help-fields.jsonl lines 4–5, and I may not edit those files (progress §5: "new Help text goes into existing cells, or all three row-count pins change together"). No new id, so the p2-shell id inventory is unaffected.
2. **STATE has no `name`/`pages` scalars.** The real keys are `doc.name`, `doc.pageCount`, `pages.count`, and the script expects those. STATE has no page-size slice, so no new STATE key was added; the A4 MediaBox is checked with mupdf in test (c).
3. **"content listing shows Invoice 42"**: STATE `content.bounds` carries no text, so the script uses `find {query:"Invoice 42"}` → `search.count 1` after the reopen, and test (c) also checks the page text with mupdf.
4. **No `mode` line**: `text-add` has no OP_MODE entry and works in Read mode. `fields` switches to Forms by itself.
5. **`close` is not a step** in either mode, so the report has exactly the steps sent (test (a)), and the batch script reports 7 steps for 8 lines. A bad live line *is* recorded in `steps` as `{error}` (audit trail); it does not affect `ok`.
6. MCP extras beyond the contract: `-32600` for a JSON non-object; `pdf4all_open`/`pdf4all_do` resolve relative `path`/`to`/`from` against the server's cwd (`${OUT}/…` is left alone), where main alone would resolve against the repo root; a missing required string argument is an `isError` result, not a JSON-RPC error; `pdf4all_do` accepts extra top-level keys (`folder`, `images`, `page`/`x`/`y` for packDrive).
7. `createSession` is async (returns a Promise of the session) because it makes a temp dir.
- D4 note: `text-add` and `fields add` work on the blank page with empty `/Resources`; no engine fix was needed. The blank page also carries an empty `/Contents` stream (kept on purpose so the content-append path has a stream).

## Verified
- Red before: `.quad/shared/p6/acceptance/agent-session-red-before.log` (3 not ok + 2 cancelled; old build treated `close` as a step: `8 !== 7`).
- `npx tsc --noEmit -p tsconfig.json` clean.
- `node --test tests/agent-session.test.mjs` (after build, under lock): tests 5 / pass 5 / fail 0.
- `node scripts/drive.mjs tests/drive/agent-session.jsonl` ×3: exit 0, 7 steps, 0 red, ok true each. `tests/drive/coded-edit.jsonl`: exit 0, 8 steps, 0 red (batch mode unchanged).
- `npm test` once: `# tests 590 / # pass 589 / # fail 0 / # skipped 1` (585 baseline + 5). `shasum -c checks/fixtures.sha256`: 0 non-OK lines. `pgrep -fl -- '--drive='` empty.
- Hand: `printf open form.pdf / state | node scripts/drive.mjs --session` → two result lines (`doc.name form.pdf`, fields `name`), then the report (`ok true`, 2 steps), exit 0. `new {size:[10,10]}` → `error "Could not read Untitled.pdf."`, doc stays hello.pdf; bare `new` → Untitled.pdf, 1 page.
- Hand MCP: initialize → `{protocolVersion:"2025-06-18", capabilities:{tools:{}}, serverInfo:{name:"pdf4all", version:"0.0.0"}}`; tools/list → six names; tools/call pdf4all_new → isError false, `state.doc.name "Untitled.pdf"` (result text ≈2.3 kB); stdin EOF → exit 0, no `--drive=` process left.
- Saved-file check (mupdf): created.pdf is A4, widget `customer` = `Ada`, text line `Invoice 42` at bbox y 684 for point [72,700]. So the `point` is the baseline, in y-down page space, and field rects are y-down too (`[72,600,300,624]` → file `/Rect [72 217.89 300 241.89]`). docs/agent.md describes it that way.
- During development, one failed run of test (d) (docs/agent.md did not exist yet) left the test's MCP server + session Electron running. I killed only the MCP node process (pid from my own test); its Electron (tmp `--user-data-dir`, `--drive=-`) then exited on stdin EOF. Test (d) now closes the server in `finally`.

## Known ceilings
- Each `new` leaves its `pdf4all-new-*` temp folder (ponytail comment in main.ts). In the GUI, plain Save on Untitled.pdf writes into that temp folder; the docs and Help say to use Save a copy.
- One session = one document window. A step that gets no STATE in 60 s ends the session (existing rule). If the app exits (e.g. a P0_ERROR), the next MCP call starts a fresh session with hello.pdf open, and the earlier document state is gone.
- STEP lines carry the full STATE; there is no slicing option.
- `shot-agent.mjs` has not been run (GUI / real keystrokes; for fable51 to run while the human is away).
