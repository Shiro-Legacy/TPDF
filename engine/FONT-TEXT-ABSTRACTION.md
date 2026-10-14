# Joining declared Unicode and advances for bounded positioning

Read-only audit and API recommendation for TEXT-POSITION-DESIGN.md. This note adds no implementation and does not change F0, M0, M1 or D0 acceptance. The output under discussion is positioned **original character codes**, not glyph IDs, glyph outlines, shaped text, reading order or selection boxes.

## The existing split is useful

A font currently has independently lazy capabilities:

- `LoadedSimpleFont` chooses authoritative strict ToUnicode or an explicit S0 encoding for Unicode. `decode_declared_text` dispatches without fallback and preserves source-code/UTF8 spans.
- `LoadedFontMetrics` supplies declared Widths with `AdvanceOrigin::{WidthsEntry,DeclaredMissingWidth,DefaultMissingWidth}`. It does not inspect Unicode or need a glyph-name encoding.
- `LoadedBuiltinMetrics` supplies exact face/name metrics through its own explicit encoding. `BuiltinMetricLookup` distinguishes UndefinedGlyph, MissingBuiltinMetric and a finite advance. It does not inspect ToUnicode.

These sources are independent by design. A ToUnicode entry mapping code 0x41 to `fi` still consumes one code and one advance. It must not change the builtin glyph-name selection from `A` to `fi`, multiply an advance by two, normalize the output, or substitute Unicode U+0020 for the original byte when deciding word spacing. An explicit Widths zero remains zero even when a builtin glyph has a nonzero advance.

The 53 page-bound fonts now have two distinct advance paths: five explicit-width fonts, 48 verified builtin fonts. That is declaration-level coverage. The two embedded Hebrew subsets using explicit WinAnsi/MacRoman mappings still do not have proven glyph/Unicode equivalence. Joining their declared Unicode and widths does not erase that limitation.

## A small standalone injected abstraction

Prefer an immutable borrowed font view in the first positioning unit. It needs no Document, loader mutation, filesystem, font program, or new data ownership:

```rust
enum UnicodeSource<'font> {
    ToUnicode(&'font ToUnicodeMap),
    Encoding(&'font SimpleEncoding),
}
enum AdvanceSource<'font> {
    Declared(&'font HorizontalMetrics),
    Builtin(&'font BuiltinMetrics),
}
struct TextFont<'font> { /* private independently selected sources */ }

TextFont::from_declared(unicode: UnicodeSource, metrics: &HorizontalMetrics)
    -> Result<TextFont, TextFontError>;
TextFont::from_builtin(metrics: &BuiltinMetrics, to_unicode: Option<&ToUnicodeMap>)
    -> Result<TextFont, TextFontError>;
```

The builtin constructor should derive its Unicode encoding from `metrics.encoding()` when no ToUnicode override is supplied. This makes a plain encoding font use the same glyph-name encoding for Unicode declarations and builtin advances without accepting an accidental mismatch between two independently supplied encoding tables. When explicit ToUnicode is supplied, preserve the override only for Unicode; builtin advances still use their original encoding. Declared widths can accompany either Unicode source because they are indexed directly by the original byte.

Validate ToUnicode once at construction: every codespace is one byte and writing mode is horizontal, as required by F0. Do not let a caller inject an arbitrary valid four-byte standalone CMap into this simple-font abstraction. HorizontalMetrics already has a byte code domain and private finite widths; SimpleEncoding already has exactly 256 slots. No copying of mappings, 256-slot encodings or width arrays is needed to construct a view.

Expose immutable source/provenance getters if useful, and one internal per-byte lookup that returns borrowed Unicode scalars plus an advance/provenance result. Preserve separate error kinds for missing ToUnicode mapping, code outside its codespace, undefined encoding code, unknown Unicode glyph name, undefined advance glyph, and missing builtin metric. A byte can have valid Unicode but no permitted advance, or a valid advance but no permitted Unicode. Strict positioned output should fail transactionally if either required capability fails. Do not return zero or U+FFFD to make the operation continue.

Using the existing D0 whole-string decoder is also sound initially, provided all decoded source widths are checked to be one and positioning work/scalar/output budgets are charged. It creates a temporary Unicode/span result; if that output is later copied into an arena, both temporary capacity and final retained allocation must be bounded. A future shared accumulator/per-byte lookup can avoid the extra copy, but should preserve D0's existing behavior and tests rather than fork its error policy.

## Registry ownership without per-operation table clones

Inject a completed registry into a standalone positioning function:

```rust
struct TextFontRegistry<'font> { /* bounded byte-name bindings to TextFont views */ }
TextFontRegistry::new(bindings, limits) -> Result<TextFontRegistry, RegistryError>;
registry.lookup(resource_name: &[u8]) -> Option<&TextFont>;
position_content(bytes, &registry, limits) -> Result<PositionedCodeList, PositionError>;
```

A registry can own bounded resource-name bytes and store small borrowed TextFont views. Resource names remain exact PDF name bytes. Reject duplicate bindings instead of defining an accidental replacement policy. Aliases may point to identical borrowed payloads without duplicating those payloads; bound alias count and cumulative retained name bytes separately. Do not build a new 256-entry encoding or metric array on Tf, Tj, TJ, q or Q.

Tf resolves the byte name once, then graphics/text state stores a private registry slot or borrowed immutable font view. A private slot belongs only to that execution/registry; do not expose forgeable numeric handles accepted by unrelated registries. Saving/restoring state copies that small selection, not the font payload. Registry mutation is unavailable during execution. Every Tf selection and every emitted code still consumes work, even when aliases or repeated selections share data.

The first API need not deduplicate small view records by pointer identity. Shared immutable references already avoid the expensive copies, and a binding-count limit bounds the small records. Font provenance can be supplied separately for diagnostics: a synthetic registry name is sufficient for standalone tests, while a document adapter can retain full FontIdentity and page/resource scope. Never infer equivalence from BaseFont strings or reuse an ObjectId across documents.

## Avoid mutable loader self-borrows in the document adapter

Current `FontResourceLoader::load*` methods take `&mut self` and return a reference into a cache. Holding one returned reference while invoking another mutable load is intentionally prohibited by Rust. Do not solve this by cloning the large cache payload, raw pointers, interior mutable references, or unsafe lifetime extension.

The simplest future adapter has explicit prepare and freeze phases:

1. Collect a bounded set of selected resource names for one page/scope, either supplied by the caller or discovered by a validated content prepass. If a prepass is used, its parsing/operation work counts in addition to the positioning pass. Unsupported inline images or other unsupported content cannot be skipped by an unvalidated string search for Tf.
2. For each selected name, call `load` and immediately retain only its copied FontIdentity. Release the returned borrow. Call `load_metrics` for that same binding; only the specific `NeedsBuiltinOrFontProgramMetrics` result permits trying `load_builtin_metrics`. Any malformed explicit Widths, reference error, range/count error, exhausted budget or unsupported explicit declaration remains an error and must not trigger fallback.
3. Record the successful metric choice and verify its complete FontIdentity equals the mapping identity. Never combine data from a different font merely because their resource names or BaseFont bytes happen to match.
4. Once all mutable preparations complete, borrow the loader immutably. Retrieve mappings with `loaded(identity)`, advances with `loaded_metrics(identity)` or `loaded_builtin_metrics(identity)`, and construct the borrowed TextFont views/registry. Rust now enforces the freeze: further mutable loading waits until those views are dropped.

For a builtin choice with ToUnicode, pass the strict map as the Unicode override and the builtin metric encoding as its own advance source. For a builtin choice without ToUnicode, the adapter should verify the F0 encoding and M1 encoding have equivalent glyph names across all 256 slots once, or rely on an explicit validated same-declaration token produced during preparation. The first implementation can compare 256 immutable slots under preparation work limits; do not perform that comparison per code. The same immutable document declaration should make this check succeed, but validating the join catches adapter mistakes.

Preparation failures may leave successful lazy cache entries and consumed work, as existing loaders already do. They must not publish a partially prepared registry as a complete positioning input. Existing non-mutating cache inspection can remain available after failures. Resource scope matters: this initial adapter is page-scoped. It cannot reuse page Resources blindly inside Form XObjects, whose resource stack needs a separate contract.

If future dynamic nested scopes make freezing inconvenient, add backward-compatible shared handles: cache `Rc<LoadedSimpleFont>`, `Rc<LoadedFontMetrics>` and `Rc<LoadedBuiltinMetrics>`, and return cloned Rc handles from separate `load_shared*` methods. A font bundle can then own those handles, with methods borrowing their fields when called; it must not contain self-referential borrowed fields pointing into its own Rc values. The existing reference-returning APIs can remain wrappers. Rc is adequate for the current single-threaded execution; Arc should follow a real threading requirement. This is an alternative future design, not a necessary F0 refactor now.

## Provenance and positioned output

`DecodedText::input()` borrows a decoded PDF string operand. Its source spans index those **decoded string bytes**, not raw literal/hex syntax or the original PDF file. `ContentOperand::String` owns decoded bytes and does not retain a per-byte lexical escape map; `Operation.span` bounds the whole operation. Therefore a positioned record can honestly preserve:

- operation index and operation content span;
- string operand index, or TJ array element index;
- decoded-string source-byte range and original code byte;
- resulting UTF8 span in the output text arena;
- selected registry/font provenance, advance origin, origin point and finite advance vector.

Do not label the decoded-string range as an exact raw PDF/file offset. Mapping a logical page-content offset to original streams also requires the existing page-content segment/padding map; a PDF string's escaped bytes need an additional lexical provenance model before exact raw byte highlighting is promised.

Store each original code once and advance once. Multi-scalar mappings share that code's origin/advance and UTF8 range; they do not acquire invented per-scalar glyph positions. A TJ number is a separate spacing adjustment with an array-element source location and no character code/Unicode span. Word-spacing eligibility must use original one-byte code 0x20, not its Unicode mapping. Kerning, shaping, bidi reordering, ascent/descent, glyph bounds and selection quads remain outside this abstraction.

For an owned result, the original code byte plus operand location is sufficient provenance without borrowing a short-lived ContentOperand Vec. If the result also promises original decoded operand buffers, retain those explicitly in a bounded arena. Do not store slices into operation values dropped after iteration. Similarly, immutable result font provenance should be copied once per used binding or retained through a clear registry lifetime, rather than copying names/table contents per positioned code.

Advance provenance should distinguish WidthsEntry, DeclaredMissingWidth, DefaultMissingWidth and BuiltinFace. Builtin face/table version and glyph-name encoding source belong in the font-level record; code plus font identity lets a retained registry explain the glyph-name lookup without cloning the name into every code record. This describes an advance source, not a rendered glyph identity.

## Resource limits and acceptance

Keep font preparation budgets separate from positioning budgets. Existing F0/M0/M1 counters cover reference traversal, selected font attempts, map decoding, encoding Differences, processed entries and retained caches. The injected registry adds bindings and name bytes; positioning must independently bound input bytes, parsed operations, text-show calls/TJ items, font selections, original codes, Unicode scalars, UTF8 bytes, numeric adjustments, graphics-state depth, retained output records and capacities. Repeated codes and repeated empty show operations still cost work. Sharing a font cannot make output growth free.

Check arithmetic for width-to-advance conversion, font size, spacing, horizontal scale and composed matrices before publishing a record. FontAdvance being finite does not guarantee products/sums are finite. Preserve explicit negative/zero advances; a width is not a positive rectangle dimension. The positioning module's exact text-state scope and arithmetic order belong to TEXT-POSITION-DESIGN.md.

Required independent integration tests include a ToUnicode ligature with a conflicting builtin glyph-name width, an explicit zero width over a nonzero builtin metric, valid Unicode with missing advance, valid advance with unknown Unicode, non-ASCII/escaped resource names, missing font selection, duplicate registry bindings, repeated aliases without table clones, code0x20 mapping to a nonspace and a different code mapping to U+0020, mixed multi-scalar destinations, TJ adjustment provenance, and invalid injected multibyte/vertical CMaps. Test exact cumulative budgets across many short strings and many repeated Tf operations; per-call TextDecodeLimits alone are insufficient.

Document-adapter tests should prove same-identity joins, no builtin fallback after malformed Widths, no Unicode fallback after malformed ToUnicode, immutable freeze/shared-handle lifetimes, reuse of successful caches without omitted work, page versus Form scope separation, and explicit rejection of unsupported Type0 fonts. Native position comparisons may substantiate code origins/advances only within the accepted declaration/number domain; they do not turn positioned source-code records into Rust-rendered glyphs.
