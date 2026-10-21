# Pinned standard-font outlines, packed

The four `.bin` files beside this README hold the unhinted glyph outlines of the
pinned Base14 faces the view draws unembedded standard fonts with: Helvetica and
Times-Roman (`helvetica.bin`, `times_roman.bin`) and the two substitute faces
Helvetica-Bold and Helvetica-BoldOblique (`helvetica_bold.bin`,
`helvetica_bold_oblique.bin`). `packed.rs` decodes a face once, on first use,
into the tables `builtin_outlines.rs` serves; the API and every value are those of
the generated Rust tables the files replaced (October 21, 2026).

## Licence of the font data (OFL-1.1)

Derived canonical outline data from pinned URW CFF font resources (NimbusSans and
NimbusRoman). Copyright (URW)++, Copyright 2014 by (URW)++ Design & Development.
Copyright 2016 URW++ Design & Development. Full SIL Open Font License 1.1 and the
applicable notices: `engine/NOTICE` and `engine/reference/licenses/urw-outlines/OFL.txt`.
The engine code around the data, including `packed.rs`, is MIT.

Source CFF SHA-256 (also `BuiltinOutline::asset_sha256`):

| Face | SHA-256 |
|---|---|
| Helvetica | 366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706 |
| Times-Roman | eb609a306b81357853ba91c2a3c47f3bba04f3a8cfdeedc733b1aa4af5d03d4c |
| Helvetica-Bold | d4ea80d1d0e29a1e950808009ca8f96fe07a0670b60b556f6b2cf0705337fa9f |
| Helvetica-BoldOblique | 9cdfc2d0947339dce76f99bdf9e2d9fe01e472a384cf683007e6a0bb35634623 |

## Format (version `TPDFOUT1`, little-endian)

```
0   8 bytes  magic "TPDFOUT1"
8   u32      command count
12  u32      glyph count
16  commands: tag u8, then i16 coordinates in font units (1000 per em)
             0 MoveTo x y | 1 LineTo x y | 2 CubicTo x1 y1 x2 y2 x y | 3 Close
    glyphs:   u8 name length, name bytes (exact AFM/CFF glyph name, at most 128,
             strictly ascending bytewise for the binary search), u16 native CFF
             glyph index, u32 first command, u32 command count
```

Nothing follows the last glyph. The decoder refuses any other layout.

## Regenerating

1. `engine/oracle/generate-builtin-outlines.py` and
   `generate-substitute-outlines.py` (test-only, AGPL: `engine/oracle/README.md`)
   write the Rust tables from the pinned CFF assets through the MuPDF oracle, as
   before, into a scratch directory.
2. `engine/generate/pack_outlines.py OUT_DIR TABLE.rs...` packs those tables into
   these files and re-reads what it wrote before it exits 0.
