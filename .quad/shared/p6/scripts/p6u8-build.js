export const meta = {
  name: 'p6u8-build',
  description: 'Opus 5.5 implementer builds P6 U8 (duplicate text editable per run at depth 0: content-edit.ts editMatchRuns depth rule, tests, drive, Help), then an integrator runs the full gate',
  phases: [
    { title: 'Build', detail: 'one implementer, all owned files (lean mode)' },
    { title: 'Integrate', detail: 'locked build, suite, drives x3, hashes' },
  ],
}

const ROOT = '~/Documents/pdf4all'
const SCRATCH = '~/Documents/pdf4all/.quad/scratch/p6u8'
const LOCK = `BUILD/TEST LOCK (shared build/ directory; other agents may run concurrently): every npm run build, npm test, node --test, node scripts/drive.mjs or Electron launch runs inside ONE Bash call that takes the lock with absolute paths and releases it on exit: ( until mkdir ${ROOT}/.quad/run.lock 2>/dev/null; do sleep 5; done; trap 'rmdir ${ROOT}/.quad/run.lock' EXIT; cd ${ROOT} && npm run build && node --test tests/<files> ) — run_in_background: true and wait for the completion notification, or foreground with timeout 600000. Build + the tests that need it go in ONE hold; never hold the lock while editing; never remove a lock you did not create. Typecheck WITHOUT the lock any time: npx tsc --noEmit -p tsconfig.json (noEmitOnError: a type error leaves the old build/ in place, so a drive run after a failed build is not evidence). Electron hygiene after any drive: only processes whose command line carries --drive= or a --user-data-dir under tmp/review-out are stragglers (SIGTERM); NEVER kill any other Electron process (the human's PDF4All app may be open). Never pass --no-sandbox. Never save-in-place a fixtures/ path; fixture hashes must be unchanged when you finish (shasum -a 256 -c ${ROOT}/.quad/shared/p6/checks/fixtures.sha256 → 9 OK; find fixtures -type f \\( -name '*.pdf' -o -name '*.png' \\) | sort | xargs shasum -a 256 | diff ${ROOT}/.quad/shared/p6/checks/u8-hash-before.txt - empty; nothing may be added under fixtures/). Secrets under fixtures/p5/*.p12, *.pem, false-ca/*.key, fixtures/p0b/.ca/ never appear in logs or evidence. Scratch files under ${SCRATCH}/<role>/ only.`

const SCHEMA = {
  type: 'object',
  properties: {
    role: { type: 'string' },
    filesChanged: { type: 'array', items: { type: 'string' } },
    done: { type: 'array', items: { type: 'string' }, description: 'contract § items shipped, one line each with the proof (test name / drive step)' },
    notDone: { type: 'array', items: { type: 'string' } },
    redBeforeFix: { type: 'array', items: { type: 'string' }, description: 'each new test shown failing on the pre-unit build: test name + the assertion message seen' },
    testResults: { type: 'string', description: 'exact command(s) run under the lock and their pass/fail counts' },
    risks: { type: 'array', items: { type: 'string' } },
  },
  required: ['role', 'filesChanged', 'done', 'notDone', 'redBeforeFix', 'testResults', 'risks'],
}

phase('Build')
const build = await agent(`You are the implementer for PDF4All at ${ROOT} (Electron 44 + MuPDF 1.28.1 WASM PDF editor, TypeScript; NO git, so a lost file cannot be recovered: NEVER overwrite a whole existing file — read it, then edit in place with targeted edits; new files may be written whole). You are Claude Opus 5.5. Lean mode: you own every file of this small unit (fable51's call, contract §4 last sentence).
READ FIRST, in this order: .quad/shared/p6/contract-p6-unit8.md — ALL of it (header D1–D2, §0–§6 and §9; §9 holds the folded objections and OVERRIDES §1–§6 wherever they differ); the objections files .quad/shared/p6/objections-p6-unit8-{engine,renderer,tests}.md (their 'checkedFine' and 'buildRisks' sections are for you; the tests file's 'Exact line templates' are normative for tests/drive/direct-edit9.jsonl unless §9 says otherwise); progress.md §5 with sed -n 103,127p (never cat the file); then the code: src/renderer/content-edit.ts editMatchRuns (:1422-1480) and its three call sites (:1269, :2360, :2941), src/renderer/renderer.ts:3950-3960 and the Help <li> list in src/renderer/index.html:569, tests/p6-objects7.test.mjs:450-470, tests/p2-shell.test.mjs:205-225, the engine test harness tests/p6-objects6.test.mjs:1-112 and the helpers in tests/p6-objects7.test.mjs, the drive test template tests/p6-drive8.test.mjs (copy drive()/pick()/near() — never share), tests/drive/direct-edit3.jsonl for step shapes.
FILES YOU OWN (edit nothing else — in particular NOT src/renderer/engine.ts, mutate, scripts/drive.mjs, src/main.ts, any existing drive .jsonl, fixtures/): src/renderer/content-edit.ts, src/renderer/renderer.ts, src/renderer/index.html, tests/p6-text8.test.mjs (new), tests/p6-objects7.test.mjs (the flipped asserts per §9.5), tests/p3-targets.test.mjs (ONLY the four pins named in §9.1), tests/p2-shell.test.mjs (Help pins only), tests/drive/direct-edit9.jsonl (new), tests/p6-drive9.test.mjs (new), .quad/shared/p3/design-agent-drive.md (append one '## P6 U8' section), .quad/shared/p6/checks/shot-direct9.mjs (new, per §9's GUI-proof clause: copy the launch/session()/pagePoint()/click()/shot() machinery of .quad/shared/p6/checks/shot-direct8.mjs:1-200, drop cases A–E, add the U8 case; the chord helper sends a REAL macOS keystroke through osascript addressed to the Electron you spawned by unix id: execFileSync('osascript', ['-e', 'tell application "System Events" to tell (first process whose unix id is ' + child.pid + ') to set frontmost to true', '-e', 'delay 0.4', '-e', 'tell application "System Events" to keystroke return using command down']); type text with Input.insertText or per-character dispatchKeyEvent through CDP; if osascript fails (error -1712 or 1002) log 'OSA-UNAVAILABLE' and exit 2 — never fall back to a CDP chord silently). The engine test file MUST show §6.3 red-before-fix: before you touch content-edit.ts, write tests/p6-text8.test.mjs cases (a), (b), (e) (and any §9 additions), run them under the lock against the CURRENT build/ with 'node --test tests/p6-text8.test.mjs' (no npm run build in that hold) and record each failing assertion under redBeforeFix; then make the engine change and rebuild.
ORDER: 1) the failing tests as above; 2) content-edit.ts change (§1.1–§1.3 as amended); 3) renderer/Help + p2-shell pins; 4) the p6-objects7 flip; 5) drive JSONL + p6-drive9 test + design-agent-drive.md; 6) shot-direct9.mjs; 7) under the lock: npm run build && node --test tests/p6-text8.test.mjs tests/p6-objects7.test.mjs tests/p2-shell.test.mjs tests/p3-targets.test.mjs tests/p3-edit.test.mjs tests/p3-replace.test.mjs tests/p6-direct3.test.mjs tests/p6-paragraph.test.mjs tests/p3b-content.test.mjs && node scripts/drive.mjs tests/drive/direct-edit9.jsonl && node scripts/drive.mjs tests/drive/direct-edit3.jsonl && node scripts/drive.mjs tests/drive/direct-edit8.jsonl && node --test tests/p6-drive9.test.mjs; 8) run shot-direct9.mjs ONCE under the lock (it launches a GUI window and sends a real keystroke; the human is away) with dangerouslyDisableSandbox: true on that Bash call; view its PNG with Read and say what it shows. If an existing test other than the ones the contract names flips because of D1, do NOT weaken it: record it under risks with file:line and the assertion, and leave it red for the integrator's decision — unless §9 already names it as a flip, in which case update it as §9 says. Never run the full npm test more than once. ${LOCK}
DONE MEANS: the contract's behaviour is complete, your targeted tests and drives are green under the lock, and the evidence file .quad/shared/p6/p6-unit8-build.md (≤ 40 lines: paths, numbers, redBeforeFix lines, every re-pin as file:line old → new, what was not done) is written as your LAST step. Return ONLY the structured object.`,
  { label: 'build:all', phase: 'Build', schema: SCHEMA, model: 'opus' })
log(`builder returned; notDone ${build.notDone.length}, risks ${build.risks.length}`)

phase('Integrate')
const integ = await agent(`You are the INTEGRATOR for PDF4All at ${ROOT} (Claude Opus 5.5; no git; never overwrite a whole existing file; edit in place; never pass --no-sandbox; never kill an Electron process that lacks --drive= or a tmp/review-out --user-data-dir).
One implementer just built P6 U8. Its structured report:\n\n${JSON.stringify(build, null, 1)}\n\nRead .quad/shared/p6/contract-p6-unit8.md (all of it; §9 overrides) and .quad/shared/p6/p6-unit8-build.md.
Steps, all under the lock (until mkdir ${ROOT}/.quad/run.lock 2>/dev/null; do sleep 5; done … trap rmdir on EXIT; if a lock directory exists whose mtime is older than 20 minutes — ls -ld — and no --drive= / node --test Electron is running, remove it once):
1. npx tsc --noEmit (no lock); then npm run build; then npm test ONCE (record tests/pass/fail/skip and every failing test name with its assertion message). Expected: 530 + the new tests, 0 fail, 1 skip. Diff src/renderer/engine.ts against the copy in .quad/backup/pre-p6u8-20260924.tgz (tar -xOzf … src/renderer/engine.ts | diff - src/renderer/engine.ts): it must be EMPTY (this unit does not touch engine.ts); also diff scripts/drive.mjs and src/main.ts the same way (empty).
2. node scripts/drive.mjs tests/drive/direct-edit9.jsonl three times, plus direct-edit3.jsonl, direct-edit8.jsonl, direct-edit8-cut.jsonl, content-edit.jsonl, debt1.jsonl, object-edit.jsonl once each; record exit codes and, for any red step, the step index, cmd, expect and the STATE it saw.
3. If red comes from the drive script or tests/p6-drive9.test.mjs being wrong about the contract (a wrong centre, key name or absolute value), fix those files. If red comes from engine or renderer behaviour not matching the contract, DO NOT patch around it in the tests — fix the source only when the fix is ≤ 10 lines and clearly what the contract (as amended by §9) says; otherwise record an open defect with file:line and the exact failing expect. Never edit an expect to match a wrong renderer. Never weaken an existing test.
4. Fixture hashes: shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256 (9 OK) before and after; find fixtures -type f \\( -name '*.pdf' -o -name '*.png' \\) | sort | xargs shasum -a 256 | diff .quad/shared/p6/checks/u8-hash-before.txt - must be empty.
5. Do NOT run shot-direct9.mjs (fable51 runs it in acceptance). pgrep -fl -- '--drive=' must be empty when you finish.
6. Write .quad/shared/p6/p6-unit8-integration.md (≤ 40 lines): suite numbers, the three empty diffs, drive results ×3, hashes, applied fixes, open defects with file:line.
Return ONLY the structured object.`, { label: 'integrate', phase: 'Integrate', model: 'opus', schema: {
  type: 'object',
  properties: {
    suite: { type: 'string', description: 'tests/pass/fail/skip' },
    failingTests: { type: 'array', items: { type: 'string' } },
    untouchedDiffsEmpty: { type: 'boolean' },
    drive: { type: 'string', description: 'exit codes for direct-edit9 x3 and the regression drives' },
    redSteps: { type: 'array', items: { type: 'string' } },
    hashesUnchanged: { type: 'boolean' },
    appliedFixes: { type: 'array', items: { type: 'string' } },
    openDefects: { type: 'array', items: { type: 'string' }, description: 'file:line + failing expect/assertion' },
    electronClean: { type: 'boolean' },
  },
  required: ['suite', 'failingTests', 'untouchedDiffsEmpty', 'drive', 'redSteps', 'hashesUnchanged', 'appliedFixes', 'openDefects', 'electronClean'],
} })
return { build, integ }
