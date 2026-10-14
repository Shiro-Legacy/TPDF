# Independent review: compressed objects and LZW

Reviewed `document.rs`, `stream.rs`, and `stream_object.rs` after the R4c implementation. Production code was read only; regression tests were written independently.

## Closed finding

`/First` originally acted as an artificial lexical separator for the final object-stream header integer. A header `8 0 9 2` immediately followed by body `900` was accepted as two members with rotation 90, while MuPDF interpreted the joined header token differently and failed to load the rotation object. The parser now checks a real lexical boundary when the last header word reaches `/First`. Independent regression rejects that input and permits an actual delimiter. Member-to-member slices remain bounded: native MuPDF agrees that offsets may separate adjacent numeric member bytes.

## Validation

- Three independent object-stream tests cover the header boundary, native-compatible member slices, and exact shared xref/object-stream metadata, container, and member budgets.
- Nine existing independent revision/xref-stream tests pass with the type-2 expectation updated to malformed-container refusal.
- Four independent LZW tests cover native-confirmed KwKwK and dictionary reset vectors, cumulative nested-filter output, PNG row-tag intermediate accounting, zero-output limits, and padding refusal.
- Targeted Clippy passes with warnings denied.
- A second differential corpus, seed `20261004`, passed 194 cases / 574,287 decoded bytes against native MuPDF and independently encoded source bytes. Both EarlyChange settings exercise widths 9–12; the corpus includes 810 clear codes and nine malformed strict refusals. Report: `/private/tmp/tandem-lzw-independent-review.json`.

The LZW dictionary is fixed-size; new prefixes point backward and decoded lengths are checked before output allocation. Selected compressed members are grouped by effective container, containers are decoded once, and all declared members consume the header/member budget. Structural metadata remains limited to direct parameters and identity or one Flate stage; this intentionally does not promise LZW object-stream bootstrap support. LZW requires initial clear, complete EOD, zero final padding, no trailing bytes, and clear/EOD when the dictionary is full. These strict conditions are compatibility limitations, not universal PDF conformance claims.

No remaining blocking findings were identified in this reviewed scope. This is targeted review and regression coverage, not a security audit or proof of full PDF compatibility.
