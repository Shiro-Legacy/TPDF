# Mixed page-program architecture: independent critique

Status: design review only. No production implementation or execution-coverage claim is authorized by this note. The initial conclusions below are based on the accepted P0/T0 APIs, the pinned native source, and discussion of the incoming PAGE-PROGRAM-DESIGN.md draft.

## State ownership and composition

A page program needs one logical graphics state and one ordered operation stream. Recreating a PathInterpreter or TextPositioner for each segment loses text-state persistence, pending paths, clip ancestry, resource limits and original operation numbering. Keep the engines alive for the entire logical page content.

The safest long-term boundary is a shared internal graphics coordinator, with standalone wrappers preserving the existing public APIs. A smaller initial bridge with P0 owning CTM/clip/colors and T0 mirroring CTM/text-state saves can be acceptable only under explicit invariants:

- The coordinator decides q/Q admission, underflow and depth once. Both engines receive the same shared command and original operation context. Before/after shared operations, assert equal CTM coefficients and save depths. An error in either engine permanently invalidates the combined executor, so partially advanced private state can never be published.
- CTM is initialized once from page geometry. Apply UserUnit/Rotate/crop translation once, not separately in a path adapter and again in text output. Initial page clipping uses the same page-space region for both branches; an empty page clip is distinct from Unbounded.
- Q restores fill/stroke colors, clip, line/dash parameters, CTM, font selection and every persistent text parameter. Text matrices belong to the text object. Keep the accepted refusal of q/Q/cm inside BT.
- An active path and pending W/W* are not graphics-state stack fields. q/Q must not snapshot/reset them. Existing P0 refusal of a CTM change with an active path remains authoritative.

A strict first mixed subset can refuse BT whenever a current path or pending clip exists, and refuse path construction/ending inside BT. This excludes native-tolerated interactions explicitly without inventing a reset. Spell out that these are path operations, not ordinary fill-color setters: color changes within BT must either be supported with immediate snapshots or be explicitly refused. Do not accept them but apply only the final color to all text.

Each text placement/run must own a small immutable snapshot of the active fill color and path-clip ID at the moment it is emitted. A single combined event arena indexes path events and text events in logical execution order; child arena IDs must be translated once with checked offsets if separate arenas survive. All text clip IDs refer to the same retained path clip arena. Clip restore events preserve ancestry rather than numeric depth alone. Tr3 retains position and intent, but native ignore_text has no color callback parameter; compare its color snapshot to independent state expectations rather than claiming native paint-color parity.

## Original provenance and bounded work

Dispatching only relevant operators makes child-local operation counters differ from page operation indices. Add an explicit internal original-operation context, including logical global index/span; do not manufacture no-op commands or reconstruct indices after failure. Keep one global operation budget and separate specialized counters for codes, paths, adjustments, shows, nodes, clips and resource work. A shared q counted by both children may conservatively consume specialized work, but must not become two logical operations in public provenance.

Page Contents arrays are one logical stream, using the already verified native space-join semantics. Never execute each stream as a separate program. Strings, comments, operands, q/Q and BT/ET can cross member boundaries. Operation spans are offsets in the joined decoded bytes. They can cross multiple ContentSegments and inserted padding; do not label them raw PDF offsets or attribute the entire operation to a single stream. Per-code string spans remain decoded-string offsets.

The page/session ledger must cap at least: visited/requested pages, parsed/discovered/executed operations, resource selections/reference hops, logical content members, referenced encoded bytes, all filter/predictor output stages, CMap work, joined bytes, retained output, cumulative allocation requests, and all specialized path/text records. Document xref/ObjStm metadata, page-content decoding and font ToUnicode decoding are currently separate subsystems; their combined work needs a stated document-session cap rather than three independently reusable full limits. Repeated stream/page references count as work even if identities repeat. Cache hits may avoid decoding but still charge lookup/selection work.

Failure charging matters: load_page_content currently returns usage only on success. A reusable page session cannot repeatedly retry expensive failed decoding with a refreshed global budget. Return charged usage on failures, conservatively reserve per-call remaining work before starting, or make the session terminal on such failure. Apply the same rule to preparation/discovery retries. Distinguish requested allocation/work totals from current retained bytes and allocator/process memory; existing P0 growth-delta accounting and T0 full-request accounting must not be silently summed under one ambiguous label.

## Prepare, freeze, execute ownership

Font discovery must collect every Tf resource selection, including a name selected but never shown. Missing Tf targets already fail T0 before showing; an adapter cannot weaken that rule by loading only names that produced glyphs. Preserve exact byte resource names and page/inherited resource identity. Two aliases may share the resource cache while remaining distinct named bindings in the page registry.

Use an explicit preparation phase that resolves the page's declarations and all needed Unicode/metric capabilities under the session ledger. Then freeze the loader for execution and build a borrowed TextFonts registry in that immutable execution scope. Avoid a self-referential owned PreparedPage containing both a loader and a registry borrowing it. PreparedPage can own joined bytes, segment provenance, needed identities/names and policy; the executor obtains immutable loaded references only after all mutable loading is complete. Output owns its descriptors and must remain usable after dropping the registry/input/session.

Builtin metrics were explicit in T0's /Builtin configuration. The document adapter needs an explicit caller policy for declared-only versus opt-in pinned Helvetica/Times-Roman substitution. Missing Widths must not silently enable a broader fallback. Preserve substitution provenance, refuse embedded-font/alias cases per M1, and keep ToUnicode authoritative for Unicode without using it to select advances. Do not turn incomplete maps or metric failures into successful partial page text.

If preparation reparses content, charge that second pass and prove the two passes consume identical immutable bytes and parse policies. If preparation retains Operation trees instead, bound their complete retained allocation and payloads alongside the joined bytes. Operator discovery alone is not validation/execution success.

## Required adversarial cases

| Case | Required result |
| --- | --- |
| Select F1, q, select F2/change every text parameter, show, Q, show | Second object uses F1 and every restored parameter; both path/text CTMs agree. |
| Red fill, q, green fill, text, Q, text | First text green, second red; snapshots cannot share mutable color storage. |
| BT text, change rg/g, text, ET | Distinct text colors within one object, or explicit complete-page refusal if outside subset. |
| q, clip A, q, clip B, text, Q, text, Q, text | Clips B∩A, A, initial clip respectively; IDs/parentage and unified event order valid. |
| W/W* pending followed by q/Q and n | Pending rule survives saves/restores and takes effect only at path end. |
| Active path or pending W followed by BT | Explicit atomic refusal under initial restricted policy; no discarded path. |
| BT followed by m/l/f/W/n | Explicit atomic refusal under restricted policy, including invisible Tr3. |
| Active path then cm, or Q restoring a different CTM | Preserve P0 mixed-transform refusal even across dispatch boundaries. |
| q, noncommuting cm, path fill, text, Q, more path/text | Exact independent transforms and native float32-compatible positions under original event order. |
| Alternating many tiny path/text objects just above global op limit | One cumulative limit with the correct failing original index/span; no reset or partial output. |
| Valid path/text prefix followed by gs/Do/BI/marked content/unknown op | Complete page execution fails; prefix is never a successful page program. |
| Contents split inside literal string/comment/TJ/BT or before Q | Same result as the native joined logical stream; honest cross-segment provenance. |
| Shared font encoding/CMap across pages, aliases, failed map, repeated page requests | Immutable cache reuse plus cumulative calls/decoding/failure charges; no renewed full budgets. |
| Same binding bytes in distinct inherited/direct resource dictionaries | Identity does not alias across owners/pages/documents. |
| Font selected without show, missing binding, then ET | Preparation/execution still rejects the missing selection. |
| ToUnicode maps byte32 away from space and another byte to space | One width per source code; Tw depends on original byte; selected glyph-width provenance retained. |
| Zero page area/empty initial clip with otherwise invalid text or paths | The adapter still validates/refuses unsupported content; invisibility cannot bypass budgets or semantics. |
| Huge number of pages/content references with tiny per-page programs | Document-level page, decode, operation and retained-result caps hold, including failed later pages. |

## Native evidence and actual fixture acceptance

Extend the bounded native mixed device only after authorization. It should emit one callback-order trace for fill/stroke paths, path clips/restores, fill_text and ignore_text under identity/raw content CTM or a separately documented page transform. Include path geometry, matrices, exact native color components, clip ancestry, text primary/continuation items and source-font identity where recoverable. Native batching boundaries need not equal Rust run boundaries; compare the normalized ordered semantic records while retaining the raw callback evidence. Do not discard degenerate path callbacks, empty paints, invisible text or fallback warnings without recording the known policy difference.

Require generated rational mixed-state programs and targeted native cases for every state interaction above. Native text uses the predeclared T0 float32 tolerance; path/style/color endpoint fields and event/clip topology should remain exact under the P0 policy. Do not broaden tolerances to hide a state mismatch. Native glyph IDs, fallback Unicode, reading order and rendered text bounds are not this milestone's oracle.

The proposed 543/572-page operator-inventory ceiling is only a candidate set. In particular, 500 pages belong to large.pdf; report these separately from the remaining43 candidate pages and the distinct documents. Final acceptance needs, for each original hash-checked page: actual preparation result, actual complete execution result, native/reference comparison or an explicit unverified status, source-code/path/event counts and precise refusal class. A recognized operator can still fail fonts, missing mappings, metrics, numeric policy, state structure, clipping, or budgets. Do not report a file as supported because some pages execute, and do not equate a successful parser/inventory scan with a successfully executed page trace.

## Pinned source observations

In MuPDF1.28.1 `source/pdf/pdf-op-run.c`, pdf_run_BT resets text/line matrices and bidi only; it does not clear the current path. pdf_run_ET flushes text/text clipping, not pending path clipping. pdf_run_W/Wstar set a pending rule and pdf_show_path applies it after painting, then clears it. pdf_run_Q, pdf_run_cm and material color setters explicitly flush native buffered text. The checked pdf_run_f/pdf_show_path entry does not itself establish an unconditional text-flush rule; the initial prohibition of path painting inside BT avoids depending on this corner. Keep these exact observations separate from general PDF normative claims and the already documented q/Q-inside-BT erratum difference.

## Draft review disposition

Read the completed PAGE-PROGRAM-DESIGN.md draft after the initial critique. It incorporates persistent engines, guarded shared-command broadcasts, immediate text context snapshots, original-index injection, explicit preparation/freeze, used-selection discovery, failed-work charging and the inventory-versus-execution distinction. The proposed guarded bridge is reasonable for a narrowly scoped first implementation; implementation authorization and passing execution gates remain separate decisions.

Before implementation, settle four details in the contract/API:

1. Root has selected retention of exact joined PageContent bytes/segments. Replace the draft's optional/discarded-buffer language accordingly and charge both bytes and segment occurrence records to retained output and cumulative allocation limits. This is useful for independently auditable cross-stream provenance.
2. Make the M1 substitution policy an explicit named caller option. Declared-only is the least surprising default relative to T0; an opt-in pinned-two-face policy may enable the larger fixture gate. If another default is chosen, document it prominently and preserve its per-code provenance rather than implying actual font-program metrics.
3. An already-open borrowed Document has already incurred xref/ObjStm decoding and allocations. Define these as a separately bounded external prerequisite, or explicitly admit measured prior usage when creating the session. A session created afterward cannot retroactively enforce an unknown earlier allocation cap.
4. State the CTM equality assertion convention. Both mirrors must remain finite; numeric coefficient equality matches the existing P0 semantic comparison and need not reject positive versus negative zero alone. Any genuine divergence is an invariant failure, not a synchronization opportunity.

The implementation must still resolve the pre-reserve aggregate budget plumbing; the draft correctly leaves after-the-fact usage summation insufficient. A prospective inventory ceiling does not satisfy any of these execution/resource requirements.

Follow-up: the final draft now incorporates all four clarifications above: exact retained content, DeclaredOnly/AllowPinnedBase14 policy, separately bounded prior Document work, and finite coefficient-wise equality with signed-zero equivalence. No remaining design clarification blocks a narrowly scoped implementation decision. The stated adversarial, aggregate-budget and actual-execution gates remain required before claiming support.
