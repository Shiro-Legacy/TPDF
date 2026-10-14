# R20 pinned Base14 outlines

R20 adds explicitly selected canonical outline assets for exact Type1 Helvetica
and Times-Roman declarations. The assets are derived from the same two pinned
URW CFF files as M1 metrics. Runtime code is safe Rust and introduces no new
library/native dependency. General CFF/Type1 decoding, other faces, fuzzy naming,
system-font lookup and implicit Encoding remain unsupported by this capability.

## Data and origin

The reproducible generator verifies the pinned CFF/AFM hashes,654 exact shared
names per face, native face identity, glyph indices, units and complete closed
outlines. It independently reads each CFF Top DICT: both omit FontMatrix, whose
specified default is uniform1/1000 with zero translation. Native units are1000.
Actual page drawing-origin comparison is an additional acceptance gate; observing
NO_SCALE alone is not taken as sufficient matrix evidence.

The immutable arrays contain1308records and30626commands:12788Helvetica and
17838Times-Roman. Commands are integral Move/Line/Cubic/Close in the unhinted CFF
drawing origin. No TrueType xMin/side-bearing correction applies. Source glyph
indices belong to each CFF asset and are not PDF CIDs or TrueType GlyphIds.
Valid empty .notdef/space/uni00A0 records remain inspectable in the low-level API;
PDF selection of .notdef/undefined names is explicitly refused.

Generated source uses its own OFL-1.1 notice; implementation code remains AGPL.
Original notices and full font license are under reference/licenses/urw-outlines/.
Manifest: reference/builtin-outlines-manifest.json. The generated source hash is
`d811729c399645d18c013015871b98f9ed22a3f7e2902e84d2c55fa9602ef4dd`.
It occupies2190965source bytes. Native compiled logical data is908842bytes,
including table records, commands and name payloads; this does not claim linker
size, deduplication or RSS. Two fresh generations must produce identical bytes.

## Explicit independent policies

PageOutlinePolicy defaults to EmbeddedOnly. AllowPinnedBase14 enables the two
exact supported Type1 face declarations. PageMetricPolicy remains independent:

| Outline policy | Metric policy | Base14 behavior |
| --- | --- | --- |
| EmbeddedOnly | AllowPinnedBase14 | Pinned advances may support a PP1 trace, but G1 outlines refuse. |
| AllowPinnedBase14 | Declared-only metrics | Valid explicit PDF Widths can advance pinned outlines. |
| AllowPinnedBase14 | AllowPinnedBase14 | Explicit PDF Widths still win; eligible missing widths may use M1 fallback. |

GlyphFontLoader::load preserves embedded-TT semantics. Its load_source entry
point dispatches by declared subtype under the explicit policy; it never catches
an embedded-font error and silently retries another face. Exact BaseFont name,
optional descriptor type and all three FontFile entries are resolved under the
same reference/metadata ledger. Meaningful embedded data, malformed dictionaries,
broken references and unsupported names refuse. Missing/null program fields are
not fabricated font resources.

The original source byte's explicit named Encoding/Differences selects its exact
glyph name. ToUnicode supplies text independently and is never inverted into an
outline selection. Source-byte spacing and PDF advances remain PP1/T0 decisions.
The loader therefore does not reuse M1 fallback routing, which excludes explicit
Widths and would couple two independent capabilities incorrectly.

## Typed ownership and bounds

PageGlyphSource distinguishes EmbeddedTrueType(EmbeddedTrueTypeSource) from
PinnedBase14(&'static BuiltinOutline). Common code/event/context/font identity,
units and command range remain on PageGlyph. TT-only accessors live on the TT
source; builtin metadata never invents a FontFile object, cmap, hmtx range or
TrueType outline identity. Existing TT Arc sharing and source ownership remain.

BuiltinOutline has private immutable fields and allocation-free exact-name
lookup. An owned binary search fixes the comparison-count bound. Generation caps
names at128bytes (actual maximum20); lookup_work_bound includes conservative
comparison/name-byte work. Low-level complete membership inspection includes
.notdef for asset verification, while document binding refuses it.

Each font dictionary has its own frozen encoding and256byte-to-static-record
choices. Aliases to the same dictionary reuse the cache; distinct dictionaries
with the same face retain separate mappings. Every first or failed lookup pays
its admitted search cost, and failed requests retain usage. Cache entries,
choices, loaded names and requested capacities remain in G0/G1 budget partitions.
The canonical asset arrays are a fixed compile-time partition, not repeated
runtime glyph allocations. Cache hits do not skip occurrence/transform charges.

Canonical points divide by1000 and receive CodePlacement's em-to-page transform
once. Cubic control points and endpoints all count toward transformation limits.
PDF Widths never rescale an outline. PageRasterSession feeds the new CubicTo arm
through its existing bounded device-space flattener; clip/order/target/resource
policies and atomic-image publication are unchanged.

## Inspection and acceptance

`--builtin-outlines=Helvetica` or `--builtin-outlines=Times-Roman` needs no input
file and emits exact canonical assets with source identity and pinned advances.
`--page-outlines` inspects all pages in one bounded session;
`--page-outlines=N` inspects one page atomically. These modes explicitly enable
pinned metric and outline policies and emit source_kind with distinct metadata.
Legacy `--page-glyph=N` / `--page-glyphs` keep EmbeddedOnly and their TT JSON shape.
`--page-render=N@DPI` explicitly enables pinned outlines alongside pinned metrics.

Acceptance requires source/generator/asset reproducibility, native checks of all
1308names/30626commands and drawing origin, independently bounded rational page
geometry, source/width/encoding/policy/provenance tests, cache/failure/resource
checks, cubic rasterization and complete corpus/refusal accounting. Preserve
R18 embedded semantics and R19 raster approximation/AA distinctions. Final normal
and minimum-toolchain regressions, formatting, strict Clippy and wasm compilation
are required. REVIEW-R20-BASE14.md and STATUS.md record acceptance separately from
implementation. Full engine and application replacement remain unfinished.
