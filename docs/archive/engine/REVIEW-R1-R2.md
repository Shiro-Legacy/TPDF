# Independent R1/R2 review

Review scope: `src/lexer.rs`, `src/object.rs`, `src/parser.rs`, `src/main.rs`,
existing lexer/parser tests, and the public lexical/object boundary. Conducted
by the reference-tooling agent separately from implementation ownership. No
production Rust files were edited by the reviewer.

## Result

No correctness blocker found for the documented strict lexical and direct-object
subset. This is not an engine-wide PDF compatibility or security certification.
Whole documents, binary streams, rendering and writing remain outside R1/R2.

The lexer preserves names/strings as bytes, bounds encoded token size and token
count, handles literal nesting iteratively, checks signed integer conversion,
and rejects non-finite real values. Error state is sticky. The parser checks
cumulative object/decoded-byte counts, caps configured nesting depth, preserves
reference lookahead, reports offsets and terminal errors, and gives duplicate
keys an explicit policy. Partial-object spans exclude bytes only inspected by
lookahead, allowing callers to slice the original input safely.

## Independent verification

`cargo test --test review` passed all **8** additional review tests:

- Complete dictionary followed by a binary stream suffix: dictionary span stays
  exact, while complete-object parsing rejects the trailing stream.
- Each possible single byte after a separated integer preserves that integer.
- Indirect-reference lookahead across comments preserves intervening integers,
  gives exact spans, and reparses each returned slice to the same value.
- Exhausting the lexical token budget during lookahead does not erase buffered
  scalar tokens; the deferred error is eventually reported and remains terminal.
- All 256 byte values round-trip through literal octal and hexadecimal strings.
- Extreme real lengths either produce a finite value or an explicit lexical
  error; negative underflow retains its signed-zero result.
- Every prefix of a nested compound object terminates with in-range error or
  success offsets; successful slices parse identically in isolation.
- **20,000 deterministic mixed-syntax inputs** (0–256 bytes) with constrained
  depth, object, token, and decoded-byte budgets terminate with bounded progress,
  finite reals and terminal error behavior.

These complement the implementation tests and the primary agent's differential
harness. This is deterministic adversarial testing, not coverage-guided fuzzing.
No peak-memory benchmark or allocator-failure injection was performed.

## Limits to preserve in later milestones

- `parse_one` is intentionally a prefix parser. Success says nothing about the
  suffix; document callers must validate wrappers and stream boundaries.
- At most two lookahead tokens can exist beyond consumed objects. Their lexical
  byte limit is separate from cumulative consumed decoded-byte accounting.
- Real values retain f64 precision, whereas the MuPDF oracle reports f32.
  Malformed-input recovery and overflow are intentional compatibility boundaries.
- Parser collections and CLI formatting use ordinary Rust allocations. Limits
  bound logical work; they are not a guarantee of graceful recovery from process
  memory exhaustion. The owned object enum can also be constructed manually by
  callers, outside parser depth limits.
- The reference adapter is native C linked to MuPDF. It stays test-only and is
  not evidence that the Rust engine contains no native dependencies if future
  codecs are added.
- API compatibility with one mature engine is only one correctness signal.
  Subsequent document support needs independent fixtures/readers and explicit
  malformed-file policies.

One attribution mismatch in the new NOTICE was corrected during review:
`source/pdf/pdf-lex.c` says 2004–2024; `document.h` says 2004–2025. The Rust lexer
already had the correct lexical-source attribution and was not changed.
