# Independent R4 review

Reviewed classic `/Prev` revisions and xref-stream loading in `document.rs`,
TIFF/PNG predictors in `stream.rs`, and dictionary parameter integration in
`stream_object.rs`. No production Rust source was changed by the reviewer.
This review extends R3; object streams, hybrid xref, encryption, rendering and
PDF document writing remain outside the reviewed implementation.

## Result

No remaining correctness blocker found for the documented R4a/R4b/predictor
subset. Newest-first xref merge is keyed by object number and retains free-entry
tombstones. Current live values are parsed within physical regions that include
old object offsets and historical metadata boundaries; superseded malformed
bodies are not parsed. Root inheritance is distinct from the raw newest trailer.
Revision and scanned-entry budgets apply across the entire chain.

Xref streams validate direct metadata, widths/ranges, exact decoded row lengths,
bootstrap self identities and physical metadata boundaries. Sparse omission of
the stream's own entry only registers its verified bootstrap object; conflicting
self entries fail. Metadata ownership copies are charged before cloning. Type-2
compressed entries and hybrid sections fail explicitly.

Predictor dimensions use checked arithmetic; packed TIFF operations keep color
history within a row and clear sub-byte padding. PNG byte-level operations use
the actual row tag and prior decoded row, without a separate large row scratch
allocation. Filter/predictor outputs share one cumulative budget. Malformed or
partial rows fail without returning partial output.

## Independent verification

- `cargo test --test review_r4`: **9 passed**.
- `cargo clippy --test review_r4 -- -D warnings`: passed.
- `cargo test --test review_predictor --test review_stream_object`: **12 passed**
  (five new predictor tests plus seven earlier mapper regressions).
- `cargo clippy --test review_predictor --test review_stream_object -- -D warnings`:
  passed.

The revision tests cover stale invalid bodies replaced by valid newer objects;
old borrowed streams separated from updates by historical xref/footer data;
newest free tombstones; generation replacement; inherited Root while retaining
raw newest trailer; exact revision and total scanned-entry budgets; physical
metadata offsets; self/forward Prev refusal; sparse bootstrap registration;
compressed/unknown/conflicting self entry refusal; and cumulative metadata
accounting across two predictor-compressed sections.

In that last regression, each section inflates seven bytes then produces six
PNG-predicted bytes. Two sections succeed with **26** cumulative metadata bytes
and fail with **25**. Their implicit bootstrap entries also count toward the
xref-entry budget (nine succeeds, eight fails).

The predictor tests use independently hand-computed packed 1-bit/4-bit and
16-bit TIFF examples, per-color history and row resets, all five PNG row modes
under declarations 10–15, byte-wise packed PNG behavior, partial/invalid row
refusals, and preflight validation of a later filter before earlier decompression.
A Flate+PNG+ASCIIHex chain produces six intermediate, five predicted and two
final bytes: budget 13 succeeds and 12 fails at the final stage.

## Global metadata budget analysis

`decode_xref_metadata` currently supports identity or a single Flate stage,
optionally followed by prediction. Its cost calculation is exact for this
restricted shape: final bytes for identity/plain Flate; twice final bytes for
TIFF; twice final bytes plus row count for PNG. Strict complete-row validation
makes this equal to the actual inflated intermediate plus transformed output.
Both the decoder's per-call cap and the persistent document-wide charge apply.
If metadata filter chains are later broadened, return an explicit decoder usage
report rather than inferring arbitrary intermediate sizes from final output.

## Remaining verification limits

The owner's reference comparison reports all 49 unencrypted existing fixtures
matched across 572 pages, with five encrypted fixtures explicitly unsupported.
That corpus result complements these generated adversarial fixtures but is not
general compatibility proof. Object streams and hybrid xref need separate
precedence, recursion and ownership gates. Native MuPDF's tolerant partial-row
behavior is deliberately not copied. The deterministic tests and reference
comparisons do not replace coverage-guided fuzzing, allocator-failure injection
or a full peak-memory profile. Logical limits bound work and allocations; process
OOM is not guaranteed to become a recoverable Result.
