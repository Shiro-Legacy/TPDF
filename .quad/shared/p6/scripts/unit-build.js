export const meta = {
  name: 'unit-build',
  description: 'Generic lean unit build: one Opus 5.5 implementer owning every file of a small PDF4All unit, then an Opus 5.5 integrator running the full gate (parameterised by args)',
  phases: [
    { title: 'Build', detail: 'one implementer, all owned files (lean mode)' },
    { title: 'Integrate', detail: 'locked build, suite, drives x3, hashes' },
  ],
}
const A = args
const ROOT = '~/Documents/pdf4all'
const SCRATCH = `${ROOT}/.quad/scratch/${A.unit}`
const LOCK = `BUILD/TEST LOCK (shared build/ directory; other agents may run concurrently): every npm run build, npm test, node --test, node scripts/drive.mjs or Electron launch runs inside ONE Bash call that takes the lock with absolute paths and releases it on exit: ( until mkdir ${ROOT}/.quad/run.lock 2>/dev/null; do sleep 5; done; trap 'rmdir ${ROOT}/.quad/run.lock' EXIT; cd ${ROOT} && npm run build && node --test tests/<files> ) — run_in_background: true and wait for the completion notification, or foreground with timeout 600000. Build + the tests that need it go in ONE hold; never hold the lock while editing; never remove a lock you did not create (if one exists, another agent is building: wait for it). Typecheck WITHOUT the lock any time: npx tsc --noEmit -p tsconfig.json (noEmitOnError: a type error leaves the old build/ in place, so a drive run after a failed build is not evidence). Electron hygiene after any drive: only processes whose command line carries --drive= or a --user-data-dir under tmp/review-out are stragglers (SIGTERM); NEVER kill any other Electron process (the human's PDF4All app may be open). Never pass --no-sandbox. Never save-in-place a fixtures/ path; fixture hashes must be unchanged when you finish (shasum -a 256 -c ${ROOT}/.quad/shared/p6/checks/fixtures.sha256 → 9 OK; find fixtures -type f \\( -name '*.pdf' -o -name '*.png' \\) | sort | xargs shasum -a 256 | diff ${ROOT}/.quad/shared/p6/checks/${A.hashBefore} - empty; nothing may be added under fixtures/). Secrets under fixtures/p5/*.p12, *.pem, false-ca/*.key, fixtures/p0b/.ca/ never appear in logs or evidence. Scratch files under ${SCRATCH}/<role>/ only.`
const SCHEMA = { type: 'object', properties: {
  role: { type: 'string' }, filesChanged: { type: 'array', items: { type: 'string' } },
  done: { type: 'array', items: { type: 'string' }, description: 'contract § items shipped, one line each with the proof (test name / drive step)' },
  notDone: { type: 'array', items: { type: 'string' } },
  redBeforeFix: { type: 'array', items: { type: 'string' }, description: 'each new test shown failing on the pre-unit build: test name + the assertion message seen' },
  testResults: { type: 'string', description: 'exact command(s) run under the lock and their pass/fail counts' },
  risks: { type: 'array', items: { type: 'string' } } },
  required: ['role', 'filesChanged', 'done', 'notDone', 'redBeforeFix', 'testResults', 'risks'] }

phase('Build')
const build = await agent(`You are the implementer for PDF4All at ${ROOT} (Electron 44 + MuPDF 1.28.1 WASM PDF editor, TypeScript; NO git, so a lost file cannot be recovered: NEVER overwrite a whole existing file — read it, then edit in place with targeted edits; new files may be written whole). You are Claude Opus 5.5. Lean mode: you own every file of this unit (fable51's call).
UNIT: ${A.title}.
READ FIRST, in this order: .quad/shared/p6/${A.contract} — ALL of it (the header decisions, every §, and §9 "Folded objections" which OVERRIDES the body wherever they differ); the objections files .quad/shared/p6/objections-${A.unit}-{engine,renderer,tests}.md (their checkedFine and buildRisks sections are for you; the tests file's 'Exact line templates' are normative for new drive scripts unless §9 says otherwise); the code map .quad/shared/p6/${A.map} if it exists; progress.md §5 with sed -n 103,127p (never cat the file); then the code the contract cites. ${A.readFirst || ''}
HOUSE RULES for drive scripts and tests: a new drive is a NEW .jsonl with its own test file (copy the helpers drive()/pick()/near()/red() from tests/p6-drive8.test.mjs — never import them); never insert lines into an existing drive script (a value edit at a named line is allowed only where the contract names it); expects are exact subset matches with absolute revisions; header line "# N steps, M click lines" counted from the real file; Electron drive tests use { skip: CI_NO_ELECTRON, timeout: 300_000 }. Every new op gets an OP_PERMISSION row (engine-core.ts) and the coverage test must pass. STATE existing key shapes never change; new keys are fine. Refusals never dirty. Every gesture a human can make ships with its drive path and STATE slice.
RED BEFORE FIX (§6.3): before you touch source, write the new engine/renderer test cases the contract marks as red-before-fix, run them under the lock against the CURRENT build/ (no npm run build in that hold) into ${ROOT}/.quad/shared/p6/acceptance/${A.unit}-red-before.log and record each failing assertion under redBeforeFix; then implement and rebuild.
FILES YOU OWN (edit nothing else): ${A.owned}
UNTOUCHED (never edit): ${A.untouched}
GUI PROOF SCRIPT ${A.gui}: copy the launch/session()/Cdp/pagePoint()/click()/shot() machinery of .quad/shared/p6/checks/shot-direct9.mjs (or shot-direct8.mjs:1-200 if direct9 is missing); every osascript call addresses the Electron YOU spawned by unix id ('tell application "System Events" to tell (first process whose unix id is ' + child.pid + ') …') and checks frontmost in the SAME call before any keystroke; on "not frontmost" send nothing, log 'blocked: frontmost <name>' and exit 1, never retry into another app; on osascript error -1712/1002 log 'OSA-UNAVAILABLE' and exit 2; SIGTERM the child in finally; check fixtures.sha256 (9 OK) at the end; write PNGs under .quad/shared/p6/checks/review-out/. Do NOT run it yourself (fable51 runs it in acceptance; it sends real keystrokes while the human is away) — but make sure it parses (node --check).
ORDER: ${A.order}
Then under the lock: npm run build && ${A.targeted}; then the drives: ${A.drives.map(d => `node scripts/drive.mjs tests/drive/${d}.jsonl`).join(' && ')}. If an existing test the contract does not name flips, do NOT weaken it: record it under risks with file:line and the assertion and leave it red for the integrator — unless §9 names it as a flip. Never run the full npm test more than once. ${LOCK}
DONE MEANS: the contract's behaviour is complete, your targeted tests and drives are green under the lock, and the evidence file .quad/shared/p6/${A.unit}-build.md (≤ 40 lines: paths, numbers, redBeforeFix lines, every re-pin as file:line old → new, what was not done) is written as your LAST step. Return ONLY the structured object.`,
  { label: 'build:all', phase: 'Build', schema: SCHEMA, model: 'opus' })
log(`builder returned; notDone ${build.notDone.length}, risks ${build.risks.length}`)

phase('Integrate')
const integ = await agent(`You are the INTEGRATOR for PDF4All at ${ROOT} (Claude Opus 5.5; no git; never overwrite a whole existing file; edit in place; never pass --no-sandbox; never kill an Electron process that lacks --drive= or a tmp/review-out --user-data-dir).
One implementer just built ${A.title}. Its structured report:\n\n${JSON.stringify(build, null, 1)}\n\nRead .quad/shared/p6/${A.contract} (all of it; §9 overrides) and .quad/shared/p6/${A.unit}-build.md.
Steps, all under the lock (until mkdir ${ROOT}/.quad/run.lock 2>/dev/null; do sleep 5; done … trap rmdir on EXIT; if a lock directory exists whose mtime is older than 20 minutes — ls -ld — and no --drive= / node --test Electron is running, remove it once):
1. npx tsc --noEmit (no lock); then npm run build; then npm test ONCE (record tests/pass/fail/skip and every failing test name with its assertion message). Expected: ${A.baseline} + the new tests pass, 0 fail, 1 skip. For each UNTOUCHED source file (${A.untouchedFiles}) diff against the copy in .quad/backup/${A.backup} (tar -xOzf … <path> | diff - <path>): each must be EMPTY.
2. ${A.drives.map(d => `node scripts/drive.mjs tests/drive/${d}.jsonl`).join(', ')} — the unit's new drive(s) THREE times each, the regression drives once; record exit codes and, for any red step, the step index, cmd, expect and the STATE it saw.
3. If red comes from a new drive script or test being wrong about the contract (a wrong centre, key name or absolute value), fix those files. If red comes from engine or renderer behaviour not matching the contract, DO NOT patch around it in the tests — fix the source only when the fix is ≤ 10 lines and clearly what the contract (as amended by §9) says; otherwise record an open defect with file:line and the exact failing expect. Never edit an expect to match a wrong renderer. Never weaken an existing test.
4. Fixture hashes: shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256 (9 OK) before and after; find fixtures -type f \\( -name '*.pdf' -o -name '*.png' \\) | sort | xargs shasum -a 256 | diff .quad/shared/p6/checks/${A.hashBefore} - must be empty; nothing new under fixtures/.
5. Do NOT run ${A.gui}. pgrep -fl -- '--drive=' must be empty when you finish.
6. Write .quad/shared/p6/${A.unit}-integration.md (≤ 40 lines): suite numbers, the empty diffs, drive results ×3, hashes, applied fixes, open defects with file:line.
Return ONLY the structured object.`, { label: 'integrate', phase: 'Integrate', model: 'opus', schema: { type: 'object', properties: {
  suite: { type: 'string', description: 'tests/pass/fail/skip' }, failingTests: { type: 'array', items: { type: 'string' } },
  untouchedDiffsEmpty: { type: 'boolean' }, drive: { type: 'string' }, redSteps: { type: 'array', items: { type: 'string' } },
  hashesUnchanged: { type: 'boolean' }, appliedFixes: { type: 'array', items: { type: 'string' } },
  openDefects: { type: 'array', items: { type: 'string' } }, electronClean: { type: 'boolean' } },
  required: ['suite', 'failingTests', 'untouchedDiffsEmpty', 'drive', 'redSteps', 'hashesUnchanged', 'appliedFixes', 'openDefects', 'electronClean'] } })
return { build, integ }
