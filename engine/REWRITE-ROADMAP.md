# Completion roadmap for the Tandem PDF Rust engine

R24 update (2026-10-06): proved-opaque image SMask support is accepted after independent review and complete automated gates. See R24-IMPLEMENTATION.md before advancing. This user request covers R24 only; general transparency and all later work below remain open.

Updated 2026-10-06. This is the completion inventory for the user's instruction to
continue the rewrite until it is done. It supersedes the original overnight time
limit, not the safety, licensing or isolation requirements in [CONTRACT.md](CONTRACT.md).
The recurring automation remains paused. Current work stays inside `engine/`;
production application code and immutable fixtures remain untouched.

**The rewrite is not complete.** Accepted R23 provides bounded reading, restricted
page programs, embedded simple TrueType and two pinned standard-font outlines,
and unhinted filled/solid-stroked page rasterization with ordinary nested Forms and opaque Gray8/RGB8 image XObjects. R23 passed796current/MSRV tests plus complete native/model/corpus acceptance:557/572pages at18DPI and60supplementary72DPIrenders. Validated opaque soft masks are the next reviewed image extension.
These remain internal subsets of the application engine. Compiling to WebAssembly
or passing the present fixture corpus does not complete the replacement.

## What completion means

The concrete target is the PDF behavior used by Tandem PDF's existing worker and
application, with a safe Rust implementation and explicit ownership and resource
limits. This is not a promise to implement every MuPDF format or every optional
PDF feature. Existing application restrictions remain legitimate restrictions;
a new refusal where the application currently supports an operation is a tracked
compatibility gap, not a completed feature.

The source of scope is [engine.ts](../src/renderer/engine.ts), particularly its
`SessionRequest` union and `dispatch` implementation, [worker.ts](../src/renderer/worker.ts),
[engine-core.ts](../src/renderer/engine-core.ts), [content-edit.ts](../src/renderer/content-edit.ts),
[redact.ts](../src/renderer/redact.ts), and [sign-prepare.ts](../src/renderer/sign-prepare.ts).
The existing application tests provide operation-level acceptance cases. Browser UI,
Electron shell, filesystem dialogs and independent CMS/PKCS12/trust cryptography
are not automatically rewritten in Rust; the PDF data and serialization interfaces
those components require are part of the engine replacement.

## Capability inventory

The table retains the original R17 baseline to show the full replacement scope.
Since then, R18 accepted embedded outline placement, R19 accepted owned filled-page
rasterization, and R20 accepted explicit Helvetica/Times-Roman canonical outlines.
R22 renders 545/572 fixture pages, including all 500 stress pages, with 27 explicit
refusals. STATUS.md records current gates and remaining gaps.

| Application behavior and current call surface | Accepted Rust capability at R17 | Remaining completion work |
| --- | --- | --- |
| `open`, `close`, `page-ids`, page bounds and document information | Strict unencrypted PDF reading, classic/incremental/xref streams and object streams; page inventory, inherited geometry; 49 unencrypted fixture documents / 572 pages | Session lifecycle, metadata and stable page identity matching the worker, application-compatible supported-input policy, password handling; explicit diagnosis of every refused existing fixture. No silent repair claims. |
| `render`, `thumbnail`, `compare-render`; RGBA page pixels with view bounds and scale | P0 paths, T0 positioned source codes, C0 mixed programs, PP1 resource/content loading; restricted R0 integer orthogonal raster experiment | Full page execution and rasterization for the application's inputs: text outlines, fills/strokes/dashes/joins, arbitrary transforms, antialiasing, clipping, images, nested Forms, color spaces, transparency/blending and annotation/widget appearances; scale, alpha and pixel ownership contract; bounded thumbnail/cache behavior. Current traces are not pixel rendering. |
| Embedded and substitute fonts used for rendering and new text | Simple font encodings/widths/ToUnicode; explicit pinned Helvetica/Times-Roman metric policy; format4 cmap; supported simple FontFile2 selection; unhinted simple glyf outlines | R18 placement first, then composite TrueType, required cmap/encoding and glyph-name selection branches, Type0/CID fonts and relevant CFF/Type1 paths, Base14/fallback fonts, vertical text where required, font embedding/writing and consistent advances. Missing glyphs, unsupported mappings and valid empty outlines must remain distinguishable. Source codes, Unicode and GIDs cannot be conflated. |
| Images and Form XObjects in existing documents; `pdf-from-images`, image stamps, object/clipboard editing | General bounded stream filters and predictors; raw streams preserved by full rewrite | Image decoding, masks/soft masks, decode arrays and color conversion; inline images; shared Form resources, matrices, recursion and cycles; bounded image construction/embedding and pixel output. Resource aliases must not create unbounded repeated work. |
| Structured text returned with `render`; `search`, `text-all`, `compare-text` | Positioned simple-font codes and explicit ToUnicode decoding within the supported trace subset | Structured lines/spans/quads and application text order, ligatures and ActualText/marked content, CID/vertical text as required; extraction/search hit identity and caps; comparison/export text behavior. Glyph-outline mapping is independent of extraction Unicode. |
| `fields`, `field-set`, `field-add`, `field-props`, `field-delete` | Generic PDF object reading only | AcroForm field tree, inheritance, text/checkbox/radio/combo/list/signature metadata, values and appearance regeneration; field creation/deletion/properties and bounds; read-only/required flags and supported validation. Preserve fields during page graft/copy. PDF JavaScript remains disabled. |
| `annots`, `annot-add`, `annot-set`, `annot-move`, `annot-delete`, `stamp` | Generic dictionaries only | Existing Text, Highlight, Underline, StrikeOut, FreeText, Square, Circle, Line, Ink and Stamp behavior; quads, styles, opacity, author/content metadata, appearance generation and rendering, bounded ink/image payloads. Drawn signature appearances are separate from cryptographic signatures. |
| `outline*`, `links`, `link-*`, `labels*`, `layers`, `layer-set` | Generic objects and page inventory only | Bookmark hierarchy and destinations, link hit areas and safe destinations, page-label number trees; optional-content visibility/state and its effect on render/edit; preserve/remap navigation during page changes. |
| `page-rotate`, `page-delete`, `page-move`, `insert-pages`, `extract-pages`, blank/image PDF creation | Read-only page inventory and geometry; full document serialization without mutations | Owned document mutation, inherited attributes, page-tree rebuild/identity, object allocation and cross-document graft maps; resource/annotation/field/link sharing and remapping; deletion semantics and valid new documents. |
| `edit-targets`, `edit-text`, `edit-paragraph`, `text-add`, `edit-replace-preview/apply` | Bounded token/parser spans and source provenance; no application edit API | Match the existing supported edit subset: stable stream/operator/hash identities, stale-target rejection, text and paragraph geometry, font/code repertoire checks, ActualText maintenance, copy-on-write Form traversal and exact content rewriting. Preview and apply must agree and failures must leave the document unchanged. |
| `edit-object`, `edit-objects`, `edit-batch`, `clip-copy/paste` | No editing transaction or clipboard layer | Transform/remove/copy text, images, Forms and application-supported field/link/mark objects; clip preservation, shared resource isolation, bounded clipboard payload and grafting, one atomic undoable operation per requested batch/paste. |
| `redact-add/set/search/preview/apply` and sanitization | No redaction implementation | Existing preview/inventory plus permanent text/image/vector removal and overlay behavior; shared resource handling and optional metadata/attachment/field/etc. sanitization as implemented by `redact.ts`; save/reopen and forensic verification that removed information is absent. Painting a rectangle is not redaction. |
| `save`, `validate`, `saved`; batch/export operations | Bounded canonical object serializer and strict full rewrite of eligible unencrypted documents; raw stream preservation; native reopen/preservation evidence | Serialize mutations and new resources, garbage collection semantics, save validation, dirty/saved revisions, encryption preservation/settings and signature-aware incremental serialization. Existing full rewrite explicitly refuses signature-bearing documents and other unsupported cases. |
| `undo`, `redo`, session and compare document lifecycle, `batch-file` | No application session journal | Atomic commit/abandon journal, saved-position tracking, generation/revision cache invalidation and stale handles; separate compare and scratch batch documents; isolated failure and bounded lifetime of returned buffers/resources. |
| `authenticate`, `security-set`, permission gating and encrypted saves | Encrypted documents explicitly unsupported | Required RC4/AES variants, password authentication, owner/user permissions, encryption dictionary and crypt filters, password change/removal and encrypted serialization; reopen after authentication to avoid stale decrypted objects. Preserve current operation permission policy and signed-document restrictions. |
| `sign-list`, `sign-prepare`; signature verification inputs | Signature-bearing full rewrite explicitly refused | PDF signature dictionary/field inventory, stable ByteRange and Contents reservation, appearance and DocMDP handling; incremental output preserving the exact signed prefix and earlier signatures; bounded reserved space and exact offsets for external CMS signing. Existing external signature verification/crypto remains connected through a defined interface. |
| Worker messages, transferred buffers, runtime isolation and packaging | Safe Rust crate, native CLI and wasm32 compile check | A concrete worker-compatible WASM binding and session adapter, typed errors/replies, ArrayBuffer lifetime/transfer behavior, worker isolation/CSP/no Node/no JavaScript execution, responsiveness and budget cancellation, cross-platform runtime/build/package checks. Wasm compilation alone establishes none of these. |

## Sequenced work packages and acceptance

1. **Finish the read/render foundations.** R18 outline placement, R19 filled-page
   rasterization, R20 pinned standard-font outlines and R21 solid strokes are accepted.
   Ordinary Forms are accepted at R22. Complete images, then remaining required font formats,
   color/transparency and a complete supported-page raster path. Keep exact semantic
   comparisons and independent geometry models, then introduce native pixel gates
   with stated tolerances and separate text/annotation visibility cases. Preserve
   per-page failure isolation, output ownership and cumulative resource accounting.
2. **Complete read-only application behavior.** Add structured extraction/search,
   navigation, layers, fields and annotation inventory; encrypted reading and
   authentication are part of this capability gate. Test against application
   fixtures and operations, not just synthetic Rust unit tests.
3. **Add owned mutation and transaction architecture.** Establish object allocation,
   copy-on-write/shared-resource semantics, journal/undo/redo and valid atomic saves.
   Then implement page assembly/navigation, annotation/form editing, content edits,
   clipboard and batch operations. Each change requires reopen validation and
   independent preservation checks on unaffected content/resources.
4. **Complete security-sensitive writes.** Permanent redaction and sanitization,
   encryption changes, and signature-aware incremental preparation receive their
   own adversarial and native interoperability gates. Do not infer these from a
   generic serializer or use appearance-only evidence for security properties.
5. **Build and verify the application adapter.** Exercise the complete existing
   worker request/reply contract with the Rust implementation, retain the current
   UI limits and transaction semantics, run the existing app and drive suites,
   smoke/package gates and browser/Electron checks. Current isolation means this
   implementation remains in `engine/` until an explicitly recorded integration
   stage; no production replacement has happened.

The ordering may interleave independent components, but every package needs its
own accepted design, regression/adversarial tests, independent review and evidence.
Do not assign a percent complete from milestone count or corpus page count: the
remaining capabilities differ greatly in size and many are shared dependencies.

## Final completion gate

Completion requires a reviewed operation-by-operation matrix covering the rows
above and all current `SessionRequest` variants; no unrecorded runtime delegation
to MuPDF, unsupported-operation fallbacks or regressions hidden behind subset tests.
Every accepted compatibility restriction must be explicit and tested. All current
application-supported workflows must pass their engine, worker and end-to-end
checks on the replacement, including save/reopen and failures, with rendering/text
and resource/performance evidence on representative documents beyond the small
fixture corpus. Test supported desktop targets and the actual WASM runtime.

Preserve no-unsafe Rust, no OCR, MuPDF-derived AGPL and dependency attribution,
pinned reference provenance, immutable fixtures and bounded untrusted-input work.
Logical limits must describe what they count; do not present them as a universal
RSS ceiling or guaranteed recovery from allocator failure. Maintain native oracles
as test tools only. The final status must distinguish accepted engine completion,
application integration and actual rollout; no commit, push or publication is
implied by this roadmap.
