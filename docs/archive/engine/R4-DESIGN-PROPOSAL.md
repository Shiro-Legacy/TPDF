# Proposed R4 read-only architecture (not an implementation)

R3 fixture evidence: 37 supported fixtures, 560 matched pages, 17 explicitly
unsupported and zero unexpected failures. Of the unsupported fixtures, 10 use
classic incremental revisions, two use xref streams and five are encrypted.
The highest-value next unit is therefore **R4a classic `/Prev` revisions**, then
**R4b xref streams**, with object streams a separately gated **R4c**. Encryption,
hybrid `/XRefStm`, repair heuristics and writing remain unsupported initially.

## Shared foundation

Separate three operations now interleaved by `Document::open`:

1. Discover and parse cross-reference sections plus their trailer/footer spans.
2. Merge an effective index newest-first, keyed by **object number**.
3. Parse only effective live object values using validated physical boundaries.

An index entry should distinguish `Free { generation, next_free }`,
`Direct { generation, offset }`, and later
`Compressed { container_number, index }`. Free entries must remain in the merged
index: removing them resurrects stale objects from older sections. Generation is
part of reference validation, not the merge key. A newer generation replaces
an older generation even if the requested reference names the old generation.

Maintain one document budget throughout discovery, merge, decoding and parsing.
Add explicit revision count, **total xref entries scanned across all sections**,
metadata decompression bytes, object-stream count and compressed-member count.
Do not equate winning entry count with scan work. Keep existing input, syntax,
owned-object, decoded-string, parser-depth and page limits. Account filter stage
outputs cumulatively; account retained owned values separately from temporary
metadata buffers so memory claims remain precise. No budget resets per revision
or object stream. Charge work before allocation or an expensive decode.

## R4a: classic incremental revisions

Start from the final validated footer and follow direct integer `/Prev` offsets.
Require older offsets to be strictly smaller, inside the file and outside known
payload/metadata regions. Keep a visited-offset set and revision limit anyway.
Parse each section at its exact offset; do not skip arbitrary bytes looking for
`xref`. Validate each section's local subsection ranges and duplicate entries,
its trailer and associated startxref/EOF footer. Reject non-null `/XRefStm` and
any actual encryption entry. Trailer `/Root` inheritance/requirements must be
explicitly specified and tested; do not merge section-local `/Prev` or `/Size`
like ordinary document metadata. Retain the latest complete trailer for callers
and make any inherited Root resolution visible in the implementation contract.

Merge newest-first using insert-if-absent, including Free tombstones. Bound the
number of encountered entries as well as the final map size. Validate the newest
`/Size` against selected object numbers, but do not allocate a vector from Size.
Repeated offsets for the same object across revisions are normal. Distinct
winning objects aliasing a physical offset are not.

The current `object.region = offset .. next_live_offset_or_latest_xref` rule
cannot survive incremental updates: an older live object can be followed by an
old xref table/footer and newer revision objects. Collect physical boundary
markers from **all discovered direct entries**, including stale overwritten
ones, and from each known xref/metadata/footer section. Parse an effective live
object only up to the earliest subsequent boundary. Validate its wrapper,
stream Length and actual endobj there; do not parse stale object bodies merely
to validate otherwise irrelevant old content. Strict gap policy should allow
only trivia in unclassified gaps and must distinguish known old metadata spans
from arbitrary bytes. Reject incompatible/overlapping selected object spans.
Never use regex scans for endobj/endstream to infer boundaries inside payloads.

Gates: multi-revision Root/page changes; a stale malformed object superseded by a
valid newer object; newer free tombstones; generation replacement; unchanged
old objects separated from new ones by old xref metadata; repeated sections,
cyclic/forward/out-of-range Prev; subsection duplicates; aliased/overlapping
selected offsets; global-budget exhaustion spread over many tiny revisions.
Re-run all 54 existing fixtures and require the ten classic incremental fixtures
to match MuPDF page count/geometry rather than silently moving failure categories.

## R4b: xref streams

Bootstrap the indirect dictionary/stream at a startxref or Prev offset without
requiring a pre-existing object index. To avoid circular resolution initially,
require direct Length, W, Index/Size, Filter and supported DecodeParms metadata.
Explicitly reject unsupported indirect metadata rather than guessing boundaries.
Use exact Length and endstream/endobj checks; decode under the same global
metadata budget. Reuse the bounded filter layer, not an unbounded convenience
inflate function. Xref streams must not be encrypted.

Validate `/W` as exactly three nonnegative widths, each at most eight bytes, with
a checked nonzero row size. Decode big-endian fields with checked conversion.
When W[0] is zero, entry type defaults to 1; other zero-width fields default to
zero. Index defaults to `[0 Size]`; otherwise require an even, bounded list of
nonnegative range pairs, no overlapping object ranges, and checked sums/products.
Require exactly the expected decoded byte count before reading entries. Accept
only supported entry types; never turn unknown types into an implicit live entry.
Type 2 can remain explicit Unsupported until R4c.

A xref-stream object is both metadata and an indirect object. Register the
already validated bootstrap object once; do not parse/decode it again through
the ordinary live-object pass, or let its bytes become trailing data for an
adjacent older object. The two current stream fixtures use plain Flate,
`W [1 4 1]`, sparse Index arrays and Prev pointing to an older classic table.
They do not need predictors or object-stream decoding.

**Observed compatibility detail:** these two fixtures omit the xref-stream's own
object from Index. MuPDF explicitly installs that bootstrap object in
`pdf_read_new_xref`. A bounded compatibility policy can register its verified
wrapper ID and physical offset when absent, while rejecting conflicting explicit
entries. Record this policy instead of inventing ordinary missing object entries.

Gates: mixed classic/stream Prev chains, zero-width fields, sparse Index, exact
row byte counts, huge/overflowing widths/counts, missing and conflicting self
entries, malformed compressed input, metadata decompression bombs and repeated
section references. The two existing certified update fixtures should then match.
Predictors can remain explicit Unsupported pending their own defined gate.

## R4c: compressed object streams

Group winning type-2 entries by container. A container must resolve to a live,
uncompressed indirect stream with Type ObjStm; reject recursive/compressed
containers. Decode each needed container once under the document-wide budget.
Track Loading/Loaded states to reject cycles if resolution becomes lazy.
Require direct nonnegative `/N` and `/First` initially. Validate N before allocating
header tables, parse exactly N `(object_number, relative_offset)` pairs within
First, and permit only trivia between the header and First. Require unique valid
member numbers and strictly increasing, in-range member offsets. Checked-add
First to each offset and bound each member by the following offset or stream end.

For each winning type-2 entry, verify that its index is in range and that the
header object number at that index equals the requested number. A compressed
member has generation zero. Parse exactly one direct value from its bounded
slice and reject wrappers, nested stream syntax and nontrivial suffix data.
Only materialize selected members; stale or superseded members must not override
newer entries. Owned Objects can outlive a temporary decoded container buffer,
so no self-referential Rust ownership is needed. Drop the decoded buffer after
selected members have been owned, or use an explicitly bounded cache.

Gates: type-2 index/header mismatch, duplicate member IDs/offsets, cyclic container
references, huge N/First, member overlap, generation constraints, later free/direct
replacement of compressed members, many references to one container without
repeated decode, and cumulative budgets across many individually small streams.
Add generated fixtures plus independent MuPDF comparison before claiming this
capability; the current two unsupported xref fixtures do not demonstrate it.

## Deliberate first exclusions

Keep hybrid `/XRefStm` Unsupported. It combines same-revision table/stream
precedence with Prev traversal and deserves dedicated conflicting-entry tests.
Retain explicit Unsupported for encryption and predictors until separately
implemented. Read-only support for signed PDFs makes no claim about signature
validation, permission authorization, editing safety or preservation on save.
