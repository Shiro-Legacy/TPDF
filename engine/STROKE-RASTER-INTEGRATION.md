# R21 stroke raster integration

Implementation is frozen with 684 current/MSRV tests, strict Clippy, formatting
and MSRV wasm compilation passing. Independent source/geometry tests pass; final
acceptance awaits native/corpus reference gates. The reviewed geometry/numerical proof
is in R21-STROKE-PROPOSAL.md and R21-STROKE-REVIEW.md.

## Event preparation and ownership

The existing mixed PP1 event order remains authoritative. Whole-page preflight
checks stroke color and constant-size solid style/dash consistency under a fixed
eight-unit logical work charge before geometry or pixel allocation. The stroke
error is wrapped with the original global event, operation and source span.

Preparation passes the original UserSpace path, saved StrokeStyle and matching
DashPattern to StrokeSession::expand. The saved user-to-page transform composed
with the raster target is supplied only for stroke admission/error control. The
owned result remains UserSpace Move/Line/Close components. Existing user_path
applies that map once, quantizes at1/256pixel and builds checked scan edges.

All components of an event become one nonzero paint with its saved stroke color
and clip identity. No new clip stack or separate blending pass is introduced.
Existing fill-then-stroke operators retain their ordered events. Geometry for
every event must succeed before sample, clip-mask or output pixels are allocated.
Any unsupported style, geometry, quota or scanner error returns no image.

## Resource partitions

PageRasterLimits.stroke is a whole-page expansion cap. The session's raster.stroke
field additionally limits each page request; PageRasterSessionLimits.stroke is
the whole-session cumulative cap. Every page creates one temporary stroke session
under the component-wise intersection of these two page caps and the cumulative
session remainder. Every event uses that same local session.

After preparation/execution returns, its admitted stroke counters are merged into
PageRasterSession::stroke_usage even on failure. This includes work spent on a
valid prefix when a later stroke or scanner fails. Checked remaining/intersect/
accumulate helpers preserve structural depth policy and avoid wrapping or partial
ledger updates. Rejected quota increments retain the previously admitted value.

PP1 source storage, stroke temporary/output storage, raster planning and scanner
work retain separate ledgers. Raster source_commands includes the expanded commands
passed into user_path; those commands are charged independently by the stroker
when emitted. Full requested vector capacities, rather than only capacity deltas,
remain allocation charges. These limits describe logical work/requests, not RSS.

## Focused verification

Root integration tests cover exact stroke color/clip/paint order, analytic samples
under shear/unequal scale/reflection, failure after a successfully expanded prefix,
repeated-failure accounting and both page/session stroke caps. CLI tests verify
exact solid-stroke PPM colors/dimensions at72/144DPI and no output for unsupported
hairline suffixes, including fill-then-stroke. Earlier filled-page refusal tests
use an explicitly unsupported stroke style so their atomicity assertions survive
the new supported capability. Independent geometry/source tests and full fixture
gates are separate; see the final R21 review/reference reports when completed.
