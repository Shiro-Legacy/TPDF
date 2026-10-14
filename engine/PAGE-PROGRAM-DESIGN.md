# Mixed page path and positioned-code traces: PP0 proposal

Read-only design; this file does not authorize implementation or expand CONTRACT.md. PP0 should execute a bounded, explicitly supported mixed page program into an owned immutable trace combining P0 paths and T0 positioned source codes in original operation order. It adds document/page/resource orchestration. It does not draw glyphs, flatten clips into glyph bounds, infer reading order, execute Form/image resources or interpret ActualText.

The existing engines already supply useful pieces: `PageGeometry::user_to_page()` applies crop, rotation and UserUnit once; `load_page_content` produces logical joined bytes and stream segments; F0 resolves inherited page resources lazily; M0/M1 select declared or explicitly opted-in substitute advances; P0 owns flat path/clip/dash arenas; T0 owns text/source-byte arenas and code/adjustment/end records. Preserve their independent standalone APIs.

## Measured scope ceiling

`reference/page-program-inventory.json` records a read-only classification from the hash-pinned content and font inventories, using the Rust content parser for detailed sequencing. All multi-reference pages use the accepted logical page loader. A single-source scan uses its hash-pinned decoded stream; a trailing array-member separator cannot add an operator. The report is an operator/state/selected-font ceiling, **not** successful mixed execution.

- 572 logical pages across49 unencrypted fixture documents.
- 543 candidate pages across31 documents:500 pages in `large.pdf`, plus43 pages in30 other documents. Report these two populations separately in acceptance.
- 23 pages contain unsupported operations. Page-content occurrences include40 `Do`,10 `BDC`,10 `EMC`,2 `BI`,703 `cs` and703 `sc`. Inline image scans stop immediately; later bytes are unclassified. Two incomplete BI pages are included in these23, not added again.
- Five otherwise operator-clean pages select Type0 fonts: `cjk.pdf`, `p0b/edit-embedded.pdf`, `p0b/edit-subset.pdf`, `p3/edit-subset2.pdf`, and `rtl.pdf`.
- `edit-tj.pdf` page3 has an unbalanced q/Q stack after exact logical joining. Do not add a synthetic Q or reset at content-stream boundaries.
- Operator-clean pages have no observed q/Q/cm inside BT, active/pending path at BT, path construction/painting inside BT, or unsupported Tr. Font decoding, actual advances, arithmetic, resource budgets and mixed-transform paths have not yet been executed by this inventory and may further reduce acceptance.

The executable report records543 candidates plus29 refusals. Unsupported pages may contain additional unexamined font/state failures. The report intentionally records overlapping reasons rather than claiming mutually exclusive root causes.

The first gate should execute every candidate and explain any decrease. It should not broaden unsupported operators just to reach543. In particular the Quartz `cs/sc` operations require named color-space state; silently treating them as Gray/RGB would be incorrect. Form XObjects require scoped resources, recursion and transforms. Marked-content semantics must be designed before retaining or ignoring them.

## Document-bound session and prepare/freeze lifetime

Proposed public shape (names illustrative):

```rust
pub struct PageProgramSession<'doc, 'input> { /* document, pages, F0 cache, ledger */ }
impl<'doc, 'input> PageProgramSession<'doc, 'input> {
    pub fn new(document: &'doc Document<'input>, limits: PageProgramSessionLimits)
        -> Result<Self, PageProgramError>;
    pub fn trace_page(&mut self, page_index: usize, limits: PageProgramLimits)
        -> Result<PageProgram, PageProgramError>;
    pub fn usage(&self) -> PageProgramSessionUsage;
}
```

Page indices are explicitly zero-based in the library, one-based in the CLI. Validate page membership/geometry once per document session, and reuse one document-bound FontResourceLoader across calls. Returned PageProgram owns its trace, labels, segment metadata and font identity descriptors; it borrows neither the session nor temporary registry/content operands. A caller can retain previous results while invoking the next page, but every invocation still charges cumulative session work and output allocation. Repeating a page is work again; no implicit unbounded result cache.

Each page follows these phases:

1. Reserve/charge the page attempt, page-content stream/decode/join allowance and page geometry. Build exact logical content once; preserve every repeated stream occurrence and inserted separator.
2. A bounded prepass parses and validates operators, enforces the supported routing/state subset, and collects only names encountered by `Tf`. Deduplicate names under binding/name-byte limits; preserve the first global operation occurrence for resource errors. An unused unsupported resource must not block a page. A Tf is a selection and must resolve even if no later show uses it.
3. With only mutable loader access, load F0 and copy/release each FontIdentity borrow. Try M0. Only the exact `NeedsBuiltinOrFontProgramMetrics` result permits the explicitly enabled M1 policy; malformed explicit widths and all other errors refuse. Load M1 independently of ToUnicode; use its own encoding for advances. Check all loaded identities agree for the selected resource, including aliases/direct dictionaries. No Unicode fallback and no inferred glyph-ID equivalence.
4. After all cache mutations, use immutable `loaded`, `loaded_metrics` and `loaded_builtin_metrics` getters to build bounded borrowed TextFont views and an immutable TextFonts registry. No 256-slot tables/maps are cloned per operation. Registry IDs are local to this result and copied output descriptors carry both resource-name bytes and document FontIdentity/advance policy. Resolve aliases once but preserve names as separate bindings.
5. Reparse the same immutable logical bytes to execute, charging the second parse pass separately. This avoids retaining an unbounded operation AST and self-referential loader structures. A page error publishes no trace; prior independent successful page results remain valid. Used cache entries and consumed failure/work reservations remain in the session.

Prefer this explicit phase boundary to Rc/RefCell self-borrow workarounds. A later dynamic Form-resource implementation may need owned Rc cache handles; PP0 does not need them. Use an explicit caller policy: DeclaredOnly by default, or AllowPinnedBase14 to opt into verified M1 Helvetica/Times-Roman substitution. Preserve that selection in output; never infer it merely from a failed M0 parse. A CLI may select the opt-in only through a documented explicit option.

## One coordinator, persistent engines and shared state

Create exactly one P0 PathInterpreter and one T0 TextPositioner for the entire page. Never instantiate an engine per stream, BT block, path run or event; such segmentation loses persistent text state, current paths, clips, dash storage and q/Q stacks. Both use the same initial `user_to_page` transform. P0 initial clip is `Rectangle(PageGeometry.bounds())`, not Unbounded. PageSpace is the existing top-left cropped/rotated/scaled coordinate system; apply neither rotation nor UserUnit a second time.

The smallest compatible bridge has P0 as the authoritative graphics CTM/color/clip provider and T0 retaining its existing CTM mirror for positioning. There must be one global q/Q admission/underflow decision and one global text-object flag in the coordinator. Broadcast q/Q/cm outside BT to both engines, which save their disjoint path/text state. Assert CTM coefficients and stack depths agree before and after each shared operation. CTMs must already be finite; use coefficient-wise numeric equality (positive and negative zero compare equal), matching existing P0 transform checks, with no tolerance or corrective rewrite. A child failure immediately invalidates the coordinator and prevents any further child operation or finish publication; it is acceptable for one private child to have mutated because no partial result is observable. Never add a third independently computed CTM or silently synchronize a divergent one afterward.

This is a deliberately small bridge, not a recommendation to keep two authoritative graphics stacks forever. A later internal shared GraphicsState coordinator can remove the mirrored CTM while retaining existing public standalone wrappers. That refactor needs its own tests; PP0 should not simultaneously rewrite all accepted path/text arithmetic merely to hide the bridge.

Routing:

- q/Q/cm: coordinator validates outside BT and global balance, then broadcasts in a fixed documented order. P0's existing mixed-transform current-path refusal remains authoritative. Check the T0 numerical policy too; a finite P0 matrix outside T0 bounds cannot be accepted and then bypassed for later text.
- Path construction, clipping and ending/painting operators: P0 only, outside BT in PP0.
- Device color and accepted stroke-style setters: P0 only; these can occur inside or outside BT and remain saved/restored by q/Q. Their values must be visible on later text snapshots. They do not become fake text operations.
- Text operators: T0 only. BT requires no active current path and no pending W/W* in PP0. Text state persists across BT/ET. q/Q/cm inside BT remain explicitly Unsupported.
- Every other operator, including BI/ID/EI, Do, gs, cs/CS/sc/SC/scn/SCN, marked content/ActualText, compatibility, shading and Type3 metrics: explicit error. Existing P0/T0 refusals remain; PP0 cannot skip an unfamiliar operator and return an apparently complete page.

**Path/text boundary policy:** PDF can carry a current path across unrelated operations; PP0's refusal of BT with an active/pending path and path construction/painting inside BT is a strict subset restriction, not a claim that text resets the path. Do not clear, close, paint or commit a pending clip at BT. These refusals cost no additional operator-clean fixture pages in the current inventory. Color/style setters inside BT are distinct and remain supported. Expand this restriction only after a dedicated state/native gate.

## Minimal read-only interpreter additions

Keep constructors/private state and existing push/finish intact. Add narrow introspection and context injection, not mutable arena getters or a partially publishable trace:

- P0 `snapshot() -> PathStateSnapshot` containing Copy CTM, active ClipId, fill/stroke DeviceColor, StrokeStyle and save depth; `has_active_path()` includes a pending clip even without path commands. Clip/dash IDs remain scoped to the final owned PathProgram.
- Both engines expose event counts (and T0 code/adjustment/end counts). The coordinator records ranges of new child events after each successful push. No need to clone whole programs or inspect every prior event.
- T0 accepts an internal/coordinator `push_at(validated, global_operation_index)` or equivalent explicit provenance context. Its existing processed-operation counter remains separate for work limits. The new index must not be substituted into that counter, which sees only dispatched/shared operations. Standalone push continues assigning its own sequential index. Require monotone global indices in the coordinator and preserve original Operation.span without synthesizing operators.
- Both engines expose immutable cumulative usage, including requested allocation/work categories. Budgeted push must cap additional work/allocation **before** a reserve, not merely report an overshoot afterward. See the session ledger below.
- A context/count accessor does not expose complete partial records. Only successful final PageProgram can expose path/text payload arenas. If internal crate-only new-event inspection is useful, keep it private to orchestration and never serialize before finish succeeds.

A single paint operator can create fill, stroke, then ClipPush events; preserve their existing order. Q can emit ClipRestore. A text operator can emit several code/adjustment events. The coordinator's global event arena references all these existing indices in the order they were appended.

## Owned result and honest source provenance

Suggested result owns: PageGeometry/page ID/index, ContentSegments, selected font descriptors, final PathProgram, final PositionedText, bounded text-context snapshots and one global event arena. A global event contains global operation index/span and `PathEvent(index)` or `TextEvent(index, context_id)`. Optionally add a bounded operation-index/span table once rather than cloning spans per event; define whether non-event operations remain available for diagnostics. All indices are private/validated at construction.

A text context snapshots active ClipId, fill color and rendering intent immediately before each show operation's new records are appended. It can be shared across all codes of that show (or only across consecutive records with exactly equal context); do not create one full graphics-state copy per Unicode scalar. Tr0 carries fill intent. Tr3 is invisible text intent; retain the color/clip state for provenance but do not claim native ignore_text paints with it. Neither makes glyph bounds, clip coverage or actual text pixels available. Text adjustments/end records can reference the same context or an explicit nonpainting context, but their ordered positions must not disappear.

Each decoded code already has decoded-string byte range, retained source-byte arena range, Unicode range, operand/TJ element location and text-object index. Add/retain the global operation index and enclosing logical content byte range. These are not raw PDF offsets. A logical operation can span multiple Contents streams; `ContentSegment` occurrence identity must include occurrence index as well as ObjectId, because repeated references are distinct positions. Project a logical range to one or more `(segment occurrence, decoded stream range)` pieces, plus explicitly synthetic padding offsets. Do not attribute a joined token/string to whichever segment contains its start. Do not map decoded-string offsets through literal escapes/hex whitespace to raw file bytes; that mapping is not currently retained.

The first result retains the exact joined PageContent bytes and ContentSegments so logical source ranges remain directly inspectable. Charge retained content capacity to the page and cumulative session output budget in addition to decoding work. Code source bytes are separately retained because they represent decoded string payloads. A later metadata-only output mode may omit joined bytes explicitly, but must not promise directly sliceable content from metadata alone.

## Aggregate session budgets and failed work

Existing nested limits remain useful but cannot reset their full allowance for every page, substream, BT object or parse pass. The session ledger needs cumulative page attempts, content references, encoded bytes, all decoded intermediate bytes, joined bytes processed/retained, prepass/execution operations and parser work, selected-name preparation, output events/codes/scalars/UTF8/source bytes, path commands/dash work, and requested output allocation. F0's cumulative resource/reference/metadata/map/encoding/M0/M1 budgets stay live in the same loader across pages; record deltas and report them alongside page work.

Hard aggregate limits must be admitted before the relevant operation. A safe bridge is an internal budgeted push that lowers a child's effective cumulative ceiling to `child_usage + parent_remaining` (never above its configured child ceiling), executes, and charges the delta before another child executes. Failed calls retain spent or conservative reserved work, and make the page atomic. For phases with no actual usage getter, reserve a documented worst-case allowance before the phase and refund only verified unused allowance on success; on failure retain the reservation. Do not run a child under its full independent cap and then discover after allocation that the shared cap was exhausted.

Prepass and execution lexing/parsing are real repeated work. Add bounded ContentParser usage access or conservatively reserve both passes' node/token/owned-byte allowances; do not assume parsing is free because the logical input was decoded once. Font preparation similarly needs a combined content/font decode-work admission strategy: either budgeted loader calls capped by remaining aggregate allowance, or explicit nonoverlapping parent subbudget partitions established at session creation. Clearly document partitions; an after-the-fact sum check is not a hard aggregate memory bound.

Allocation semantics currently differ: T0 counts full requested allocation capacities on every growth; P0 counts capacity growth including discarded storage. Do not rename their sum “cumulative requested allocations” without harmonizing or separately retaining both metrics. PP0 should choose and test a single conservative envelope (prefer full requests for new orchestration buffers and budgeted child requests), while preserving the accepted standalone guarantees. The already-open borrowed Document and its prior parsing/metadata allocations are a separately bounded external prerequisite, not retroactively capped by the new session ledger. Its limits/usage should be reported separately by the caller. Font/cache work performed after session creation is governed by the live session resource budgets. Copied page output descriptors and caller-retained prior outputs still consume cumulative output-allocation/work charges and receive no implicit refund at page return.

Avoid always reserving a huge maximum for a tiny page when an inexpensive checked upper bound from content length/limits is possible. Geometric output growth and bounded interned context/name arrays prevent quadratic concatenation and per-code table cloning. Document failure/exhaustion may block further session calls until the caller starts a separately budgeted session; no automatic reset.

## Acceptance and compact inspection

`--page-program=N` can return a bounded full trace for one page, with atomic buffered output under the CLI's existing cap. Add a separate compact document inventory mode which opens the document once and uses one PageProgramSession across all its pages, emitting page status/counts/hash/provenance and aggregate usage. Do not implement corpus acceptance as500 process reopens or dump every stress-page record into an unbounded JSON envelope. Fixture documents are distinct sessions; full-corpus driver totals are separate from per-document budgets.

Require:

1. Execute all572 fixture pages and classify every refusal, separating500 stress pages from43 other candidate pages. Native comparisons on accepted pages cover ordered path paints/clips and primary source-code transforms/Unicode/advance intent, not glyph IDs or pixel equality. Record any candidate reduction with the exact triggering operator/resource/offset.
2. Hand-authored mixed programs: path fill/stroke before and after text; clipping W then n and text; Q restores text clip/color; fill-plus-stroke-plus-clip order; color/style inside BT; text state saved across complete BT objects; noncommuting cm/rotation/UserUnit; Tr3 intent. Initial page clip is represented even when no explicit W occurs.
3. Cross-stream tokens, comments and escaped strings; repeated stream references; operations crossing synthetic padding; global indices with many path-only/color ops before/between text; no confusion between lexical and decoded-string positions.
4. Used-only resource preparation, unused Type0/bad maps, Tf without a show, aliases, authoritative ToUnicode + unrelated builtin encoding, malformed Widths refusing fallback, and cache retention across multiple pages. F0/M0/M1 identity/provenance checks stay visible.
5. Exact/beyond aggregate budgets across many tiny pages, repeated page requests, failed prefix/suffix, many empty shows/paths, shared op stack depth, failure in the second broadcast child, output-capacity growth, and no trace publication on any unsupported suffix. Dropping or retaining a prior output cannot reset session work.
6. Strict refusals for active path/pending W at BT, path paint/construction inside BT, q/Q/cm inside BT, Form/images, named color spaces, marked content and unsupported Tr. A test may prove these are deliberately unsupported, not recover them.

Independent reviewers should inspect the global coordinator invariants and output joins in addition to running existing P0/T0 tests. Finish with full normal/MSRV/wasm gates and no new runtime dependency. Native reference behavior comes from pinned MuPDF1.28.1 `pdf-op-run.c`/`pdf-interpret.c` (Copyright2004–2026 Artifex); retain existing AGPL attribution. This stage stops at a mixed trace and resource adapter.
