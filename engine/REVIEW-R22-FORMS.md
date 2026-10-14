# R22 independent Form implementation review

## Current disposition

The implementation has no remaining independent source-review blocker. All 26
independent Rust tests and strict all-feature Clippy on their target pass. Final capability acceptance is complete as of 2026-10-06; the reference comparisons and full root toolchain gates pass. The design invariants in R22-FORM-REVIEW.md and the R22 contract
remain required. Earlier accepted R21 reports and tests remain frozen.

## Independent test coverage

`tests/review_page_forms.rs` constructs original PDF files, including complete
xref tables and separate Form streams. It uses public document/page APIs rather
than constructing the production traversal's private invocation plan. Its 26
cases cover:

- A caller-selected font inherited independently of the Form's replacement
  resource scope, local same-name Tf selection, and subsequent caller restoration.
- Repeated calls to the same resource-less child under A/B/A providers; missing,
  null and explicit empty Resources; category-level replacement; direct font
  identities in distinct providers; used Tf without visible/showing output.
- Noncommuting caller and Form matrices, original transformed BBox sides, clip
  ancestry, color/text-state restoration, and an independent 4x sample calculation
  that distinguishes a sheared BBox from its AABB and checks subsequent paint.
- q/Q frame floors, local graphics/text/path balance, active-ancestry cycles versus
  completed aliases/reuse, lazy unused malformed Forms, meaningful alternate
  semantics and malformed geometry.
- Global operation origins, synthetic Enter/Leave boundaries, local byte spans,
  repeated-source identity versus invocation identity, nested callsites and
  cross-page cache entries whose owned stream/scope IDs must remain resolvable.
- Decoded ASCIIHex source provenance, indirect-reference aliases to one retained
  stream, active cycles through reference chains, and cached stream size checks
  under a stricter later per-page limit.
- Owned output and diagnostic lifetime; a full 64-Form diagnostic chain; the
  structural 65-depth refusal; no-Do pages' lack of diagnostic allocation and
  preservation of their original operation numbering.
- Exact and one-less admission for all eleven Form counters; logical invocation
  charging once across two passes; repeated failures/replays; per-page/session
  cap intersection; parser consumption on both sides of suspended child calls.

The parser checkpoint types are private. Five owner unit tests complement these
observable integration tests with source allocation/length/shift identity, sticky
lexer/parser failure, exhausted operation/token/node/payload quotas and repeated
resumption without a cursor or accounting reset.

## Independent exact model

`tools/form_review_model.py` consumes a fixture-author supplied operation graph,
not a production flattened trace. Exact Fraction arithmetic composes the original
matrices, tracks selected font binding independently of scope, saves/restores
state, applies BBox and explicit clip nodes, and records original invocation and
source-local provenance. The PDF encoder supplies byte spans from its original
streams; the model replaces each Do with Enter and appends Leave without rewriting
PDF bytes. It computes simple-font advances from fixture widths and separate text
parameters. It never obtains its expected geometry or state from Rust output.

The model limits operations, invocations, structural depth, codes, path commands,
clips and rational size. It refuses missing local bindings, cycles, frame-floor
underflow and unfinished state. These are a declared reference subset, not a
second complete PDF parser or renderer. Explicit malformed metadata is checked by
separate fixture/reference validation; ignored metadata must not be presented as
accepted by this model. Every model refusal is observable, never an outside sample
or silently omitted event.

The saved `reference/form-review-model-selfcheck.json` records 15 exact
state/affine/provenance identities, four frame refusals, five zero-budget refusals,
two scope/cycle refusals, six exact-limit checks and five one-less checks. Model
source SHA256 is
`148dc79eee87fcebc80f952624844631b413a8a8fd25507ff1b7511fc7363db7`.
The native gate records its deliberately broader outer-resource fallback separately:
an explicit replacement dictionary missing a requested name can be a strict Rust
refusal even when the pinned native engine accepts it. Do not count that as native
refusal parity.

## Source review and resolved findings

The C0 boundary hook uses the original four BBox corners, avoiding cancellation
from reconstructing a far corner as origin plus rounded width. Internal typed
operations retain child work/allocation charges under one public boundary index.
The interpreter keeps its own Form save floor, and failures are sticky. The opaque
frame token is private to the traversal; its execution-index identifier alone is
not a cross-interpreter authentication mechanism, so tokens must remain with their
own interpreter by construction.

The scoped resource loader distinguishes current effective scope, Form object and
Resources provider. A review of metadata lookup work prompted charging fixed-key
byte-comparison bounds rather than counting only dictionary entries. C0 also now
precharges the selected-font table scan on every resolved Tf, in addition to
checking/charging its original name; indexed selection cannot hide repeated
linear searches behind constant dispatch cost.

The first independent run exposed remaining page-only validation in scoped metrics
helpers: explicit Form resources failed as NotPage even after mapping resolution
had succeeded. Removing that inappropriate page check from the registered scoped
helpers fixes local font shadowing, direct Form dictionaries, repeated inherited
scopes and cross-page caches. Public page-only entrypoints retain their membership
checks.

A further source audit found first-Do decode failures lacking the original outer
span/index, and C0 entry failure's synthetic 0..0 span being incorrectly labeled
as caller stream coordinates. Decode failures now receive the original Do location;
entry failure preserves the internal synthetic ProgramError while the enclosing
page error reports the actual caller Do. Nested diagnostics run deepest-first,
with every record carrying its parent object and local callsite. Their owned chain
is retained without growth, including the full 64-frame failure case. These cases
have dedicated regressions; they are not waived comparison differences.

Preparation alone charges logical attempted Do invocations, including failed
resolution. Replay checks the prepared original callsite and origin sequence and
pays its own parser, lookup and interpreter work. Every suspended checkpoint points
to unchanged bytes held by the retained immutable Arc allocation. Only a completed
successful parser phase refunds unused conservative reservations; nested failure
retains all still-active reservations. Metadata/source caches retain validated
inputs only, and repeated invocations execute under their current caller state.

Result-local source/scope IDs are remapped from session caches, so a second page
can outlive the session and resolve its own IDs after unrelated prior cache entries.
The decoder's allocation envelope is a conservative full-request charge, not a
process-RSS claim. Arc storage, orchestration buffers and fixed-capacity diagnostics
have distinct owner partitions. Per-page and cumulative limits intersect; decoded
cache hits still enforce the current per-stream size limit. Both a 64-deep valid
chain and its malformed deepest suffix are covered, with a hard refusal at 65.

The exact Form counter tests pass for all eleven partitions at measured usage and
fail one below. There is no refund/reset on caller retry, no cached painted result,
and no partial Program/image publication after a failed suffix. Review model and
Rust tests are frozen pending reference evidence or a concrete new blocker.

## Final gates — completed 2026-10-06

R22 ordinary Form XObject execution is accepted after the complete native/model/corpus gate. Scoped resources and fonts, nested Forms, affine Matrix/BBox clipping, state restoration, owned provenance and cumulative failure budgets are verified. These are reusable drawing streams, not interactive AcroForm fields.

- Rust 1.98.1 and minimum 1.85.1: **724 tests pass each**. Formatting, strict all-target/all-feature Clippy and minimum-toolchain wasm32 library compilation pass.
- **58 positive generated Form graphs**, four explicit native-fallback/strict-scope differences and ten malformed/cycle/state refusals pass. **66 generated renders and 10 independent pixel models** pass with zero certified channel-interval violations.
- **545/572 corpus pages render at 18 DPI**, with 27 explicit refusals; all 500 stress pages pass. All **48 supplementary 72-DPI renders** pass. The two new Form-only pages render at both resolutions. All **589 previously accepted render hashes remain identical**; all 29 old refusal pages receive a Form-aware census.
- Reference validation: 23 native comparator mutations, 25 Rust comparator mutations, 13 native process faults, six Rust process faults, three actual native negative-wrapper cases and eight pixel/baseline mutations are rejected as expected.
- The new `tools/verify-form-gate.py` requires exact corpus identities/counts, all generated/model/census coverage, preserved hashes and current evidence hashes. Ten deliberately shortened or corrupted reports are rejected. It prevents a successful `--skip-corpus` or `--native-only` run from being treated as final acceptance.

Native AA/edge differences and the existing source-certified 229-versus-230 color policy remain diagnostic. No tolerance or policy was widened. This is a bounded Forms milestone, not complete PDF rendering or application parity.

