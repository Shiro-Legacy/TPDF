# Rust engine open edit save and reopen plan

Current plan, October 12, 2026: [Rust PDF engine completion plan](rust-engine-completion-plan.md) supersedes this first-workflow sequence for ongoing work. As of October 14, 2026, 36 of 54 fixtures fully render, 13 support independently verified edit/save/reopen, and 872 Rust tests pass on both toolchains. See [current implementation status](../engine/WORKFLOW-STATUS.md). The remainder below is the historical first-workflow plan; its old next-step instructions are not current.

Keep the accepted R24 engine and build a complete text-editing workflow on top of it. The first deliverable is a restricted but usable loop: open an eligible PDF, render it, select and replace existing text, undo and redo, save a copy, close the session, and reopen the saved bytes with the edit intact. Broader compatibility follows this loop rather than blocking all integration until rendering is complete.

Prepared October 12, 2026. This is a planning checkpoint, not an implementation or rollout. R24 remains the latest accepted engine milestone. The existing application remains the reference for behavior; the replacement runtime must execute the admitted workflow without calling MuPDF.

## Starting point

The isolated engine was at `/Users/owner/.codex/worktrees/rust-pdf-engine/TPDF/engine/` when this plan was written; the live copy is now `engine/` in the main workspace. Its `STATUS.md`, `CONTRACT.md`, and `REWRITE-ROADMAP.md` record the baseline and full replacement scope. Preserve the R24 recovery checkpoint at `.quad/backup/rust-r24-accepted-20261006/` and its frozen reference evidence.

R24 records 814 passing tests, 558 of 572 fixture pages rendered at 18 DPI, and 61 of 62 supplementary attempts rendered at 72 DPI. These are recorded acceptance results, not fresh test runs for this plan. Five hundred pages come from a text-only stress document; report document diversity and feature coverage separately from page totals.

| Workflow step | Available at R24 | Missing for the first usable loop |
| --- | --- | --- |
| Open | Bounded PDF reader, object resolution, page inventory | Owned session lifetime, generation and revision, edit eligibility |
| View and select | Restricted rasterization, positioned character codes, font mappings, source spans | Worker replies, text targets and page-space bounds, proven reverse encoding |
| Edit | Parsed objects and content syntax | Mutable object storage, isolated stream replacement, atomic transactions |
| Undo and redo | No application journal | Reversible changes, saved-state identity, cache invalidation |
| Save | Full rewrite of eligible unmodified documents | Serialization of modified objects and streams, output validation, disk-write acknowledgement |
| Reopen | Reader and renderer can consume eligible output | Fresh-session evidence that edits persist and unrelated content survives |
| Run in the app | Native CLI and WASM compilation | Executable WASM bindings, worker adapter, UI and filesystem integration tests |

The existing writer retains all effective live objects and original raw streams. It is not yet a writer for an edited document, and it does not supply the application's garbage-collection or signature-preservation behavior.

## First workflow scope

The first edit replaces a discovered, unshared page-level text run using its existing supported font. Start with horizontal simple-font text, a proven mapping from replacement characters to drawable PDF codes, and geometry the current engine can evaluate. Permit different replacement lengths when they fit and cannot disturb following text; follow existing application refusal rules for overflow and changed advance before a following run. A page rotation may be an early storage smoke test, but does not satisfy the text-edit milestone.

Admission requires the intersection of reader, renderer, edit-target and writer support. An editable text run on one page does not establish that the entire document can be saved safely. Preflight document-wide writer restrictions before offering edit mode. For the initial acceptance set, every page must render and all non-target content must have preservation checks.

Initially refuse encrypted files, signature-bearing files including unsigned signature fields, linearized files, and other existing writer exclusions. Refuse shared content or Form placements for editing until isolation is implemented and tested. Unsupported font mappings, marked-content semantics, text clipping and unsupported geometry must have explicit diagnoses. These restrictions are compatibility gaps against the current app and belong in the capability ledger.

Save As to a scratch destination is the first filesystem path. Save-in-place is a later gate using the existing safe-write machinery. Text replacement does not promise secure erasure of old content; redaction and sanitization remain separate work.

## Implementation sequence

Use W0 through W6 as planning identifiers. They do not claim acceptance or allocate new R milestones.

| Stage | Deliverable | Acceptance condition | Depends on |
| --- | --- | --- | --- |
| W0 | Frozen workflow cases and capability ledger | Every selected input has a baseline render, target text, expected edit, writer eligibility and preservation assertions; exclusions have reasons | R24 |
| W1 | Owned session and transactional document | Changes are atomic; undo and redo restore semantic state; stale handles and failed operations cannot corrupt state | W0 |
| W2 | Writer for modified documents | Changed dictionaries and streams survive fresh Rust and native reopen; unrelated objects and stream payloads are preserved | W1 |
| W3 | Text target discovery and replacement | A selected run changes visibly and semantically, supports undo and redo, and passes save and fresh reopen | W2 |
| W4 | WASM worker workflow | The same sequence runs through actual worker messages with correct buffer ownership and error behavior | W3 |
| W5 | Application workflow | Open, select, edit, undo, redo, Save As, close and reopen work through the experimental app path | W4 |
| W6 | Compatibility expansion | Shared content, additional text patterns and document families pass the same end-to-end gates | W5 |

### W0 Establish evidence and interfaces

Inventory the current request and reply contract in `src/renderer/engine.ts`, the worker entry point in `src/renderer/worker.ts`, and the renderer's save flow. Freeze the subset needed for `open`, `close`, `page-ids`, `render`, `thumbnail`, `edit-targets`, `edit-text`, `undo`, `redo`, `save`, `validate`, and `saved`. Record supporting UI reads that occur during open and selection; either implement them or explicitly disable their dependent controls in the experimental path. Do not invent empty success replies for unimplemented features.

Candidate cases include `fixtures/hello.pdf`, `fixtures/p0b/edit-transformed.pdf`, and the workflows in `tests/drive/content-edit.jsonl`. Their exact suitability must be established by the intersection check; a legacy test name is not proof of R24 support. Use `tests/drive/subset-edit.jsonl` and `tests/drive/cow-edit.jsonl` as later compatibility targets. Add separate deterministic fixtures when needed without modifying or repinning existing fixtures.

Freeze both positive and refusal cases. Include escaped and hex strings, compressed content, content arrays, a second unaffected page, a second unaffected run, absent glyphs, overflow, stale targets, shared streams and writer-ineligible files. Keep the 500-page stress case in a separate performance category.

### W1 Own the document and its history

Resolve the lifetime boundary first: `Document<'a>` currently borrows source bytes. A long-lived WASM session must own those bytes and expose object and stream access safely without a self-referential structure or unsafe Rust. Choose and review an owned backing store with ranges or an equivalent safe representation before wiring sessions into the worker.

Add mutable object and stream storage over the immutable baseline. Define object allocation, replacements, reference validation and bounded storage. Changed stream dictionaries must agree with the emitted payload: recompute Length and deliberately remove or regenerate Filter and DecodeParms rather than attaching stale compression metadata to new bytes.

Stage each edit before publication. A successful edit commits one journal operation; a refused edit leaves content, history, dirty state and redo intact. Track session generation, monotonically increasing revision, stable page identity, history nodes and the saved history node. A numeric journal position alone cannot distinguish a new branch from an old saved branch at the same position. Undo or redo invalidates targets and derived caches even when it restores earlier content.

Bound retained source, overlays, history, decoded streams and rendering work. Budget refusal must leave the session usable. Test close and reopen lifetime, edit after undo, failed edit after undo, and cache reuse after revision changes.

### W2 Serialize and validate changed state

Extend the writer against the effective mutable document view while retaining its existing strict refusals. Serialize replaced objects and streams, valid references, stream lengths, trailer and cross-reference offsets. Preserve unrelated raw streams and reachable document structures. Explicitly record the initial retain-live-objects policy; do not claim garbage collection or sanitization parity.

Produce a new output buffer without changing the saved-state marker. Validate it by opening those exact bytes in a fresh Rust document and comparing expected structure and content. Independent MuPDF inspection remains a test oracle only; it must reopen without repair and confirm preservation. Check rendering before and after the edit, with differences confined to the expected edit region and its antialiasing boundary.

Exercise changed payload lengths, originally compressed streams, xref-stream inputs where eligible, dangling references, output limits and injected serialization failures. A failed serialization or validation must not discard edits or report a successful save.

### W3 Replace existing text

Construct targets from source provenance, document generation, revision, page, stream identity and operator location. Include decoded text, font facts and bounds in the view coordinate system expected by the UI. Refuse ambiguous associations. Reverse encoding must prove drawable replacement codes; ToUnicode extraction alone is insufficient.

Plan the replacement and geometry checks before mutation. Preserve unaffected content bytes and text-state behavior. Keep empty and identical replacements explicit: deletion must remain valid PDF syntax; an identical replacement produces no history entry. Return the application's relevant reason tokens for stale target, unsupported encoding, overflow and unsupported target.

The native acceptance sequence is open, render, discover target, replace, render, undo, render, redo, serialize, validate, close, reopen from saved bytes, render, and rediscover the edited text. Repeat an edit after reopening to prove that the output remains editable. Compare unaffected page geometry, resources, streams and text; a valid xref and a success flag alone are insufficient.

### W4 Execute through the worker

Add an explicit experimental backend and WASM session adapter. Implement typed request and reply conversion, errors, session handles, pixels and PDF byte transfers, cleanup and bounded work. Preserve worker isolation, CSP, no Node access and disabled PDF JavaScript. The supported loop must not fall back to MuPDF at runtime.

Verify actual browser and Electron WASM execution, not just compilation. Test detached buffers, closed sessions, repeated open and close, large admitted inputs, budget failures and worker termination. Define timeout and cancellation behavior before claiming responsiveness; a synchronous WASM call cannot process a queued cancellation message while it is running.

Resolve the save acknowledgement race explicitly. Today `SavedRequest` has no generation or revision, and the renderer releases its worker gate before awaiting the disk write. Bind acknowledgement to the exact serialized generation and history node, using a token or a proven edit gate. A late acknowledgement must never mark newer edits or another document clean. Any protocol extension must remain compatible with the existing backend.

### W5 Complete the application loop

Reuse the Electron shell, dialogs and `src/safe-write.ts`. Expose only supported editing actions in the experimental path with clear capability errors. Test real selection, text entry and display refresh as well as drive commands.

Follow the existing separation: serialize, validate, write to disk, then acknowledge the saved snapshot. Check the distinct semantics of Save As, exported copies and save-in-place; only mark the appropriate session snapshot saved after the corresponding successful write. A cancelled dialog, validation refusal, disk-write failure or document switch must not incorrectly clear dirty state.

The end-to-end gate reads the actual saved file into a new session or process. Confirm replacement text, expected pixels, preserved pages and resources, clean reopened state, and the ability to make and save a second edit. Confirm the original file hash remains unchanged during Save As. Exercise safe-write failure and recovery on disposable copies before enabling save-in-place.

Run the relevant existing application and drive tests, then supported desktop runtime and packaging checks before rollout. A native-only pass establishes engine progress; a Mac experimental app pass establishes only that integration target. Neither establishes full editor replacement.

### W6 Expand using complete workflows

First add copy-on-write for shared page streams and nested Form placements. Edit one placement while proving that every other use is unchanged, including after undo, save and reopen. Then expand text-run forms, fonts and encodings, paragraph editing and object edits according to actual blocked document families. Add required rendering features alongside those families so admitted documents stay inspectable.

Page assembly, annotations, form filling, navigation edits, clipboard and batch operations each reuse transactions and the changed-document writer but require their own semantics and preservation gates. Encryption, signatures, redaction and sanitization remain separate security-sensitive packages. They are not implied by successful text replacement.

## Completion evidence

Maintain one workflow ledger with input identity, feature family, backend, supported or refused outcome, reason, edited target, save identity, fresh-reopen result and preservation evidence. Count distinct documents and feature families alongside pages. Measure open, render, edit and save latency, peak memory and retained history on the same inputs; set performance acceptance budgets before calling a stage complete.

Preserve R24's accepted output hashes and refusal controls, and keep new reports separate from frozen evidence. Require independent review of the owned state, writer and adapter boundaries. Evidence must bind source, dependency and executable identities and include the raw artifacts needed to reproduce the result.

The first workflow is complete only when a supported document can be edited in the experimental UI, undone and redone, saved to disk, reopened independently, inspected and edited again, with explicit refusals outside that scope. Full application replacement still requires the remaining operation matrix in `REWRITE-ROADMAP.md`.

The next implementation unit should cover W0 and the W1 ownership and transaction design. The first engineering gate is atomic mutable state with undo and redo; the first product gate is the W5 text-edit workflow. No calendar estimate is justified until the ownership prototype and initial document intersection have been measured.
