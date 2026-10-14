# R23 image adapter accounting and ownership review

2026-10-06. Focused implementation review by the adapter author, with separate review of the root-owned image dispatch hook and existing loader. This is not independent acceptance of the whole R23 renderer or the Rust rebuild. It covers `form_resources.rs`, image integration in `page_program.rs`, `ImageLoader` ownership/counters, and direct `ProgramInterpreter::push_image` behavior.

## Findings resolved

- Image event exhaustion initially reached a generic allocation-count check and reported `AllocatedBytes`. The root added an explicit `Events` precheck; direct exact/one-below tests verify the typed refusal and sticky failure.
- The shared scoped resolver charged metadata/cycle work but omitted the document-index lookup reservation already added to `ImageLoader`. Root page, indirect metadata/XObject, and Form decoder object lookups now precharge twice the document object count plus two. Decoder reservation also covers the two separate stream-byte lookups. A test adding 64 unused objects measures the expected 128-unit root lookup increase and 256-unit root-plus-image increase; a one-unit ceiling refuses before the first indexed lookup. These accounting corrections change work usage/refusal boundaries, not rendered output.
- Wrapping an image loader failure in `Box<ImageError>` introduced an unreserved failure allocation. An inline variant was evaluated, but enlarged `PageProgramError` to 176 bytes and failed Clippy's large-error check. The adapter now reserves one `size_of::<ImageError>()` allowance in the Form allocation partition before the first image load on each page attempt. There can be only one returned image failure per attempt. If admission fails, an inline allocation-limit error is returned before loader work. The reservation remains charged on success or failure; no-image pages do not reserve it. Exact/one-below tests verify the allocation admission, loader-attempt count, original index/span and absence of execution.

## Ownership and cumulative accounting

`ImageLoader` binds its cache to one immutable document and the actual indirect image stream ID. Its compact samples and typed filter metadata are owned by `Arc<ImageAsset>`. Result-local `PageProgram::images()` contains only the assets used by that result; unrelated earlier session cache entries cannot change result-local IDs. Alias references share the actual asset, while names and effective scopes remain per-call facts. A cached Form without Resources resolves its image names anew in each caller's effective scope.

The image partition precharges filter descriptors, asset/Arc storage, geometric cache-table growth, retained samples and all admitted decoder output-stage allocations. Work refunds occur only after successful decoder reporting; exact-length failure still cannot publish an asset. Failed intended row/pixel/sample admissions remain consumed. Cache hits charge attempts and lookup work, recheck structural bounds, and do not invent decoding work. `begin_page` intersects new page ceilings with remaining cumulative session capacity without clearing caches or prior failures.

The Form traversal partition pays full geometric allocation requests for result asset handles, frozen image-call records, copied call names and origin records. Replay charges name comparison work and consumes those frozen calls sequentially, checking original source span, scope, asset object and result-local ID. It performs no image decoding or mutable cache access. Every image Do contributes one original operation and one image event, with no Form entry/exit synthesis. Parser, image decoding, traversal and program output remain distinct partitions. Logical conservative reservations are not measurements of allocator overhead or process RSS.

Late unsupported images or suffix operators discard the whole page result. Successfully decoded immutable assets may remain cached after the failed attempt; their admitted counters persist. Nested errors retain the enclosing Form chain and the image object's typed loader error. This review does not re-audit every legacy PP1/font/Form error representation or all previously accepted parser/decode allocation rules.

## Verification

After the accounting corrections, `cargo test --offline --test page_images --test page_forms --test review_page_forms` passed **45 tests**: 15 image adapter, four Form adapter and 26 independent Form regressions. The image suite includes all eleven Form/traversal counter partitions at exact and one-below ceilings, in both page and session limits (44 boundary cases), plus image diagnostic admission. Five direct image-hook unit tests passed before these adapter-only corrections; they cover state preservation, full CTM/clip snapshots, invalid operator/span/state, original indices, sticky failure, and operation/event/work/allocation boundaries. `cargo clippy --offline --all-targets -- -D warnings` passed after the corrections.

Remaining whole-stage gates are the root's integrated current/MSRV tests, formatting/WASM checks, exact image-aware full-page native trace/model corpus comparison, CLI atomic refusal evidence, and preservation of prior corpus rendering evidence. Native raster policy differences must stay explicitly separate from matches. No whole-stage acceptance is claimed here.

## Follow-on image family: evidence and bounded proposal only

The pinned original-object inventory `reference/image-corpus-inventory-r23b.json` contains 20 image objects: 18 accepted byte matches and two deliberate soft-mask refusals. The R23 invocation proposal identifies those image blockers on `p3/edit-objects.pdf:1` and `redact.pdf:1`; this inventory includes unused objects and does not by itself prove page reachability or eventual render success.

Direct inspection of the original fixture objects narrows the immediate follow-on candidate:

| Base image | Mask object | Mask format | Independently inspected decoded bytes |
| --- | --- | --- | --- |
| `p3/edit-objects.pdf` object 10, 200×300 Gray8 | object 12 | 200×300 DeviceGray8, Flate, no predictor | 60,000 bytes, all 255 |
| `redact.pdf` object 10, 180×18 RGB8 | object 11 | 180×18 DeviceGray8, unfiltered | 3,240 bytes, all 255 |

The base image in the first file uses PNG Predictor 15; its mask does not. Both masks omit Decode, Interpolate and Matte. Thus a deliberately narrow **validated fully opaque soft-mask** capability could be examined before general alpha compositing: authenticate and validate the mask stream under bounded depth/cycle/cache rules; require the admitted Gray8/default-or-normal Decode profile and compatible dimensions; meter a complete all-255 scan; retain base/mask provenance; refuse any nonopaque sample or unimplemented mask semantics. Existing `ImageLoader` cannot simply be called recursively without a mask dependency/cycle contract. This is a proposal, not implemented support or a substitute for future nonopaque transparency.

General Gray8 soft masks would additionally require independently specified sample alignment, exact alpha arithmetic/rounding and premultiplication policy at the 4× sample level, overlap/group/graphics-state compatibility, dependency budgets and native/model evidence. The current fixture masks cannot verify those behaviors because both are uniformly opaque. `redact.pdf:1` also has later marked-content semantics according to the R23 proposal, so removing its image refusal alone does not establish a newly accepted page. The inventory provides no measured need to prioritize JPEG/JPX or image interpolation ahead of this soft-mask family.
