# R22 independent Form execution design review

Design only. The revised R22-FORM-PROPOSAL.md is suitable for the narrow ordinary
Form capability once the following invariants and tests are part of its contract.
There is no need to flatten or rewrite source PDF bytes, reset an interpreter per
Form, or introduce a semantic result cache. No production changes were made here.

## State and frame boundaries

One C0 interpreter with global path/text/clip/event arenas is the correct design.
A Form inherits the current selected font ID, text parameters, colors, line style,
CTM and clip, then saves/restores them at its own boundary. Resource scope changes
must not reinterpret an already selected font. Test a caller-selected F1, a local
F1 with a different object, inherited text with no Tf, a local Tf and subsequent
caller text. Reusing the same form with different current colors/fonts must repaint
under each caller state, including when decoded bytes come from cache.

Entry may internally execute typed q/cm/re/W/n, with one public execution index
and explicit synthetic span 0..0; internal operations still pay child work/storage
charges. Their intermediate events must map to the Enter boundary origin and its
retained original Do callsite. Exit similarly emits the actual clip restoration
in order. A token must be opaque, single-use and LIFO; stale/out-of-order use must
fail before state mutation. Keep a frame floor inside the interpreter so an
ordinary child Q cannot pop the implicit save or caller state. Entry or child
failure makes the private interpreter sticky and never publishes a prefix.

Entry, exit and every child Do must enforce outside-BT and no active/pending path.
Local q/Q and BT/ET balance is checked before exit; a child cannot begin text and
leave it for the caller to finish. Preserve the current T0 refusal of q/Q/cm inside
BT. Broader PDF text semantics cannot be inferred from that restriction: the
[PDF Association text-state correction](https://pdf-issues.pdfa.org/32000-2-2020/clause09.html#h2-9.4.1)
requires text matrices to participate when q/Q occurs inside a text object. R22
does not broaden that existing unsupported case.

Form Matrix composes before the caller CTM, with page rotation/crop/UserUnit still
applied once. BBox clipping uses its four transformed sides, never their page AABB.
Test reflection/shear/noncommuting transforms and a painted region inside the AABB
but outside the actual parallelogram. Exit restores the caller's exact clip parent
and every graphics-state parameter. Original Form corners, page bounds and empty
clip semantics must not be normalized into an unbounded region.

## Resources and identities

The proposed absence/null compatibility and explicit-dictionary replacement are
an acceptable declared subset. The [approved Form Resources correction](https://pdf-issues.pdfa.org/32000-2-2020/clause08.html#h2-8.10.2)
requires independent Forms to carry all their named resources, while distinguishing
historical dependent usage. This is not a claim that absent Resources conforms to
every PDF version. Pinned native outer-frame fallback may accept missing names
that Rust deliberately refuses; record a strict-scope refusal, not native refusal
parity or a matched rendered result.

Keep three identities separate: decoded Form stream, the explicit Resources
provider, and the invocation's effective resource scope. An absent-Resources form
invoked under scopes A and B must bind separately at each call; never freeze A into
its metadata cache. Explicit empty Resources replaces lookup scope. Category/name
misses do not search an outer dictionary. A selected inherited TextFontId still
works until a new Tf requests resolution. Every effective resource scope gets its
own meaningful DefaultGray/RGB/CMYK exclusion checks.

Scoped `(scope_id, original_name)` bindings and indexed registry selection avoid
name rewriting. The original Tf operand must match the prepared binding, and the
registry ID must belong to that prepared scope/name. Preserve existing public
name-based duplicate checks. Indirect font object identity can reuse mapping/font
caches across aliases; direct dictionaries need an explicitly defined owner/entry
identity that cannot merge unrelated scopes. Inherited page Resources providers
and shared indirect Resources dictionaries deserve tests, rather than assuming all
physical resource owners are leaf page objects. All used Tf operations are prepared,
including those with no shown text or inside a clipped-away Form.

## Provenance and ownership

Every global execution step must have an authoritative origin: root joined-content
span, invocation-local Form byte span, or synthetic boundary. Do not project a
Form span through PageContent's root occurrence map. Repeated uses share retained
source bytes but have different invocation IDs/call origins; nested invocation
chains preserve original Do spans. Boundary events have no fabricated stream span.
No-Form pages retain their existing operation numbering.

Successful output owns its bytes, names and scope/invocation/origin descriptors
after document/session destruction. A failure cannot return just an invocation ID
from a discarded private output: retain a bounded owned diagnostic chain with page,
Form object, local operation/span and callsites. Error construction itself must
not allocate an unbounded chain or copy arbitrary source text. The root and Form
source caches must have stable identities even when vectors grow between calls.

## Traversal and budgets

The revised consuming parser/lexer checkpoint is preferable to reparsing suffixes.
It must retain source identity, cursor, sticky state, limits, all consumed counters
and any owned parser state. Resuming validates source identity and bounds and cannot
reset quotas, permit a previously failed parser, or reinterpret a token fragment.
The checkpoint's parser reservation stays active while its frame is suspended;
resumption neither refunds nor reserves a second copy of the same phase. Actual
child work and both parser passes remain cumulative. On a nested failure, every
active phase retains its conservative reservation under the existing PP1 policy.

Explicit DFS controls depth and avoids self-referential movable cache borrows.
The fully frozen source set in pass two may be borrowed; pass one must use the
approved consuming checkpoints or owned independent source handles. Bound the
complete frame allocation/stack partition, not just logical depth. Actual Do
traversal, cache lookups, ancestry comparisons, name-byte comparisons and child
execution remain charged on cache hits. A branching DAG can explode in invocations
without adding unique streams, so both counters are necessary.

Cycle detection uses resolved stream object IDs on the active ancestry. An alias
reference chain leading back to the same object is a cycle. Reuse after return is
not a cycle and must execute again. Cache insertion occurs only after its required
validation; a later semantic failure may retain valid decoded bytes but cannot
mark an invalid scope or prepared invocation successful.

Define invocation accounting unambiguously before implementation: one logical
used Do may be admitted in preparation, while the second-pass replay pays parser
and interpreter work separately. Do not accidentally report two invocations for
one execution merely because there are two passes. Every failed resolution attempt
must still consume the intended attempt quota. Distinguish session-retained unique
streams/scopes from per-page output references and from failed admission counters.

Checked remaining/intersect/accumulate helpers should cover every ledger and merge
on all exits. Name the unique owning allocation partition for decoded buffers,
Arc handles, frame/checkpoint buffers, registry bindings, scope indexes, invocation
records and origins. Parent reservations must not be refunded when merely suspended.
Original parser/schema array-walk preflight protections must also apply to Form
operations. Return no partial page if a late image, unsupported operator, malformed
Tf or budget failure follows otherwise valid child output.

## Acceptance gate

The proposal's generated state/resource/cycle/geometry/ownership cases are required;
the two actual Form-only pages do not exercise these hazards. Add explicit tests
for resource-provider aliases/inheritance, cross-frame Q, stale boundary tokens,
source mismatch/sticky checkpoint resume, nested failure diagnostics and repeated
same-stream calls under different caller state. Exact/one-less and repeated failure
checks must cover all nonzero counters, with no semantic cache work escape.

The measured corpus gain is two Form-only pages, not all 16 Do-refused pages.
Image-containing children still refuse atomically. Use an independent native
Form-aware entry/exit trace plus a small exact affine/state model; production
flattening must not supply its own expected provenance or clip result. Distinguish
strict replacement-scope refusals from native recovery. Preserve the accepted R21
sampling, numeric and known native color-policy distinctions without widening them.
