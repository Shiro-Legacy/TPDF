# Workstream 08 notes

## What was done

- `engine/src/app/ops/pages/resolution.rs`: `exif`, `photoshop_dpi`,
  `jfif_dpi`, `png_dpi`, `jpeg_dpi` per spec 08, with unit tests for the
  section 6 rows and the section 7.2/7.3 cases.
- `engine/src/app/ops/pages/images.rs`: the two `todo!()` call sites, plus
  the section 2.1 glue in `jpeg_info` (first JFIF APP0, first EXIF APP1,
  first APP13 that yields a resolution) and two small private helpers
  (`png_resolution`, `JpegInfo::resolution`) so the glue can be unit
  tested without the object layer.
- `engine/tests/cleanroom_jpeg.rs`: the five workstream-08 `#[ignore]`
  attributes removed.

## Acceptance status in this tree

Every test that goes through `images::read` fails in this tree before it
reaches workstream-08 code: `read` builds the image dictionary with
`Dictionary::insert` (`src/object.rs:222`, workstream 01 stub). So:

- `tests/cleanroom_jpeg.rs`: all 16 tests (including the 5 un-ignored ones)
  panic at `src/object.rs:222` (workstream 01).
- `src/app/ops/pages/images.rs`
  `truncated_jpegs_and_pngs_are_refused_without_panicking`: same,
  `src/object.rs:222`.
- `tests/app_pages.rs` (26 of 27), `app_accept_minors.rs` (7),
  `app_edit_objects.rs` (22), `app_annots.rs` (30), `app_scan.rs` (6):
  panic in `src/parser.rs` (workstream 01), `src/object.rs` (01),
  `src/app/forms/text.rs` (07) or `src/app/annots/appearance.rs` (06).
  None panics in a workstream-08 file.

To cover the glue without the object layer, `resolution.rs` has
end-to-end unit tests through the real marker walk (`jpeg_info`) and the
real PNG decoder (`png_resolution`): EXIF unit 1 + JFIF 150 gives 150,
JFIF 118 dots/cm gives 300, first-segment rules, XMP skipped, JFXX not
read. All 17 workstream-08 unit tests pass (`cargo test --lib pages::`
shows 17 passed, 1 failed: the `object.rs` one above).

`cargo clippy --all-targets -- -D warnings` reports nothing in this
workstream's files; it fails on other workstreams' stubs
(`view/text_unicode.rs`, `forms/scripts.rs`, `forms/text.rs`,
`view/mod.rs`, `view/fonts.rs`, `serialize.rs`).

## Design

- All conversions use exact integer fractions (u128): EXIF n/d, ×254/100
  for centimetres, ×254/10000 for PNG metres, /65536 for the Photoshop
  Fixed. Rounding is floor((2n + d) / 2d), i.e. nearest, ties away from
  zero, with no floating-point error near ties (e.g. 25 per cm = 63.5
  dpi rounds to 64 exactly).
- Owner decision J1: a result below 10 dpi is `None` in every source, so
  it falls through to the next source (`jpeg_dpi` also skips any value
  below 10 defensively). This overrides the spec's section 6 row
  "JFIF units 1, density 1 → 1".
- No allocation in the readers; all offsets use checked arithmetic.

## Spec ambiguities and how they were resolved

1. Duplicated tag whose first occurrence is unreadable (wrong type,
   count 0): the first occurrence still wins (literal reading of "first
   occurrence of each tag wins"); the value is then unusable.
2. ResolutionUnit present but unreadable (wrong type, count 0, value out
   of bounds): treated as "no unit", so no EXIF resolution (and the next
   source is read). Only an absent tag defaults to inches.
3. YResolution fallback: used whenever XResolution gives no usable dpi,
   including when X converts below 10 dpi or past `u32`.
4. SRATIONAL with both parts negative (e.g. -300/-1): the value is
   positive, so usable.
5. Photoshop `hRes` is read as a signed 16.16 Fixed (Adobe's type); a
   negative value is unusable and the scan goes on.
6. A ResolutionInfo whose data runs past the payload ends the scan even if
   its first 4 bytes are present (spec 3.3: data past the payload ends the
   scan).
7. A SHORT/LONG field whose values do not fit in 4 bytes (count > 2 /
   > 1): the first value is read at the offset.
8. Section 7.3 item 6 (end to end through `images::read`) is covered by
   the integration tests, which cannot run here (workstream 01), and by
   unit tests through `jpeg_info`/`png_resolution` instead.
