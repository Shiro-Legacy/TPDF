# R20 proposal: pinned Base14 outlines

This is a design proposal, not an implemented or accepted capability. R19 remains
frozen while its independent evidence and full gates complete.

## Recommendation and scope

Generate immutable canonical outlines offline from the same two pinned URW CFF
files already used for M1 metrics. Add safe Rust lookup, PDF binding and G1
placement for exact Type1 Helvetica and Times-Roman declarations. Keep runtime
CFF parsing for a later stage; it requires a separately bounded CFF INDEX/DICT,
Type2 charstring/subroutine and FontMatrix implementation that is unnecessary for
these two fixed assets.

Initial glyph membership is the same 654 exact AFM/CFF shared names per face as
M1, including the explicit .notdef record for verification. PDF selection of
.notdef or an undefined byte remains a refusal. A present, valid empty glyph
(such as space) succeeds with an empty command slice. Do not use Unicode fallback,
name normalization, subset-prefix removal, or another face when lookup fails.
The new capability cannot turn an invalid embedded font into a builtin face.

The default outline policy remains EmbeddedOnly. The explicit AllowPinnedBase14
policy is independent of PageMetricPolicy. A declared-width Base14 font can use
pinned outlines while retaining its exact PDF advances. Conversely, enabling
pinned metric fallback does not implicitly enable builtin outlines. Existing
PP1 errors and default behavior remain authoritative prerequisites.

## Pinned assets and measured size

The source files are the already-pinned MuPDF1.28.1 URW assets:

| Face | CFF bytes | CFF SHA256 |
| --- | ---: | --- |
| NimbusSans-Regular | 33,151 | 366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706 |
| NimbusRoman-Regular | 48,556 | eb609a306b81357853ba91c2a3c47f3bba04f3a8cfdeedc733b1aa4af5d03d4c |

A temporary counting adaptation of the existing pinned native metric oracle
loaded all 654 names per face with FT_LOAD_NO_SCALE, FT_LOAD_NO_HINTING,
FT_LOAD_NO_BITMAP and FT_LOAD_IGNORE_TRANSFORM, and decomposed each outline. It
ran under MuPDF's FreeType lock using its existing linked pinned library:

| Face | Moves | Lines | Quadratics | Cubics | Explicit closing commands | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Helvetica | 1,214 | 6,435 | 0 | 3,925 | 1,214 | 12,788 |
| Times-Roman | 1,215 | 7,508 | 0 | 7,900 | 1,215 | 17,838 |

There are 30,626 commands including one explicit close per contour. A simple
static enum using six i32 coordinates and a discriminant would normally occupy
28 bytes per command, about 837KiB, plus records and names. Budget approximately
1MiB compiled immutable data and a few MiB generated Rust source until the actual
generator reports size_of, exact coordinate ranges and output bytes. This is an
estimate, not a portable layout guarantee or a proven bound from compressed CFF
size. The production generator must reproduce the counts and pin its complete
output hash. Temporary counting files live under /private/tmp/r20-count*; they
are exploratory evidence, not the reproducible acceptance generator.

Existing NOTICE and reference/licenses/urw-metrics preserve OFL notices for the
metric derivation. The new outline data must explicitly retain original URW
copyright/OFL notices and identify the derived generated data. Code remains
AGPL-3.0-or-later. Record original CFF, generator and output hashes in the manifest;
do not represent converted outlines as newly authored fonts or native font files.

## Public interface proposal

The final implementation may refine names, but must retain these distinctions:

```rust
pub enum PageOutlinePolicy { EmbeddedOnly, AllowPinnedBase14 }
// New field, default EmbeddedOnly:
PageGlyphSessionLimits { outline_policy: PageOutlinePolicy, /* existing fields */ }

pub enum PageGlyphSource {
    EmbeddedTrueType(EmbeddedTrueTypeSource),
    PinnedBase14(PinnedBase14Source),
}
// EmbeddedTrueTypeSource preserves current program_id, choice, selection,
// Arc<GlyphOutline>, metric_lsb, origin_shift and hhea/hmtx ranges.
// PinnedBase14Source holds BuiltinFace, a &'static canonical glyph record,
// its native CFF gid and immutable asset identity. No PDF program object exists.

impl PageGlyph {
    pub fn source(&self) -> &PageGlyphSource;
    pub fn embedded_true_type(&self) -> Option<&EmbeddedTrueTypeSource>;
    pub fn pinned_base14(&self) -> Option<&PinnedBase14Source>;
    // code_index/event_index/context_id/font_id/units_per_em/command_range
    // retain common meaning and types.
}

pub enum PageGlyphCommand {
    // Existing variants unchanged in meaning.
    CubicTo { control1: Point<PageSpace>, control2: Point<PageSpace>,
              end: Point<PageSpace> },
    /* MoveTo, LineTo, QuadTo, Close */
}

pub enum BuiltinOutlineCommand {
    MoveTo { x: i32, y: i32 },
    LineTo { x: i32, y: i32 },
    CubicTo { x1: i32, y1: i32, x2: i32, y2: i32, x: i32, y: i32 },
    Close,
}
// Borrowed immutable record; runtime lookup allocates nothing.
BuiltinOutline::lookup(face: BuiltinFace, exact_name: &[u8])
    -> Option<&'static BuiltinOutline>;
// Record getters: name, native_gid, units_per_em, commands, asset_identity.
```

TT-specific getters must not panic or synthesize metadata for builtin glyphs.
Preserve their names and original return types on EmbeddedTrueTypeSource;
compatibility convenience getters on PageGlyph, if retained, return Option.
Update CLI JSON to an explicit source discriminator, with TT metadata only in
the TT branch and asset identity/name/CFF gid only in the builtin branch. The
original font dictionary ID and source/event/context provenance remain common.
CFF gid is an asset-local native glyph index, never a TrueType GlyphId or a PDF
CID by implication.

## PDF binding and loader changes

Extend the existing document-bound GlyphFontLoader instead of creating an
unmetered parallel lookup. Keep its current load(ObjectId) API and TT semantics.
Proposed new entry point:

```rust
pub enum LoadedGlyphFont {
    EmbeddedTrueType(EmbeddedGlyphFont),
    PinnedBase14(LoadedBuiltinGlyphFont),
}
GlyphFontLoader::load_source(font_id: ObjectId, policy: PageOutlinePolicy)
    -> Result<LoadedGlyphFont, GlyphError>;
// LoadedBuiltinGlyphFont owns exact font_id, face and SimpleEncoding.
```

Dispatch using explicitly resolved subtype/declaration, before attempting a
particular capability. Never implement this as catching arbitrary load errors
and retrying builtin. Charge one load attempt and all reference/metadata/name
work under the existing cumulative GlyphLoadLimits. Shared private helpers may
avoid duplicate admission without changing the old load API's accounting.

Builtin binding requires an exact Type1 subtype and exact BaseFont name, a
valid optional FontDescriptor, and absent/null FontFile, FontFile2 and FontFile3
entries. Resolve each entry under the reference ledger: broken references,
streams in the wrong place, malformed dictionaries and meaningful programs
refuse. No font-file decode is needed for a valid builtin source. Existing PP1
validation still rejects malformed declared widths and malformed ToUnicode.

Reuse the loader's bounded explicit named Encoding/Differences reader. Mapping
uses the original byte's exact glyph name independently of the text Unicode
mapping. Missing Encoding or implicit BaseEncoding remains unsupported in this
stage; accepting it would change the existing PP1 scope and needs its own
contract/evidence. Do not call load_builtin_metrics as a routing shortcut: that
capability deliberately excludes explicit Widths and would improperly couple
outline selection to metric fallback.

A font cache entry owns its frozen encoding/byte-to-canonical-record choices.
Resource aliases to the same dictionary reuse it; distinct dictionaries sharing
a BaseFont name retain distinct encodings. Output builtin source descriptors
point only into immutable static data and stay valid after document/session
drop. TT Arc sharing and original program provenance remain unchanged.

## Drawing origin and resource accounting

Canonical coordinates must represent the actual unhinted CFF drawing origin
and FontMatrix. The generator verifies the two pinned faces' units and matrix,
and compares to native page drawing before accepting a simple units-per-em
normalization. If the FontMatrix is not the expected uniform normalization,
encode and apply the verified transform explicitly or refuse generation. Do
not reuse R18's hmtx.lsb-minus-glyf.xMin correction for CFF. Transform canonical
coordinates into em space, then use the existing CodePlacement transform once.
PDF widths govern pen movement independently of outline bounds or native width.

Static asset bytes form a fixed documented compile-time partition. Runtime
encoding slots/name bytes and font cache entries remain admitted under bounded
G0/G1 allocation/cache quotas. Charge all per-code lookups, including failures,
and all output occurrences, cubic commands and transformed control points.
Per-face static record reuse can avoid allocating/copying outlines; it must not
skip per-occurrence transformation/output charges. Record canonical lookup work
separately or include it explicitly in existing lookup_work. A binary search
must charge both comparisons and bounded compared name bytes, with a fixed
maximum asset name length documented by generation. Full requested vector
capacity accounting and failed-attempt reservations retain R18 semantics.

Raster integration adds only the cubic command arm to its existing bounded
flattener. Event order, immutable clip masks, numeric guards, depth/edge/work
budgets and atomic output remain unchanged. There is no separate builtin draw
path that bypasses G1's positions or raster provenance.

## Acceptance evidence

1. Reproducible generator: source/generator/output hashes, complete membership,
   sorted unique names, valid indices/ranges, coordinate extrema, units/matrix,
   empty-glyph classification, closed contours and command/byte totals. Run twice
   and require identical bytes. Independent Rust tests must validate the compiled
   tables themselves (names, ranges, closure and coordinate bounds), since PDF
   budgets do not validate immutable generated data. No runtime/native dependency
   is introduced.
2. Independently query every accepted name in each native face and compare exact
   unhinted commands/origin and M1 advance. Native extraction must use the drawing
   origin rather than a bounds-normalized outline. Missing and .notdef selection
   fail; space remains successful and empty.
3. PDF model tests: explicit width differing from native advance, both independent
   policies, authoritative ToUnicode disagreeing with Encoding, Differences,
   aliases and distinct same-face dictionaries, malformed descriptor/program
   refs, wrong subtype/name, missing encoding and supported-prefix/bad-suffix
   atomicity. Exact cumulative quotas must cover repeated failures and calls.
4. G1-to-raster native/model gates: noncommuting CTM/Tm, horizontal scale/rise,
   crop/rotation/UserUnit, clipping/save/restore, Tr3, mixed path/text paint order,
   valid empty outlines and both faces. Curve and ordinary native AA comparisons
   retain R19's stated limits rather than broad pixel-parity claims.
5. Full corpus records coverage increase and every refusal without candidate
   reduction. Existing TT, PP1 and raster reports/tests remain unchanged in
   meaning. CLI atomicity, comparator mutations, wrapper checks, minimum/current
   toolchains, strict Clippy, formatting and wasm builds remain required.

## Agreed root integration layout (pending R19 checkpoint)

Root will own `builtin_outlines.rs`, its low-level API tests, CLI changes and the
new cubic arm in page_raster.rs. page_adapter owns loader dispatch and page_glyph
source/policy/cache changes plus its owner test migrations. page_oracle owns the
reproducible native asset generator, generated `builtin_outlines_data.rs`, pinned
manifest and native/reference comparisons. page_review owns independent static
integrity, binding and page-model tests, including its existing TT test migrations.

The data module is a private child of builtin_outlines. Each face has a flat
static command array and name-sorted records containing face, exact byte name,
native_gid:u16, command_start:u32 and command_len:u32. Public borrowed record
getters provide commands(), name(), face(), native_gid(), units_per_em() and
asset_sha256() without runtime allocation. lookup(face,name) and glyphs(face)
provide binary search and complete membership inspection. The generator records
per-face coordinate/name maxima and all array/range counts. Root will specify
conservative lookup comparison/name-byte work for the G1 caller to charge.

The outline policy belongs in a lower-level font/source module and is reexported
as PageOutlinePolicy if needed; the low-level loader must not conceptually depend
on page interpretation. Existing TT CLI metadata remains valid for the TT branch;
builtin records have an explicit distinct source kind and no fabricated PDF
program, cmap, metrics-table range or TrueType outline identity.
