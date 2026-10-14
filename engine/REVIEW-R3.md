# Independent R3 review

Scope: strict classic-xref `document.rs`, `pages.rs`, `stream.rs`, the
`--document`/explicit-filter CLI paths, and their resource boundaries. Reviewed
independently from production implementation ownership. No production Rust files
were changed by the reviewer.

## Findings and resolution

1. **Page-content reference amplification — fixed.** A shared Contents array
   could previously be copied once per page, creating multiplicative memory
   growth beyond the page/node limits. `PageLimits.max_total_content_refs` now
   bounds all returned content IDs cumulatively (default 100,000). Arrays are
   checked before allocating a result vector, and each page charges its entries.
   The regression covers a shared array succeeding at the exact total budget
   and failing one entry below it.
2. **Null optional dictionary values — fixed.** Null CropBox, MediaBox, Rotate
   and UserUnit values now behave as absent, preserving inheritance/defaults.
   Direct and resolved null values are covered, along with the fact that
   UserUnit is not inherited. Null root Parent is accepted. Null trailer
   Encrypt/Prev/XRefStm entries are treated as absent instead of causing a false
   encrypted/incremental/hybrid classification.
3. **Reversed rectangle diagonals — fixed by implementation owner.** Page box
   corners are normalized before geometry checks. Degenerate rectangles remain
   refused; the owner's page tests cover normalization.

No remaining blocker found for the reviewed classic-xref document, page
inventory and explicit-filter subset. This does not establish general PDF
compatibility or a complete security audit.

## Independent checks

`cargo test --test review_r3`: **9 passed**.
`cargo clippy --test review_r3 -- -D warnings`: passed.

The independent suite verifies:

- Shared Contents-array cumulative budget and exact-boundary success.
- Optional null inheritance, rotation, crop geometry and UserUnit defaults.
- Null trailer feature entries.
- Raw stream bytes containing NUL/high bytes and fake endstream/endobj/xref/EOF
  text are borrowed exactly from the original input, without interpretation.
- Stream lengths must land on the actual boundary; arbitrary whitespace or
  comment-like payload cannot be silently skipped to find endstream.
- A two-filter chain expands five encoded bytes to 129 intermediate bytes then
  64 final bytes: per-stage limit and cumulative 193-byte limit are enforced
  even though the final output is smaller. Error stage indices are checked.
- 64 KiB Flate output succeeds at its exact limit, fails one byte below, and
  valid empty Flate succeeds with zero output budget.
- 4,000 deterministic arbitrary byte inputs across all four filters (**16,000
  decoder calls**) terminate with bounded output and in-range error offsets.
- Every truncation and five single-byte mutations at every position in a small
  classic PDF fixture exercise document parsing and page collection without
  panic. An omitted final newline is correctly permitted.

The native filter oracle additionally checks all four filters, malformed decoder
errors, known tolerant missing-EOD behavior, and a 16 MiB + 1 decompression output
refusal. Native `fz_read` catches some decoder exceptions internally; the oracle
checks the stream error flag and emits no partial stdout on failure.

## Review limits

Document range partitioning prevents duplicated/overlapping xref offsets from
being parsed repeatedly. Stream payloads remain borrowed; direct or one-hop
indirect lengths must resolve to integers and validate against exact boundaries.
Page traversal is iterative with node/depth/count/queue limits and shared/cyclic
node detection. Numeric value reference chains are bounded separately. Stream
filter chains bound each intermediate output and cumulative output, and enforce
strict end markers/checksums/trailing-data policy.

The tests are deterministic stress/regression checks, not coverage-guided
fuzzing. No allocator-failure injection, full memory-profile benchmark or
independent PDF specification conformance corpus was used. Collection allocations
can still abort on process-wide OOM; logical limits are not such a guarantee.
Repeated public stream-decoder calls require a document-wide budget at the caller.
Xref/object streams, incremental/hybrid documents, encryption, predictors,
rendering and document writing are outside this review.

## Dictionary-aware stream decoding follow-up

Independently reviewed `stream_object.rs` and the `--stream=N:G` CLI path after
they landed. **Seven additional tests** in `tests/review_stream_object.rs` pass;
`cargo clippy --test review_stream_object -- -D warnings` also passes. No blocker
found for the mapper's stated strict parameter subset.

The tests cover indirect Filter and DecodeParms arrays with paired metadata,
chain order and cumulative decoded-byte accounting; exact 64-hop success versus
65-hop refusal; parameter cycles detected before malformed payload decoding;
stream-valued metadata rejection; filter-count preflight before resolving
missing member references; null metadata identity decoding with output limits;
and raw binary stdout plus predictor refusal through the CLI. External-file
streams are already refused by `Document::open`, before the mapper is reached.

The mapper does not silently ignore non-null unsupported DecodeParms. It accepts
only full filter names and null/empty dictionaries or Flate Predictor 1. Its
limits remain per call, with multi-stream callers responsible for global work
accounting. The current CLI decodes one selected stream per invocation.
