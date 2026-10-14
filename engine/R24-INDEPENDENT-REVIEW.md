# R24 independent review

Review performed on 2026-10-06 by a separately delegated reviewer after the user's explicit request. Scope is the isolated Rust engine in `/Users/owner/.codex/worktrees/rust-pdf-engine/TPDF/engine`. Production source, fixtures, R23 reference artifacts, application, commits and publication were not modified by this reviewer.

## Initial findings and disposition

The initial production loader review found no blocking correctness defect in the claimed opaque Gray8 soft-mask subset. The acceptance verifier did have two blocking evidence-completeness defects:

1. **P1 — Require the raw evidence that acceptance claims to replay.** Original `tools/verify-soft-mask-gate.py:29`, `:46` and `:62` made positive attached-native-mask evidence and preserved Rust render artifacts optional. Removing all 44 positive `native_mask` artifacts, assigning fabricated native refusal text, and removing all 617 preserved `rust` artifacts still passed full `verify()`. Thus an incomplete run could be reported as the complete native/provenance/pixel gate. Require category-specific evidence sets and coherent refusal/success fields before interpreting artifacts.
2. **P1 — Require complete source, tool and executable identities.** Original `tools/verify-soft-mask-gate.py:39-41` required only part of the identity map; the six executables, frozen R23 baseline, imported model/generator tools and native pins could all be omitted. The Rust-check source manifest similarly iterated supplied entries without requiring completeness, permitting an empty source binding. Require complete explicit identity and source manifests, executable role coverage, and bind executed executable paths/digests to verified check evidence.

The combined original reproduction is `/private/tmp/r24-independent-omissions.json`; full verification returned all success counts in 10.76 seconds despite deletion of 44 native masks, 617 rendered artifacts and 12 identity entries. This demonstrated actual false acceptance, not just a hypothetical missing assertion.

The primary agent prepared fixes in the R24-only verifier and differential harness: exact evidence schemas, stored refusal diagnostics and unmasked Rust outputs, explicit executable roles, complete source/tool identity sets, fresh check/source bindings and log outcome checks. Initial independent source inspection of those fixes found the omission paths closed. Fresh evidence and independent replay now confirm both findings resolved; see the final verification results below.

## Production and reference-source audit

Reviewed the R24 design and design-review requirements, implementation record and contract; `src/image_resources.rs`; owner tests and page diagnostic integration; the two native R24 oracles/build scripts; differential runner; corpus/source-glyph model; and acceptance verifier/reports.

The loader preserves document-local ownership and semantic role admission. A regular cache hit cannot supply a mask without mask-role checks; cached masked bases reauthenticate their dependency, and cached masks still obey current shape/filter limits. The separate completed certificate table is populated only after a complete effective-byte opacity scan. Normal all255 and inverted all0 masks are treated consistently; partial planes cannot publish the dependent base. Dependency attempts are charged before edge resolution, scan reservations cover the complete plane, failed scans retain work, page ceilings intersect session allowances, and successful certificates alone avoid rescanning. Arc clones do not receive spurious header charges; assets retain full dependency ownership after document/session/input lifetime. Certificate/cache capacity growth is charged before allocation, and the page diagnostic reservation uses the expanded ImageError size.

The dedicated stream-edge resolver preserves the final indirect stream identity, bounded alias/reference handling and ordinary metadata's stream refusal. Mask-role validation excludes nested dependencies, transfer functions, embedded-mask semantics and non-Gray masks; normal image admission supplies the remaining image dictionary/filter/sample-shape checks. The one-edge role flag prevents recursive mask loading.

The native oracle loads the original base and enumerates its actual attached `image->mask`, with original source edge and Gray8 checks. The tracing device additionally verifies that the native clip callback operand is the loaded base's mask, records all alpha bytes and requires original clip/fill/pop ancestry. It does not prove opacity by rendering only. Live independently rerun native and Rust probes matched the original independent decoder for all 60,000 samples in `p3/edit-objects.pdf` and 3,240 in `redact.pdf`. An independently rerun transparent control changed 1,728 rendered RGB channels from the unmasked document. Results: `/private/tmp/r24-independent-live-native.json`.

The source-glyph correction uses original text operands and transforms, pinned AFM widths and native unhinted raw CFF coordinates; it does not feed Rust glyph output into the model. All 21 source/native placements are checked and unsupported text syntax is refused. The correction preserves the inherited path/channel-bound policy rather than widening it. In an independent in-memory countercheck, replacing source reconstruction with the native f32 command coordinates reproduced exactly nine violating channels in three pixels at 72 DPI; the source correction validates all 21 glyphs. Results: `/private/tmp/r24-independent-glyph-correction.json`.

## Independent regressions

Five independently authored Rust tests in `tests/review_r24_soft_masks.rs` pass against the reviewed loader:

- 64 reference hops including the terminal mask stream succeed; 65 refuse, preserving final identity and budget.
- A denied certificate allowance retains the independently decoded mask but does not publish the base; a later page succeeds with only the base redecoded.
- Failed inverted late-alpha scans reserve all four samples again across page resets and respect session certificate/scan caps.
- Ordinary cached mask images cannot hide indirect TR or SMaskInData mask-role semantics.
- A null alias charges its source dependency and reference hops, while stream-valued ordinary TR metadata remains invalid.

The independently authored `tools/test-soft-mask-independent-review.py` first requires the complete unmodified report/checks to pass, then removes raw artifacts from all representative result categories, removes each identity/source binding, mutates executable roles/digests, fabricates positive native refusal and removes/fails every Rust check. The positive control prevents stale hashes from producing meaningless all-rejected mutation results.

## Final verification results

**Independent review passes for the bounded R24 milestone. Both initial P1 findings are resolved; no remaining blocker was found.** The fresh unmodified report and Rust check manifest passed positive controls before any negative mutation was attempted. All 407 independent omission/source/check/executable mutations were rejected. A separate full final verifier invocation passed and rejected its additional 23 built-in structural/raw/witness mutations.

The verified report covers all 634 corpus attempts: 617 byte-identical preserved R23 outputs, two new opaque-mask outputs, and 15 remaining refusals. It includes 44 generated positive and 12 generated negative cases and 63,240 actual corpus alpha samples. Complete source/check binding covers 156 Cargo/Rust source/test/example files, all six executables, and 181 report identities. Fresh check logs confirm 814 tests on each current/MSRV toolchain, formatting, strict Clippy, release build and wasm compilation. The five independent Rust regressions are included in those totals. The reviewer also directly compiled/executed those five tests before integration.

Reproduction commands, run from the engine directory:

```sh
python3 tools/test-soft-mask-independent-review.py /Users/owner/.codex/worktrees/rust-pdf-engine/TPDF/engine --output /private/tmp/r24-independent-gate-results.json
python3 tools/verify-soft-mask-gate.py --output /private/tmp/r24-independent-final-gate.json
```

Preserve these independently produced files with the recovery checkpoint:

- `/private/tmp/r24-independent-gate-results.json`: 407 detailed rejection results, following successful positive controls.
- `/private/tmp/r24-independent-final-gate.json`: separate complete replay and 23 built-in mutation rejections.
- `/private/tmp/r24-independent-live-native.json`: live original-mask/native/Rust comparisons and transparent native control.
- `/private/tmp/r24-independent-glyph-correction.json`: independent f32-only countercheck reproducing nine channel errors in three pixels.
- `/private/tmp/r24-independent-omissions.json`: historical pre-fix false-acceptance reproduction; intentionally invalid under the corrected gate.

This acceptance concerns only exact proved-opaque Gray8 soft masks with the documented dependency/profile restrictions. No general transparency, broader image masks, or complete application replacement is claimed. `redact.pdf` retains its separate later marked-content refusal. The primary agent should preserve the final verified checkpoint before advancing.
