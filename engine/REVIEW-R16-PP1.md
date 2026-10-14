# R16 PP1 independent implementation review

2026-10-03. Scope: document-bound mixed page tracing, resource preparation, cumulative admission, ownership and provenance. This review does not establish glyph rendering, image rendering, complete PDF compatibility, or production application integration.

Disposition: no remaining blocking issue found in the reviewed implementation. The 17 independent tests in `tests/review_page_program.rs` pass. Final whole-crate, compiler-target, CLI and native corpus gates are recorded separately by the coordinating agent; this review does not substitute for them.

## Findings resolved during review

1. The initial joined-buffer allocation refund used twice the final capacity. That is not a valid full-request bound after a capped last growth: requests of 64, 128 and 129 bytes total 321, exceeding 258. The owner added allocation admission/accounting to the page-content loader, reserves a conservative three-times-cap envelope, and refunds only to the measured full requests on success. The independent regression uses 129 empty stream occurrences and checks the measured minimum, including segment storage.
2. Page geometry was initially collected as one `Result`, so one otherwise inventory-valid page with severe cancellation prevented the entire session from opening. The owner now retains one geometry result per page. The regression has valid pages around a distant-origin/UserUnit page whose transform loses extent precision; both valid traces remain available.
3. Root review identified page groups and default device-color overrides as unsupported prerequisites, even when every content operator is recognized. The owner added bounded resource validation. Independent tests verify meaningful Group and DefaultGray/RGB/CMYK refusals, inherited overrides, null values and whole-dictionary Resources replacement.
4. Session inventory allocation had not been identified in the partition description. The module now explicitly states that page/geometry/ID storage is bounded by `fonts.pages` separately from the per-attempt output ledger.

## Independent evidence

Command executed against the frozen source:

```
CARGO_TARGET_DIR=/private/tmp/tpdf-page-review-target /Users/owner/.cargo/bin/cargo test --test review_page_program --offline
```

Result: 17 passed, 0 failed. The review test file also passes `rustfmt --edition 2024 --check`.

Coverage includes:

- Used-only font loading and resolution of every selected Tf, including selections without a show; missing and unsupported selections fail.
- DeclaredOnly default and explicit AllowPinnedBase14 policy; malformed widths never attempt builtin fallback.
- Authoritative ToUnicode with unrelated builtin advances; Unicode remapping cannot change source-byte word spacing or source-code widths. Malformed maps retain failed F0 reservations across retries and never fall back.
- Alias cache sharing, unchanged cache object/metric counts across repeated pages, distinct direct font identity under same-named page resources, and preserved first selection context.
- Cross-stream literal strings and comments, exact joined bytes, synthetic padding projection, repeated stream occurrence identity, original operation indices and decoded-string spans. The output remains usable after dropping the session, document and original input bytes.
- Crop, 90-degree rotation and UserUnit applied once to text and paths, with the initial page bounds clip retained.
- Atomic unsupported suffixes and preservation of an earlier successful output; empty shows/paths and second-child failure remain subject to execution limits.
- Exact and one-below repeated-page boundaries for all five parser partitions; cumulative page/operation/content-reference limits; failed parser and failed content-decode reservations; dropping earlier results does not refund session work.

## Source audit conclusions and limits

Preparation and execution operate on the same retained content bytes. Both parser passes reserve remaining allowance before work; successful parses refund to recorded usage, while failures retain reservations. Preflight resource descriptors preserve the first original operation span. The loader stays document-bound, mutates only during preparation, and is immutably borrowed by the execution registry. Font identity matches are checked before publication.

C0 receives the session's remaining operation/work/output-allocation allowances before execution. Measured C0 usage is charged even on execution failure. The result is constructed only after successful parsing and interpreter finish. Source projection is a nonallocating iterator over retained occurrence segments and does not pretend that decoded-string spans are raw PDF offsets.

The budget is a set of explicit partitions, not a universal allocator or process-RSS limit. The borrowed Document's prior parsing, font/cache limits, inventory under `fonts.pages`, and parser node/payload/token work are separate from the output/orchestration allocation ledger. Conservative reservations may exhaust a session after failed calls even when the decoder/parser failed early. This is deliberate bounded accounting, not measured peak-memory reporting.

Builtin substitution retains the existing narrow M1 rules, including explicit encoding requirements. Used Type0 fonts, Form/image operations, marked content, named color spaces, page groups, default color overrides, unsupported text rendering modes, and disallowed mixed path/text state remain explicit refusals. Native corpus acceptance and generated semantic comparisons belong to the separate oracle gate.

## Coordinated final gate evidence

The coordinating agent reports that the normal and Rust 1.85.1 suites each pass all 538 tests, including these 17 independent tests; minimum-toolchain wasm checking and formatting also pass. Final Clippy results are recorded by that agent in the milestone status.

The completed `reference/page-program-differential-pp1.json` was inspected after the oracle agent reported completion. It records 572 pages from 49 hash-verified original documents: 543 accepted and native-matched pages, 29 classified refusals, and zero candidate reductions. The accepted population comprises 500 `large.pdf` stress pages and 43 other pages, totaling 223,657 source codes and 225,157 semantic events. The separate 220 generated real-PDF cases all match both native execution and the independent rational C0 model with the page transform applied once (2,200 source codes). The oracle agent additionally reports 26 comparator/self-checks passing.

The oracle preserves the previously declared P0/T0 numeric tolerances and exact source-code, Unicode, event-order and clip-topology comparisons. Native batching normalization and repeated-move/equal-cubic path normalization remain documented. Native invisible-text callbacks do not expose color, the initial page clip is compared as bounds metadata, and native resource alias/font identity is not recovered. Those properties receive independent Rust assertions where applicable; none establishes glyph selection, outlines, annotations or pixel parity.
