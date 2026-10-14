# R22 proposal: bounded Form XObject execution

Status: design only. R21 production and tests remain frozen. This proposal does not authorize implementation or claim that the rewrite is complete.

## Recommendation and measured value

Implement ordinary Form XObjects as a bounded extension of PP1/C0, then add narrow opaque image XObjects. Forms are a prerequisite for compositional page execution, but they are not the main remaining corpus coverage gain by themselves.

The accepted R20 outline report, `reference/page-outlines-differential-b0.json`, has 572 pages: 543 accepted and 29 refused. R21 changes raster stroke support, not these PP1 refusal causes. Grouping the 29 first refusals gives:

| First refusal | Pages | Files/pages |
| --- | ---: | --- |
| `Do` / XObject | 16 | classified below |
| Type0/composite font | 5 | cjk:1, p0b/edit-embedded:1, p0b/edit-subset:1, p3/edit-subset2:1, rtl:1 |
| BDC marked content properties | 5 | layers:1, p0b/edit-tagged:1, p3/edit-replace:1, p3/edit-tail:1, p6/paragraph:2 |
| Unfinished text/graphics state | 1 | edit-tj:3 |
| Inline image `BI` | 1 | p3/targets-unsupported:1 |
| Non-device color-space selection | 1 | p6/quartz:1 |

A temporary native processor was compiled against the already pinned MuPDF 1.28.1 static libraries and run on the actual 16 `Do`-refused pages. It counted executed `Do` callbacks, recursively processing Form bodies; it did not infer use from resource dictionary membership. Temporary diagnostic source/output are `/private/tmp/r22-inventory.c` and `/private/tmp/r22-inventory.txt`. These temporary files are exploratory evidence, not a new accepted reference gate.

| File | Page | Executed Forms | Executed images | Other relevant observation |
| --- | ---: | ---: | ---: | --- |
| p0b/edit-shared.pdf | 1 | 2 | 1 | shared text Form |
| p0b/edit-shared.pdf | 2 | 0 | 1 | opaque RGB8 |
| p0b/edit-transformed.pdf | 1 | 0 | 1 | transformed opaque RGB8 |
| p3/edit-objects.pdf | 1 | 3 | 4 | one image has SMask; one Form Matrix translates by (10,10) |
| p3/edit-objects.pdf | 2 | 1 | 1 | opaque Gray8 with Flate PNG predictor |
| p3/targets-streams.pdf | 1 | 4 | 1 | includes one nested Form invocation; root Contents is an array |
| p3/targets-streams.pdf | 2 | 1 | 1 | shared Form |
| p6/direct3.pdf | 1 | 1 | 2 | path Form, Gray8 and RGB8 |
| p6/direct3.pdf | 3 | 0 | 1 | opaque Gray8 |
| p6/objects6.pdf | 1 | 1 | 5 | path Form, Gray8 |
| p6/objects6.pdf | 2 | 0 | 1 | opaque Gray8 |
| p6/paragraph.pdf | 1 | 0 | 1 | opaque Gray8 |
| redact-shared-only.pdf | 1 | 1 | 0 | text Form, caller text follows |
| redact-shared-only.pdf | 2 | 1 | 0 | same Form object as page 1 |
| redact.pdf | 1 | 1 | 3 | SMask plus later marked-content semantics |
| redact.pdf | 2 | 1 | 1 | shared text Form and opaque RGB8 |

Totals: 17 Form invocations, 24 image invocations; 2 Form-only pages, 5 image-only pages, 9 mixed pages. All used Forms have explicit Resources, valid ordinary BBoxes, no Group or OC. There are nine distinct used Form streams across these files. Only the noted Form has a nonidentity Matrix. The corpus therefore does not test missing Resources, name shadowing, recursive cycles, malformed BBoxes, or noncommuting Form matrices; generated adversarial gates are mandatory.

Expected incremental value, not a promise of acceptance: Forms alone should unlock the two `redact-shared-only.pdf` pages. Opaque Gray/RGB8 image support alone could unlock five image-only pages. Together they remove the XObject blocker from up to fourteen of these sixteen pages; the remaining two contain soft masks, and later operators can still refuse. This is not a prediction that all fourteen will rasterize under every existing stroke/numeric limit. Composite fonts would address five first refusals but need a separate source-code/CID/GID/width design; a ToUnicode inversion is not acceptable. Marked content cannot simply be ignored because OC/ActualText semantics differ. Existing AcroForm fixture pages already accepted by PP1 are not evidence that annotation appearance rendering exists.

## Initial semantic scope

Accept an indirect stream selected by a used `/Name Do`, resolved from the current effective Resources/XObject dictionary, with `/Subtype /Form`, absent or valid `/FormType 1`, a required finite nonempty ordered BBox, and absent/identity or a valid nonsingular finite six-number Matrix within existing geometry limits. The implementation may support an empty BBox as a proved empty clip only if P0/R19 preserve that distinction; otherwise it must explicitly refuse it. Missing, reversed, malformed, cyclic-reference, or numerically collapsed bounds never become an unbounded clip.

Resolve metadata references under cumulative limits. Null optional entries have their documented absence meaning, but malformed references never become absence. Refuse meaningful Group (including transparency), OC, external/reference Form semantics (`Ref`), `Subtype2` alternate dispatch, OPI, and meaningful default Gray/RGB/CMYK overrides in the effective resource dictionary. A missing Type may be tolerated when the selected stream has valid Subtype Form; a present incompatible Type refuses. Unused resources do not force decoding or font loading, although scanning/indexing their names must be bounded. The exact accepted metadata-key policy must be reviewed against the pinned native implementation before final contract.

Images, PostScript XObjects, inline images, annotation appearances, transparency, patterns, shadings, Type3 glyph programs, composite fonts, and unsupported operators remain explicit refusals. No substitution or “skip unsupported child” behavior is introduced. Existing independent metric and outline policies remain unchanged.

Initially require each `Do` to occur outside BT/ET and with no active current path or pending clip. This intentionally refuses valid broader PDF placements rather than discarding or corrupting the caller's path. Each Form body must end outside text, with balanced local q/Q, and no unfinished path/pending clip. A form-local Q cannot pop the implicit Form save or any caller save. This is stricter than native recovery from malformed/unbalanced streams and must be recorded as such.

## State and geometry invariants

Use one C0 interpreter and its existing global path, text, clip, context, and event arenas. Do not instantiate a fresh interpreter with default colors/line styles for each Form. Existing q/Q already preserves P0 paint state and T0 font, size, spacing, rise, horizontal scale and rendering mode. Preserve T0's existing refusal of q/Q/cm inside BT. Its current text matrices are stored in text-object state, and this implementation must not be generalized into a claim about all PDF text-state save rules: the PDF text-object errata additionally specifies matrix saving for q/Q inside BT. R22 keeps that broader operation sequence unsupported. The Do-outside-BT restriction avoids cross-Form text-object lifetime.

Entry performs the equivalent of an implicit saved graphics state, composes Form Matrix before the caller CTM using the existing `cm` composition convention, and intersects the active clip with the Form BBox. The BBox is a closed rectangle in Form user space transformed by the full Form CTM. Its four transformed sides must remain a path clip; replacing it by an axis-aligned page bounding box would be wrong under rotation or shear. The original page geometry and initial page clip are applied once, exactly as PP1 already does. A Form never receives a second page rotation/UserUnit transform.

The caller's selected font is inherited as a resolved TextFontId, regardless of whether the new Resources contains that resource name. A later Form-local Tf resolves against the Form scope. On exit the previous selected ID and every other graphics-state field are restored. For example, page /F1 can select Helvetica, a Form's /F1 can select Times-Roman, and caller text after the Form must again use Helvetica. A Form with explicit empty Resources can use an inherited selected font without a new Tf; a new `/F1 Tf` there must fail. No local resource-name lookup is needed for an already-selected ID.

BBox clip entry, descendant paint events and clip restore remain in one ordered Program. Clip IDs are global arena indices; child clips refer to the caller's current parent. TextContext snapshots retain the actual clip and fill color at each text event. R18 glyph attachments and R19/R21 rasterization consume the resulting event order; they must not reapply the Form Matrix.

No painted Form result is cached by stream ID. The same bytes can execute under different CTMs, clips, colors, line widths, selected fonts and resources. Only validated metadata/decoded bytes may be cached independently of current graphics state.

## Resource scopes and frozen font preparation

The current `FontResourceLoader` is page-bound, and PP1's registry is keyed by raw resource name. Neither can be reused unchanged for local names. Introduce a document-bound internal resource-scope resolver shared by XObject and font lookup. It never accepts an arbitrary caller-supplied dictionary as belonging to the document.

An explicit scope is interned from the actual Page or Form object that provides an explicit Resources dictionary; the absence of Form Resources reuses the caller's effective scope. Form stream identity, explicit Resources provider identity, and per-invocation effective scope are three distinct concepts. A Form object alone is never the effective-scope cache key: the same absent-Resources Form invoked under callers A and B records scope A and B respectively. The metadata cache may record that this stream has no Resources, but it must not remember the first caller's effective scope as a property of the stream. Explicit empty Resources replaces the caller dictionary and does not fall back per missing name or resource category. Nested absent Resources inherits the immediate caller scope. Resolution of Resources itself, Font/XObject subdictionaries and target references is checked, with cycles and stream-versus-dictionary type mismatches refused. Default-device-color checks apply to every newly admitted effective scope.

This is a deliberately strict scope subset, not a claim that pinned native lookup behaves identically on missing local names. MuPDF `pdf_lookup_resource` walks outer processor resource frames and can recover such a name; Rust should report an explicit unsupported/missing binding instead. The [PDF Association approved Form Resources correction](https://pdf-issues.pdfa.org/32000-2-2020/clause08.html#h2-8.10.2) distinguishes independent Forms, which contain their named resources, from historical dependent usage. The initial implementation is not a PDF-version conformance validator; absent-resource compatibility and strict replacement lookup are stated capabilities, and any broader fallback policy needs its own explicit contract and tests.

Preserve the existing public page loader entry points. Add crate-private scope preparation/load entry points with authenticated handles created only by this document-bound resolver. Keep indirect FontIdentity based on the actual font dictionary object, allowing aliases to share decoded font/metrics caches. For direct dictionaries, retain an unambiguous physical owner plus entry index (page or form Resources owner); scope/name remains a separate binding identity. G1 continues to refuse direct-font outline loading, so this does not silently expand outline support.

PP1 font bindings become `(ResourceScopeId, original_name)` entries whose stable position is TextFontId and PageProgram.fonts() index. Add `PageFont::scope_id()`. Do not manufacture renamed PDF font names or replace their source byte strings. Different scopes may contain the same name, while different names/scopes can point to one underlying font object.

Add a crate-private indexed TextFonts constructor that permits duplicate display names, while keeping the public name-based constructor's existing duplicate rejection. C0's new resolved-font path must use `by_id`; the old name-based path remains for standalone C0 callers. Before selecting a resolved Tf, validate that its supplied ID corresponds to the prepared scope/name and original operand. This prevents mismatched ID/name calls from bypassing preparation. Font metrics/Unicode capabilities are still prepared for every used Tf, including selections with no following text show.

## Proposed API and ownership

Keep `PageProgramSession::trace_page(page_index, PageProgramLimits)` and the PageGlyph/Raster session chain. Add `forms: FormLimits` to page and session limits; supported ordinary Forms are a capability rather than a substitution policy. A zero invocation limit disables them. Existing pages without Do keep their behavior and original operation numbering.

Place document/resource/stream traversal and its ledgers in a new `form_resources.rs` (name subject to final ownership split); keep page orchestration in `page_program.rs`. Proposed owned output types:

```rust
pub struct ResourceScopeId(usize);
pub struct FormInvocationId(usize);
pub struct FormStreamId(usize);

pub enum OperationOrigin {
    PageContent { span: Range<usize> },
    FormContent { invocation: FormInvocationId, span: Range<usize> },
    FormBoundary { invocation: FormInvocationId, phase: FormPhase },
}
pub enum FormPhase { Enter, Leave }

pub struct FormInvocation { /* private fields, getters below */ }
// id(), parent()->Option<FormInvocationId>, stream_id(), object_id(),
// caller_scope(), effective_scope(), call_origin(), matrix(), bbox(),
// execution_range()->Range<usize>
pub struct FormStream { /* object id + owned decoded bytes */ }
// object_id(), bytes()->&[u8]
```

All public IDs above are local to one returned PageProgram. Session-internal cache handles are remapped into that result's owned tables; a result does not require every previously cached scope/stream or access to the live session. `PageProgram::content()` remains the exact root joined PageContent. Add `form_streams()`, `form_invocations()`, `resource_scopes()`, and `operation_origin(execution_index)`. The last is authoritative for interpreting event/code operation spans. Root PageContent spans still project through its exact occurrence/padding map. FormContent spans refer directly to the retained decoded stream bytes; no page stream occurrence or logical offset is invented for them. Invocation chains distinguish repeated and nested uses of the same stream. Boundary origins are explicitly synthetic and link to the original Do call, not fabricated source bytes.

Decoded streams can be retained once through `Arc<Vec<u8>>` across a session cache and returned results. Full requested buffer allocations are charged before allocation; shared handles have a separately counted fixed-size/output partition and the same documented process-OOM limitation as R18 Arc outlines. A returned PageProgram owns all source bytes, names, invocation records and origin records needed after both session and document are dropped. Internal document references never escape. Form metadata may be copied into small fixed-size typed records rather than retaining entire borrowed dictionaries.

C0 additions are crate-private and do not expose child mutable state:

```rust
fn push_resolved_font(&mut self, op: &ValidatedOperation<'_>, id: TextFontId)
    -> Result<(), ProgramError>;
fn enter_form(&mut self, matrix: Affine<UserSpace, UserSpace>, bbox: Rect<UserSpace>)
    -> Result<FormFrame, ProgramError>;
fn leave_form(&mut self, frame: FormFrame) -> Result<(), ProgramError>;
```

FormFrame is an opaque interpreter-local LIFO token with entry stack floor, not a forgeable save-depth integer. Both boundaries are explicit execution steps with span `0..0`; the invocation record separately retains the exact original Do call origin. Entry can internally reuse bounded typed q/cm/rectangle/W/n machinery, but must not serialize and reparse synthetic PDF or pretend its synthetic clipping operation has an original stream span. All child operations/events/allocations are pre-admitted under ProgramLimits, even if the parent boundary is a single execution step. Pass the single parent execution index explicitly to emitted child provenance; child-local operation counts are only accounting counters and must not remap public indices. On any child failure the parent interpreter is sticky and cannot publish a prefix.

One original Do corresponds to the Enter execution step; Leave adds a synthetic step. All real nested operators receive their actual local spans. Original no-Form pages retain their current numbering. The output origin table maps every execution index, including nonpainting operations and boundaries. Add a boxed Form error category to PageProgramError and source-aware error context. A failure must own enough bounded diagnostic context to identify page/form object, local span and invocation chain even though no partial PageProgram is returned. Do not return bare invocation IDs that cannot be resolved outside the failed private result.

Failure provenance has a separately bounded diagnostic partition, independent of output/origin quota. Reserve lazily immediately before the first used Do and before any nested traversal, with a fallible reservation for at most 64 fixed-size DiagnosticFrame records. A page without Do performs no diagnostic reservation, preserving its existing allocation/limit behavior. Each record contains the current Form ObjectId/local span and its parent page/Form ObjectId plus Do callsite span, so64 Form levels fit64 records without needing a65th root record; no names, source bytes or recursive heap structure. A private Vec may hold this fixed capacity, but its methods prohibit growth and the frame count is hard-capped at64. Count the full requested capacity bytes in a cumulative `diagnostic_allocated_bytes` field governed by `max_diagnostic_allocated_bytes` (default64 MiB); a successful attempt also retains its allocation request charge. Physical allocator over-allocation is not a reason to request or expose more diagnostic slots.

On failure, populate and move the already-reserved chain into the error without any allocation from the exhausted Program/origin/source budgets. Existing PageGlyph/Raster boxed PageProgramError wrappers retain this self-contained chain. Early page-attempt failures use the existing small error. A first-Do diagnostic-reservation failure returns a small zero-frame root object/span error before descending. A diagnostic reservation failure is itself a small allocation/limit error; it does not recursively attempt another diagnostic allocation. Existing wrapper Box allocations retain their documented process-OOM limitation. Tests must exhaust output, work and source quotas at different nesting depths and still recover the original object/span/callsite chain, with zero further diagnostic allocation requests during error construction.


## Two passes and bounded execution plan

Pass one recursively parses root content and used Form streams, validates the strict state grammar, resolves each used Do, records a compact invocation tree/preorder schedule, and discovers all scoped Tf bindings. Retain decoded streams and fixed-size per-invocation metadata, not every parsed operand payload. Resolve every required font/metric capability, then freeze the font cache and indexed registry before pass two.

Pass two parses each actual invocation again and executes it in the recorded order using its prepared scope, Matrix, BBox and font IDs. Do not repeat mutable font loading while the registry borrows it. Verify that callsite sequence and source identity match the prepared plan; immutable document bytes make a discrepancy an internal invariant failure. Every repeated invocation incurs parser and interpreter work, even if its bytes or font objects were cached. No semantic cache bypasses the repeated execution ledger.

Use explicit bounded DFS frames for traversal rather than native Rust recursion. A frame holds stream/source cursor, local source operation ordinal, invocation ID, effective scope, interpreter boundary token and parser state. Parent operand payload need not remain live across child execution after its Do name/callsite is copied into bounded metadata. Existing page Contents joining remains at the root; Form streams are not concatenated with parent bytes, because token and state boundaries differ.

There is a concrete safe-Rust lifetime issue: ContentParser borrows its bytes, so a frame cannot both own an Arc and contain a parser borrowing that Arc. Add crate-private consuming checkpoint/resume methods at completed operation boundaries in ContentParser and Lexer. A checkpoint retains byte offset, original limits, lexer emitted-token count, parser token/node/payload/operation counts and sticky failure state. The frame holds authenticated source identity and its Arc; each active parser temporarily borrows those bytes, then is consumed back into its checkpoint before descending. Resume verifies source length/identity through the owning traversal and does not re-admit or reset per-stream counters. Recreating a fresh parser for each suffix is forbidden: it would reset limits and corrupt local spans. Pass two may instead borrow the already-frozen source arena for its explicit frame parsers, since no more source insertion occurs. Test suspended/resumed parsing against uninterrupted parsing, including limit edges and errors. No unsafe self-reference, leaked source buffer or unbounded native recursion is needed.

Exact checkpoint interface proposal (crate-private, opaque, deliberately not Clone):

```rust
struct ContentCheckpoint { /* all parser state plus LexerCheckpoint */ }
struct LexerCheckpoint { /* source pointer/length, cursor, emitted, limits, failure */ }
enum ContentResumeError { SourceMismatch, InvalidCursor }
enum LexResumeError { SourceMismatch, InvalidCursor }
impl ContentParser<'_> {
    fn into_checkpoint(self) -> ContentCheckpoint;
}
impl<'a> ContentParser<'a> {
    fn resume(bytes: &'a [u8], checkpoint: ContentCheckpoint)
        -> Result<Self, ContentResumeError>;
}
// Lexer has matching consuming into_checkpoint/resume methods.
```

The saved source identity is the original slice pointer and length. Resume compares them without dereferencing any saved pointer and checks cursor <= length. The traversal frame owns the same immutable Arc<Vec<u8>> for the entire checkpoint lifetime, preventing deallocation/address reuse; the root PageContent buffer likewise stays alive. These private APIs do not accept a changed Vec, mutable source, fresh suffix slice or public forged checkpoint. No source address escapes into output or errors. Higher-level frame records also carry the source arena ID and invocation ID; source-pointer equality is an internal identity assertion, not PDF object identity. If future cross-thread parser transfer is required, replace this private assertion with an owned source handle rather than declaring raw pointers Send.

Checkpoint conversion moves all fields, including any lexer and parser sticky errors, with no heap allocation and no cloning of error payloads. It carries the original ContentLimits/LexLimits, lexer emitted-token count, parser tokens/nodes/owned_bytes/operations, absolute cursor and failure state. Resume restores all of them exactly. The live parser reservation is attached to the traversal frame for the whole source pass; suspension neither refunds it nor issues a new reservation. Only final successful EOF can apply the existing measured refund rule; an error keeps the accepted conservative failed reservation. Small checkpoint/frame storage is charged once in the pre-admitted bounded frame arena. Each suspend/resume charges constant orchestration work before the operation.

Tests must prove uninterrupted versus suspended/resumed operations, spans, usage, and failures are identical, including every operation boundary, EOF and sticky failure; one-below token/node/payload/operation caps; different allocations containing equal bytes, different lengths and shifted-slice rejection; already-exhausted limits; separate sticky lexical and parser failures; stable absolute error offsets; and repeated nested suspension without quota reset. Checkpoint APIs are crate-private to keep source lifetime/identity guarantees under the authenticated traversal's control. This is not a general public guarantee for arbitrary callers that drop/reallocate a source between calls; the owning traversal, not pointer equality alone, enforces the lifetime invariant.


Cycle detection compares the active ancestry's resolved Form stream object IDs, charging each comparison before access. A stream invoked again after an earlier invocation completed is valid reuse, not a cycle. Even if resources change, a repeat on the active ancestry explicitly refuses. No global visited set may suppress later painting. A branching DAG is bounded by invocation count as well as depth; decoded-byte caching alone does not bound its expanded work.

## Limit and usage proposal

Use the existing PP1 cumulative encoded-byte, decoded-work, parser input/token/node/payload/operation, prepared-font, Program operation/work and allocated-request partitions for both root and Form work. Child decoders/parsers receive the intersection of local limits and remaining session quotas, reserving before invocation. Cache hits do not re-charge decode work that is not done, but charge lookup, reference validation when performed, each parser pass, and every actual invocation. Parser failures retain their full conservative reservation unless the parser exposes complete failed work. This preserves the accepted PP1 failure policy.

Add a small Form-specific checked ledger with corresponding fields in FormUsage. Proposed default values below are for the cumulative session; per-page limits may be lower and are intersected with remaining session quotas. Values need checking against the two new corpus pages and generated fanout tests, not inflation to hide failures.

| FormLimits field | Proposed default | Admission/meaning |
| --- | ---: | --- |
| max_invocations | 100,000 | each attempted used Do before resolution, including failure; no reset per page |
| max_unique_streams | 4,096 | newly retained decoded Form source records |
| max_resource_scopes | 8,192 | newly authenticated effective resource dictionaries |
| max_resource_bindings | 262,144 | indexed Font/XObject name entries before traversal/storage |
| max_name_bytes | 16 MiB | owned resource/callsite names; exact byte lengths |
| max_metadata_items | 1,000,000 | visited resolved dictionary/array values before examination |
| max_reference_hops | 1,000,000 | each resolution hop, including failed chains |
| max_lookup_work | 100,000,000 | ancestry checks, scope/cache/binding comparisons including compared name bytes |
| max_origin_records | 12,000,000 | real-operation and synthetic-boundary provenance entries before storage |
| max_allocated_bytes | 256 MiB | additional Form scope/cache/invocation/origin full allocation requests |
| max_diagnostic_allocated_bytes | 64 MiB | one lazily pre-reserved <=64-frame diagnostic buffer per attempt containing Do; separate from output allocation |
| max_depth | 32, hard ceiling 64 | structural active Form frame bound, not a cumulative counter |
| max_stream_decoded_bytes | 8 MiB | structural per-Form decoded length; overall decode work still PP1-cumulative |
The Form allocation partition must not double-charge bytes already charged in PP1's root/form decoded buffer partition. Name the owning partition for every new Vec, Arc payload/handle, scope index and source record. Temporary fixed-size depth arrays are a separately documented bounded stack partition; if parser-bearing frames require heap storage, pre-admit the full fixed-capacity frame allocation instead. Fallible Vec reservation records the full requested new capacity at each growth, not just the delta/final size. No unchecked BTreeMap collection of all Form resources.

Use checked `remaining`, `intersect`, and transactional usage accumulation helpers, as in R21. Per-page failure merges all work and reservations already consumed. Form lookup failure, cyclic metadata, unsupported child operator, failed Tf preparation and invalid trailing suffix cannot refund earlier decode/parser/execution costs. Cache insertion is transactional after successful validation; partial invalid metadata is not marked reusable. Valid decoded bytes may remain cached after a later semantic refusal, with their original allocation/decode charges retained.

## Required tests and evidence gates

1. Baseline preservation: all existing normal/MSRV tests, strict all-target/all-feature Clippy, formatting and wasm checks; no change in accepted no-Form page traces, font identities, glyph commands, or certified R19/R21 behavior.
2. Original documents: both `redact-shared-only.pdf` pages compare ordered events, source bytes, font IDs, placements and path/glyph geometry against a native Form-aware oracle. Mixed-image pages continue to refuse atomically at their unsupported image with zero CLI stdout. Report all 29 old refusals again, without counting unsupported pages as matches.
3. Resource identity: colliding local/caller F1; explicit empty replacement; absent/null Resources inheritance through two levels; same Form invoked under different caller scopes; selected inherited font with no local Tf; alias names to same indirect font; same name to different objects; missing local name must not search outer scope. Used Tf without shown text still prepares and can fail.
4. Geometry/state: noncommuting caller cm/Form Matrix/page rotate/crop/UserUnit; sheared/rotated BBox clipping versus its incorrect AABB; nested clip parentage and restores; caller fill/stroke/line style/text parameters before and after Form; multiple identical Form calls preserve order. Include a nontrivial glyph Form, a filled/stroked path Form, inherited text font and inherited color.
5. Structural refusal: direct/indirect self cycle, two-Form cycle, DAG fanout, depth exact boundary, malformed references, non-stream target, wrong subtype, Group/OC/default-color overrides, invalid Matrix/BBox, q/Q underflow or unclosed save, BT/ET leakage, current path/pending clip at Do, unfinished child path, late unsupported suffix.
6. Provenance and ownership: original root cross-stream padding/repeated occurrence remains exact; identical Form stream invoked at different callsites has distinct invocation IDs but same retained bytes; nested error identifies each callsite; all successful output remains valid after dropping document/session.
7. Budgets: exact limit and one-below for each nonzero ledger; repeated successful pages, repeated failed selection/decode/parser/font/child execution; no cache-hit work escape; no per-invocation reset; full allocation growth requests. Failure remains atomic and session counters never decrease.
8. Independent reference: extend the native processor at actual Form entry/exit with resolved resource scope and actual object IDs. Do not flatten fixture strings with the production logic as the only oracle. A separate small exact affine/state model should generate nested cases and verify placement/clip geometry. Pixel tests reuse the accepted R19/R21 sampling policy and certify boundary-sensitive cases rather than widening tolerance.

## Suggested ownership after approval

Adapter owner: `form_resources.rs`, scoped loader hooks and PP1 two-pass traversal/provenance; coordinate any `font_resources.rs` changes explicitly. Adapter additionally owns the coordinated ContentParser/Lexer checkpoint hooks if that explicit-stack design is adopted. Root: C0/P0/T0 typed boundary and resolved-font hooks, TextFonts indexed registry support, exports/CLI and downstream origin display. Reviewer: independent scope/state/cycle/budget/ownership tests and generated model. Oracle: Form-aware native reference, full corpus classification and geometry/pixel evidence. If this split would make resource and registry borrowing too coupled, first land reviewed crate-private interfaces, then assign disjoint file ownership. No concurrent edits to the same interpreter files.

R22 acceptance should name its narrow Form execution capability and the remaining image/composite/marked-content limitations. It must not imply that all PDF forms, transparency groups, annotation appearances or the complete engine rewrite are implemented.
