# M1 independent review

Reviewed `builtin_metrics.rs`, generated table provenance, and the separately lazy `FontResourceLoader::load_builtin_metrics` path. No remaining implementation blocker was found in the authorized exact Helvetica/Times-Roman subset.

Four independent regression tests pass in `tests/review_builtin_metrics.rs`:

- Two fonts sharing one Encoding dictionary preserve their distinct face advances (fi: 500 versus 556), while an alias reuses its font identity cache without recharging encoding work.
- F0 and M1 share the cumulative Differences name budget. An M1 failure after a successful Unicode load retains its processed-slot charge, publishes no builtin cache entry, and leaves the prior Unicode cache intact.
- Every verified glyph name absent from the Unicode registry still resolves its exact builtin advance. This tests that metric selection does not depend on Unicode interpretation; more than 100 such names per face are exercised.
- Dangling and non-null embedded-program declarations fail before processing builtin slots or encoding. Descriptor reuse as a program declaration cannot bypass embedding refusal.

The implementation resolves bounded references, validates exact Type1 and BaseFont declarations, refuses non-null explicit Widths or orphan endpoints, and checks all three embedded-program fields. Null fields are treated as absent. Successful M1 results retain 256 slots; failed selection/encoding cannot publish a partial result. Existing M0 and Unicode caches remain independent. Cache hits still charge calls, attempts and resource selection/reference work.

Tables are sorted exact byte names and retain source AFM/CFF hashes. Only the verified 654-name intersection per face is exposed. Undefined slots and explicit .notdef remain UndefinedGlyph; missing names remain MissingBuiltinMetric. MacExpert has 23 advances, 91 undefined slots and 142 missing metrics per face. Low-level explicit .notdef glyph queries are intentionally distinct from byte-code lookup. The generated data keeps its OFL notice separately from engine code; no runtime font programs were added.

Validation: four independent tests and targeted strict Clippy pass in the isolated `/private/tmp/tandem-review-builtin-target` target. Parent owns the complete 1,308-name native table comparison and 48-font fixture differential; this review does not claim font-program parsing or general Base14 substitution support.

The completed CLI/native gate is recorded in `reference/builtin-metrics-differential-m1.json`, reproducible with `python3 tools/differential-builtin-metrics.py`. It freshly requeries 1,982 native glyph rows, compares all 1,308 full table entries, tests all 402 excluded AFM-only names, and covers all eight named face/encoding combinations plus deterministic Differences (192 cases / 49,152 slot outcomes). All 37 invalid CLI inputs refuse with diagnostics and empty stdout. Every original fixture hash is verified, including encrypted files before their explicit corpus exclusion. All 49 unencrypted documents, 572 pages and 582 bindings match: 48 eligible builtin identities / 12,288 slot outcomes / 570 loaded bindings; five explicit-width fonts and seven Type0 bindings remain distinctly refused. Binary and evidence hashes are unchanged across the run. No failures.
