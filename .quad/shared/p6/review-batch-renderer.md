# Batch review — renderer + human UX, real CDP mouse (Opus 5.5, 2026-09-25) — REVISE
Probe: .quad/shared/p6/checks/review-batch-renderer-mouse.mjs (Electron with --inspect for the main process [the File ▸ Batch… MenuItem.click() runs in main], --remote-debugging-port for the renderer, --batch-dir=<scratch copy>, drive hold opens nav.pdf + header "Open doc"). Sessions a (4 files), b (40 copies of nav), c (notes.txt + a directory named fake.pdf), d (pdf4all-out symlinked elsewhere). Run under the lock: 28 checks, 1 real FAIL (A7). The first b-run FAILs came from the probe itself (Start was below the fold), re-run with scrollIntoView: 4/4 ok.

## Findings
**R1 (blocking) renderer.ts:8182-8193 (startBatch) + 8130-8135 (prefill only in openBatchDialog); §9.13, §9.10, D1.** The GUI re-uses a stale `#batch-bates-start`. After a run, batesNext is 6 but the field still reads 1. Clicking Start again, or picking another folder and clicking Start, runs `batesNext = Number(start)` and sets the counter back to 1. Repro, probe A7: after the first run, click #batch-start again. STATE `batch.bates` is `{start:1,next:1,pad:4}`, but §9.13 requires next to stay unchanged (6). A second folder in the same dialog would be stamped B-0001… again, giving duplicate Bates numbers, and the stamp dialog's prefill also drops to 1. The drive does not catch this because driveBatch writes String(batesNext) into the field. Fix: in runBatch's finally, add `batchBatesStartInput.value = String(batesNext)` (and `batchBatesPadInput.value = String(batesPad)`). Pin it in the probe or a drive with two GUI-style starts.
**R2 (should) index.html:795 / styles.css (no #batch-dialog rules); D4.** The table has no height limit. With 40 files the dialog is 1387px tall in an 860px viewport, and #batch-start/#batch-cancel sit at y≈1378, off-screen (probe B0; screenshot b0). The human must scroll to find Cancel while the run is going, and #batch-summary (the live progress) and the running row scroll away. Fix: copy the verify rules, `#batch-list` inside a wrapper with `max-height:45vh; overflow:auto`, plus padding on th/td.
**R3 (nit) renderer.ts:8113-8119; §9.17 / D4.** The table has no Bates column. The per-file range appears only in STATE and report.txt. The "Status Pages" headers run together without cell padding (a2-done.png), and the folder field is cut off. Fix: add a Bates column (`from–to`, padded) and the verify-dialog th/td padding.
**R4 (nit) renderer.ts:8863.** #batch-close looks enabled while running but does nothing. Disable it in renderBatch while running.
**R5 (nit) renderer.ts:8214.** `batch.bates.pad` reports 6 when the text has no {bates} (probe D1 `pad:6`), even though batesPad was not used. Report `batesPad` instead.
**R6 (nit) index.html:795.** `#batch-list` has no accessible name (no caption or aria-label). Every other batch-* control is labelled.

## checkedFine (what ran)
- Menu item File ▸ Batch… clicked in main: accelerator `Alt+CmdOrCtrl+B`, the dialog opens, Start is disabled, Pick is enabled, and the STATE `batch` slice appears (it was absent before). Pick under smoke uses --batch-dir, with no native sheet. The folder is filled, 4 rows are queued in the order encrypted/hello/nav/rotated, and Start is enabled.
- Real-pointer run: STATE `batch` matches §9.16 exactly. Announcement and #batch-summary read "Batch done: 3 ok, 1 failed, report in pdf4all-out". The open nav doc stays at revision 1, dirty, canUndo, with no error. Batch keys are unchanged. pdf4all-out holds hello/nav/report.txt/rotated. report.txt matches §9.17 byte for byte. The nav output has B-0002..4 and no "Open doc".
- Rerun: 3 files `exists` plus encrypted. report is `exists` and report.txt is byte-identical. The announcement matches §9.13 (only the Bates counter is wrong, R1).
- While running, Pick and Start are disabled. Escape keeps the dialog open and cancels after the running file: done 7 of 40, remaining rows queued, report.txt has 7 lines. Clicking #batch-cancel mid-run stops at done 20 with the dialog still open.
- Close, then reopen through the menu: folder, rows and summary are PRESERVED, which matches §9.14 (the dialog is not in resetDocument). Escape when idle closes the dialog.
- The open document carries no batch footer: search "B-000" finds 0, the control search "Open doc" finds 3.
- A folder with no PDFs (notes.txt plus a directory named fake.pdf) shows "No PDF files in this folder": total 0, Start disabled, no error.
- Symlinked pdf4all-out: encrypted/unwritable×3, report `unwritable`, the counter did not move, and nothing was written to the symlink target.
- Help li is truthful and pinned (⌥⌘B, pdf4all-out, report.txt, {bates} continues, encrypted/read-only fail). All 15 ids are in REQUIRED_IDS, and every input/select has a label.
- The diffs against pre-batch tgz for main.ts, preload.cts, index.html, p2-shell and policy.ts contain only contract items. The p2-shell diff only adds lines. styles.css is unchanged.
- Drives exit 0: bates, bates-pad, dialogs, refusals, help-fields, direct-edit8-chords, replace-page, direct-edit3, object-edit, content-edit, debt1. p2-shell passes 11/11. tsc is clean.
- fixtures.sha256 9 OK; the 55-line hash list is identical; no pdf4all-out under fixtures; no stragglers.

## Screenshots (checks/review-out/)
- review-batch-a0-open.png: empty dialog.
- a1-filled: fields set.
- a2-done: the finished table (readable, no Bates column).
- a3-rerun: all rows exists.
- a4-opendoc: search results on the open nav doc.
- b0-running: 40 rows, buttons at the bottom of a scrolled dialog.
- b1-cancelled: after Escape.
- b2-cancel-click: after #batch-cancel.
- c0-nopdf: the no-PDF message.
- d0-symlink: unwritable rows.
