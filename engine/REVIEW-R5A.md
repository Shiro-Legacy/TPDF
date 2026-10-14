# Independent R5a object-serialization review

Reviewed `src/serialize.rs` and the `--canonical` CLI path. This scope serializes
owned direct object values; it does not save, edit, sign or rewrite PDF documents.
No production source was modified by the reviewer.

No correctness blocker found for the stated subset. The writer bounds encoded
output before extending its buffer, counts dictionary keys in its object budget,
uses a hard recursive container ceiling of 256, preserves binary values with hex
strings/name escaping, rejects null name bytes and nonfinite reals, and returns
no partial buffer on failure. BTreeMap ordering yields deterministic dictionary
output. Integral reals retain a decimal point and negative zero retains its sign.

`cargo test --test review_serialize`: **6 passed**.
`cargo clippy --test review_serialize -- -D warnings`: passed.

Independent tests cover exact expected wire syntax (including empty names,
delimiters/high bytes, neighboring integers, max-range references and typed
reals); **12,282 finite float patterns** spanning every finite exponent band,
with wire grammar checks and Rust's independent numeric conversion rather than
the engine's own PDF parser; exact escaped-name expansion/dictionary-key budgets;
a manually built 256-level container success and 257-level hard refusal; unchanged
owned input across repeated error paths; and canonical CLI bytes/no partial
stdout on malformed input.

This complements the owner's 5,000 object round trips and 1,000 generated-tree
MuPDF lexer/idempotence comparisons. Full f64 formatting deliberately exceeds
MuPDF's float32/recovery range; successful Rust round-trip is not a promise that
all PDF consumers preserve extreme values. Output allocations are bounded, but
formatting tiny scalar values uses ordinary temporary String allocations; no
allocator-failure injection was performed. Manually constructed trees beyond the
parser's depth limit remain the caller's responsibility when later dropped.
