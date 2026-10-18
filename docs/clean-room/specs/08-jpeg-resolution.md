# Spec 08: image resolution and orientation for pdf-from-images

Workstream 8 of `docs/mit-clean-room-plan.md`. Status: draft for review,
2026-10-18. Rules: `docs/clean-room/SPEC-RULES.md`.

This spec replaces the parts of `engine/src/app/ops/pages/images.rs` that
read an image's resolution and orientation from its metadata: the EXIF
reader, the Photoshop image-resource reader, the choice between EXIF,
Photoshop and JFIF values in the JPEG path, the JFIF density capture, and
the PNG `pHYs` conversion. Everything else in `images.rs` (the JPEG marker
walk, frame header, Adobe and ICC handling, PNG decoding, Deflate, storage,
`add_page`) is not part of this workstream and stays.

Public sources the implementer works from:

- TIFF Revision 6.0 (Adobe, 1992): image file header, IFD layout, field
  types, tags 274, 282, 283, 296.
- EXIF 2.3 (CIPA DC-008 / JEITA CP-3451C): APP1 layout, the `Exif` identifier,
  0th IFD, Orientation values 1..8.
- JFIF 1.02 (ECMA TR/98 / ITU-T T.871): the APP0 `JFIF` segment.
- Adobe Photoshop File Formats Specification: Image Resource Blocks and the
  ResolutionInfo structure (resource ID 0x03ED); the JPEG APP13
  `Photoshop 3.0` wrapper.
- PNG (ISO/IEC 15948, W3C PNG 3rd edition): the `pHYs` chunk.

The implementer may read this spec, those standards, the acceptance tests in
section 7, and the surviving code of `images.rs`. The implementer may not
read the removed functions, their git history, or MuPDF source.

---

## 1. Scope

### 1.1 What the component does

Given the metadata segments of a JPEG, or the `pHYs` values of a PNG, the
component answers two questions:

1. **Resolution:** how many pixels per inch the image declares along its x
   axis (0 when it declares none that can be used).
2. **Orientation:** for a JPEG, the EXIF orientation 1..8 (1 when none).

`pdf-from-images` uses them to size each page (the image at its declared
size, upright). The image-stamp and image-replace paths use the
orientation to draw the image upright.

### 1.2 Callers (surviving code)

| Caller | Uses | Needs |
|---|---|---|
| `images::read` → `png`, `jpeg` (same file) | the functions of section 2 | fill `Placed::dpi` and `Placed::orientation` |
| `images::add_page` (same file, not rewritten) | `Placed::dpi`, `Placed::orientation` | page size: a `dpi` of 0 or 96 counts as none and the page is then sized at 72 dpi (the app's own rule, kept); orientations 5..8 swap width and height |
| `ops::annots::decode_stamp_image`, `annot-add` image stamps | `Placed::orientation` | the unit-square placement of the stamp image |
| `ops::edit::object` (image replace, `TURNED_KEY`) | `Placed::orientation` | an image page made from a photo replaces upright |
| `ops::scan` (`scan-compose`) | `images::read` | the page JPEG; orientation and dpi as above |

`Placed` itself is unchanged:

```rust
pub struct Placed {
    pub width: u32,
    pub height: u32,
    /// Pixels per inch along the image's x axis; 0 when it declares none.
    pub dpi: u32,
    /// EXIF orientation 1..8 (1 when none).
    pub orientation: u8,
    pub arithmetic: bool,
    /* private fields unchanged */
}
```

### 1.3 Non-goals

- Reading any EXIF IFD other than the 0th (no Exif sub-IFD, GPS, thumbnail).
- XMP resolution (`tiff:XResolution` in an APP1 XMP packet). An image
  carrying resolution only in XMP is treated as declaring none.
- Non-square pixels. Only one number is reported; `add_page` uses it for
  both axes, as today.
- Photoshop resources split across several APP13 segments (section 4.3).

---

## 2. Interface

A new private module `crate::app::ops::pages::resolution`
(`engine/src/app/ops/pages/resolution.rs`), declared from `pages.rs` or
`images.rs` as the implementer prefers. All items `pub(crate)`.

```rust
/// What the 0th IFD of one EXIF APP1 segment declares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Exif {
    /// Pixels per inch, when the segment declares a usable resolution.
    pub dpi: Option<u32>,
    /// 1..8, when the segment declares a valid orientation.
    pub orientation: Option<u8>,
}

/// `payload`: an APP1 segment's bytes after its two length bytes
/// (so it starts with the identifier `Exif\0`).
pub(crate) fn exif(payload: &[u8]) -> Exif;

/// `payload`: an APP13 segment's bytes after its two length bytes
/// (starting `Photoshop 3.0\0`). Pixels per inch from ResolutionInfo.
pub(crate) fn photoshop_dpi(payload: &[u8]) -> Option<u32>;

/// `payload`: an APP0 segment's bytes after its two length bytes
/// (starting `JFIF\0`). Pixels per inch from the density fields.
pub(crate) fn jfif_dpi(payload: &[u8]) -> Option<u32>;

/// A PNG pHYs x value and whether its unit is the metre.
pub(crate) fn png_dpi(pixels_per_unit_x: u32, unit_is_metre: bool) -> Option<u32>;

/// The JPEG's resolution: the first of the three that is Some, else 0.
pub(crate) fn jpeg_dpi(exif: Option<u32>, photoshop: Option<u32>, jfif: Option<u32>) -> u32;
```

None of these functions panics, allocates in proportion to anything but its
input, or reads outside `payload`. `Option<u32>` is never `Some(0)`.

### 2.1 Glue in `images.rs` (the implementer writes it)

The extraction leaves `images.rs` with the old readers removed and these
call sites to fill:

- `jpeg_info`: for each marker segment before the first scan,
  - APP0 (0xE0) whose payload starts with `JFIF\0`: the first such segment
    gives `jfif_dpi(payload)`.
  - APP1 (0xE1) whose payload starts with `Exif\0`: the first such segment
    gives `exif(payload)`. Other APP1 segments (XMP) are skipped.
  - APP13 (0xED) whose payload starts with `Photoshop 3.0\0`: the first
    such segment that yields `Some` from `photoshop_dpi` is kept.
- `jpeg`: `orientation = exif.orientation.unwrap_or(1)`;
  `dpi = jpeg_dpi(exif.dpi, photoshop, jfif)`.
- `png`: from the decoder's physical pixel dimensions, `png_dpi(x, unit ==
  metre)`, 0 when absent or `None`.

---

## 3. Required behaviour

### 3.1 Rounding (all sources)

Every conversion to pixels per inch rounds to the **nearest** integer, ties
away from zero. A result of 0, or one that does not fit in `u32`, is `None`.
One centimetre is exactly 2.54 inches (so one metre is 39.37007874… inches;
compute `ppm × 0.0254`).

Reason: a scanner set to 300 dpi that stores 118 dots per cm (299.72 dpi)
should give a 300 dpi page. Truncating gives 299 and a page 0.3% too large.
See section 6 for the observed difference.

### 3.2 EXIF (`exif`)

Layout (TIFF 6.0 §2, EXIF 2.3 §4.5–4.6):

1. The payload starts with the six-byte identifier: `Exif`, `0x00`, and one
   more byte. The standard says that byte is `0x00`. Any value is accepted.
   Anything shorter, or without `Exif\0`, gives `Exif::default()`.
2. The TIFF header starts at payload offset 6. All offsets below are counted
   from the start of the TIFF header. The header is `II` followed by 42 as a
   little-endian 16-bit value, or `MM` followed by 42 big-endian. Anything
   else gives `Exif::default()`. The next four bytes are the offset of the
   0th IFD.
3. The 0th IFD is a 16-bit entry count, then that many 12-byte entries: tag
   (16 bits), field type (16), count (32), then a 4-byte value field. The
   value field holds the value itself when the value fits in 4 bytes, left
   justified. Otherwise it holds the offset of the value.
4. Field types used here: SHORT = 3 (16-bit unsigned), LONG = 4 (32-bit
   unsigned), RATIONAL = 5 (two LONGs, numerator then denominator),
   SRATIONAL = 10 (two signed 32-bit values).

Tags read (first occurrence of each tag wins; later duplicates are ignored):

| Tag | Name | Accepted types | Meaning |
|---|---|---|---|
| 274 (0x0112) | Orientation | SHORT or LONG, count ≥ 1 | 1..8 as EXIF defines; any other value is not a valid orientation |
| 282 (0x011A) | XResolution | RATIONAL or SRATIONAL, count ≥ 1 | pixels per ResolutionUnit along x |
| 283 (0x011B) | YResolution | as XResolution | used only when XResolution is absent or unusable |
| 296 (0x0128) | ResolutionUnit | SHORT or LONG, count ≥ 1 | 1 = no absolute unit, 2 = inch, 3 = centimetre; **absent means 2** (TIFF 6.0 default) |

Resolution:

- A rational with a zero denominator, a negative or zero value, or a value
  whose offset points outside the payload is unusable.
- ResolutionUnit 2: `round(value)`. ResolutionUnit 3:
  `round(value × 2.54)`. ResolutionUnit 1 or any other value: no resolution.
  Then `dpi = Some(n)` per section 3.1.
- Orientation: `Some(v)` for v in 1..=8, else `None`.

Bounds: the walk reads at most the declared entry count and stops at the
first entry that runs past the payload. Each value offset is checked against
the payload. Offsets are unsigned 32-bit values added with overflow
checks.

### 3.3 Photoshop (`photoshop_dpi`)

Layout (Photoshop File Formats Specification, "Image Resources Section"):

1. The payload starts with `Photoshop 3.0` and a zero byte. Otherwise the
   result is `None`.
2. Then come image resource blocks, one after another:
   - signature, 4 bytes (`8BIM`);
   - resource ID, 16-bit big-endian;
   - name, a Pascal string (a length byte, then that many bytes), padded
     with a zero byte when needed so that its total size (length byte
     included) is even. An empty name is two zero bytes;
   - data size, 32-bit big-endian;
   - data, padded with a zero byte to an even size.
3. Resource ID 0x03ED is ResolutionInfo. Its first 4 bytes are `hRes`, a
   16.16 fixed-point number of pixels per inch. Photoshop stores pixels per
   inch here whatever unit it displays. The 16-bit `hResUnit` after it only
   says how to display the value, so it is ignored. The result is
   `round(hRes)` per section 3.1. A ResolutionInfo shorter than 4 bytes is
   unusable and the scan goes on.
4. The first usable ResolutionInfo wins.

Tolerance (a design choice; see section 6 for what MuPDF does):

- A block whose signature is not `8BIM` is skipped using the same layout,
  because other writers use other signatures.
- Named blocks are skipped correctly by the padding rule above.
- A block whose header, name, size or data runs past the payload ends the
  scan. The result is what was found before it.

### 3.4 JFIF (`jfif_dpi`)

Layout (JFIF 1.02 §B.2): identifier `JFIF\0` (5 bytes), version (2 bytes),
units (1 byte), Xdensity (16-bit big-endian), Ydensity (16-bit big-endian),
then the thumbnail fields.

- Units 1 (dots per inch): `Some(Xdensity)` when Xdensity > 0.
- Units 2 (dots per cm): `round(Xdensity × 2.54)`.
- Units 0 (aspect ratio only) or any other value: `None`.
- A payload too short for Xdensity: `None`.
- `JFXX` extension segments are not JFIF headers and are not read.

### 3.5 PNG (`png_dpi`)

`pHYs` (PNG §11.3.5.3): pixels per unit x (32-bit), pixels per unit y,
unit specifier (0 unknown, 1 metre). `png_dpi(x, true)` is
`round(x × 0.0254)` per section 3.1. `png_dpi(_, false)` is `None`. A
missing or malformed chunk is the decoder's business and gives 0.

### 3.6 Precedence (`jpeg_dpi`)

The JPEG's resolution is the first **usable** value among:

1. EXIF (`exif(...).dpi`),
2. Photoshop ResolutionInfo,
3. JFIF density,

else 0. A source that is present but unusable (an EXIF segment with
ResolutionUnit 1, a zero denominator, or no XResolution and no YResolution)
does not block the next source.

Why this order: EXIF and Photoshop values are written by the application
that last set the resolution. JFIF density is often a default (aspect 1:1,
or 72) that encoders write without thought. Why fall through: an unusable
EXIF value carries no information, and a usable JFIF or Photoshop value is
better than none.

---

## 4. Tolerant behaviour and limits

### 4.1 Robustness

- Every function returns a value (never panics) for any byte sequence,
  including every prefix of a valid payload (acceptance 7.3 checks all
  prefixes).
- No allocation proportional to declared counts or sizes. Work is linear in
  the payload length.
- Only safe Rust.

### 4.2 Duplicates

- Several EXIF APP1 segments: the first one with the `Exif\0` identifier is
  used, even if it declares nothing (keeps behaviour predictable). Several
  APP13 segments: the first that yields a resolution. Several JFIF APP0
  segments: the first.
- A duplicated tag within the 0th IFD: the first occurrence wins.

### 4.3 Split Photoshop resources

Photoshop may split a large resource section across several consecutive
APP13 segments. This spec reads each segment alone. A ResolutionInfo cut by
a segment boundary is not found. It is rare, because ResolutionInfo is
normally among the first blocks.

---

## 5. User-visible changes

| Input | Before (current code, matches MuPDF here) | After |
|---|---|---|
| JFIF 118 dots/cm | 299 dpi | 300 dpi |
| EXIF 299.7 dpi | 299 dpi | 300 dpi |
| EXIF with ResolutionUnit 1 and a JFIF 150 dpi header | no resolution (72 dpi page) | 150 dpi |
| Photoshop ResolutionInfo after a named resource block | read (current code) | read |
| Photoshop 200.5 dpi | 200 | 201 (ties away from zero) |

The current code already reads a missing ResolutionUnit as inches, and
already accepts XResolution without YResolution. Both are kept.

---

## 6. Experiment log (black-box, MuPDF 1.28.1 via npm `mupdf`)

Harness: `reviewer archive spec-06-08/jpeg-res.mjs`
(run with `slot node jpeg-res.mjs`). It builds a 1×1
greyscale baseline JPEG (DQT, SOF0, DHT, SOS, EOI) with the listed metadata
segments in front, and 1×1 PNGs with and without `pHYs`. It prints
`new mupdf.Image(bytes).getXResolution()/getYResolution()`. MuPDF reports 96
when an image declares no resolution.

| Input | MuPDF x / y | This spec |
|---|---|---|
| no metadata | 96 / 96 (none) | 0 (none) |
| JFIF units 1, 300 | 300 / 300 | 300 |
| JFIF units 2, 118 | 299 / 299 (truncated) | 300 (rounded, §3.1) |
| JFIF units 0 | none | none |
| JFIF units 1, density 1 | 1 / 1 | 1 (see open question Q1) |
| JFIF units 1, density 0 | none | none |
| EXIF II, XResolution 300/1 only, unit 2 | none | 300 (Y not needed) |
| EXIF II, X and Y 300/1, unit 2 | 300 / 300 | 300 |
| EXIF MM, X and Y 300/1, unit 2 | 300 / 300 | 300 |
| EXIF X and Y 300/1, no ResolutionUnit | none | 300 (TIFF default inch) |
| EXIF X and Y 118/1, unit 3 | 299 / 299 | 300 |
| EXIF X and Y 2997/10, unit 2 | 299 / 299 | 300 |
| EXIF X and Y 300/1, unit 1 | none | none |
| EXIF X 300, Y 150, unit 2 | 300 / 150 | 300 (x axis) |
| EXIF X and Y 300/1 + JFIF 150 | 300 | 300 |
| EXIF unit 1 + JFIF 150 | none (EXIF blocks JFIF) | 150 (fall through, §3.6) |
| EXIF X and Y with zero denominators + JFIF 150 | 150 | 150 |
| EXIF X and Y 300 + Photoshop 200 | 300 | 300 |
| Photoshop ResolutionInfo 200 | 200 | 200 |
| Photoshop 200.5 | 200 | 201 |
| Photoshop 200, hResUnit 2 (cm display) | 200 | 200 (unit ignored) |
| Photoshop after an unnamed 4-byte block | 240 | 240 |
| Photoshop after a block named `ab` (padded name) | none | 240 (correct padding, §3.3) |
| Photoshop after a block named `abc` | none | 240 |
| Photoshop after a block with signature `MeSa` | none | 240 (skipped, §3.3) |
| Photoshop 200 + JFIF 150 | 200 | 200 |
| PNG no pHYs | none | 0 |
| PNG pHYs 11811 /m | 300 | 300 |
| PNG pHYs 3780 /m | none (96) | 96 (then `add_page` treats 96 as none) |
| PNG pHYs 2834 /m | 72 | 72 |
| PNG pHYs unit 0 | none | none |
| PNG pHYs x 11811, y 5906 | 300 / 300 (x for both) | 300 |

The harness was not able to build an Orientation probe: npm `mupdf` exposes
no orientation getter. Orientation behaviour comes from EXIF 2.3 alone and
the existing tests (section 7.1).

---

## 7. Acceptance

### 7.1 Keep (black-box, unchanged)

| Test | Checks |
|---|---|
| `tests/app_pages.rs` `images_become_pages_at_their_resolution` | PNG `pHYs` 5906/m → 150 dpi page (96 × 144 pt for 200 × 300 px); JPEG EXIF MM, Orientation 6, XResolution 144/1, unit 2 → 72 × 144 pt upright page; refusals |
| `tests/app_pages.rs` `images_stop_once_they_pass_the_save_ceiling` | budget |
| `tests/app_accept_minors.rs` `a_png_past_the_decoding_budget_is_too_large` | budget |
| `tests/app_edit_objects.rs` `an_image_page_made_from_a_photo_replaces_upright` | orientation through EXIF (its `jpeg(orientation)` helper builds an MM EXIF with Orientation) |
| `src/app/ops/pages/images.rs` `truncated_jpegs_and_pngs_are_refused_without_panicking` | JFIF 72 dpi → 72; every prefix refused or read without panic |
| `src/app/ops/pages/images.rs` `deflate_matches_miniz_and_stops_at_its_cap` | not this workstream |
| `tests/app_annots.rs` `image_stamps_and_signatures_draw_the_image_over_their_rect`, `stamps_on_turned_pages_draw_inside_their_rect` | image stamps (orientation path) |
| `tests/app_scan.rs` (6 tests) | scan-compose reads page JPEGs through `images::read` |
| node: `tests/p2-assemble.test.mjs`, `tests/p2-pages.test.mjs` pdf-from-images cases | page sizes of the fixture images |

### 7.2 Replace

| Test | Why | Replacement |
|---|---|---|
| `images.rs` unit test `exif_reads_resolution_and_orientation_in_both_byte_orders` | calls the removed private `exif(segment)` with its old signature | the same II segment (Orientation 6, XResolution 300/1, unit 2) through `resolution::exif` gives `Exif { dpi: Some(300), orientation: Some(6) }`; the same in MM; every prefix gives a value without panic; `Exif\0\0XX*\0` gives `Exif::default()` |
| `images.rs` unit test `photoshop_resolution_and_icc_chunks` | calls the removed `photoshop(segment)` | Photoshop part: ResolutionInfo 150 → `Some(150)` through `photoshop_dpi`, every prefix without panic. The ICC chunk part (`icc_profile`) stays as it is |

Comments to scrub in the surviving `images.rs` **before hand-off** (they
name MuPDF functions, describe MuPDF internals or cite MuPDF as the
reason): the module doc ("as MuPDF does", "Unlike MuPDF"), the PNG CRC
comment ("as MuPDF reads it"), the `pHYs` conversion comment that states
MuPDF's integer formula (gone with that code), the `store_shared` doc that
names a MuPDF function, the removed functions' doc comments (gone with
them), and "MuPDF prefers EXIF, then Photoshop, then JFIF" (gone with the
precedence code). Reword any that remain as plain statements of
behaviour. The removed functions themselves are cut from the implementer's
copy, not left commented out.

### 7.3 New cases (`resolution.rs` unit tests)

Each row of the section 6 table is a case, with the "This spec" column as
the expected value. Inputs are built in the test, as the harness builds
them. In addition:

1. `exif`: Orientation as LONG; Orientation 0 and 9 → `None`; ResolutionUnit
   as LONG; SRATIONAL −300/1 → unusable; XResolution unusable, YResolution
   200/1 → 200; a duplicated XResolution, where the first wins; an entry
   count of 65,535 with a 40-byte payload returns quickly; an IFD offset of
   `0xFFFF_FFF0` returns `Exif::default()`; a value offset pointing past the
   end gives an unusable resolution and keeps a valid orientation.
2. `photoshop_dpi`: data size `0xFFFF_FFFF` ends the scan; a zero-length
   name, which takes two bytes; an odd data size with its pad byte;
   ResolutionInfo of 2 bytes, then a valid one, gives the valid one.
3. `jfif_dpi`: payload cut after the units byte → `None`.
4. `png_dpi`: `u32::MAX` per metre → `Some(109_092_169)` (fits);
   0 → `None`; 19 per metre (0.48 dpi, rounds to 0) → `None`.
5. `jpeg_dpi`: `(None, None, None)` → 0; `(None, Some(200), Some(150))` →
   200; `(Some(300), Some(200), Some(150))` → 300.
6. End to end through `images::read`: a JPEG with an EXIF ResolutionUnit 1
   and JFIF 150 gives `dpi == 150`; a JPEG with JFIF 118 dots/cm gives 300.

### 7.4 Open questions for the owner

- **Q1. Implausible resolutions.** A JFIF density of 1 dpi, which some broken
  encoders write, gives a page 72 times the pixel size, then shrunk to the
  14,400 pt cap. Should values below some floor (for example 10 dpi) count
  as none? This spec keeps them, as before.
- **Q2. XMP.** Should a resolution in XMP be read when no other source has
  one? Not in this spec.

---

## Leak-check change log (2026-10-18, independent checker)

- 7.2: scrub list completed (the `store_shared` doc naming a MuPDF
  function, the `pHYs` comment giving MuPDF's integer formula, "Unlike
  MuPDF" in the module doc) and marked as a before-hand-off step.
