# Independent R6a review

Reviewed content grammar, decoder usage reporting, page-content joining, typed local operator validation, and CLI buffering. No remaining blocking finding was identified in this bounded read-only scope.

## Closed findings

- Page joining initially requested exact vector growth for each part. That can repeatedly copy the full accumulated prefix when many small streams are joined. The loader now grows geometrically while capping requested capacity at the joined-byte limit. The implementer added a 10,000-member test.
- `ValidatedOperation` initially exposed a mutable public classification field. Callers could change the validated kind while keeping unrelated operands. Its kind is now private with a read-only accessor, preserving the classification invariant.

## Independent tests and comparisons

Sixteen added tests pass: seven grammar tests, five page-loader tests, and four typed-operator tests. They check byte spans, no file-reference inference, immediate BI refusal before malformed binary, sticky failures, dictionary-key charging, cumulative budgets after previous operations are dropped, hard depth caps, source inventory mismatch, exact padding and segment offsets, repeated encoded input, predictor intermediates, borrowed binary text, nested text-array rejection, pattern color shape, and range edges. Targeted Clippy with warnings denied and formatting checks pass.

Native comparison results:

- 300 generated content programs plus 590 complete real streams: 890 matched inputs and 31,221 operations. Fourteen strict malformed/inline-image cases refuse without stdout. See `reference/content-differential-r6a.json`.
- All 572 pages in 49 supported fixture documents plus seven generated boundary cases: 579 byte-exact native joins, totaling 2,543,602 compared bytes. See `reference/page-content-differential.json`.
- Comparison tools record binary hashes, validate immutable fixture/content hashes, and reject a run if a binary changes during comparison.

MuPDF inserts spaces after every Contents-array member, including final and empty members. This preserves comment continuation and adds a literal space inside strings spanning parts. A direct single stream has no pad; an array containing one stream does. The loader preserves this distinction and separates decoded source ranges from synthetic padding offsets.

## Scope

The content grammar is separate from indirect-object parsing and cannot construct references. Unknown ordinary ASCII operator words survive grammar parsing; typed validation can reject them. BI, stray ID/EI, file-structure words, duplicate dictionary keys, and dangling operands are refused. Typed validation checks local arity/types/ranges; it does not execute graphics or text state, interpret compatibility sections, resolve resources, fonts, or color spaces, expand Forms, render, or extract text. Inline-image data is only opaque bytes in page loading and is explicitly unsupported in grammar parsing. Native token comparison permits documented float32 rounding tolerance. These are targeted regression and compatibility checks, not a full security audit.
