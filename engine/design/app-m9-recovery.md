# Recovery for long operations (M9)

October 17, 2026. Branch `app-p9-m9-replay`. The owner chose the replay design proposed in `app-accept-integration.md` ("Major 1"). The renderer side is shared, so it works the same on both engines. Nothing in `engine/src` changed.

## Problem

The worker owns the open document. The renderer stops a worker in four cases, and each stop used to drop the document and every unsaved edit:

- an ordinary request passes the 20 s watchdog;
- the Rust engine traps and answers `reason: "worker"`;
- the worker fails (its error event: a crash or a failed load);
- the worker does not start within 20 s.

## Design

The renderer (`src/renderer/recovery.ts`, wired into `request`/`workerFailed` in `renderer.ts`) keeps three things in its own memory:

- **The base:** a copy of the bytes the document was opened from, taken before `open` transfers them.
- **The log:** every request since that open that changed the worker's session, in the order the worker answered. Each entry holds a structured-clone copy of the request (its buffers included, taken before `postMessage` transfers them) and a fingerprint of the answer. Only successful answers are logged; a refusal changes nothing.
- **The saved copy:** the bytes of the last save in place that main acknowledged.

Passwords (`authenticate`, `security-set`) stay only in renderer memory, like the rest of the log. They are never in STATE, a log line or IPC.

**Classification** (`OPS` in recovery.ts). It is a `Record` over `SessionRequest["op"]`, so tsc refuses an op that is not listed. `tests/m9-recovery.test.mjs` also parses the union in engine.ts and fails on any difference.

| kind | ops | logged |
| --- | --- | --- |
| `base` | open, close | they start a new log |
| `mutate` | every journaled edit, undo, redo, `saved`, `security-set`, `layer-set`, `redact-search`, `redact-apply`, `insert-pages`, `stamp`, labels, `clip-paste`, `edit-batch` | yes |
| `unlock` | a successful `authenticate` | yes |
| `clipboard` | `clip-copy` (fills the clipboard a later paste reads) | yes |
| `read` | renders, thumbnails, search, the lists, `edit-targets`, the previews, `save`, `validate`, `extract-pages`, `pdf-from-images`, `sign-prepare`, `batch-file`, `text-all`, `page-ids` | no |
| `compare` | compare-* (the view-only slot) | no; lost on a restart |

**After a stop.** `workerFailed` does the following:

1. Rejects the pending requests and restarts the worker, as before. The restart cap of 3 is unchanged.
2. If a document is open and its log is available, takes a snapshot of what the window showed: the document, page, auth, security state, fill-anywhere state, layers state and clipboard.
3. Starts `restoreDocument`, which holds every other request behind a barrier and calls `replay`:
   - `replay` sends `close` (generation − 1) times. A fresh worker numbers its first open 1, and edit handles and replace preview tokens carry the generation (below).
   - It opens the base and sends each logged request again. Every answer must have the original's fingerprint: every field but timings (`*Ms`), the request id and `compareClosed`, with buffers by length, hashed.
   - On a match, the window gets the snapshot back, with the journal state from the worker's last answer. It reloads its lists, renders the same page (clamped) and shows the notice. Selection, search hits and open dialogs are reset.
4. The request that hung or trapped (the culprit) was never answered, so it is not in the log and is not replayed. Neither are later changes that were posted but not answered. The notice counts them.

**Notices** (in the existing error line, also announced):

- "Adding the annotation took too long, so it was stopped. That change was not applied; your other changes were restored."
- If the culprit only read: "… Your document and changes were restored."
- "Saving took too long, so it was stopped. Nothing was saved. Your document and changes were restored."
- A trap: "Searching failed inside the PDF engine, so it was stopped. …"
- A worker error: "Searching stopped the PDF engine unexpectedly. …" (the request the worker was running), or "The PDF engine stopped unexpectedly. …" when none was.
- A page draw: "Showing the page took too long, so it was stopped. Your document and changes were restored. Page 1 was not drawn again; edit the document or open it again to try again."
- Over the replay budget: "… Restoring your changes took too long, so the document was reopened as last saved."

**Fallback, with no half-replayed state and no loop.** The replay fails on any of:

- an answer that differs from the original (`ReplayMismatch`);
- a refusal;
- a replayed request that passes its own time limit;
- the restarted worker dying.

A worker that dies or hangs during the replay is restarted (this counts toward the cap) without starting a second restore. The restore then reopens the fallback through the normal open path, and that open becomes the new base:

- the bytes of a save in place that main wrote but that is not yet marked saved (the restore waits for that write);
- the saved copy;
- the base, if this window never saved in place;
- nothing, if the last save's bytes could not be kept. The worker's half-replayed document is then closed, and the notice says to open the document again.

The notice says the changes since the last save could not be restored and the document was reopened "as last saved" or "as it was opened". If the restarted worker cannot even open the fallback, the old message applies. The cap stops a chain of restarts: a restored document whose own page render hangs is restored at most 3 times and then stopped with the old "reopen the app" message. A successful user change resets the cap, as an open does; renders and the restore's own requests do not.

**Long jobs and thumbnails.**

- **Which request is blamed:** the worker answers in order, so the oldest unanswered request is the one it is running. A timeout blames that one, not the request whose timer fired. An ordinary request queued behind a long job that is still within its own limit is not overdue: its timer waits for that limit, then for its own.
- **Thumbnails (10 s):** unchanged. The request is rejected and the worker keeps running; a thumbnail is decoration. A worker that is really stuck is caught by the next ordinary request's watchdog.
- **Other long jobs (120 s):** `save`, `validate`, `redact-apply`, `insert-pages`, `extract-pages`, `batch-file`, `sign-prepare`.
  - If the open document can be restored, the timeout now stops the worker and restores, like the watchdog. Before, the worker kept computing, every request queued behind it, and its late answer was dropped anyway. A `redact-apply` or `insert-pages` that finished after its timeout used to change the document behind the window's back.
  - If it cannot be restored (nothing open, or recovery off), the old behaviour stays: the request is rejected (`reason: "worker"`) and the worker is kept. Such a request (and a slow thumbnail) stays listed as abandoned, so a later timeout blames it.
- **Saves:** a save that times out is never marked saved. `save` and `validate` are reads, so the replay never runs them, and `saved` is only sent after main reports the write. The save flows report `reason: "worker"` ("Not saved"), and the restore notice adds "Nothing was saved."
- **Replay limits:** a replayed request has its op's own limit (120 s for `redact-apply` and `insert-pages`, else 20 s). The whole replay has a budget scaled to the log: 30 s plus twice what the open and the logged changes took the first time (request to answer, as the renderer measured). It is checked between steps; past it the restore reopens the last saved state and says so. From 20 changes the status line shows "Restoring name… 12 of 340 changes" (STATE `recovery.progress`).
- The review measured MuPDF at 3,000 notes: the replay took 98.8 s, growing faster than linearly. Such a replay runs within its scaled budget, since the original changes took as long.

## Review round 1 (October 17): restart paths that lost edits

The M9 review accepted the classification, determinism and budget, and found these restart paths that lost edits silently. All are fixed, each with an Electron test. The queued-edit, blame and trap tests were run against the previous commit and fail there; the other two read STATE fields that commit lacks.

- **Edits queued behind a hung save (major):** `request` checked the restore barrier and `ready` before the save's gate. When the save timed out, an edit waiting at the gate was posted to the fresh, still-loading worker and dropped. Its own watchdog then forced a second restart, and the notice claimed the other changes were restored.
  - Now `request` loops until the restore barrier, the worker's readiness, the gate and a worker being replaced all hold at once. Only then does it post.
  - A worker whose engine answered `reason: "worker"` (a trap) is never posted to before its restart on the next task (`failingWorker`).
  - Test: a note made 300 ms into a hung Save a copy is applied after the restore and saved later. Two watchdog periods later there is still one restore.
- **A page whose drawing hangs (major):** the restore drew the same page, hung again, and looped through 3 restores until the document was gone. The cap message was then overwritten by "worker timed out".
  - A read the window issues by itself (render, thumbnail, the panels' lists, edit targets, the redaction preview, layers, labels, page ids) that hung or crashed the worker is now paused. Until the document's revision changes or another document opens, `request` refuses it locally (`reason: "skipped"`) without posting it.
  - For a page, render and thumbnail of that page are paused. The page shows blank, the status line says "Page 1 is not shown: drawing it took too long. Edit the document or open it again to try again.", and the notice says so.
  - No automatic restore reissues the request that hung. Replays send only logged changes, and a change that hung is never logged.
  - Once the cap stops the worker, its notice is the last word: `showError` ignores later errors.
  - Test: a page that hangs when an annotation holds "HANGPAGE". The restore keeps all three notes, the page is paused, and two watchdog periods later there is no further restore. An undo changes the revision and the page draws again.
- **A failed replay during a save in place (minor 3):**
  - While main writes a save in place, `diskWrite` holds a copy of the bytes. It stays set until `saved` is recorded or another document opens.
  - A restore that falls back waits for that write, and if it landed, reopens those bytes, never an older state.
  - A save whose `saved` was lost to the restart reports `{ ok: false, reason: "worker", written: true }` instead of "nothing was written".
  - Cost: up to 50 MiB more renderer memory during a save in place.
  - Test: a note hangs during the write, then the replay hangs. The window reopens the written file (both notes), clean.
- **Main's dirty flag during a restore (minor 4):** `syncDirty` reports `dirty || restoreDirty` to main, and `restoreDirty` is the snapshot's flag until the restore ends, so a close or an open during "Restoring…" still asks. Test: STATE `recovery.dirtyReported` is true mid-replay.
- **No overall replay limit or progress (minor 5):** the budget and progress above. Unit test: `ReplayOverBudget` with a fake clock, and progress steps.
- **A trap during the replay (minor 6):** the replay failure used to post the fallback open to the trapped worker before its deferred restart. The restore now waits for that restart inside the restore, so it does not start a second one. Test (Rust): a change that traps only when replayed; the document is reopened as it was opened.
- **Wrong op named (minor 7):** the blame rule above. Test: an annotation list requested during a hung `insert-pages` (limit 6 s, watchdog 3 s); the notice names "Inserting the pages".
- **Mobile budget (minor 8):** `recoveryBudgetFor` gives 256 MiB on desktop, 128 MiB where `navigator.deviceMemory` reports 4 GiB or less, and 96 MiB for mobile (`MOBILE_RECOVERY_BUDGET_BYTES`). The desktop renderer passes `navigator.deviceMemory`. The mobile app has its own worker client with no recovery yet; a mobile client should construct `new RecoveryLog(recoveryBudgetFor({ mobile: true }))`.
- **Not fixed (pre-existing, not M9):** `saved` is sent without the save's ticket. An edit made while main writes a save in place is marked clean with it, on both engines, with or without a restart.

## Determinism

The engines are deterministic given their inputs. The inputs a request did not carry are now recorded with it:

- `annot-add` gets the renderer's local date as `today` (a signature stamp's "Signed by … on YYYY-MM-DD").
- `redact-apply` gets 32 bytes of `entropy` from `crypto.getRandomValues`.

The adapter (`engine-rust.ts` `wire`) keeps a caller's `today` (`YYYY-MM-DD`) and `entropy` (32 bytes) and draws them itself only when they are absent. engine.ts's `signatureContents` reads `today` the same way. A redact-apply replay reuses the original's entropy on the same input, so it writes the same bytes. The intermediate rewrite is never written to disk: a save draws fresh entropy.

Replay reproduces:

- **Generations:** reached by the closes above. Edit handles (`generation` on edit ops) and `edit-replace-apply`'s `previewToken`, a hash of `generation|revision|…`, therefore match. The log refuses entries from two generations (`generation`); a log whose generation needs more than 10,000 closes is not kept.
- **Revisions:** an open resets the revision to 0, and every logged change moves it as before.
- **Object numbers:** annotation, field, link and page ids. The Rust engine numbers a new object past every number the session used, and replaying the same requests from the same bytes uses the same numbers.
- **Undo history:** undo, redo and `saved` (sent without a ticket, as the renderer does) are logged. The redo branch and the clean baseline are those of the original.

Measured: `tests/m9-recovery.test.mjs` replays 13 logged changes in a fresh engine. They include a FreeText, a signature stamp with an image, `annot-set`, `edit-replace-apply` at generation 3, a rotation, a bookmark, undo, redo, `saved`, and an undo after it. On both engines, every answer matches and so do the annotation, field, edit-target and page ids, the bookmarks, the next undo/undo/redo answers and the id of the next new annotation. The saved files are byte-identical on both engines. In Electron, on both builds, the restored window's revision, FreeText ids and rects match the pre-hang STATE, and a Save a copy afterwards is byte-identical to the same edits without the hang.

## Deviation from the brief: the log is not cleared on save

The brief said to clear the log on save and use the saved bytes as the new base. That cannot replay exactly:

- **Ids:** the Rust engine numbers new objects past every number the session used, while a reopened save numbers from its largest written object plus 1. An add, delete, save, add sequence gives the second add a different id after a reopen.
- **Revisions:** the revision restarts at 0 on a reopen. Edit handles' revisions and replace preview tokens (which hash the revision) would no longer match.
- **Undo history:** undo cannot reach past the save point.

So the base stays the opened bytes. `saved` is logged like any change, which reproduces the clean baseline. The acknowledged save in place's bytes are kept only as the fallback.

- **Save a copy** (`save-to`, Save As) changes nothing. The window still edits its source file, which stays dirty, so a copy must not become the base.
- **Save in place**, including Save on an untitled or "(filled)" document, logs `saved` and replaces the saved copy.
- **Open and close** start a new log. A failed open, and any open the renderer starts, drop it first.

## Memory budget

`RECOVERY_BUDGET_BYTES` is 256 MiB on desktop (128 MiB on a desktop reporting 4 GiB of memory or less, 96 MiB as the mobile default; review round 1) for the base, the saved copy and the log together:

- **What fits:** the app opens up to 120 MiB (`MAX_PDF_BYTES`) and saves up to 50 MiB (`MAX_SAVE_BYTES`), so even a near-ceiling document keeps 86 MiB of log.
- **How a request is counted:** buffers in full, strings at 2 bytes a unit, 16 bytes of overhead per object. A note is about 180 bytes; an image stamp or `insert-pages` is about its file.
- **Past the budget:** a request whose copy would pass the budget is not copied. If it then succeeds, recovery turns off for that document (`budget`) and the log, base and saved copy are freed. A refused one changes nothing.
- **A saved copy that does not fit** is not kept, and the stale one is dropped. The replay still works, but a failed replay then has no fallback.
- **Saying so:** only a restart says it: "The PDF worker stopped (…). Restarted it (1 of 3); open the document again to continue. Your unsaved changes could not be kept: they were too large to keep a recovery copy."
- **Other cases that turn recovery off:** a paste whose clipboard was filled outside this log (a copy in another document, or before this open), and edit handles of two generations.
- **When it comes back:** recovery is back at the next open.
- **The cost:** the base copy doubles the opened bytes in the renderer for as long as the document is open. Before, the renderer transferred them to the worker and held none.

## What is and is not recovered

- **Recovered:**
  - the document as the window showed it, with its unlock, pending security change, layer visibility, edits, undo/redo history and clean baseline;
  - the clipboard, when the copy that filled it is in the log;
  - the page;
  - the fill-anywhere Save-as-copy state.
- **Not recovered:**
  - the request that hung or failed, and later changes posted before the stop (the notice says how many);
  - the compare slot;
  - search hits, the selection, open dialogs, and signature verification results (shown again on demand);
  - a clipboard filled in another document (recovery is off once such a paste is made);
  - anything when recovery is off (budget, clipboard, generation), and a document that was still opening.
- **Not recovered on a failed replay:** the changes since the last save in place. The document is reopened as last saved.
- **Replay time:** a replay costs what the original changes cost, again. A long session replays all of its changes since the open, each within its own time limit, while the window shows "Restoring …".
  - Measured in Node through the production adapter: large.pdf (478 KB, 500 pages) with 240 logged changes (200 notes across pages, 20 undo, 20 redo) holds 37,780 bytes of log, about 157 bytes a change.
  - The replay took 422 ms on Rust (the original changes 440 ms) and 619 ms on MuPDF (712 ms).
  - Peak RSS was 145 MB and 230 MB.

## Differences between engines

- **Shared:** both builds share the renderer path, the classification and the tests. The Electron recovery tests pass on both.
- **Traps:** only Rust traps (`reason: "worker"`). MuPDF turns allocation failures into refusals.
- **Clocks:** neither engine writes a clock value on these paths. Measured on engine.ts over MuPDF: two sessions adding a Text, a FreeText, a Square, a Highlight and a text field, run 2 s apart, save byte-identical files with no `/M` or `/CreationDate`. The signature date is the recorded `today`. So a replay is byte-identical on both engines. If a future op stamps the time, its saved bytes after a replay would differ by that date; its answers would differ only if they carried the time, and then the replay would fall back.
- **Entropy:** engine.ts ignores `entropy`, because MuPDF draws its own.

## Tests

- **`tests/m9-recovery.test.mjs`** (unit; runs under `node --test` on MuPDF and under `tests/run-rust.mjs` on Rust): exhaustive classification against the union, budget arithmetic, fingerprints, log rules (reads, refusals, unlock, open, close, budget, clipboard, generation, saves), the engine replay above, mismatch detection (a changed answer, a changed open, a failed send), and an encrypted document's unlock and `security-set none` replayed.
- **`tests/m9-recovery-drive.test.mjs`** (Electron). Each test drives an app copy whose watchdog and long-job limits are 3 s and whose worker busy-loops on a marked request. A worker counts its incarnations in IndexedDB.
  - Hang: edits before it survive with their ids and revision, the hang is reported, undo and redo work, and the save equals the control run.
  - Replay failure: a request that hangs only when replayed. The document is reopened from the save in place, and the next edit and save work.
  - Save timeout: not saved, not replayed, and a later save works.
  - Budget: a 4 KiB budget gives the old behaviour and its message.
  - Unlock: an encrypted document comes back unlocked with its rotation.
  - The drive script.
- **`tests/drive/recovery-log.jsonl`:** the real build's log across opens, edits, a find, undo/redo, Save a copy, Save in place and an unlock. The password appears in no STATE.
- **`tests/worker-build.test.mjs`:** the trap and startup-failure tests now expect the restored document. The trap test adds a note before the trap and checks it survives. The startup test's restore loses its worker to the injected load failure and reopens from the base on the third worker; 21 s later no stale timer has fired.
- **`tests/m9-recovery-drive.test.mjs`, review round 1:** the queued edit, the hanging page, the blame behind a long job, the save in place written during a failed replay (with main's dirty flag), and the trap during a replay (Rust).
- **STATE:** `recovery` reports:
  - `available`, `unavailable`, `entries`, `bytes`, `saved` and `budget`;
  - `restoring`, `progress` and `dirtyReported`;
  - `paused` (the read keys paused at the current revision);
  - `last` (outcome, dropped op, cause, lost count, steps, `overBudget`).
