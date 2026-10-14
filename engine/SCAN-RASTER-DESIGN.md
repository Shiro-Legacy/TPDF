# R19 rational scan conversion

The scanner accepts explicitly closed line edges on a 1/256 output-pixel lattice.
It emits ordered nonoverlapping spans in the four-times sample grid, whose
centers are 32+64k lattice units. Coordinates are bounded to absolute 2^29 and
sample dimensions to 1..=2^23. Callers own curve approximation, quantization,
implicit closure, page clipping, colors and sample resolution.

Nonhorizontal edges normalize to increasing Y and retain signed winding. The
active interval is half-open [ymin,ymax). Every visible row recomputes exact
rational intersections, sorts using an explicitly metered heap sort, and groups
equal rational crossings before updating winding/parity. Spans are half-open,
using signed Euclidean ceiling division to locate the first sample at or after
each boundary. Offcanvas crossings remain in the winding calculation.

The arithmetic uses checked i64 intersection construction and checked i128
cross products. Coordinates constrain differences to 2^30; intermediate terms
and their sum fit below 2^61. Rational ordering products fit below 2^91. The
sort is independent of standard library sorting implementation details.

A ScanSession retains its intersection scratch and cumulative ledger across
calls and failures. Admission counts every input edge, visible row, inspected
edge, sorting comparison/swap/traversal, grouped crossing and emitted sample.
Limits also cover spans and full scratch allocation requests. Scratch growth
charges the full requested new capacity before fallible reservation, retaining
prior requests in the cumulative total. This describes logical requests, not
process RSS or guaranteed recovery from all allocator failures.

The scan callback is streaming: a later failure may follow earlier callbacks.
Each callback is charged for its complete sample span before invocation; it
should only write into caller-reserved dimension-checked storage. PageRasterSession
keeps that storage private and publishes a page image only after full success.
Unsupported page painting must be refused before pixel allocation. An input
edge list with unclosed contours violates the caller geometry contract; this
low-level scanner does not reconstruct or repair contours.

Independent tests use per-sample signed rational ray casting without production
sorting, intersection spans, or rounding helpers. Page-level tests separately
exercise curve approximation, binary clipping, initial bounds and paint order.
R19 acceptance evidence is recorded in STATUS.md and REVIEW-R19-R1.md.
