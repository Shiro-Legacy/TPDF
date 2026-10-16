# App acceptance evidence: op-to-test map and full suites (October 16, 2026)

This folder holds the evidence for completion plan M0 ("map legacy test outcomes to every ledger row") and M9 ("run every app unit and drive test with the Rust backend"). For each of the 67 `SessionRequest` ops it lists:

- the unit tests that dispatch the op, and their outcomes on the Rust engine and on MuPDF;
- the drive scripts whose app run sends the op to the worker, and their outcomes on the Rust build and on the MuPDF build;
- the full suite (`node --test tests/*.test.mjs`, Electron included) on both builds, per file and per test.

**Result in one line:** no test fails on either engine or either build. 674 tests ran in each run.

| Run | Pass | Fail | Skip |
|---|---|---|---|
| Rust unit run | 560 | 0 | 114 |
| MuPDF unit run | 561 | 0 | 113 |
| Full suite, Rust build | 666 | 0 | 8 |
| Full suite, MuPDF build | 661 | 0 | 13 |

The 114 Rust skips are 103 Electron tests skipped under `CI_NO_ELECTRON`, plus the 11 tests that `RUST_SKIPS` lists. All 50 drive scripts pass on both builds.

## Files

- `op-evidence.json`: the per-op map. It holds:
  - `table`: one row per op with its counts and flags;
  - `operations[op]`:
    - `unit.rust.tests` and `unit.mupdf.tests`: each test with its outcome and its calls, split into ok, refused and threw replies;
    - `unit.viaEditBatchItems`: the op as an `edit-batch` item;
    - `drive.rust.scripts` and `drive.mupdf.scripts`: each script with its outcome and its requests sent, ok, refused and other, with the refusal reasons;
    - `flags`;
  - `tests`: every unit test, with both engines' outcomes and the ops each dispatched. It also lists ops a test sent through a stand-in bridge (`fakeBridgeOps`, not counted as evidence) and MuPDF oracle calls made during the Rust run;
  - `driveScripts`: every script, with the tests that run it and their suite outcomes, each Electron run's exit code and failing steps, and the ops;
  - `parity`: a comparison of the two engines, described under [Engine parity](#engine-parity);
  - `runs`: each run's exact command, commit, build, machine and timing. It also holds the drive runs that were not a `tests/drive` script, plus the reruns;
  - `flags`;
  - `opsOutsideContract`: `nope`, `shred` and `(no op)`. Unit tests send these on purpose to check refusals.
- `suite-rust.json` and `suite-mupdf.json`: the full suite on each build. Each holds the command, totals, per-file counts and every test's outcome, with reason, duration and, for a failure, the error.
- `docs/rust-engine-operation-ledger.csv`: `op-evidence.mjs report --ledger` rewrote its two stale M0 columns.
  - `candidate_legacy_tests` now lists the drive scripts, then the unit test files, that dispatched the op in these runs.
  - `remaining_verification` gave the counts and flags below, then "M10 final acceptance". The acceptance results replaced it later the same day; see [Acceptance review results](#acceptance-review-results-2026-10-16).
  - All other columns were left unchanged byte for byte in that step, and `python3 engine/tools/operation-ledger.py` passes.
  - That checker also rewrites `engine/reference/m1-20261012/operation-evidence.json` as a side effect. That file was already stale before this change: it is out of date against `engine.ts`. It is left as committed.

## Provenance

- **Measured commit:** `e747ef6` (branch `app-p7-evidence`), with a clean tree. Every `meta.dirty` list is empty.
  - The app and engine code are `d352fcd`, the integration of the Rust app engine. `e747ef6` adds only the instrumentation and the tool.
  - The commit that adds this folder changes only the report step: the parity section and the reruns in `runs`.
- **WASM package:** `engine/web/pkg` was built from this tree by `WASM_BINDGEN_BIN=$HOME/.local/tpdf-wasm-tools/bin/wasm-bindgen CARGO_BUILD_JOBS=2 sh engine/tools/build-workflow.sh`.
  - Toolchain: rustc 1.98.1 and wasm-bindgen 0.2.129.
  - `tandem_rust_workflow_bg.wasm` sha256: `6bfdbe39f96524e846410f96bff8f3d1d168762fa6210ecf6593c6297d92dffe`.
  - `scripts/build.mjs` checks the package's `sources.sha256` against `engine/` on every Rust build, and that check passed.
- **Machine:** MacBookAir10,1 (Apple M1, 8 cores, 8 GB), macOS 26.6.2 (Darwin 25.6.0), Node v22.23.1, Electron 44.4.0.
- **Memory cap:** every phase ran under `/Users/owner/Documents/tpdf-wt/tools/memcap 3000`.

| Phase | Peak memory |
|---|---|
| Rust suite | 2046 MB |
| Rust unit | 1104 MB |
| Rust drive | 2393 MB |
| MuPDF suite | 1948 MB |
| MuPDF unit | 1027 MB |
| MuPDF drive | 1931 MB |

## Exact commands

From the repository root, in this order. `RAW` was `/Users/owner/Documents/tpdf-wt/scratch/p7-evidence/raw-final`; raw logs are not committed.

```sh
CAP="/Users/owner/Documents/tpdf-wt/tools/memcap 3000"
$CAP node engine/tools/op-evidence.mjs suite --engine=rust  --raw=$RAW    # npm run build, then the full suite
$CAP node engine/tools/op-evidence.mjs unit  --engine=rust  --raw=$RAW --concurrency=3
$CAP node engine/tools/op-evidence.mjs drive --engine=rust  --raw=$RAW
$CAP node engine/tools/op-evidence.mjs suite --engine=mupdf --raw=$RAW    # npm run build:mupdf, then the full suite
$CAP node engine/tools/op-evidence.mjs unit  --engine=mupdf --raw=$RAW --concurrency=3
$CAP node engine/tools/op-evidence.mjs drive --engine=mupdf --raw=$RAW
$CAP npm run build
$CAP node engine/tools/op-evidence.mjs rerun --engine=rust --mode=unit  --raw=$RAW --file=tests/p3-edit.test.mjs '--name=U3 review: /W expansion is bounded to 2-byte codes and to the CMap code ceiling' --times=3
$CAP node engine/tools/op-evidence.mjs rerun --engine=rust --mode=suite --raw=$RAW --file=tests/p3-edit.test.mjs '--name=U3 review: /W expansion is bounded to 2-byte codes and to the CMap code ceiling' --times=3
node engine/tools/op-evidence.mjs report --raw=$RAW --out=engine/reference/app-acceptance-20261016 --ledger
```

Each phase writes its full node command into `runs.*.command` and `suite-*.json` `meta.command`.

### What each phase runs

**`suite`** runs the npm build, then:

```sh
node --test --test-reporter=spec --test-reporter-destination=stdout \
  --test-reporter=./engine/tools/op-evidence-reporter.mjs --test-reporter-destination=<file> \
  --test-concurrency=2 tests/*.test.mjs
```

This is `npm test` (Rust build) or `npm run test:mupdf` (MuPDF build), plus a JSON reporter beside the spec one.

**`unit --engine=rust`** runs `tests/run-rust.mjs`'s own arguments. `run-rust.mjs` now exports them as `rustTestArgs()`; run as a script it behaves as before. The run uses:
- `--import ./tests/engine-select.mjs`;
- the 11 `RUST_SKIPS` `--test-skip-pattern`s;
- `TPDF_ENGINE=rust CI_NO_ELECTRON=1`;
- `TPDF_OP_LOG`.

**`unit --engine=mupdf`** is the default suite with `CI_NO_ELECTRON=1`, plus `--import ./tests/engine-select.mjs` and `TPDF_OP_LOG`. With `TPDF_ENGINE` unset, that import only adds the op log.

**`drive`** runs the 22 test files that name a `tests/drive` script, as the suite runs them, with `NODE_OPTIONS=--require=engine/tools/op-log-electron.cjs` and `TPDF_OP_LOG`.

### Concurrency

npm's default is `os.availableParallelism() - 1`, which is 7 files at once on this Mac. That does not fit under the 3000 MB cap: a Rust-build suite at `--test-concurrency=7` was killed at 3050 MB before its first result. The recorded suites therefore ran at 2.

A supplementary Rust-build suite at `--test-concurrency=4` also passed: 666 pass, 0 fail, 8 skip, at 2961 MB. It ran on the same app and engine code, before the tooling commit, and is not recorded here. Concurrency changes only how many test files run at once. Each file still runs in its own process, and tests within a file run in order.

## How the attribution works, and why it is exact

### Unit tests (`tests/op-log*.mjs`, loaded only when `TPDF_OP_LOG` is set)

`engine-select.mjs` preloads `op-log.mjs` and redirects a test file's own imports:

| The test file imports | It gets |
|---|---|
| `build/renderer/engine.js` | `op-log-engine.mjs`. This returns what the test would otherwise get: `rust-engine.mjs` under `TPDF_ENGINE=rust`, else `engine.ts`. Each created engine's `dispatch` is wrapped. |
| `engine.js?real` | The same wrapper, always over MuPDF. |
| `build/renderer/engine-rust.js` | `op-log-rust.mjs` |

The wrapper passes the request and reply through untouched. It counts:
- each top-level dispatch: op, ok/refused/threw, and the refusal reason;
- the item ops of an `edit-batch`.

Internal dispatches are not counted. These are engine.ts's own sub-dispatch of batch items, and `rust-engine.mjs`'s test-bridge `layers` call behind `session.doc`.

The test a dispatch belongs to comes from node:test itself. Every test and hook is an `AsyncResource` of type `Test` whose body runs in its own async scope. An `async_hooks` init hook copies that owner onto every resource created inside it. A dispatch therefore names its test from inside the body, after any `await`, and inside the test's own `t.after()` hooks.

This is exact, not inferred from timing. Two runs confirm it: no dispatch was left unattributed in either run (`unattributedDispatches: 0`), and node:test registration is unchanged, so the reporter's file, line and name are the test's own.

The engine is recorded per dispatch:

| Engine label | What it is | Counted for |
|---|---|---|
| `rust` | Through `rust-engine.mjs` | Rust |
| `rust-direct` | The test built the adapter over `engine/web/pkg` or a subclass of its `RustApp` | Rust |
| `rust-fake` | A stand-in bridge that runs no engine | Neither (listed separately) |
| `mupdf` | engine.ts | MuPDF; in the Rust run, oracle calls only |

The 11 `RUST_SKIPS` tests never start on Rust, and node:test does not report pattern-skipped tests at all. The report adds them to the Rust run as `skip` with the `RUST_SKIPS` reason. Their ops come from the MuPDF run and are marked `opsFrom: "mupdf run"`.

### Drive scripts (`engine/tools/op-log-electron.cjs`)

The hook loads into Electron's main process through `NODE_OPTIONS=--require`. It changes no app source.

1. It registers itself as an extra session preload. Before any page script runs, the preload wraps `Worker.prototype.postMessage` in the page's main world (`contextBridge.executeInMainWorld`).
2. The wrapper reports every request carrying an `op`, and its reply, as `console.debug` lines. The main process appends them to the log.
3. It records the drive script's sha256. `scripts/drive.mjs` hands Electron a packed copy of the script, and the report matches that hash against `packDrive()` of each `tests/drive` script.
4. It patches `app.exit` to record the exit code and the failing steps from the drive report. main.ts exits 0 only if every expect matched.

The hook matched all 50 scripts on both builds, each run once.

Ten other Electron runs belong to tests that drive the app some other way:
- 7 stdin sessions (`--drive=-`): agent-session, the MCP server and drive-guard;
- 3 scripts the tests write themselves: tmp-cleanup `open.jsonl`, p6-batch `symlink.jsonl`, and worker-build's trapping app copy.

Two of these ten exit 1 on both engines, as their tests expect; the tests pass. They are listed under `runs.drive*.otherRuns` and not counted as scripts.

The script-to-test links (`driveScripts[].tests`) are static: a test that names the script, directly or through a file constant. All 57 links resolve to a test in both suite runs.

## What the numbers mean

**Rust unit pass/total.** The unit tests that dispatched the op at least once on Rust (`rust` or `rust-direct`), counting any reply. Total also includes the `RUST_SKIPS` tests that dispatch it on MuPDF; their number is in the "skipped on Rust" column.

**MuPDF unit pass/total.** The same, from the MuPDF unit run (`mupdf` dispatches).

The two totals differ for two reasons:
- tests that run only on one engine, for example the `rust-engine-*` and `rust-cjk-font` tests;
- tests that take another branch per engine. For example, `text-all` is dispatched by 11 tests on Rust and 5 on MuPDF.

**Drives pass/total.** The scripts whose app run sent the op to the worker. A script passes when every run of it exited 0, that is, when every expect matched.

**`withOkReply`** (in `operations`). How many of those tests or scripts got at least one ok reply. Every op has an ok reply in a passing Rust unit test, or, for `page-ids`, in the Rust drives.

**On the Rust build, the full suite runs Rust only in its Electron tests.** The worker in the app is Rust. The non-Electron unit tests in `npm test` still import engine.ts, because the default suite does not use `engine-select.mjs`. So:
- the Rust engine's unit-level evidence is the Rust unit run;
- the app-level evidence is the drive run and the Electron tests of `suite-rust.json`.

## Per-op table

| op | Rust unit pass/total | skipped on Rust | MuPDF unit pass/total | Rust drives pass/total | MuPDF drives pass/total | flags |
|---|---|---|---|---|---|---|
| `open` | 428/438 | 10 | 429/429 | 50/50 | 50/50 |  |
| `render` | 22/22 |  | 15/15 | 50/50 | 50/50 |  |
| `thumbnail` | 5/5 |  | 5/5 | 30/30 | 30/30 |  |
| `search` | 11/11 |  | 11/11 | 8/8 | 8/8 |  |
| `authenticate` | 16/16 |  | 16/16 | 1/1 | 1/1 |  |
| `close` | 10/11 | 1 | 7/7 | 0/0 | 0/0 | no drive coverage |
| `save` | 126/131 | 5 | 131/131 | 32/32 | 32/32 |  |
| `validate` | 23/25 | 2 | 25/25 | 32/32 | 32/32 |  |
| `security-set` | 11/11 |  | 11/11 | 0/0 | 0/0 | no drive coverage |
| `annots` | 50/56 | 6 | 56/56 | 50/50 | 50/50 |  |
| `annot-add` | 43/49 | 6 | 49/49 | 8/8 | 8/8 |  |
| `annot-set` | 5/5 |  | 5/5 | 2/2 | 2/2 |  |
| `annot-move` | 6/6 |  | 6/6 | 3/3 | 3/3 |  |
| `annot-delete` | 8/8 |  | 8/8 | 5/5 | 5/5 |  |
| `redact-add` | 15/15 |  | 15/15 | 5/5 | 5/5 |  |
| `redact-set` | 4/4 |  | 4/4 | 2/2 | 2/2 |  |
| `redact-search` | 4/4 |  | 4/4 | 2/2 | 2/2 |  |
| `redact-preview` | 6/6 |  | 6/6 | 50/50 | 50/50 |  |
| `redact-apply` | 7/7 |  | 7/7 | 2/2 | 2/2 |  |
| `sign-prepare` | 20/20 |  | 20/20 | 3/3 | 3/3 |  |
| `sign-list` | 4/4 |  | 4/4 | 2/2 | 2/2 |  |
| `page-rotate` | 13/13 |  | 13/13 | 4/4 | 4/4 |  |
| `page-delete` | 13/13 |  | 13/13 | 1/1 | 1/1 |  |
| `page-move` | 6/6 |  | 6/6 | 1/1 | 1/1 |  |
| `fields` | 59/59 |  | 58/58 | 50/50 | 50/50 |  |
| `field-set` | 24/24 |  | 23/23 | 8/8 | 8/8 |  |
| `field-add` | 31/31 |  | 31/31 | 8/8 | 8/8 |  |
| `field-props` | 20/20 |  | 20/20 | 6/6 | 6/6 |  |
| `field-delete` | 7/7 |  | 7/7 | 2/2 | 2/2 |  |
| `outline` | 9/9 |  | 9/9 | 50/50 | 50/50 |  |
| `outline-add` | 5/5 |  | 5/5 | 1/1 | 1/1 |  |
| `outline-set` | 3/3 |  | 3/3 | 1/1 | 1/1 |  |
| `outline-delete` | 3/3 |  | 3/3 | 1/1 | 1/1 |  |
| `links` | 27/27 |  | 27/27 | 50/50 | 50/50 |  |
| `link-add` | 9/9 |  | 9/9 | 3/3 | 3/3 |  |
| `link-delete` | 5/5 |  | 5/5 | 4/4 | 4/4 |  |
| `link-set` | 7/7 |  | 7/7 | 2/2 | 2/2 |  |
| `insert-pages` | 5/5 |  | 5/5 | 2/2 | 2/2 |  |
| `extract-pages` | 5/5 |  | 5/5 | 0/0 | 0/0 | no drive coverage |
| `pdf-from-images` | 5/5 |  | 5/5 | 0/0 | 0/0 | no drive coverage |
| `text-all` | 11/11 |  | 5/5 | 0/0 | 0/0 | no drive coverage |
| `compare-open` | 9/9 |  | 9/9 | 1/1 | 1/1 |  |
| `compare-close` | 2/2 |  | 2/2 | 1/1 | 1/1 |  |
| `compare-render` | 6/6 |  | 6/6 | 1/1 | 1/1 |  |
| `compare-text` | 8/8 |  | 8/8 | 1/1 | 1/1 |  |
| `stamp` | 14/14 |  | 14/14 | 4/4 | 4/4 |  |
| `batch-file` | 5/6 | 1 | 6/6 | 1/1 | 1/1 |  |
| `labels` | 2/2 |  | 2/2 | 50/50 | 50/50 |  |
| `labels-set` | 5/5 |  | 5/5 | 2/2 | 2/2 |  |
| `labels-clear` | 2/2 |  | 2/2 | 1/1 | 1/1 |  |
| `layers` | 2/2 |  | 2/2 | 50/50 | 50/50 |  |
| `layer-set` | 2/2 |  | 2/2 | 0/0 | 0/0 | no drive coverage |
| `edit-targets` | 200/201 | 1 | 201/201 | 30/30 | 30/30 |  |
| `edit-text` | 56/56 |  | 56/56 | 10/10 | 10/10 |  |
| `edit-paragraph` | 4/5 | 1 | 5/5 | 1/1 | 1/1 |  |
| `text-add` | 21/21 |  | 21/21 | 5/5 | 5/5 |  |
| `edit-object` | 26/26 |  | 26/26 | 10/10 | 10/10 |  |
| `edit-objects` | 8/8 |  | 8/8 | 3/3 | 3/3 |  |
| `edit-replace-preview` | 22/22 |  | 22/22 | 2/2 | 2/2 |  |
| `edit-replace-apply` | 12/12 |  | 12/12 | 2/2 | 2/2 |  |
| `clip-copy` | 27/27 |  | 27/27 | 4/4 | 4/4 |  |
| `clip-paste` | 23/23 |  | 23/23 | 4/4 | 4/4 |  |
| `edit-batch` | 10/10 |  | 10/10 | 2/2 | 2/2 |  |
| `undo` | 102/107 | 5 | 107/107 | 28/28 | 28/28 |  |
| `redo` | 31/32 | 1 | 32/32 | 13/13 | 13/13 |  |
| `saved` | 6/9 | 3 | 9/9 | 5/5 | 5/5 |  |
| `page-ids` | 0/0 |  | 0/0 | 50/50 | 50/50 | no passing Rust unit test |

## Flags

### No passing Rust unit test

`page-ids` has no unit test on either engine. The drives are its only evidence: every Rust drive sends it on open, and 50/50 pass with ok replies. No op is exercised only by tests skipped on Rust.

### No drive coverage

Six ops have unit tests that pass on Rust, but no drive script sends them:

| op | Why no drive sends it |
|---|---|
| `close` | The renderer never sends it; a new `open` replaces the document. Only the worker's own P0 smoke round trip (`worker.ts`) dispatches it, inside the worker, where the hook does not see it. |
| `security-set` | The Security dialog sends it (`applySecurityDialog`), and no drive opens that dialog. |
| `extract-pages` | No drive runs page extraction. |
| `pdf-from-images` | No drive runs New from images. |
| `text-all` | Sent only by Export as text (`exportCurrent("txt")`), and no drive exports text. |
| `layer-set` | Sent by the layers panel's visibility toggle, and no drive toggles a layer. |

### Coverage that depends on tests skipped on Rust

Twelve ops are also dispatched by `RUST_SKIPS` tests: `open`, `close`, `save`, `validate`, `annots`, `annot-add`, `batch-file`, `edit-targets`, `edit-paragraph`, `undo`, `redo` and `saved`. The "skipped on Rust" column gives the counts. Each of these ops also has passing Rust tests. The skipped tests patch MuPDF internals for fault injection, or inspect MuPDF's live document; `tests/run-rust.mjs` gives each one's reason.

## Engine parity

Beyond pass and fail, the report compares the two engines request by request.

**Unit tests.** 421 tests pass on both engines and dispatch ops. All 421 dispatch the same ops the same number of times, with the same ok, refused and threw counts.

**Drives.** In 49 of 50 scripts, the app sends the same requests on both builds and gets the same replies, refusal reasons included. In `nav-zoom-find.jsonl`, only the number of `thumbnail` requests differs:
- this run: 22 on Rust and 26 on MuPDF;
- an earlier identical run: 21 and 28.

The renderer requests thumbnails as the strip comes into view, so the count depends on how far it got before the script ended. All thumbnail replies were ok, except the last one, still in flight at exit.

Every drive refusal is one its script expects, and it occurs identically on MuPDF. The refusing ops are `batch-file`, `edit-text`, `compare-open`, `insert-pages`, `text-add`, `link-set`, `field-add`, `field-props`, `field-set`, `redact-apply`, `link-add`, `sign-prepare` and `sign-list`.

## Rust failures and their diagnosis

There are none. Every run on the Rust engine or the Rust build passed:

| Run | Pass | Fail | Skip |
|---|---|---|---|
| Unit | 560 | 0 | 103 (+11 `RUST_SKIPS`) |
| Drive test files | 119 | 0 | 0 |
| Full suite | 666 | 0 | 8 |

The 8 full-suite skips:
- 7 tests that skip outside `npm run test:rust`: 5 in `rust-engine-robust`, 2 in `rust-engine-smoke`. The Rust unit run passes them.
- `legacy p12 parses when rc2-40-cbc exists`, which skips because this OpenSSL has no RC2.

The MuPDF build's 13 suite skips:
- the same 8;
- the 3 Rust-package tests in `rust-cjk-font`;
- the 2 Rust-build-only tests in `worker-build`.

The redaction drive failures, and the 23 Rust unit failures, recorded in `engine/design/app-worker.md` on October 15 are gone at `d352fcd`.

### Known load-sensitive test

`p3-edit` `U3 review: /W expansion is bounded to 2-byte codes and to the CMap code ceiling` passed in every run. It times one scan against a 2000 ms budget. Whole-test durations:

| Run | Duration |
|---|---|
| Rust suite (where it runs engine.ts) | 432 ms |
| MuPDF suite | 532 ms |
| Rust unit run (where it runs the Rust engine) | 789 ms |
| MuPDF unit run | 583 ms |
| Alone on Rust, ×3 (`rerun --mode=unit`, the test by name, cold module load included) | 1010, 996, 950 ms |
| Alone in the suite, ×3 (`rerun --mode=suite`) | 680, 661, 729 ms |

In `npm test` it is an engine.ts check, not a Rust one. It is load-sensitive only where many test files share the CPU, as at npm's default concurrency. That load is above what these recorded runs used. The supplementary concurrency-4 suite passed it as well.

## Limitations

- **Smoke runs.** Electron runs started with `--smoke` (`p1-open`, `p1-save`, `p3-smoke2`) and the worker's internal P0 round trip are not op-mapped. The drive phase runs only the files that name a drive script. Smoke tests pass in both suites.
- **Static links.** The script-to-test links are static references. The op attribution of a script's run is dynamic: it comes from the hash match.
- **Counts.** A test counts as exercising an op on one dispatch, whatever the reply. The `ok`, `refused` and `threw` counts, and `withOkReply`, say how. Assertions are not traced per op.
- **Ledger columns.** This run refreshed only the two M0 columns. The acceptance results below rewrote `status`, `current_scope`, the evidence columns and `remaining_verification`.

## Acceptance review results (2026-10-16)

An independent acceptance review (workflow run `wf_47a8f38e-1a3`) checked every ledger op in 12 groups, plus an app integration check. Each group had up to 2 fix and re-verify rounds. All fix branches are merged into `rust-engine-app` at `3d1a300`.

- **64 of 67 ops are accepted.** View, pages and compare passed at first review. Journal, nav, annots, forms and sign passed after 1 fix round; edit and objects after 2. In session, `close`, `page-ids`, `authenticate` and `security-set` passed at first review. In redact, `redact-add` and `redact-set` passed at first review and `redact-search` after 2 fix rounds.
- **3 ops are not accepted.** Each was still open after 2 fix rounds:
  - `open`: 1 blocker, 3 majors. A dictionary missing its `>>` makes the repair lose the next object; this regression came in with `c4d2181`. The majors are all older than the fixes: an intact xref is still dropped for a header the scan missed; an untyped page-tree node shows as one blank page; one damaged byte in `trailer` makes an RC4 or AES-128 file unopenable.
  - `redact-preview` and `redact-apply`: 1 blocker and 1 major each. Clip paths in `region::form_parts` grow memory without a bound; this regression came in with `67ed038`. The major predates the fixes: many mark rects stored in the file make shared-content queries quadratic.
- **The integration check is not accepted** after 2 fix rounds. 1 blocker: decoded forms still pass the held-content budget through inherited pattern fills and Type3 fonts. 2 majors are left for the owner to decide: M9 recovery for long operations, and mobile, which still runs on MuPDF.
- `verdicts.json` lists, per group: the fix rounds, branch, tip and design note. Per op it gives the verdict and the titles of the findings still open at the final review. Minors are included there, while the review's own table counts only blockers and majors. Five minors fixed after the review carry `resolvedAfterReview`.
- Findings, fixes and decisions are in `engine/design/app-accept-<group>.md`. View, pages and compare have no note.
- `docs/rust-engine-operation-ledger.csv` was rewritten from these results: status, scope, evidence and what remains.
- The suites above were measured at `e747ef6`, before the fixes. Final package gates at `3d1a300`:

All pass at `3d1a300` (2026-10-16; every step under memcap 3000 MB, 2 build jobs, 2 test threads; logs in `tpdf-wt/gates-out/final-3d1a300/`, not committed):
- `cargo fmt --check` and `cargo clippy -D warnings`, engine and bridge, on Rust 1.98.1 and 1.92.0.
- Native tests: 1,618 passed, 0 failed in 131 test binaries on 1.98.1, and the same on 1.92.0 (peak 1,521 MB).
- Bridge tests: 4 passed.
- The WASM package (`engine/tools/build-workflow.sh`) and `npm run build` (Rust build) both build.
- App unit suite without Electron: 681 tests, 568 passed, 0 failed, 113 skipped.
- App unit suite on Rust (`tests/run-rust.mjs`): 670 tests, 567 passed, 0 failed, 103 skipped.
- Full suite with Electron on the Rust build, including the 20 test files that run drive scripts (`node --test --test-concurrency=2 tests/*.test.mjs`): 681 tests, 670 passed, 0 failed, 11 skipped (peak 1,721 MB).
