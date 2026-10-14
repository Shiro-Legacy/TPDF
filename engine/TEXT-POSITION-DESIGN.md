# Bounded horizontal text positioning: T0 proposal

This is a read-only proposal, not authorization or an implementation. T0 should produce an immutable sequence of positioned **source character codes** from standalone content, using explicitly injected font mappings and advances. It joins the existing content, Unicode and metrics stages without loading font programs, selecting glyph IDs, drawing text, reconstructing reading order or silently supplying missing fonts. One source code may map to several Unicode scalars, but receives exactly one advance.

The useful first result is a trace containing each code's Unicode slice, font identity, declared/substitute width provenance, text rendering transform, page-space origin and displacement, together with explicit numeric TJ adjustments and text-object end matrices. This is directly testable and can later feed text inspection, selection and a font-outline renderer. It is not a glyph bounding-box or text-layout API.

## Inspected interfaces and ownership

`operators::ValidatedOperation` borrows its `Operation` and operands; it classifies every requested text operator and validates basic arity/types. Its general numeric helper casts Integer to f64, so T0 must independently reject integer operands outside ±2^53 before conversion. `ContentParser` has cumulative lexical/node/payload limits and refuses inline images before binary lookahead.

`text_decode::{decode_mapped_text,decode_encoded_text}` return `DecodedText`: owned UTF8 plus one `DecodedCode` per source code, with character code and decoded-string/UTF8 ranges. The input is borrowed for that call, but the map is not retained. Missing ToUnicode entries, undefined encoding bytes and unknown glyph names remain distinct errors. Existing mapped decoding supports wider codes; T0 deliberately does not.

`HorizontalMetrics::lookup(u8)` yields `FontAdvance` and `AdvanceOrigin::{WidthsEntry,DeclaredMissingWidth,DefaultMissingWidth}`. Widths may be zero, negative or fractional. `BuiltinMetrics::lookup(u8)` yields an advance or an explicit undefined/missing glyph-name result, and exposes its chosen `BuiltinFace` and `encoding()`. M1 is an opt-in Helvetica/Times-Roman substitute-width policy; it does not identify the PDF's actual glyph outline. Preserve these distinctions in output.

Recommended borrowed font join:

```rust
pub enum UnicodeSource<'font> {
    ToUnicode(&'font ToUnicodeMap),
    Encoding(&'font SimpleEncoding),
}
pub enum AdvanceSource<'font> {
    Declared(&'font HorizontalMetrics),
    Builtin(&'font BuiltinMetrics),
}
pub struct TextFont<'font> { /* private validated sources */ }
impl<'font> TextFont<'font> {
    pub fn declared(unicode: UnicodeSource<'font>, metrics: &'font HorizontalMetrics)
        -> Result<Self, TextPositionError>;
    pub fn builtin(metrics: &'font BuiltinMetrics, to_unicode: Option<&'font ToUnicodeMap>)
        -> Result<Self, TextPositionError>;
}
```

The builtin constructor uses `metrics.encoding()` for Unicode when no ToUnicode is supplied, so two conflicting independently supplied encoding tables cannot accidentally control mapping versus widths. With authoritative ToUnicode supplied, widths still come from the builtin encoding and Unicode comes only from ToUnicode; no fallback on a map error. Declared widths are byte indexed independently of Unicode policy.

Validate a ToUnicode source once at construction: horizontal writing mode, every codespace/mapping uses exactly one byte, with normal parser validation already satisfied. Every shown byte is a simple-font code; do not let a ToUnicode codespace redefine a simple font into a multibyte font. Wider/vertical maps are Unsupported. Do not rescan the full map on each show operation.

Use an immutable bounded `TextFonts<'font>` registry constructed from resource-name bytes plus `TextFont` values. Reject duplicate names, limit count/name bytes and use private execution-local `TextFontId` indices. No arbitrary lookup callback, mutable loader or recursive resource resolution should execute during interpretation. Borrow mappings/metrics rather than cloning 256-entry encodings per operation. For later document integration, populate all selected loader caches first, then obtain immutable `loaded()/loaded_metrics()/loaded_builtin_metrics()` references; do not fight the mutable session lifetime by cloning map payloads.

Output owns only used font names and small policy/identity descriptors, once per binding, so font IDs remain meaningful after registry/input drop. Optional caller metadata may identify an existing `FontIdentity`; standalone labels are sufficient initially. Output must not claim an arbitrary injected mapping and metric belong to the same PDF font merely because the caller gave them the same name.

## Proposed execution and output API

Names can be adjusted during implementation, but keep constructors/private fields and the following separation:

```rust
pub struct TextPositionInitialState { /* finite user-to-page CTM */ }
pub struct TextPositionLimits { /* cumulative fields below */ }
pub struct TextPositioner<'fonts, 'font> { /* private state, flat arenas, sticky error */ }
impl<'fonts, 'font> TextPositioner<'fonts, 'font> {
    pub fn new(fonts: &'fonts TextFonts<'font>, initial: TextPositionInitialState,
               limits: TextPositionLimits) -> Result<Self, TextPositionError>;
    pub fn push(&mut self, operation: &ValidatedOperation<'_>)
        -> Result<(), TextPositionError>;
    pub fn finish(self) -> Result<PositionedText, TextPositionError>;
}
pub fn position_text_content(input: &[u8], fonts: &TextFonts<'_>,
    initial: TextPositionInitialState, content_limits: ContentLimits,
    limits: TextPositionLimits) -> Result<PositionedText, TextPositionError>;
```

Do not expose partial output from the mutable interpreter. On any error, mark it failed and ensure finish cannot publish its accumulated prefix. The convenience function returns no partial trace. If operations are accepted incrementally, still count operations/decoded payload and validate bounded traversal itself, because callers can construct public Operation values without passing through ContentParser. In particular preflight a TJ array's length/types before traversing its elements or decoding strings.

`PositionedText` has immutable getters for flat UTF8 and source-byte arenas, font descriptors, code placements, adjustment records and text-object records. A single ordered event arena or ordered indices ties these pieces together without nested ownership. Suggested per-code record:

- Original one-byte `CharacterCode`, `TextFontId`, text-object index and monotonically increasing code index.
- Source provenance: operation index and whole operation span, string operand index (or TJ element index), range within the **decoded string bytes**, and range in the retained source arena.
- Unicode range within the output UTF8 arena; any number of scalars belongs to this single code placement.
- Width in 1/1000 text-space units, `MetricOrigin::Declared(AdvanceOrigin)` or `BuiltinSubstitution(BuiltinFace)`, and the actual effective character/word spacing used.
- Text matrix before/after the code, or a before matrix plus checked local advance and page displacement; font size/horizontal scale/rise and Tr0/3 remain observable.
- Normalized-em-to-page transform and page-space origin. These are forward geometry, not outline bounds or an assertion of a selected glyph.

A TJ numeric adjustment record stores its numeric operand, source operation/array index, local displacement and before/after text matrices, with no fake character code or Unicode range. An EndTextObject record captures final text and line matrices. This makes numeric-only arrays, empty strings and trailing adjustments testable even without another glyph.

**Source-span limitation:** ContentOperand::String already holds bytes after literal/hex decoding. Its per-code offsets are not raw content/file offsets, especially for escapes, octal sequences, line continuations or hex whitespace. Operation.span identifies the enclosing lexical operation. Preserve this tuple honestly; exact escaped-source byte mapping needs a separate lexer/parser feature.

## Coordinate types and arithmetic

Introduce local zero-sized `TextSpace` and `EmSpace` tags. Reuse the existing generic geometry types:

- CTM: `Affine<UserSpace, PageSpace>` in saved graphics state.
- Text matrix Tm and text-line matrix Tlm: `Affine<TextSpace, UserSpace>` in the active text object.
- Font-size/scale/rise map S: `Affine<EmSpace, TextSpace>` with PDF coefficients `[size*h, 0, 0, size, 0, rise]`, where `h = Tz/100`.
- Per-code rendering map: `S.then(Tm).then(CTM)`.

Define a finite typed displacement or store explicit delta components; do not subtract points and mislabel the result as another position. No inverse transform is needed. Zero/negative sizes or horizontal scale and singular finite Tm are mathematically representable forward mappings; preserve them if within limits, rather than inventing positivity/invertibility requirements. They do not imply visible glyphs.

For original source byte c and width w, compute the local horizontal displacement using checked finite f64 operations:

```
glyph_dx = (w / 1000 * size + Tc) * h
word_dx  = (if c == 0x20 { Tw } else { 0 }) * h
Tm       = translate(glyph_dx, 0).then(Tm)
Tm       = translate(word_dx, 0).then(Tm)
```

The separate additions match the pinned native operator ordering and expose their provenance. Tw depends on byte0x20, not Unicode U+0020: a nonspace byte mapped to a Unicode space gets no word spacing; byte0x20 mapped to another character still gets it. Character spacing also follows the last code in a string. Unicode ligatures/multiscalar mappings do not multiply widths or spacing.

For TJ numeric n, displacement is `(-n / 1000 * size) * h`; it receives neither Tc nor Tw. Translate Tm in its own axes; do not translate page coordinates directly. Text rise changes each code's rendering origin, not the baseline cursor. Text showing and TJ adjustments never update Tlm. Record results only after every intermediate passes finite/magnitude checks; reject overflow even when a later multiplication by zero might hide it.

## State and operator contract

Defaults are Tc=0, Tw=0, h=1, TL=0, rise=0, Tr=0, and no selected font/size. Text state persists across BT/ET. BT opens one text object and resets Tm/Tlm to identity; ET closes it without resetting spacing/font/CTM. Reject nested BT, ET outside BT, unfinished BT at end and unbalanced q/Q.

| Operators | T0 behavior |
| --- | --- |
| Tc, Tw, Tz, TL, Ts | Set finite bounded text state; accepted outside or inside BT. Tz divides percentage by100 exactly once. |
| Tf | Resolve the injected resource-name binding and store font ID plus finite signed size; missing binding is an error even before showing. No resource/font fallback. |
| Tr | Accept0 (fill intent) and3 (invisible intent); both position identically and are retained in output. Refuse1/2 and4–7 initially, since no stroke or glyph-clip state is represented. |
| Tm | Inside BT only; set both matrices to the supplied finite bounded affine transform. |
| Td | Inside BT; `Tlm = translate(tx,ty).then(Tlm)`, then `Tm = Tlm`. Does not change leading. |
| TD | Same movement, plus TL=-ty. |
| T* | Equivalent to `0 -TL Td`; does not negate/change leading a second time. |
| Tj | Inside BT with selected font; decode string bytes, map Unicode, look up one width per code, emit placements and move Tm. |
| TJ | Inside BT with selected font, including empty/numeric-only arrays; process strings and numbers in order, emit explicit adjustment records. No nested arrays/non-string/non-number values. |
| `'` | T* followed by Tj. |
| `"` | Set Tw=first operand and Tc=second, then T*, then show third string. New spacings persist. |
| q/Q | Outside BT only: save/restore CTM and all text-state parameters/font selection. Stack underflow/unclosed save is an error. |
| cm | Outside BT only: `new_matrix.then(CTM)`, checked before replacing state. Does not modify text-state parameters. |

Other operators are explicitly Unsupported in T0, including paths/paint, colors, ExtGState, Type3, XObjects, marked content/ActualText, compatibility sections and images. Do not skip them to produce an apparently complete page trace. This is a standalone text-program stage; later an orchestrator can compose separate path/text executors under a unified state contract.

### q/Q inside BT: standards and oracle policy

Do not generalize the pinned MuPDF behavior into a universal PDF rule. In pinned `pdf-op-run.c`, `pdf_gsave/grestore` copy the graphics state; Tm/Tlm reside separately in the processor's text-object state. However, the current approved PDF2.0 erratum for9.4.1 requires q/Q **inside** text objects to additionally save/restore both matrices. See the [PDF Association errata, clause9.4.1](https://pdf-issues.pdfa.org/32000-2-2020/clause09.html).

The first milestone therefore explicitly refuses q/Q/cm during BT. Outside BT, saving/restoring CTM, font and text parameters around complete objects is well-defined and testable; a new BT resets its matrices. A later stage must choose a documented PDF-version/compatibility policy before broadening in-object special graphics-state operators. Do not hide the native/spec distinction behind a fuzz tolerance.

## Cumulative resource limits and failure policy

Suggested defaults: 100,000 operations, 128 saved graphics states, 256 font bindings, 127 bytes per resource name, 32KiB total name bytes, 10,000 text objects, 262,144 source codes, 262,144 TJ elements/adjustments, 1MiB cumulative shown string bytes, 1million Unicode scalars, 4MiB UTF8 output and64MiB cumulative requested allocation capacity. Count every occurrence, including repeated identical strings/fonts and strings in separate operations. Limit number of empty show/adjustment events independently of emitted codes.

Use separate magnitude limits for state/matrix linear coefficients (proposed absolute1e6), translations/origins/cursor positions (absolute1e9), and advances/spacing/size (absolute1e6 before multiplication). All products, affine composition and cursor updates must remain finite and within declared limits. These are deliberate resource/numerical policy limits, not PDF normative restrictions. Reject inexact integer conversion beyond ±2^53. The initial CTM receives the same validation.

Pass remaining cumulative allowances into decoding, then charge actual returned input/code/scalar/UTF8 work. Do not reset full TextDecodeLimits on every string. Temporary decoded output plus retained output capacity needs explicit peak/cumulative accounting; either move newly decoded text/code buffers into flat bounded arenas carefully or use one private accumulator with growth tracking. Avoid per-code font cloning, quadratic UTF8 concatenation and a full matrix/state clone of maps on q.

Errors distinguish syntax/operator validation, structural text state, unsupported feature, missing font, missing/undefined Unicode mapping, missing builtin width, limit, inexact integer, nonfinite arithmetic and allocation. Include operation span/index and decoded-string/TJ element context where applicable. No replacement character, missing-width guess beyond M0's already explicit DefaultMissingWidth provenance, automatic builtin substitution, malformed-ToUnicode fallback or output-prefix publication.

## Independent and native acceptance

The primary oracle should be a small independent Python rational affine/state model, fed generated finite rational settings and explicitly supplied byte widths/mappings. Use exact rationals to derive origins/advances/matrices, compare Rust f64 against a predeclared arithmetic tolerance for nonexact decimal cases, and require exact values on integer/power-of-two cases. Native uses float32, so universal bit equality to f64 is neither expected nor useful; limit sequence lengths/magnitudes and state a scale-aware float32 tolerance before running comparisons.

Extend a test-only native MuPDF text device to record fill_text/ignore_text callbacks in content order. `fz_text_span.trm` supplies a,b,c,d, and each fz_text_item supplies x,y,adv,cid,ucs,gid. Combine span/item transform with callback CTM to compare each primary source-code placement. Multi-scalar mappings produce additional native gid=-1/cid=-1 items; group them as Unicode continuation entries, never charge extra advance. Do not compare glyph IDs, glyph bounds, automatic bidi guesses or native fallback Unicode as T0 semantics.

Generate minimal PDFs using pinned bundled Helvetica/Times-Roman with explicit test /Widths and one-byte encoding/ToUnicode. Reject repair, warnings affecting fonts, system-font loading, missing glyph fallback and unsupported resources in the native harness; bound input/output/query counts and process timeouts. Native width loading can quantize fractional declarations, so integer-width cases form the initial native gate and fractional/negative/zero declarations retain an independent arithmetic gate unless actual native parity is established. Append a known marker code when observing a trailing TJ or Tc/Tw advance is necessary; the independent model separately validates final Tm/Tlm and empty/numeric-only sequences.

The acceptance matrix should include:

- All shown operators, several BT/ET objects, font changes, Tc/Tw/Tz/TL/Ts, noncommuting Tm/CTM, reflected/rotated/skewed matrices, Td after prior Tj, repeated T*, quote spacing persistence and q/Q around objects.
- One code→multiple Unicode scalars with one advance; source0x20 remapped away from space; another code mapped to space; a nonbreaking space; missing Unicode and builtin widths; declared MissingWidth/default-zero provenance.
- Mixed TJ strings/numbers, adjacent and trailing adjustments, empty strings/arrays, signed values, zero/negative font size or scale and accumulated precision cases.
- Every limit exactly at/beyond boundary, many repeated tiny strings, many numeric-only arrays, failed suffix after a valid prefix, overflowing matrix operations, integer precision failure, unclosed states, forbidden in-BT q/Q/cm and unsupported Tr modes.

A practical first gate is at least200 independently generated state programs, native comparison on at least100 supported integer-width/simple-font cases, all existing owner/review tests, Rust1.85.1 and wasm compile checks, and no new production dependency. Fixture page integration follows only after the standalone stage is accepted; existing arbitrary page streams will contain unsupported path/resource operators and must not be advertised as supported text extraction.

## Source provenance and implementation sequence

Implementation behavior references pinned MuPDF1.28.1 `source/pdf/pdf-op-run.c` and `source/pdf/pdf-interpret.c`, both Copyright(C)2004–2026 Artifex Software, Inc. The latter supplies `pdf_tos_make_trm`, movement/line helpers and matrix conventions. Relevant structs are in `include/mupdf/pdf/interpret.h`; callback representation is in `include/mupdf/fitz/text.h`. Any derived implementation should retain the crate's AGPL notice and precise attribution. The current PDF2.0 errata citation above is a separate standards cross-check, not a claim that pinned MuPDF implements every erratum.

Within the remaining work window, sequence the work as: (1) borrowed font bindings and typed finite state with unit tests; (2) all listed placement/show operators and cumulative arenas; (3) independent rational state model plus bounded native trace adapter; (4) adversarial limits, provenance/CLI inspection and compiler gates. Do not expand to font outlines, shaping, text clipping, full page-resource execution or reading-order extraction to make a larger demonstration. T0's useful deliverable is a correct, bounded position trace with explicit unsupported cases.
