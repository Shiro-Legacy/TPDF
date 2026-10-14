# Functional Rust workflow

Updated October 14, 2026. **Live source location:** `/Users/owner/Documents/TPDF/engine/` (untracked in the main workspace). The older Codex worktree at `~/.codex/worktrees/rust-pdf-engine/TPDF/engine/` stopped at the October 12 feature-gate follow-up and is superseded; do not build from it.

## October 14, 2026 (night): copy-on-write editing (M2 first increment)

The document now supports transaction-local object allocation and value replacement (`Document::allocate`, `Document::replace_value`), and the writer serializes them. Text edits use this for copy-on-write:

- **Shared page streams.** A content stream that other pages also list in /Contents is copied for the edited page, and only that page's /Contents is repointed. Streams referenced any other way, or listed twice on one page, are still refused.
- **Form placements.** Text inside a Form placed directly by page content is targetable. Editing one placement allocates an edited copy of the Form. The page gets its own copy of its effective resources (inherited or shared dictionaries are left alone) with a fresh XObject name, and only that placement's `Do` is renamed. Other placements, other pages and the original Form are unchanged. This matches the application's `tests/drive/cow-edit.jsonl` expectation. Nested Forms and annotation appearances stay outside the edit path.
- **Edit validation.** Unaffected text is compared by font identity rather than registry index, because a copied Form has its own resource scope.

Editable documents: 14 → 18 (edit-objects, targets-streams, objects6, redact-shared-only). edit-shared.pdf's Form text is now editable too. All 18 pass edit, save and fresh-process reopen, and independent MuPDF finds no changed pixels outside the edits and no repair. All render hashes are unchanged. Verification: 887 tests pass on Rust 1.98.1 and 1.85.1, with fmt and strict Clippy for core and bridge. The WASM build and desktop smoke pass. The WASM SHA-256 is `4ba4279a1e1832fecbb8e034e253981943d4a2c59c430cad9b662594c184bdbe`. Evidence: `reference/m2-cow-20261014/`.

Not yet in M2: object deletion, reachable-object collection on save, efficient history (still whole-file snapshots), and copy-on-write for nested Forms, fonts and other resource kinds.

## October 14, 2026 (evening): encryption, layers, inline images, color spaces and generated appearances (M1)

Five rendering gaps from the remaining-failure list are closed.

- **Encryption reading.** The standard security handler is implemented in `src/security.rs`: R2–R4 (RC4-40, RC4-128, AES-128) and R5/R6 (AES-256). The user or owner password is accepted; a wrong or missing one reports `needs-password`. Strings, streams and object-stream containers are decrypted at open. Xref streams, the encryption dictionary and Metadata under `/EncryptMetadata false` stay as stored. Encrypted documents are view-only, as in the application, and the writer refuses them. `Session::open_view_with_password`, WASM `RustEngine.with_password` and the CLI's `TPDF_PDF_PASSWORD` take the password. Hashes and AES come from the RustCrypto `md-5`, `sha2` and `aes` crates (see DEPENDENCIES.md). Public-key handlers and named non-Identity crypt filters are refused.
- **Xref entry 0.** A free entry 0 is accepted whatever its generation. MuPDF's object-stream output writes 1-byte xref-stream fields that cannot hold 65535, and pdf.js and Acrobat accept them.
- **Layers.** `src/optional_content.rs` evaluates the default configuration: BaseState, ON, OFF, and OCMD /P and /VE. Hidden `BDC /OC` content, Forms, images and annotations paint nothing but still update graphics state. Layered documents are view-only; usage auto-states (/AS) are ignored.
- **Inline images.** `BI … ID … EI` parses as one operation. Data length comes from /L, else the unfiltered sample size, else the first delimited `EI`. Abbreviations and named color spaces are expanded, and the image goes through the same validated loader as XObjects. A missing ASCIIHex/ASCII85 end marker is tolerated, as in MuPDF and pdf.js.
- **cs/CS color spaces.** Device spaces, resource names for them, CalGray, CalRGB and ICCBased (by /N) set colors through sc/scn/SC/SCN. Cal and ICC values are drawn as device values without profile conversion. Indexed, Separation, DeviceN, Lab and Pattern colors stay refused.
- **Generated appearances.** `src/generated_appearance.rs` builds display-only appearances for annotations without one: text fields (single-line, multiline, password, quadding), combo and list boxes, check boxes, radio buttons, signature fields (MK decoration only) and Text notes. Layout constants are measured against MuPDF. With `/NeedAppearances true`, text and choice fields are regenerated from their values. The generated streams live in a cloned Document that saving never sees. Text uses built-in Helvetica or Times with WinAnsi characters. Push buttons and other annotation types without an appearance stay refused.

Fixtures: 48 → 53 of 54 open and 36 → 51 of 54 fully render. All 58 previously rendered pages are byte-identical. Editable documents go from 13 to 14 (targets-unsupported.pdf, now that its inline image renders). All 14 pass edit, save and fresh-process reopen, and independent MuPDF finds no changed pixels outside the edits and no repair. Compared with MuPDF at 72 DPI, the new pages are 97.4–99.98% identical. The two lowest are explained:
- **quartz.pdf:** antialiasing of a thin embedded font. Total ink agrees within 0.4%.
- **empty-sigfield.pdf:** an intentional difference. MuPDF draws its own orange "SIGN" marker on unsigned signature fields and hides their stored appearance. The Rust engine draws the stored appearance and no marker, like Acrobat and pdf.js.

The remaining first failures are edit-tj.pdf's deliberately unbalanced `q`, the large.pdf page-count limit and redact.pdf's stream-valued feature.

Verification: 886 tests pass on Rust 1.98.1 and on 1.85.1. fmt and strict Clippy pass for core and bridge. The bridge builds for wasm32, and a Node WASM smoke opens the AES-256 fixture with its password and renders. The desktop workflow smoke passes on hello.pdf. The WASM SHA-256 is `85672c0ec58e054acc42acfcc05a95623697d2209e86d4c021b017dcc788097f`. Evidence: `reference/m1-encrypt-20261014/` (view-admission.json, new-pages-vs-mupdf.json, edits.json, independent-oracle.json, msrv-tests.log, wasm-build.log, wasm-smoke.log, desktop/). New tools: tools/compare-mupdf-pages.mjs and tools/generate-encrypted-cases.mjs (writes the extra cipher cases in tests/data/).

## October 14, 2026 (later): CID fonts and composite glyphs render (M1)

Type0 fonts with the Identity-H CMap and an embedded TrueType CIDFontType2 descendant now render.

- **Font loading.** `/W` and `/DW` widths are parsed. Both group forms are supported, the default width is 1000, and overlapping ranges are refused. `CIDToGIDMap` may be Identity or a stream. A ToUnicode map with a two-byte codespace is required.
- **Text positioning.** Text runs in two-byte codes, `TextCode.code` is now u32, and `Tw` applies only to the single-byte code 32.
- **Composite TrueType glyphs.** These are supported for all TrueType fonts, not only CID fonts. Components may have offsets and uniform, x/y or 2×2 F2Dot14 transforms, with FreeType-style rounding. Nesting depth is capped at 16. Anchor-point matching is refused.
- **ToUnicode tolerance.** A ToUnicode entry with a malformed UTF-16 destination (a common producer bug, such as cjk.pdf's odd-length `<1044d>`) is now dropped instead of failing the whole map. Showing a dropped code is still refused, so wrong text is never produced.
- **Still refused.** Vertical writing (Identity-V), other CMaps, CFF-based CIDFontType0, and text editing in CID fonts. Editing needs two-byte replacement writing (M3).

Fixtures fully rendered: 31 → 36 of 54 (cjk, rtl, edit-embedded, edit-subset, edit-subset2). All 53 previously rendered pages are byte-identical, and the SBI render hash is unchanged. Compared with MuPDF, the new pages are 99.41–99.62% pixel-identical, and visual inspection of the Hebrew/Arabic page shows matching glyphs. Its 48 pixels more than 64 levels apart are hairline antialiasing. The 13 editable documents still pass the independent edit check. Verification: 872 tests on 1.98.1 and 1.85.1, fmt, strict Clippy for core and bridge, and the desktop smoke. The WASM SHA-256 is `72d53ab59bb62497a0cf1a030d3c8e577d72b46f7c81fa1aad5a300325cf281e`. Evidence: `reference/m1-cid-20261014/`.

Remaining first failures (18 documents): 5 encrypted, 5 annotations needing generated appearances, 2 layers, and 1 each for NeedAppearances, redact.pdf's stream-valued feature, the page-count limit, an inline image, a `cs` color space and the deliberate unbalanced `q`.

## October 14, 2026 (later): marked-content property lists render (M1)

`BDC` and `DP` with property lists (MCID, ActualText, Artifact attributes) are now accepted as rendering no-ops, and BMC/BDC share one balance check. `BDC /OC` (optional content) stays refused until layer visibility is evaluated. Text edits on a page are withheld if its content has a property list ("Marked-content properties (ActualText, MCID) need structure-aware editing"). The page can still be viewed and saved unchanged. Other pages of the document stay editable.

Fixtures fully rendered: 28 → 31 of 54. All 50 pages that rendered before are byte-identical. The new pages (edit-tagged, edit-replace and paragraph page 2) are 99.15–99.89% pixel-identical to MuPDF, with no pixel more than 64 levels apart. Documents with editable targets: 11 → 13, because edit-replace.pdf and paragraph.pdf now render completely, which allows saving. All 13 pass the actual edit, save and fresh-process reopen. Independent MuPDF finds no changed pixels outside the edits and no repair. Verification: 869 tests on 1.98.1 and 1.85.1, fmt, strict Clippy for core and bridge, and the desktop smoke. The WASM SHA-256 is `21a39cf3c7b7fed9bf9e44b6fd0ee1a8c74d7744139b871cd3f14765916d5bae`. Evidence: `reference/m1-bdc-20261014/`.

Remaining first failures (23 documents): 5 CID/Type0 fonts, 5 encrypted, 5 annotations needing generated appearances, 2 layers, and 1 each for NeedAppearances, redact.pdf's stream-valued feature, the page-count limit, an inline image, a `cs` color space and the deliberate unbalanced `q`.

Known limit: a page that draws annotations is not text-editable yet. Its operation indices include the annotation and page-scope boundaries, and the edit path still rejects any Form invocation. Mapping edits through origins is M3 work.

## October 14, 2026 (later): annotation and form appearances render (M1)

Pages now draw their annotations' normal appearance streams after the page content. Documents with form fields or XFA open in view mode instead of being refused. Editing and saving them stays unavailable with reason `unsupported-forms` until form values and appearances can be preserved transactionally (M6). Fully rendered fixture documents went from 16 to 28 of 54, and all 35 pages that rendered before are byte-identical.

- **Placement.** ISO 32000 Algorithm 8.1: the appearance BBox, transformed by its Matrix, is fitted to the annotation Rect, and drawing is clipped to the BBox. `/AS` selects the state stream from an `/N` dictionary. A state with no stream draws nothing. Appearance streams without `/Subtype /Form` are accepted, because the context already makes them Forms.
- **Isolation.** When a page has annotations, its content runs inside a synthetic q/Q (`OperationOrigin::PageScope`). Each appearance therefore starts from the initial graphics state, whatever CTM, colors or clip the content leaves at depth zero.
- **Skipped.** These draw nothing: Hidden or NoView flags, zero-area Rects, popups, borderless links, and text-field widgets with an empty or absent value and no `/MK` border or background. MuPDF draws nothing for any of these.
- **Refused, per page.** Annotations with `/OC` are refused. So are other annotations without an appearance stream (checked checkboxes, list boxes, unsigned visible signature fields, note icons): viewers synthesize these, and generating them is M6. `NeedAppearances true` is still refused, because the stored streams are not authoritative.
- **Deliberately unlike MuPDF.** A widget outside the AcroForm field tree is drawn. Acrobat and pdf.js draw these, and MuPDF hides them. MuPDF is a comparison oracle, not the target behavior.

Oracle evidence is in `reference/m1-forms-20261014/oracle/`. It holds a synthetic page with a rotated Matrix and offset BBox, a leaked page `cm`, a unit BBox scaled 60×, an `/AS` state dictionary and a stream without Subtype. Against MuPDF, no pixel differs by more than 64 levels. Most of the rest is the existing ±1 gray rounding. The 15 newly rendered fixture pages are 99.84–99.96% pixel-identical to MuPDF, and the remaining differences are text edge antialiasing. Per-annotation MuPDF pixel contributions are recorded as well.

Verification: 868 tests pass on Rust 1.98.1 and 1.85.1. fmt and strict Clippy pass for the core and the bridge. The WASM rebuild SHA-256 is `835ca5d08582740b3a821fbcdc621bd622434f235aacfb7799170a552adefad6`. The desktop smoke passes with the same before and after hashes as M1. The fixture sweep is 48/54 opened, 28/54 fully rendered and 11/54 editable.

First failures for the 26 documents that are still not fully rendered, one per document:

| Count | First failure |
| --- | --- |
| 5 | CID/Type0 fonts |
| 5 | Encrypted |
| 5 | Annotations needing a generated appearance |
| 3 | `BDC` |
| 2 | Layers |
| 1 each | NeedAppearances; stream-valued feature (redact.pdf); page-count limit; inline image; `cs` color space; deliberate unbalanced `q` |

## October 14, 2026: SBI05131482.pdf opens and renders (M1)

The user's real target document now opens in view mode and renders its whole page in both native Rust and Rust WASM. The two renders are pixel-identical, and WASM open plus 72-DPI render takes about 0.2 s. Against MuPDF 1.28.1 at 72 DPI, 85% of pixels are identical and 98.5% are within 64 levels. The remaining differences are glyph and barcode edge antialiasing plus the substituted Arial outlines. Side-by-side output and dash-phase crops are visually identical. This is a visual and statistical comparison, not a certified pixel gate. Text editing on this page stays deliberately unavailable ("display substitute font; portable editing requires embedding"), so M3's SBI edit remains open. The document's contents are not copied into fixtures, evidence or archives.

Four blockers were fixed in order. Each has regression tests:

- **Image `/Intent`.** The four standard intent names are accepted on image XObjects. Nonstandard names are refused. Every admitted color space is device or matrix/gamma, so intent cannot change the output. MuPDF renders Perceptual and RelativeColorimetric identically for these spaces. The content-stream `ri` operator still admits only RelativeColorimetric. Widening it would follow the same reasoning.
- **CalRGB images.** These are converted to sRGB with exact double-precision math: per-channel gamma, the PDF Matrix, Bradford adaptation from the declared WhitePoint to D65, then the sRGB transfer curve. On a synthetic CalRGB gradient the result is within one level per channel of MuPDF/LittleCMS. Raw samples are kept unchanged and conversion happens per sampled texel. A nonzero BlackPoint, a WhitePoint whose Y is not 1, nonpositive X/Z, Gamma outside (0, 16], and malformed arrays are refused. CalRGB in content-stream `cs`/`CS` is still unsupported.
- **Tag-only marked content (`BMC`/`EMC`/`MP`).** This is admitted as a rendering and text-position no-op. Page preflight enforces BMC/EMC balance per content stream, within the save-depth limit. `BDC`/`DP` property lists (ActualText, MCID, optional content) remain refused.
- **Dashed strokes.** The user-space centerline is cut into "on" pieces, and each piece goes through the existing certified stroke expander with the configured caps and joins. Odd arrays repeat, and the phase restarts on every subpath. ClosePath dashes its closing line. Curves are split exactly with de Casteljau at chord-length positions (32 samples per cubic). Dashes at a closed subpath's seam are capped separately rather than joined. Negative or all-zero arrays are refused, as are zero-length dashes with round or square caps (cap-only dots).

A fifth blocker appeared only at 72 DPI. Capability probing renders at 18 DPI, where it is not visible. The page rasterizer kept one full-page 4× supersampled mask per clip, and SBI's roughly 70 small clips exceeded the 256 MiB mask budget. Clip masks are now stored only within each clip's sample-grid bounds, intersected with the parent's, and only that area is charged. All 35 fixture pages that rendered before still produce byte-identical 72-DPI output.

Verification: 863 Rust tests pass on Rust 1.98.1 and on the minimum 1.85.1, with no failures. Strict all-target/all-feature Clippy and formatting pass for the core and the bridge. The rebuilt WASM SHA-256 is `41bce21e5d6df772808e90830d195a2a3f0bc06f66c24ccaeaddf531e1514cee`. The sandboxed Electron workflow smoke on hello.pdf passes edit, save, reopen, second edit, all worker-failure cases, and the Node-global checks. The fixture view sweep is unchanged at 48/54 page trees, 16/54 fully rendered and 11/54 editable. None of the fixtures' first failures was one of the newly fixed features. Evidence is in `reference/m1-20261014/`: test logs, Clippy, view-admission.json, the desktop report, and sbi-render.json with the input hash, render hashes, MuPDF diff histograms and CalRGB oracle values.

Here are the first failures for the 38 fixture documents that are not fully rendered, counting one per document: 18 nonempty forms or XFA, 5 encrypted, 5 CID/Type0 fonts, 3 `BDC` marked content, 2 layers, and 1 each for a stream-valued feature (redact.pdf), the page-count limit, an inline image, a `cs` color space, and the deliberately unbalanced `q` page in edit-tj.pdf.

## October 12, 2026 late session (recorded retroactively)

A session after the feature-gate follow-up made the following changes without updating this file. Its evidence is in `reference/m1-20261012/` and `reference/m3-20261012/`, and its final run was 857 tests on both toolchains.

- **M1 view/edit split.** `Session::open_view` and per-page capabilities report render and edit errors without refusing the whole document. Signed documents are view-only.
- **Rendering features.** Explicit RelativeColorimetric `ri` and default flatness are accepted. Indexed DeviceRGB images with 8-bit lookup and binary color-key masks are supported.
- **Display-only Arial substitution.** Exact-name Arial and ArialMT regular, bold and bold-italic are drawn with pinned Helvetica outlines that keep the PDF's advance widths. The outlines are generated by tools/generate-substitute-outlines.py, and their asset hashes are in m1-20261012/font-assets.json. Pages that use a substitute are never editable.
- **M3 edit coverage.** TJ-array runs and content-array member streams can be replaced and saved. All 11 editable fixtures pass independent MuPDF edit checks (m1-20261012/independent-edits.json).

## Original October 12 workflow (historical)

A working native and Rust WASM editing workflow now exists on top of R24. The experimental desktop editor can open an eligible PDF, select existing text, replace it, undo and redo, Save As a new copy, adopt its filename and file identity, reopen that copy, and make and save another edit. The installed Tandem PDF application is unchanged. This is a functional preview for a restricted PDF subset, not completion of the full replacement roadmap.

## Implemented

The session owns bounded immutable PDF snapshots. Each edit changes a transaction-local decoded stream, serializes with the existing strict writer, reparses, checks text and unaffected text positions, and renders before publishing one history node. The writer resets stream Length and removes stale Filter and DecodeParms. Failed edits preserve content, dirty state and redo. Generation and revision reject stale targets; unique history nodes distinguish branches at the same undo position. Exact save receipts prevent a delayed disk write from marking later edits clean. Saving and undo remain available after the editing work budget is exhausted.

Existing horizontal simple-font Tj strings and TJ-array runs can be replaced or deleted, including in content-array member streams. Pinned Helvetica and Times-Roman use proven canonical glyphs and declared mappings for new characters. Embedded fonts currently use the proven on-page repertoire. Unchanged text positions, source resource identity and stream ownership are checked. Existing encoded, escaped and compressed strings are handled. Shared streams, Form text edits and paragraph edits remain outside this edit path.

The WASM bridge executes in a sandboxed Electron worker with no MuPDF runtime fallback. The separate editor uses the existing application's safe-write module, filesystem dialogs, validation before write, and save tickets. Cancelled and failed saves preserve dirty state. A 60-second worker timeout terminates the worker rather than allowing an unobserved late edit; unsaved changes in that worker are then unavailable. This is a deterministic failure policy, not a recovery or responsiveness guarantee.

## Verification

- The original checkpoint passed 829 Rust tests on Rust 1.98.1 and minimum Rust 1.85.1. The feature-gate follow-up passes 836 tests on Rust 1.98.1 and all 22 session tests on Rust 1.85.1; the full minimum-toolchain suite was not rerun for this follow-up.
- Core and bridge formatting and strict Clippy pass. The release WASM library and JavaScript bindings build and execute in Electron. The bridge's declared minimum toolchain is checked separately in its evidence log.
- The desktop smoke test uses the actual controls for selection, text entry, undo, redo, save and reopen. It verifies identical pixels after undo and after fresh reopen, performs a second saved edit, and checks stale targets, invalid open, cancelled save, injected disk-full, and a late save receipt. Both renderer and worker report no Node globals. Native save dialogs are bypassed only in smoke mode; interaction is automated, not a human usability review.
- All 8 fixture documents with supported targets passed an actual edit, save and fresh-process reopen. Independent MuPDF 1.28.1 reopened every output without repair, found the replacement text, and found zero changed pixels outside the edited region across all pages.
- The original workflow admission sweep opened 13 of 54 fixture documents; the current feature-gate sweep opens 16 of 54, with 11 documents exposing editable targets. All 11 pass an actual edit/save/fresh-process reopen and independent MuPDF text/pixel checks. This is separate from R24's much broader rendering-only page count; the workflow refuses unsupported writes, visible annotation appearances, nonempty forms, optional content and other gaps up front. Borderless links are preserved without active link navigation.
- Existing filesystem save tests: 22 pass, 29 Electron-spawning tests skipped for this focused run; zero failures. The new Electron workflow test runs separately.
- All 141 reference files in the accepted R24 archive manifest remain byte-identical. Existing production changes are confined to engine/src/document.rs and engine/src/lib.rs; new modules, bridge, preview and tests are additive. The full historical R24 differential/mutation replay was not rerun and is not claimed as new evidence.

This checkpoint has automated regression and independent-consumer evidence. It has not had a separate independent implementation review and does not supersede R24's formally reviewed acceptance status.

## October 12 follow-up: document feature gates

The blanket scan for Annots, AcroForm and OCProperties was replaced with bounded inspection of the live catalog and page dictionaries. Empty form field arrays, including indirect arrays with default appearance/resources, now survive open/edit/save/reopen. Borderless Link annotations without appearance streams are preserved; border-style precedence and omitted-width defaults are checked. Visible annotations, nonempty forms, XFA and layer settings remain explicitly refused with distinct messages. Tagged structure trees do not expose editable targets until associated replacement/accessibility text can be updated safely.

SBI05131482.pdf in the user's Downloads folder reproduced the reported failure: an AcroForm with an empty Fields array and no page annotations. That gate is fixed, but the original document still fails rendering at the RelativeColorimetric rendering-intent operator. It also uses unembedded Arial regular, bold and bold italic fonts, which the current outline policy does not substitute. (Superseded: the October 12 late session fixed the rendering intent and Arial substitution, and the October 14 work above makes the page render.) Its original hash is unchanged; its contents have not been added to fixtures or recovery archives.

Validation: 836 Rust tests and strict Clippy pass; 22 session tests pass on the minimum toolchain. Rebuilt native and WASM engines. The sandboxed Electron workflow passes twice through edit/save/reopen with synthetic empty-form metadata. Independent MuPDF validation confirms saved form metadata and link actions/destinations are preserved, outputs reopen without repair, and zero pixels outside the edited line change. Evidence is under the main workspace's .stage/workflow-gates. Prior R24 and workflow recovery checkpoints remain unchanged.

## Run the preview

From the main Tandem PDF workspace, with its Node dependencies and build/safe-write.js available:

```sh
engine/desktop/run.sh
```

From another directory, set TPDF_APP_ROOT to the application workspace. The preview uses the generated WASM files under engine/web/pkg and launches its own Electron process. It neither replaces nor launches the installed Tandem PDF bundle. The tested target is macOS Apple Silicon; Windows and Linux preview execution and packaging remain unverified.

The default editing limits are 16 MiB per input/output PDF, 64 pages, 64 undo steps and 64 MiB retained snapshot bytes. Target discovery admits at most 8,192 placed codes, 10,000 operations, 256 targets and 1 MiB decoded page content. These limits do not describe total process RSS. Save As uses the existing Save Copy path to protect the original, then adopts the new file as the current document before acknowledging the saved snapshot. Text replacement is not redaction and may leave old data in otherwise preserved objects.

## Rebuild

The bridge pins wasm-bindgen 0.2.129 and serde_json 1.0.151 in a separate crate and lockfile. The core dependencies are unchanged. Install the wasm32-unknown-unknown Rust target and the matching wasm-bindgen-cli, then run tools/build-workflow.sh. Set CARGO_BIN, WASM_BINDGEN_BIN and CARGO_TARGET_DIR when using workspace-local tools. The current generator is at /Users/owner/Documents/TPDF/.stage/wasm-tools/bin/wasm-bindgen. Source forbids unsafe Rust; generated glue and dependencies may contain internal unsafe code. AGPL and asset attribution are retained.

The native CLI is tandem-rust-workflow in the selected target's release directory. It supports inspect FILE and edit FILE PAGE TARGET TEXT OUTPUT, with zero-based page and target indices. It creates a new output path and refuses an existing one.

## Remaining work

Expand the workflow through shared content and Form copy-on-write, editing with substitute fonts (SBI), font coverage and larger real-document sets. Add page assembly, annotation and form mutation, paragraph and object edits through the same transaction and save gates. Encryption, signatures, permanent redaction and sanitization remain separate packages. Integration into the existing full application worker contract, broad performance measurement, independent implementation review and cross-platform packaging are still required before production replacement.

Evidence is retained under reference/m1-cid-20261014, reference/m1-bdc-20261014, reference/m1-forms-20261014, reference/m1-20261014, reference/m1-20261012, reference/m3-20261012, reference/workflow-20261012-final and the separate recovery checkpoint in the main workspace. Frozen R24 evidence and recovery archives remain intact.
