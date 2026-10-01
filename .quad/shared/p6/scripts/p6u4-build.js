export const meta = {
  name: 'p6u4-build',
  description: 'Opus 5.5 implementers build P6 U4: fixture agent first, then engine / renderer / tests in parallel on disjoint files, then an integrator runs everything',
  phases: [
    { title: 'Fixture', detail: 'links.pdf + tabs.pdf generators, provenance, pins' },
    { title: 'Build', detail: 'engine, renderer, tests agents on disjoint files' },
    { title: 'Integrate', detail: 'locked build, suite, drives x3, GUI script' },
  ],
}

const ROOT = '~/Documents/pdf4all'
const SCRATCH = (typeof args === 'object' && args && args.scratch) || '~/Documents/pdf4all/.quad/scratch' // pass args {scratch} or use the repo-local default
const COMMON = `You are an implementer for PDF4All at ${ROOT} (Electron + MuPDF 1.28.1 WASM PDF editor, TypeScript; NO git, so a lost file cannot be recovered: NEVER overwrite a whole existing file — read it, then edit in place with targeted edits; new files may be written whole). You are Claude Opus 5.5.
READ FIRST, in this order: .quad/shared/p6/contract-p6-unit4.md (all of §0, §7, §8, §9 and the sections named for your role below — §9 amendments override earlier sections), .quad/shared/p6/probe-links-u4.md (MuPDF Link API facts P1–P5: setBounds takes RAW /Rect space, /Dest shadows /A, links are not in getAnnotations, page-handle caching), the section of .quad/shared/p6/maps-p6-unit4.md for your surface (file:line anchors into the code), .quad/shared/p6/p6-unit4-fixture.md (fixture shas, y-down centres, page-1 page-space rect), then the code you own. For precedent read contract-p6-unit3.md §9 (the previous unit, kept on disk) and progress.md §5 process rules.
OWNERSHIP IS STRICT: edit only the files listed for your role. If you need a change in another role's file, do not make it — return it under 'requests' with file:line and the exact edit. The API shapes in the contract (§1.1, §1.2, §1.8, §2.1, §2.3, §3.2, §3.4 as amended by §9) are fixed; if one cannot work, return it under 'apiDeviations' with the reason and build the rest.
BUILD/TEST LOCK (shared build/ directory; other agents run concurrently; contract §7): every npm run build, npm test, node --test, node scripts/drive.mjs or Electron launch runs inside ONE Bash call that takes the lock with absolute paths and releases it on exit, e.g.:
( until mkdir ${ROOT}/.quad/run.lock 2>/dev/null; do sleep 5; done; trap 'rmdir ${ROOT}/.quad/run.lock' EXIT; cd ${ROOT} && npm run build && node --test tests/<your files> )
Run it in the background (run_in_background: true) and wait for its completion notification, or in the foreground with timeout 600000. Build + the tests that need it go in ONE hold; never hold the lock while editing; never remove a lock you did not create (fable51 handles stale locks). Typecheck WITHOUT the lock any time: npx tsc --noEmit -p tsconfig.json — tsconfig has noEmitOnError, so keep your own files type-clean at the end of every edit batch; if the build fails in another seat's file, wait and retry, never edit it. Never run the full 'npm test' more than twice; prefer targeted node --test under the lock. Electron hygiene after any drive/GUI run: only processes whose command line carries --drive= or a --user-data-dir under tmp or review-out are stragglers (SIGTERM them); NEVER kill any other Electron process — the human's PDF4All app may be open.
Never save-in-place a fixtures/ path; fixture hashes must be unchanged when you finish (shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256). Scratch files go under ${SCRATCH}/ only.
TIME: you have about 75 minutes. Ship the contract's behaviour completely; if something cannot be finished, leave the code compiling and green, and list it under 'notDone' with why. Write evidence to .quad/shared/p6/p6-unit4-<role>.md (≤ 40 lines: paths, numbers, what was not done) before returning.
Return ONLY the structured object (no prose for a human).`

const SCHEMA = {
  type: 'object',
  properties: {
    role: { type: 'string' },
    filesChanged: { type: 'array', items: { type: 'string' } },
    done: { type: 'array', items: { type: 'string' }, description: 'contract § items shipped, one line each with the proof (test name / drive step)' },
    notDone: { type: 'array', items: { type: 'string' } },
    apiDeviations: { type: 'array', items: { type: 'string' } },
    requests: { type: 'array', items: { type: 'string' }, description: 'edits needed in files you do not own: file:line + exact change' },
    testResults: { type: 'string', description: 'exact command(s) run under the lock and their pass/fail counts' },
    risks: { type: 'array', items: { type: 'string' } },
  },
  required: ['role', 'filesChanged', 'done', 'notDone', 'apiDeviations', 'requests', 'testResults', 'risks'],
}

phase('Fixture')
const fixture = await agent(`${COMMON}
ROLE: FIXTURE (runs alone, before the other three). Sections: §5 (both fixtures) and every §9 amendment that names §5. Files you own: fixtures/p6/make-links.py, fixtures/p6/links.pdf, fixtures/p6/links.provenance.json, fixtures/p6/make-tabs.py, fixtures/p6/tabs.pdf, fixtures/p6/tabs.provenance.json (all new); tests/fixtures.test.mjs (ONLY add two PINNED_SHA256 lines, in place); .quad/shared/p6/checks/fixtures.sha256 (append two lines). Copy fixtures/p6/make-direct3.py's structure (imports, HERE/sys.path, box()/run() helpers, the provenance record shape, the single-%%EOF / no-/Prev asserts) and fixtures/make-nav.py's Link dict shape; use fixtures/minipdf.py. Both generators must be deterministic (run twice, identical sha256). Prove the PDFs through the engine with a scratch node script under ${SCRATCH}/ against ${ROOT}/build/renderer/engine.js (createEngine harness as tests/p2-nav.test.mjs:25-72 does; build/ already exists — if it does not, run npm run build once under the lock): links.pdf page 0 'links' reply = 3 links with ids 0..2 in the stated order and kinds (uri, page → 2, blocked ftp), page 1 = 1 link and record its mupdf page-space rect in the provenance, page 2 = 0; tabs.pdf 'fields' reply per page lists the stated names with the stated rects and the page dicts carry /Tabs /C, /A, /S, /S. Also prove with mupdf directly (node, mupdf from node_modules) that page 0's /Annots has 4 entries (L1, L2, L5, L3) while getLinks() returns 3, and that tabs.pdf page 2's StructTreeRoot DFS reaches s2, s3, s1 in that order via /OBJR. Write .quad/shared/p6/p6-unit4-fixture.md with: both sha256s, every rect in y-up and y-down, the click centres (y-down page pt) for L1..L4 and every field, the page-1 page-space rect, the /Annots orders and the expected fields.order per tabs.pdf page, and the exact scratch commands. Under the lock run: node --test tests/fixtures.test.mjs tests/p1-worker.test.mjs (provenance scan). Hashes of every pre-existing fixture must be unchanged.`,
  { label: 'build:fixture', phase: 'Fixture', schema: SCHEMA, model: 'opus' })
if (!fixture) throw new Error('fixture agent returned nothing')
log(`fixture: ${fixture.filesChanged.length} files, notDone ${fixture.notDone.length}`)

const ROLES = [
  { key: 'engine', prompt: `${COMMON}
ROLE: ENGINE. Sections: §1.1 (listLinks annotId via the /Annots walk), §1.2 (link-set: validation order, failLink tokens, no-op, ONE mutate, setBounds with Rect.transform(rect, Matrix.invert(page.getTransform())), entry.delete('Dest') + setURI, destroy the page before the reply), §1.3 (tests/p6-links.test.mjs), §2.1 (fields reply tabs + structIndex DFS; tests/p6-tabs.test.mjs), and every §9 amendment that names them (read §9 before coding). Files you own: src/renderer/engine.ts, src/renderer/engine-core.ts (OP_PERMISSION gains "link-set": "annotate"; failLink helper), src/renderer/worker.ts (only if the request union needs it), src/renderer/redact.ts (untouched unless needed), tests/p6-links.test.mjs (new), tests/p6-tabs.test.mjs (new), tests/p2-nav.test.mjs (additive cases only; existing refusal regexes must keep matching). Per §9.4/§9.22 you do NOT edit tests/p2-shell.test.mjs; the OP_PERMISSION assert goes in tests/p6-links.test.mjs. Every write in your tests: save → reopen → re-list → check (the probe proved in-memory reads lie while a page handle is open); pin both fixture shas from p6-unit4-fixture.md. Keep every existing link-add/link-delete/links reply shape and error text. Under the lock run: npm run build && node --test tests/p6-links.test.mjs tests/p6-tabs.test.mjs tests/p2-nav.test.mjs tests/p2-shell.test.mjs tests/p2-forms.test.mjs tests/p3-forms.test.mjs tests/fixtures.test.mjs.` },
  { key: 'renderer', prompt: `${COMMON}
ROLE: RENDERER. Sections: §1.4–§1.8 (link targets painted before field targets in Read/Annotate, refreshLinks → linksPage + paintHits, follow in Read, focus/handles/drag/Delete/Enter in Annotate, inline target editor, Draw link + startAreaDrag helper + select#link-kind, panel mirror, STATE 'links[]' / direct.links / direct.active kind:'link' / nav.last / nav.linkDraft), §2.2–§2.3 (exported tabOrder, focusNextField/field-bar/activateField on it, fields.tabs/tabsOrder/order), §3.1–§3.4 (plain-arrow nudge with preventDefault + radio exception, #edit-outlines in #context-edit with localStorage default on, unarmed redact drag on the shared startAreaDrag helper with the 3 px threshold, driveDirectGesture fallback to document.elementFromPoint, driveArm gains link, runEditRequest gains outlines), §4 (Help: edit the existing 'Direct editing' cells and the ceilings <ul> only — helpRows() stays 11), and every §9 amendment that names them. Files you own: src/renderer/renderer.ts, src/renderer/index.html, src/renderer/styles.css, src/preload.cts and src/main.ts (no change expected), tests/p6-direct.test.mjs (additive tabOrder tests through the existing child-process harness — no new import-time browser API: the child has no matchMedia and localStorage returns null). The engine agent is adding, in parallel, annotId to the links reply, the link-set op and tabs/structIndex to the fields reply exactly as the contract states — code against those shapes now; until its build lands, guard with optional chaining so the renderer compiles and existing behaviour is unchanged. Do not break the existing drives: direct-edit.jsonl (75 steps, 19 clicks), direct-edit2.jsonl (50), direct-edit3.jsonl (79), links-bookmarks.jsonl (nav.links deepEqual), field-design.jsonl, help-fields.jsonl (11 help rows); expects are subset matches, so add keys, never rename or reshape; keep every existing id, shortcut and STATE key. Under the lock run: npx tsc --noEmit first, then npm run build && node --test tests/p6-direct.test.mjs tests/p2-shell.test.mjs tests/p3-drive.test.mjs, and once near the end: node scripts/drive.mjs on direct-edit.jsonl, direct-edit2.jsonl, direct-edit3.jsonl and links-bookmarks.jsonl (all must exit 0). You may run node scripts/drive.mjs on a scratch JSONL of your own under the lock to check your features against fixtures/p6/links.pdf and tabs.pdf (write it under ${SCRATCH}/, never under tests/).` },
  { key: 'tests', prompt: `${COMMON}
ROLE: TESTS + DRIVE + GUI CHECK + DOCS. Sections: §6.2, §6.3, §6.4, the drive lines of §1.8, §2.3, §3.1–§3.4, and every §9 amendment that names them (exact-value expects only; absolute revision numbers; geometry via pick() in the test file; an explicit click before every key; only STATE keys the contract defines). Files you own: tests/drive/direct-edit4.jsonl (new), tests/drive/direct-edit5.jsonl (new), tests/p6-drive4.test.mjs (new; copy drive()/pick()/savedBytes/near helpers from tests/p6-drive3.test.mjs — do NOT edit p3-drive.test.mjs or p6-drive3.test.mjs), .quad/shared/p6/checks/shot-direct4.mjs (new; copy .quad/shared/p6/checks/shot-direct3.mjs: root from import.meta.url, unique --user-data-dir under checks/review-out/u4-udata, SIGTERM the child in finally, real CDP Input.dispatchMouseEvent/dispatchKeyEvent/insertText, pagePoint scaled by the shown page's real CSS width, screenshots checks/review-out/u4-*.png), scripts/drive.mjs (only if a new packed key is needed — none is expected), .quad/shared/p3/design-agent-drive.md (append the §9.20 section "## P6 U4 drive additions", prose, no table rows). Write both drive scripts from the contract §6.2 and the fixture centres in p6-unit4-fixture.md; every step that changes the document gets an expect that would fail if the feature were absent; the header comment states the fixture sha prefixes and the y-down rule. The engine and renderer agents are building in parallel, so early runs will be red: near the end of your window (after ~50 minutes) take the lock and run npm run build && node scripts/drive.mjs tests/drive/direct-edit4.jsonl && node scripts/drive.mjs tests/drive/direct-edit5.jsonl; report exactly which steps fail and the STATE they saw, under 'risks', without weakening the scripts to hide gaps (the integrator will rerun). Also run node --test tests/p6-drive4.test.mjs under the lock once. Do not run shot-direct4.mjs unless both drives are fully green (it launches a GUI).` },
]

phase('Build')
log('Engine, renderer and tests agents building in parallel (disjoint files, shared build lock)')
const built = (await parallel(ROLES.map(r => () =>
  agent(r.prompt, { label: `build:${r.key}`, phase: 'Build', schema: SCHEMA, model: 'opus' })
    .then(x => ({ key: r.key, result: x }))))).filter(Boolean)
log(`${built.length}/3 builders returned; notDone: ${built.map(b => `${b.key}=${b.result.notDone.length}`).join(', ')}`)

phase('Integrate')
const summary = [{ key: 'fixture', result: fixture }, ...built].map(b => `### ${b.key}\n${JSON.stringify(b.result, null, 1)}`).join('\n\n')
const integ = await agent(`You are the INTEGRATOR for PDF4All at ${ROOT} (Claude Opus 5.5; no git; never overwrite a whole existing file; edit in place).
Four implementers just built P6 U4 (fixture first, then engine/renderer/tests on disjoint files). Their structured reports:\n\n${summary}\n\nRead .quad/shared/p6/contract-p6-unit4.md (§0, §6, §7, §8 and all of §9) and the evidence files .quad/shared/p6/p6-unit4-{fixture,engine,renderer,tests}.md.
Steps, all under the lock (until mkdir ${ROOT}/.quad/run.lock 2>/dev/null; do sleep 5; done … rmdir ${ROOT}/.quad/run.lock; if a lock older than 20 minutes exists — ls -ld — and no --drive=/node --test Electron is running, remove it once; Electron hygiene per §7: only --drive= / tmp user-data-dir processes are stragglers, never kill any other Electron):
1. Apply every 'requests' item from the reports (edits other roles needed in files they did not own) if it is small and consistent with the contract; list what you applied.
2. npx tsc --noEmit; then npm run build; then npm test (record: tests/pass/fail/skip and every failing test name with its assertion message).
3. node scripts/drive.mjs tests/drive/direct-edit4.jsonl and direct-edit5.jsonl three times each, plus direct-edit.jsonl, direct-edit2.jsonl, direct-edit3.jsonl, links-bookmarks.jsonl, field-design.jsonl and help-fields.jsonl once each; record exit codes and, for any red step, the step index, cmd, expect and the STATE it saw.
4. If red comes from a drive script, tests/p6-drive4.test.mjs or shot-direct4.mjs being wrong about the contract (a wrong centre, a wrong key name, a wrong absolute value), fix those files. If red comes from engine or renderer behaviour not matching the contract, DO NOT patch around it in the tests — fix the engine/renderer file only when the fix is ≤ 10 lines and clearly what the contract says; otherwise record it as an open defect with file:line and the exact failing expect.
5. Fixture hashes: shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256 must pass before and after; also compare every fixtures/**/*.pdf|png hash against .quad/shared/p6/checks/u4-hash-before.txt (only fixtures/p6/links.pdf and fixtures/p6/tabs.pdf may be new).
6. If and only if both new drives are fully green, run node .quad/shared/p6/checks/shot-direct4.mjs once and list the screenshots it wrote. Then pgrep -fl -- '--drive=' must be empty.
7. Write .quad/shared/p6/p6-unit4-integration.md (≤ 50 lines): suite numbers, drive results ×3, hashes, GUI shots, applied fixes, open defects with file:line.
Return ONLY the structured object.`, { label: 'integrate', phase: 'Integrate', model: 'opus', schema: {
  type: 'object',
  properties: {
    suite: { type: 'string', description: 'tests/pass/fail/skip' },
    failingTests: { type: 'array', items: { type: 'string' } },
    drive: { type: 'string', description: 'exit codes for direct-edit4 x3, direct-edit5 x3, and the six regression drives' },
    redSteps: { type: 'array', items: { type: 'string' } },
    hashesUnchanged: { type: 'boolean' },
    guiShots: { type: 'array', items: { type: 'string' } },
    appliedFixes: { type: 'array', items: { type: 'string' } },
    openDefects: { type: 'array', items: { type: 'string' }, description: 'file:line + failing expect/assertion' },
    electronClean: { type: 'boolean' },
  },
  required: ['suite', 'failingTests', 'drive', 'redSteps', 'hashesUnchanged', 'guiShots', 'appliedFixes', 'openDefects', 'electronClean'],
} })
return { fixture, built, integ }
