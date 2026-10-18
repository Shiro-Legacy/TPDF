# Sources used by workstream 08

Everything below was used from memory of the public documents (no web
access was needed). No MuPDF code, Tandem PDF history or other working
directory was read.

| Source | Where from | Used for |
|---|---|---|
| `specs/08-jpeg-resolution.md`, `specs/OWNER-DECISIONS.md`, `specs/SPEC-RULES.md` | this tree | behaviour, interface, owner decision J1 (below 10 dpi is unknown) |
| TIFF Revision 6.0 (Adobe, 1992), sections 2 (image file header, IFD, field types) and 8 (XResolution 282, YResolution 283, ResolutionUnit 296 and its default of 2) | public standard | `exif`: header, byte orders, 12-byte entries, value-or-offset rule, SHORT/LONG/RATIONAL/SRATIONAL |
| EXIF 2.3 (CIPA DC-008 / JEITA CP-3451C), APP1 structure and Orientation (274) values 1..8 | public standard | `exif`: `Exif\0` identifier, TIFF header at offset 6, 0th IFD, Orientation |
| JFIF 1.02 (ECMA TR/98, ITU-T T.871), APP0 marker layout | public standard | `jfif_dpi`: units byte, Xdensity, `JFXX` not read |
| Adobe Photoshop File Formats Specification, "Image Resources Section" and ResolutionInfo (0x03ED); JPEG APP13 `Photoshop 3.0` wrapper | public Adobe document | `photoshop_dpi`: block layout, even-padded Pascal name and data, `hRes` as 16.16 Fixed |
| PNG (ISO/IEC 15948, W3C PNG 3rd edition), section 11.3.5.3 `pHYs` | public standard | `png_dpi`: per-metre unit |
| `png` crate 0.18 public API (`Info::pixel_dims`, `PixelDimensions`, `Unit`, `Encoder::set_pixel_dims`) | crate API as already used by `tests/cleanroom_jpeg.rs` in this tree | reading `pHYs` in `images.rs`; building PNGs in unit tests |
| Surviving code of `engine/src/app/ops/pages/images.rs` and the acceptance tests in `engine/tests/` | this tree | call sites, marker walk, expected values |
