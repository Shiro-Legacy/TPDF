# Rust PDF engine completion plan

Updated October 14, 2026, evening (status table and sources; the plan itself is unchanged from October 12). Keep the R24 foundation and the working Rust workflow. Complete the missing PDF capabilities in stages, proving each through the actual application and saved output. The first compatibility target is `SBI05131482.pdf`; the final target is the PDF behavior supported by the existing Tandem PDF application.

## Completion target

The engine is complete when Rust supplies every PDF operation required by the existing application's `SessionRequest` contract, with equivalent supported behavior, deliberate refusals, valid saved files, and acceptable performance. This includes reading, rendering, extraction, editing, page assembly, forms, annotations, navigation, security and signatures. It does not mean every optional feature in the PDF specification.

Rust owns PDF interpretation, content mutation and serialization. Compile that engine to WASM for the application. Keep the TypeScript interface, Electron shell, dialogs, safe filesystem writes and external signing/verification cryptography where they already work. Define and test their Rust interfaces. MuPDF may remain an independent test consumer, but the replacement runtime cannot delegate PDF work to it or silently fall back.

Distinguish three outcomes: engine capability completion, complete application integration, and release readiness. A milestone passing does not automatically enable production rollout. Keep the existing app and R24 recovery checkpoint available throughout the transition. Retain the current source-informed implementation and recorded licensing/provenance; this plan does not restart as a cleanroom project.

## Verified starting point

| Area | Current state |
| --- | --- |
| Accepted foundation | R24 is the last formally independently reviewed engine milestone. Later workflow work is tested but has not had equivalent independent implementation review. |
| Functional workflow | Restricted native and WASM open, text and TJ-run replacement, undo/redo, Save As, fresh reopen and another edit. A separate view-only open reports per-page render and edit capabilities. |
| Compatibility | View mode opens 53 of 54 fixture page trees and fully renders 51 documents at 72 DPI (October 14 evening: encrypted documents with a password, default-state layers, inline images, cs color spaces and generated widget/note appearances; form, layered and encrypted documents are view-only); 18 expose editable targets after M2 copy-on-write. All 18 pass edit/save/fresh-process reopen and independent text/pixel checks. |
| Regression evidence | 887 Rust tests pass on both Rust 1.98.1 and minimum 1.85.1 (October 14 night, reference/m2-cow-20261014). The desktop WASM smoke passes. All previously rendered fixture pages stay byte-identical at each step. |
| Metadata preservation | Empty form metadata and borderless links survive editing and saving. Stored and display-generated annotation and widget appearances render, and layers render in their default state; saving generated appearances, form editing, active link navigation and layer switching remain incomplete. |
| Architecture | Bounded serialized snapshots, atomic publication, stale-target rejection and exact save acknowledgements exist. M2 has started: transaction-local object allocation and replacement are written, and text edits copy shared page streams and Form placements (October 14). Deletion, reachable-object collection, broader resource isolation and efficient large-document history remain incomplete. |
| Limits | Preview input/output 16 MiB, 64 pages, 64 undo steps and 64 MiB retained snapshot bytes. These are not total-memory ceilings. The main app currently allows 120 MiB input and 50 MiB saved output. |
| Real document | `SBI05131482.pdf` opens and renders its whole page in native Rust and WASM (October 14). This needed the rendering intent, Arial display substitution, CalRGB images, image intents, tag-only marked content, dashed strokes and bounded clip masks. Text editing on it is still refused because its fonts are display substitutes (M3 work). |

In view mode, the 3 fixture documents that are not fully rendered have these first failures (October 14 evening, after encryption, layers, inline images, color spaces and generated appearances): a stream-valued feature (redact.pdf), the page-count limit (large.pdf) and a deliberately unbalanced `q` (edit-tj.pdf). The October 12 strict-admission grouping is in engine/reference/m1-20261012/strict-admission.json. These are first failures, not a complete inventory of everything each file needs. Do not require every fixture to become editable: signed, permission-restricted and intentional refusal cases must follow existing application policy.

Sources: [workflow status](../engine/WORKFLOW-STATUS.md), [current view admission report](../engine/reference/m1-encrypt-20261014/view-admission.json), [current worker contract](../src/renderer/engine.ts), [application limits](../src/renderer/limits.ts), and [original completion inventory](../engine/REWRITE-ROADMAP.md). The live engine source is `engine/` in this workspace; the Codex worktree copy is stale. The older roadmap's R24-only stop language is historical; the user's subsequent continuation instruction is the current scope.

## Milestones and order

| Milestone | Deliverable | Acceptance gate | Dependencies |
| --- | --- | --- | --- |
| M0 | Current baseline and full operation ledger | Every request maps to implementation status, fixtures, expected success/refusal, and an owner milestone; current evidence is reproducible and archived | Current checkpoint |
| M1 | Ordinary document viewing and the SBI case | The original SBI PDF opens and all visible content renders correctly in Rust WASM; capabilities explain unavailable edits without a blanket open refusal | M0 |
| M2 | General transactions and writer | New/replaced/deleted objects, shared-resource isolation, atomic edits and bounded history survive save/reopen without changing other uses | M0; retain M1 regressions |
| M3 | Practical text editing | Required text runs, paragraphs and new text pass selection/edit/undo/save/reopen; complete a supported edit in the SBI PDF | M1, M2 |
| M4 | Rendering and extraction compatibility | Required fonts, images, color, transparency, marked content and structured text match the reference behavior across representative families | M1; interleave prerequisites with M3 |
| M5 | Page operations and navigation | Page assembly, bookmarks, links, labels and layer state remain correct through edits, grafting and reopen | M2; M4 for affected rendering |
| M6 | Annotations and forms | Required annotation and AcroForm operations regenerate correct appearances and preserve values, flags and shared fields | M2, M4, M5 remapping |
| M7 | Object editing and document utilities | Clipboard, multi-object edits, batch changes, image creation/stamps, comparison and exports match existing behavior atomically | M3–M6 as used by each operation |
| M8 | Security and permanent removal | Authentication, permissions, encrypted saves, permanent redaction and signature preparation pass dedicated adversarial/interoperability gates | M2, M4, M6; read authentication can start earlier |
| M9 | Full application replacement | All existing worker requests and supported app workflows run through Rust; no runtime MuPDF dependency or hidden unsupported controls | Adapter work starts M1; acceptance requires M3–M8 |
| M10 | Production readiness | Independent review, representative-corpus validation, resource/performance budgets and real packaged runs pass on all supported desktop targets | M9 |

This is a dependency order, not a requirement to finish every renderer feature before attempting another usable workflow. Implement the prerequisite needed by a blocked document, then immediately exercise its complete workflow. A new refusal counts as diagnosed work, not a completed feature.

## M0 Freeze scope and evidence

Use [the operation ledger](rust-engine-operation-ledger.csv) as the starting checklist. For every request, link the relevant legacy tests, document families, Rust tests, worker tests and desktop drive cases. Treat current preview operations as partial until they satisfy the production request/reply contract. Record baseline legacy outcomes before judging compatibility; preserve intentional refusal cases.

Maintain separate results for opening, rendering, extraction, target discovery, editing, saving and fresh reopening. One document may support viewing without supporting mutation. Include the 500-page stress fixture in performance results, not as hundreds of independent compatibility successes. Add independently sourced document families beyond the 54 fixtures, with provenance and privacy handling recorded before inclusion.

Freeze source/dependency/binary hashes, toolchains, input hashes, commands, raw results and known failures. Store durable milestone evidence under versioned `engine/reference/` directories only. `.stage` is deleted by the packaging script, and `.quad/backup` is gitignored and was purged on 2026-10-14; neither is durable. Preserve old evidence rather than rewriting it. Review the existing session, writer and save-token boundary before substantially extending them.

## M1 Open and render ordinary PDFs

Split read/view eligibility from edit and write eligibility. Inspect capabilities per document and page, retain the current document after a failed open, and report unsupported rendering honestly. A page with unimplemented visible content must not be presented as a faithful complete render. Signed documents should be viewable when supported even if mutation is forbidden; encrypted reading still requires authentication.

Implement rendering intents within an explicit color policy. Do not simply skip an operator if it changes visible output. Define deterministic font substitution for unembedded Arial regular/bold/bold italic and required standard fonts. Preserve PDF advances, select the correct face/style, record font-asset provenance, and avoid accidental dependence on whatever fonts happen to be installed. Prove the distinction between source codes, Unicode, glyph selection and placement. Substitution for display does not by itself establish that edited output is portable.

Use the original SBI file as a local acceptance case without copying its private contents into public fixtures. Create synthetic regressions for each encountered feature. Compare every page at normal and zoomed scales, checking text, glyph placement, images, clipping and color. Discover and track additional blockers rather than claiming the first two fixes guarantee success. M1 ends with correct viewing; M3 supplies the editable/saveable workflow.

## M2 General document mutation and saving

Extend the owned session with object allocation, replacement/deletion, reference validation and resource ownership. Support copy-on-write for shared page streams, fonts/resources and nested Form placements: editing one use must leave all other uses unchanged. Retain source provenance so targets can be rejected after any conflicting mutation.

Keep every edit transactional. Failed or cancelled work must preserve bytes, dirty state and redo. Preserve save acknowledgements bound to exact session/history identity, including document switches and late writes. Add cache invalidation and stable page identity suitable for page moves/deletion. Measure snapshot costs before replacing the current simple design; large-document support will likely need shared immutable storage or transaction deltas, with equivalent rollback tests.

Extend full rewriting to allocated objects, changed streams and required trailer structures. Define reachable-object collection separately from ordinary preservation. Support existing valid input families and expected repair behavior deliberately, with diagnostics. Linearization and incremental inputs require documented handling; signature-preserving incremental output belongs to M8. Keep Save As working throughout and enable save-in-place only through the existing safe-write validation, recovery and acknowledgement flow.

## M3 Text editing that works on real documents

Add content-stream arrays, positioned `TJ` runs, shared content and Form edits in small tested increments. Handle text matrices, spacing, rotations, line boundaries, different replacement lengths and overflow without moving unrelated content. Extend encoding/font coverage only where replacement bytes, glyphs and geometry can be proved.

Then implement paragraph changes, new text, and replacement preview/apply using the existing application's supported semantics. Maintain marked-content and structure-tree replacement text rather than leaving stale ActualText or accessibility data. Provide font embedding or another proven portable representation when new text relies on substitute assets. Do not equate on-screen success with correct saved rendering on another machine.

Exit with the existing content, subset, coded, copy-on-write, paragraph and replacement drive cases passing as applicable, plus at least one meaningful supported change to the original SBI document. For each: edit, undo, redo, save, close, reopen in a fresh process, extract/render, and edit again. Check unaffected pages and objects as well as the edited region.

## M4 Rendering and extraction coverage

Use the compatibility ledger to order required composite TrueType, Type0/CID, CFF/Type1, fallback and standard-font work; include CJK and right-to-left/vertical behavior where the app supports it. Complete required image filters and formats, inline images, masks, decode arrays, color spaces, rendering intents, blending, transparency and clipping. Add stroke/dash/transform behavior needed by blocked inputs. Do not reduce an unsupported appearance to an empty successful render.

Build structured lines, spans, quads and stable text identities for selection, search, extraction and comparison. Handle ligatures, marked content and ActualText consistently. Keep extraction Unicode distinct from glyph-outline mapping. Test supported scale and alpha behavior, thumbnails, rotated/cropped pages and cache invalidation. Establish visual tolerances per feature family before accepting results; exact byte or pixel equality with a different rasterizer is not a universal requirement.

## M5 to M7 Complete ordinary editor features

Page assembly must preserve inherited geometry, stable identities and cross-document object references. Implement rotate/delete/move/insert/extract, blank and image document creation, then remap destinations, bookmarks, links, page labels, widgets and fields when pages move between documents. Implement optional-content visibility and invalidate render/extraction/edit caches when layer state changes.

Implement existing annotation types, their editable properties and appearance streams. Implement AcroForm inheritance, supported field types, values, options, default values, flags, tab order and creation/deletion. Test shared radio groups and fields across pages. Distinguish empty signature fields from signing. Keep PDF JavaScript disabled. Do not expand XFA behavior beyond existing application support without explicitly adding it to scope.

Complete object transforms/removal, clipboard copy/paste, atomic multi-object and batch operations, image stamps, drawn signatures, comparison and exports. Test clipped and shared objects, cross-document resources, font/image embedding, refusal partway through a batch, and undo after reopen where the app contract calls for it. Every utility must use the same transaction, permission and save rules as direct edits.

## M8 Security and permanent removal

Treat these as separate acceptance packages, not extensions implied by the generic writer:

| Package | Required proof |
| --- | --- |
| Authentication and encryption | Existing password/cipher families, owner/user permissions and crypt filters; correct rejection of wrong passwords; permission checks on every affected operation; changed/removed passwords and encrypted output reopen in independent consumers. |
| Redaction and sanitization | Required text, image and vector removal, shared-resource isolation and configured metadata/attachment/form cleanup; inspection of reachable and serialized content demonstrates removal. Cover overlays, old revisions and retained unreachable objects. A visual cover or ordinary text replacement does not pass. |
| Signatures | Inventory, field/appearance handling, exact ByteRange/Contents reservation and external CMS interface; incremental saves preserve signed byte prefixes; multi-signature and certification restrictions match the application; independent verification and tamper controls pass. |

Review cryptographic, parser and writer boundaries independently. Implement required algorithms through reviewed dependencies where appropriate, rather than inventing cryptography. Existing intentional signed-document restrictions remain unless the corresponding preservation path is proven.

## M9 Application integration

Build the production adapter incrementally while each feature lands. Use the real `SessionRequest` union and dispatch behavior, not a parallel collection of success-shaped stubs. Preserve reply types, errors, buffers, generation/revision identities, compare/scratch sessions and cleanup. Keep save-token protection when adapting the existing `saved` request.

Run every applicable application and drive test with the Rust backend, including failures and human-facing controls. Test CSP, sandboxing, no Node globals, disabled PDF JavaScript, close/reopen lifetime and worker crashes. Define bounded cooperative work or safe worker termination and recovery for long operations; the current 60-second termination policy loses unsaved edits and is not production recovery.

Complete actual file dialogs, Save As filename adoption, save-in-place, printing/export and packaging paths. Prove the production build works with runtime MuPDF assets unavailable. Keep independent MuPDF testing outside that runtime build.

## M10 Release gates

Measure on named machines using a fixed, representative benchmark set: open/first-page latency, zoom/render/thumbnail, search, edit, save, reopen, peak memory, retained history, repeated session growth and large-document responsiveness. Freeze numerical budgets after M1 establishes a baseline against the current app, before broad expansion. Record warm/cold results and tail latency. Deliberately address the 16/64 preview limits versus current app limits; do not merely raise limits on an unbounded full-document pipeline.

Run complete current and minimum-toolchain suites, formatting/lints, mutation/fuzz and malformed-input campaigns, differential semantic/render checks, save failure/recovery tests and the complete app drive suite. Review all changes since the last accepted checkpoint. Close regressions and unexplained crashes, corruption, wrong rendering and data-loss failures before replacement.

Verify real packaged builds on macOS ARM64, Windows x64 and Linux x64, the targets in the current packaging script. Include WASM startup, fonts/assets, dialogs, filesystem recovery, printing and high-DPI rendering. Package dependencies, attribution and reproducible build instructions consistently. Archive evidence outside `.stage` before invoking packaging.

## Execution and reporting

Start M0 and M1 next: preserve the current checkpoint, expand the request ledger with legacy test expectations, establish per-operation capabilities, implement the SBI rendering-intent/font prerequisites, and run its viewing acceptance test. Then add the M2/M3 features its actual content requires to complete its edit/save/reopen loop.

After every implementation package, update the ledger with scope, source/build hashes, tests, documents newly supported, regressions, unresolved failures and the next dependency. Use statuses planned, implementing, verified subset and accepted; reserve accepted for the complete named scope with independent review. Never infer percentage complete from test counts, milestone counts or the stress-document page total.

Calendar dates are not committed yet: font/color work, shared-object mutation and security serialization contain unresolved design work. Re-estimate from measured completed packages after M1 and M2. This plan sets the work order and completion gates; it does not start an automation, publish a build or mark the engine complete.

Final completion requires every operation ledger row to be accepted or an explicitly agreed scope exclusion matching the application's behavior, all required workflows passing on Rust, and the M10 evidence complete. A passing small corpus or the SBI document alone cannot satisfy that gate.
