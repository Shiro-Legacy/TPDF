# Functional engine continuation

The user authorized continuing the rewrite on October 12, 2026 until a functional engine is available. Keep the accepted R24 baseline and follow the open, edit, save and reopen plan in /Users/owner/Documents/TPDF/docs/rust-workflow-plan.md. This supersedes the R24-only stop instruction for implementation work. Existing application behavior and fixtures remain the compatibility reference. No installed application replacement, commit, push or publication is implied.

First implementation: an owned bounded session, atomic changes, branch-aware undo and redo, a writer for modified streams, and a native text-edit/save/reopen loop. Retain source-informed AGPL attribution and forbid unsafe Rust. Preserve all frozen R24 evidence. New work and evidence are recorded separately; R24 remains the accepted baseline until the new gates pass.

## Binding dependencies

The separate bridge crate uses pinned wasm-bindgen 0.2.129 to generate safe Rust-facing JavaScript/WASM bindings, and serde_json 1.0.151 for bounded metadata messages. They are needed for actual worker execution; the core engine's runtime dependencies remain unchanged. The bridge and engine source forbid unsafe code, but generated binding glue and dependencies may contain internal unsafe code; this is not an all-dependencies-safe claim. Lockfiles pin transitive versions. The matching CLI generator is installed under the main workspace's .stage/wasm-tools, not globally.

## Snapshot design

The initial session owns serialized PDF snapshots and reparses within each operation, avoiding self-referential ownership. Transaction-local decoded streams overlay the borrowed document; the existing strict writer emits a candidate, and a fresh parser/trace/render validates it before history publication. History is bounded to 64 edits and 64 MiB retained PDF bytes; editable inputs to 16 MiB and 64 pages. Failed operations preserve history. Save receipts carry session identity, unique history node and save sequence. This favors correctness and measurable behavior over caching; it does not claim production throughput or a total RSS ceiling.
