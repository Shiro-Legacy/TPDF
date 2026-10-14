# Bounded path and graphics-state interpreter proposal

This is a read-only design. It proposes a standalone interpreter that turns validated content operations into immutable path-paint and clipping events. It is not a rasterizer, stroke expander, page renderer, font engine or complete PDF graphics implementation.

## First useful stage: P0 path display lists

Consume the accepted content parser, `operators::ValidatedOperation` and tagged finite `geometry` types. Return an owned, independently inspectable display list containing original path geometry, paint transforms, fill rules, device-color values, line state and exact clipping ancestry. This is useful as an execution boundary that can later feed a rasterizer, without pretending that an axis-aligned path bound is the painted or clipped region.

Supported initial operators:

| Group | Operators / behavior |
| --- | --- |
| Graphics save/restore | q, Q with strict stack balance |
| Transform | cm with checked affine composition; restricted while a path is active |
| Stroke style | w, J, j, M, d, preserving zero-width hairline and exact dash data |
| Paths | m, l, c, v, y, h, re |
| Painting / path end | S, s, f/F, f*, B, B*, b, b*, n |
| Pending clipping | W, W*, applied when a path-ending operator executes |
| Explicit device colors | G/g, RG/rg, K/k within the initial supported range |

Everything outside that explicit list is a typed Unsupported operation, including text operations, images/BI, Do, sh, gs, CS/cs, SC/SCN/sc/scn, patterns, rendering intent, flatness, marked content and compatibility sections. This first stage refuses BX/EX rather than silently discarding unknown extensions. A later driver can implement the already designed bounded compatibility policy before dispatching validated operations, but known unsupported operators must still fail inside BX. No unknown operation may become a no-op accidentally.

Allow only finite device components in [0,1] initially. Values outside that range are explicit unsupported inputs for this stage; PDF clamping compatibility needs its own test. Keep device Gray/RGB/CMYK tags and components rather than converting colors to RGB or claiming color-managed pixels. Defaults are device-gray black for fill/stroke, alpha 1, normal blend, width 1, butt caps, miter join, miter limit 10, solid dash. Transparency, overprint, soft masks and optional-content visibility are unsupported, not silently assumed away when their operators occur.

## Proposed API and ownership

```rust
struct PathProgram { /* private arenas and ordered events */ }
struct PathInterpreter { /* mutable state, current path, limits, output */ }
struct PathInitialState {
    // Checked construction, private fields.
    ctm: Affine<UserSpace, PageSpace>,
    clip: InitialClip,
}
enum InitialClip { Unbounded, Rectangle(Rect<PageSpace>), Empty }
PathInterpreter::new(initial, limits) -> Result<Self, PathError>;
interpreter.push(&ValidatedOperation<'_>) -> Result<(), PathError>;
interpreter.finish(self) -> Result<PathProgram, PathError>;
```

A convenience `interpret_path_content(bytes, initial, content_limits, path_limits)` can stream one parsed/validated operation at a time; it must not first collect an unbounded operation array. All operation attempts count, including style changes, q/Q and n with no paint. If push fails, the builder enters a sticky failed state; finish cannot return partial success. The whole-program convenience API is transactional. Input spans remain attached to errors and events, including derived rectangle/curve steps. No event is exposed before successful finish.

Use private `PathId`, `ClipId` and optional `DashId` indices into immutable arenas, not publicly constructible raw integers. IDs belong to a single program; expose borrowed accessors through that program. A clip node references an earlier parent ID and a path ID. Save/restore copies handles and small scalar state; it does not clone path histories or recursively owned clip trees. Dropping a program walks flat Vec storage and has no recursion proportional to clipping depth. Paths are owned once even when fill, stroke and clipping all reference the same geometry.

Represent commands explicitly as MoveTo, LineTo, CubicTo and ClosePath with finite `Point<UserSpace>` values. Expand shorthand v/y and re at construction with checked arithmetic; native path-packing optimizations are not part of this format. Preserve degenerate segments and winding orientation until a separately justified canonicalization. Negative rectangle width/height must retain the resulting directed contour; `Rect::from_corners` normalizes corners and must not replace re construction. A bounding box cannot replace a zero-area path, clip region or stroke.

Suggested event model:

- Fill: path ID, fill rule, CTM, color and active clip ID.
- Stroke: path ID, CTM, stroke style/dash ID, color and active clip ID.
- ClipPush: immutable new node ID, path ID, rule, CTM and previous clip ID.
- ClipRestore: restored clip ID when Q changes the active clip.

This retains clipping-only programs and ordering even without later paint. A native trace adapter can normalize ClipRestore to the appropriate pop count. IDs and immutable shared arenas avoid copying every earlier clip/path into every event. There is no polygon intersection, curve flattening or pixel coverage claim in P0.

## Current path and graphics-state invariants

The current path is **not** part of saved graphics state. q/Q save and restore CTM, stroke parameters, fill/stroke device colors and the active completed clip head. They must not save, restore, clear or clone the current path. Pending W/W* is also outside the q/Q frame in pinned MuPDF; preserve that policy explicitly.

Maintain current point and current subpath start separately from the command vector. m establishes both. l and c require a current point; v expands with its first control point equal to the current point, y repeats its endpoint as the second control point. h closes only the current subpath and makes the subpath start current. A repeated h should be a defined idempotent operation while still consuming work. A following l continues from the restored start point. re emits a closed, directed four-edge contour and establishes the rectangle start as current. Invalid current-point use returns a source-offset error rather than importing MuPDF's warning-and-ignore recovery.

Consecutive m commands may remain explicit (with no intervening drawable segment). Native comparison must account for MuPDF collapsing repeated moves and optimizing horizontal/vertical or degenerate curves; compare semantic normalized paths or exact generated cases that avoid those transformations. Never remove all degenerate segments indiscriminately: a degenerate stroked subpath can matter with non-butt caps.

Painting semantics:

1. For s, b and b*, append close to the current subpath before the action. Other fill operators imply closed-region semantics without changing the stroke's open subpath geometry. B/B* therefore cannot simply append ClosePath globally before stroking.
2. Emit fill, then stroke, when requested, both under the **previous** active clip.
3. Apply a pending W/W* after painting, creating a clip node intersected with the previous clip. Do not retroactively apply it to the paint that ended this same path.
4. Clear current path/current point/subpath start and pending clip exactly once, including n. The path can be shared by the emitted paint and clip events.

These steps follow pinned `pdf_show_path`: its fill/stroke callbacks precede `fz_clip_path`. W and W* only set a pending flag/rule until a path-ending operation; a later W/W* replaces the pending rule (last wins), with every operation charged. A pending clip can survive q/Q because it is not part of the saved frame. Empty-path clipping is an empty region, not an unbounded region or absent node. Initial Empty also remains empty. Keep that semantic distinction even when geometry bounds are degenerate.

Define strict completion: unmatched Q is an error; remaining q frames, unfinished current path or pending clip at EOF are errors. MuPDF may recover or discard some of these; strict refusals are separately recorded, not native parity failures. Empty input and n on an empty path are valid no-paint operations. Closing/painting an empty path needs explicit chosen behavior and tests: recommend h/s/b/b* without a current subpath be strict errors, while S/f/B on an empty path emit no paint and still consume the end operation; W n still creates empty clipping.

## Coordinate and transform policy

Existing `Affine::then` applies its left-hand transform first. PDF cm's pinned implementation is `fz_concat(new_matrix, current_ctm)`. With types, create `new: Affine<UserSpace, UserSpace>` and compute `new.then(current_ctm)`. Never reverse this composition. An initial page transform is provided by the caller; standalone P0 does not silently apply page rotation, UserUnit, CropBox translation or a device Y flip.

Source path points remain UserSpace; each emitted paint/clip carries `Affine<UserSpace, PageSpace>`. Avoid pretending each dynamic CTM defines the same geometric coordinate system while mixing construction transforms. The first stage should **refuse any effective CTM change while the current path contains commands or a pending clip**, including a Q that would restore a different CTM. q itself does not change the CTM and remains allowed. Q with the same CTM is allowed and leaves the path intact. cm resulting in an exactly unchanged finite coefficient tuple may be allowed, but conservatively refusing all cm while active is also acceptable if stated and tested.

This deliberate restriction keeps one stable transform for each path without importing untested mixed-CTM behavior. Pinned MuPDF stores source coordinates in the current path and supplies the paint-time CTM; later mixed-transform support must have explicit normative/reference evidence and a chosen model, including current-point handling. It cannot be introduced by accidentally applying cm twice or transforming only new points.

Check finite arithmetic for rectangle endpoints, expanded curves, transforms and optional bounds. Directly constructed ContentOperand integers larger than f64's exact range need a declared policy: conservatively reject coordinates/style values outside exact integer conversion, or document f64 rounding and test it against the existing validator. Do not silently claim integer-exact geometry from `i64 as f64`. Singular finite matrices can remain representable when no inverse is needed; P0 should not force inversion. If a later operation requires inverse, use the current geometry API's explicit conservative Singular result.

## Stroke and dash state

Preserve width in user-space units. Width zero is a hairline request, not an empty stroke. Store caps/joins as checked enums, not arbitrary integers. Miter limit and dash validation should reuse the typed operator validator (width >=0, cap/join 0..2, miter >=1, dash entries nonnegative and not all zero unless empty, phase >=0). Do not rescan a huge dash array without charging work; the path layer needs a separate cap even though operand grammar already caps objects.

Store the original finite dash list and phase rather than normalizing it prematurely. An odd-length list repeats across the alternating pattern; empty list is solid, and large phase must not lead to an iterative subtraction loop. Stroke expansion and phase normalization belong to a later stage. Finite individual dash values can still have a nonfinite sum; if the interpreter computes totals, reject overflow. A bounded immutable dash arena allows q/Q and many paint events to share the same list without quadratic copying. Charge every incoming dash element and new owned dash array, including subsequently replaced styles.

## Work, memory and error bounds

Suggested initial limits (all independently tested at the edge):

| Resource | Initial cap |
| --- | ---: |
| Operations attempted | 1,000,000 |
| q frames | 128 |
| Current-path commands | 100,000 |
| Cumulative constructed commands, including discarded n paths | 1,000,000 |
| Stored paths / events | 100,000 / 300,000 |
| Clip nodes | 100,000 |
| Dash elements per list / cumulative incoming elements | 256 / 100,000 |
| Total retained payload budget | 64 MiB |

Count derived re/c/v/y command work before appending; moving a path into an arena must not reset cumulative work. Clipping-only paths, overwritten pending clips and repeated q/Q still cost operations. All Vec capacities grow geometrically with checked arithmetic and an allocation/payload ceiling; charging only lengths while overallocating beyond budget is insufficient. State snapshots must not clone arbitrary dash arrays, path lists or clip chains. References into flat arenas avoid recursive destructor risk. Querying a long clip chain repeatedly is a separately bounded consumer operation; do not expose a convenience method that materializes the whole ancestry without a bound.

Errors carry the originating byte span/operation kind and a stable typed reason: Unsupported, InvalidState, StackUnderflow/Unbalanced, MissingCurrentPoint, MixedPathTransformUnsupported, GeometryOverflow, LimitExceeded, AllocationFailed. Syntax/operator errors remain distinguishable from execution errors. Valid PDF constructs outside this initial subset should be Unsupported rather than described as corrupt documents.

## Independent and native acceptance

Use generated standalone content programs and an independent Python model for state transitions, transforms and event order. Include every q/Q/path separation pattern; previous path surviving q/Q; closepath plus subsequent line; v/y expansion; signed rectangles; zero-area/degenerate subpaths; fill rules; fill-before-stroke; pending W last-wins; W followed by paint using the old clip; clip restoration; W surviving q/Q; repeated n; stack/path EOF errors; every budget; arithmetic overflow and unsupported operations at exact offsets.

Pin affine order with noncommuting cases: translation followed by scale, scale followed by translation, rotation+translation, shear, reflection and singular transforms. Compare independently transformed sample points rather than only self-inverse round trips. Include Q that changes CTM with a live path and require the explicit P0 refusal.

A native test-only trace needs no system fonts: create minimal generated PDF pages with path-only streams and run pinned MuPDF through a custom `fz_device` or the existing WASM `Device` callbacks. The WASM binding exposes fillPath, strokePath, clipPath, popClip and `Path.walk`. Copy path/matrix/style/color data during callbacks (borrowed native objects must not outlive the callback). Reject unexpected text/image/shading/group/resource callbacks, warnings or document repairs for strict generated cases; impose input/path/event/output limits and subprocess timeouts.

Use a clearly specified identity page setup or factor out MuPDF's known page CTM before comparison. Native geometry uses f32; compare exact small integers where possible and use recorded absolute/relative tolerances elsewhere. Compare path semantics after only justified normalization; keep native path compression separate from Rust command storage. Test clip state and paint order through callbacks, not inferred pixels alone. Native recovery cases should be distinct from valid parity cases. A later optional native raster replay can confirm that identical recorded events produce matching pixels, but P0 itself still performs no rasterization.

Most current fixture streams contain text or resources; explicit refusal there is expected. Do not count a page as supported because its prefix produced a few path events. Acceptance requires complete-program success, no omitted unsupported operation and preservation of existing reader/decoder/content/geometry/font gates.

## Inspected pinned implementation

All source is from the verified MuPDF 1.28.1 archive: `source/pdf/pdf-op-run.c` (pdf_gsave/pdf_grestore, pdf_run_cm, path callbacks, pdf_show_path, W/W*), `source/fitz/path.c` (current-point/close semantics and path optimizations), and the existing WASM 1.28.1 public Device/Path bindings. The existing Rust `operators.rs` supplies local shape/range validation; `geometry.rs` supplies tagged finite affine/point/rectangle primitives. This document changes no engine code.
