# Independent review: bounded document rewrite

Reviewed `writer.rs`, shared serializer changes, effective trailer iteration, and CLI buffering. The writer retains every effective live object, object IDs and generations, original header version, and raw stream bytes. Stream lengths become direct integers. The new classic xref is sparse, avoiding allocation based on the largest object number. Object/xref stream containers remain as dormant objects so references are preserved.

No blocking production finding was identified. Six independent tests pass:

- Cyclic references survive without recursively expanding the graph, and output is deterministic.
- A manually counted nine-object syntax budget includes the generated trailer; eight fails. Exact output size succeeds and one byte less fails.
- Indirect signature fields and type-reference cycles fail closed.
- Binary stream bytes containing signature-like text remain literal, byte-for-byte data.
- Missing references in unreachable live objects report the correct object number and generation.
- CLI rejection writes no partial PDF to stdout.

Targeted Clippy passes with warnings denied. These tests supplement the implementer's tests and native reopen corpus; they are not solely a serializer/parser roundtrip argument.

The native document oracle was corrected to query `pdf_was_repaired` after loading and bounding all pages. Its rectangle storage is bounded by 10,000 pages, and stdout errors are checked. An object-stream smoke file reopens with three expected pages and `repaired:false`. This reports repairs triggered by document/page loading; it does not claim that MuPDF eagerly validated every orphan object or decoded every raw stream.

Rewrite remains a conservative experimental operation returning a new buffer. Unknown meaningful historical trailer entries, linearization, signature markers, unresolved references, and resource-limit excess are refused. It does not preserve signatures, garbage collect, render, or overwrite existing files.
