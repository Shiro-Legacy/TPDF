// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acceptance for spec 08 (image resolution and orientation for
//! pdf-from-images): every case is an image built here, read through
//! `images::read`, and judged by the `dpi` and `orientation` it reports.
//! Expected values come from TIFF 6.0, EXIF 2.3, JFIF 1.02, the Photoshop
//! image-resource layout, PNG `pHYs` and the spec's own rules (nearest-integer
//! rounding, EXIF then Photoshop then JFIF with fall-through, resolutions
//! below 10 dpi read as unknown).
use std::time::{Duration, Instant};
use tandem_pdf_engine::app::ops::pages::images::{Placed, read};

// ------------------------------------------------------------------ builders

/// A 1 × 1 greyscale baseline JPEG's headers with `segments` (marker,
/// payload after the length bytes) in front of the frame. The engine stores
/// the scan data unread, so headers are enough.
fn jpeg(segments: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8];
    for (marker, payload) in segments {
        bytes.extend_from_slice(&[0xFF, *marker]);
        bytes.extend_from_slice(&(payload.len() as u16 + 2).to_be_bytes());
        bytes.extend_from_slice(payload);
    }
    bytes.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
    bytes.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 0x3F, 0, 0xFF, 0xD9]);
    bytes
}

fn placed(bytes: &[u8]) -> Placed {
    read(bytes, usize::MAX).unwrap_or_else(|error| panic!("unreadable: {error:?}"))
}

fn dpi_of(segments: &[(u8, Vec<u8>)]) -> u32 {
    placed(&jpeg(segments)).dpi
}

/// A JFIF APP0 payload: version 1.02, `units`, x and y density `density`.
fn jfif(units: u8, density: u16) -> (u8, Vec<u8>) {
    let mut payload = b"JFIF\0\x01\x02".to_vec();
    payload.push(units);
    payload.extend_from_slice(&density.to_be_bytes());
    payload.extend_from_slice(&density.to_be_bytes());
    payload.extend_from_slice(&[0, 0]);
    (0xE0, payload)
}

/// TIFF field types (TIFF 6.0 §2).
const SHORT: u16 = 3;
const LONG: u16 = 4;
const RATIONAL: u16 = 5;
const SRATIONAL: u16 = 10;

const ORIENTATION: u16 = 274;
const X_RESOLUTION: u16 = 282;
const Y_RESOLUTION: u16 = 283;
const RESOLUTION_UNIT: u16 = 296;

/// One 0th-IFD entry: tag, type, count and the value's bytes in the file's
/// byte order (left-justified in the value field when four bytes or fewer,
/// otherwise stored after the IFD at an offset).
struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    value: Vec<u8>,
}

fn short(big: bool, tag: u16, value: u16) -> Entry {
    let value = if big {
        value.to_be_bytes()
    } else {
        value.to_le_bytes()
    };
    Entry {
        tag,
        kind: SHORT,
        count: 1,
        value: value.to_vec(),
    }
}

fn long(big: bool, tag: u16, value: u32) -> Entry {
    let value = if big {
        value.to_be_bytes()
    } else {
        value.to_le_bytes()
    };
    Entry {
        tag,
        kind: LONG,
        count: 1,
        value: value.to_vec(),
    }
}

fn rational(big: bool, tag: u16, numerator: u32, denominator: u32) -> Entry {
    let mut value = Vec::new();
    for part in [numerator, denominator] {
        value.extend_from_slice(&if big {
            part.to_be_bytes()
        } else {
            part.to_le_bytes()
        });
    }
    Entry {
        tag,
        kind: RATIONAL,
        count: 1,
        value,
    }
}

fn srational(big: bool, tag: u16, numerator: i32, denominator: i32) -> Entry {
    let mut value = Vec::new();
    for part in [numerator, denominator] {
        value.extend_from_slice(&if big {
            part.to_be_bytes()
        } else {
            part.to_le_bytes()
        });
    }
    Entry {
        tag,
        kind: SRATIONAL,
        count: 1,
        value,
    }
}

/// An EXIF APP1 payload (`Exif\0\0`, then a TIFF header whose 0th IFD
/// starts at 8 and holds `entries`).
fn exif(big: bool, entries: &[Entry]) -> (u8, Vec<u8>) {
    let u16b = |v: u16| {
        if big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let u32b = |v: u32| {
        if big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        }
    };
    let mut tiff = if big {
        b"MM\0*".to_vec()
    } else {
        b"II*\0".to_vec()
    };
    tiff.extend_from_slice(&u32b(8));
    tiff.extend_from_slice(&u16b(entries.len() as u16));
    // Out-of-line values follow the entries and the next-IFD offset.
    let mut data_at = 8 + 2 + 12 * entries.len() + 4;
    let mut data = Vec::new();
    for entry in entries {
        tiff.extend_from_slice(&u16b(entry.tag));
        tiff.extend_from_slice(&u16b(entry.kind));
        tiff.extend_from_slice(&u32b(entry.count));
        if entry.value.len() <= 4 {
            let mut field = entry.value.clone();
            field.resize(4, 0);
            tiff.extend_from_slice(&field);
        } else {
            tiff.extend_from_slice(&u32b(data_at as u32));
            data.extend_from_slice(&entry.value);
            data_at += entry.value.len();
        }
    }
    tiff.extend_from_slice(&[0; 4]);
    tiff.extend_from_slice(&data);
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&tiff);
    (0xE1, payload)
}

/// EXIF with XResolution and YResolution `n/d` and an optional
/// ResolutionUnit.
fn exif_resolution(big: bool, n: u32, d: u32, unit: Option<u16>) -> (u8, Vec<u8>) {
    let mut entries = vec![
        rational(big, X_RESOLUTION, n, d),
        rational(big, Y_RESOLUTION, n, d),
    ];
    if let Some(unit) = unit {
        entries.push(short(big, RESOLUTION_UNIT, unit));
    }
    exif(big, &entries)
}

/// One image resource block (signature, ID, Pascal name padded to even,
/// size, data padded to even).
fn resource(signature: &[u8; 4], id: u16, name: &[u8], data: &[u8]) -> Vec<u8> {
    let mut block = signature.to_vec();
    block.extend_from_slice(&id.to_be_bytes());
    block.push(name.len() as u8);
    block.extend_from_slice(name);
    if (name.len() + 1) % 2 == 1 {
        block.push(0);
    }
    block.extend_from_slice(&(data.len() as u32).to_be_bytes());
    block.extend_from_slice(data);
    if data.len() % 2 == 1 {
        block.push(0);
    }
    block
}

/// ResolutionInfo (0x03ED): hRes as 16.16 fixed point, its display unit,
/// width unit, vRes, its display unit, height unit.
fn resolution_info(h_res: f64, h_res_unit: u16) -> Vec<u8> {
    let fixed = (h_res * 65536.0).round() as u32;
    let mut data = fixed.to_be_bytes().to_vec();
    data.extend_from_slice(&h_res_unit.to_be_bytes());
    data.extend_from_slice(&1u16.to_be_bytes());
    data.extend_from_slice(&fixed.to_be_bytes());
    data.extend_from_slice(&h_res_unit.to_be_bytes());
    data.extend_from_slice(&1u16.to_be_bytes());
    data
}

/// An APP13 payload of `blocks`.
fn photoshop(blocks: &[Vec<u8>]) -> (u8, Vec<u8>) {
    let mut payload = b"Photoshop 3.0\0".to_vec();
    for block in blocks {
        payload.extend_from_slice(block);
    }
    (0xED, payload)
}

fn photoshop_dpi_block(h_res: f64) -> Vec<u8> {
    resource(b"8BIM", 0x03ED, b"", &resolution_info(h_res, 1))
}

/// A 1 × 1 grey PNG, with a `pHYs` chunk when `phys` is (x, y, metre).
fn png(phys: Option<(u32, u32, bool)>) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 1, 1);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        if let Some((x, y, metre)) = phys {
            encoder.set_pixel_dims(Some(png::PixelDimensions {
                xppu: x,
                yppu: y,
                unit: if metre {
                    png::Unit::Meter
                } else {
                    png::Unit::Unspecified
                },
            }));
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[128]).unwrap();
    }
    out
}

// ------------------------------------------------------------------ section 6 rows

#[test]
fn jfif_density_in_inches_or_none() {
    assert_eq!(dpi_of(&[]), 0, "no metadata declares nothing");
    assert_eq!(dpi_of(&[jfif(1, 300)]), 300);
    assert_eq!(dpi_of(&[jfif(0, 1)]), 0, "units 0 is an aspect ratio only");
    assert_eq!(
        dpi_of(&[jfif(0, 300)]),
        0,
        "units 0 is an aspect ratio only"
    );
    assert_eq!(dpi_of(&[jfif(1, 0)]), 0, "density 0 is none");
    assert_eq!(dpi_of(&[jfif(3, 300)]), 0, "unknown units are none");
}

#[test]
fn exif_resolution_in_both_byte_orders_and_units() {
    for big in [false, true] {
        assert_eq!(
            dpi_of(&[exif(
                big,
                &[
                    rational(big, X_RESOLUTION, 300, 1),
                    short(big, RESOLUTION_UNIT, 2)
                ]
            )]),
            300,
            "XResolution alone is enough (big {big})"
        );
        assert_eq!(
            dpi_of(&[exif_resolution(big, 300, 1, Some(2))]),
            300,
            "big {big}"
        );
        assert_eq!(
            dpi_of(&[exif_resolution(big, 300, 1, None)]),
            300,
            "an absent ResolutionUnit means inches (TIFF 6.0 default), big {big}"
        );
        assert_eq!(
            dpi_of(&[exif_resolution(big, 300, 1, Some(1))]),
            0,
            "ResolutionUnit 1 has no absolute unit, big {big}"
        );
        assert_eq!(
            dpi_of(&[exif(
                big,
                &[
                    rational(big, X_RESOLUTION, 300, 1),
                    rational(big, Y_RESOLUTION, 150, 1),
                    short(big, RESOLUTION_UNIT, 2),
                ],
            )]),
            300,
            "the x axis is reported, big {big}"
        );
    }
}

#[test]
fn exif_comes_before_photoshop_and_photoshop_before_jfif() {
    let exif300 = exif_resolution(false, 300, 1, Some(2));
    assert_eq!(dpi_of(&[jfif(1, 150), exif300.clone()]), 300);
    assert_eq!(
        dpi_of(&[exif300.clone(), photoshop(&[photoshop_dpi_block(200.0)])]),
        300
    );
    assert_eq!(
        dpi_of(&[jfif(1, 150), photoshop(&[photoshop_dpi_block(200.0)])]),
        200
    );
    assert_eq!(
        dpi_of(&[
            jfif(1, 150),
            photoshop(&[photoshop_dpi_block(200.0)]),
            exif300
        ]),
        300
    );
    // An EXIF segment whose rationals have zero denominators is unusable.
    assert_eq!(
        dpi_of(&[exif_resolution(false, 300, 0, Some(2)), jfif(1, 150)]),
        150
    );
}

#[test]
fn photoshop_resolution_info_in_pixels_per_inch_whatever_its_display_unit() {
    assert_eq!(dpi_of(&[photoshop(&[photoshop_dpi_block(200.0)])]), 200);
    let cm_display = resource(b"8BIM", 0x03ED, b"", &resolution_info(200.0, 2));
    assert_eq!(
        dpi_of(&[photoshop(&[cm_display])]),
        200,
        "hResUnit only says how to display"
    );
    let before = resource(b"8BIM", 0x0400, b"", &[1, 2, 3, 4]);
    assert_eq!(
        dpi_of(&[photoshop(&[before, photoshop_dpi_block(240.0)])]),
        240,
        "after an unnamed 4-byte block"
    );
}

#[test]
fn png_phys_per_metre_rounds_to_the_nearest_inch() {
    let dpi = |phys| placed(&png(phys)).dpi;
    assert_eq!(dpi(None), 0);
    assert_eq!(dpi(Some((11811, 11811, true))), 300);
    assert_eq!(dpi(Some((3780, 3780, true))), 96);
    assert_eq!(dpi(Some((2834, 2834, true))), 72);
    assert_eq!(
        dpi(Some((11811, 11811, false))),
        0,
        "unit 0 is an aspect ratio only"
    );
    assert_eq!(dpi(Some((11811, 5906, true))), 300, "the x axis");
    assert_eq!(dpi(Some((0, 0, true))), 0);
    assert_eq!(dpi(Some((19, 19, true))), 0, "0.48 dpi rounds to 0");
}

#[test]
fn conversions_round_to_the_nearest_integer() {
    assert_eq!(dpi_of(&[jfif(2, 118)]), 300, "118 dots per cm");
    assert_eq!(
        dpi_of(&[exif_resolution(false, 118, 1, Some(3))]),
        300,
        "118 per cm"
    );
    assert_eq!(
        dpi_of(&[exif_resolution(true, 2997, 10, Some(2))]),
        300,
        "299.7 dpi"
    );
    assert_eq!(
        dpi_of(&[photoshop(&[photoshop_dpi_block(200.5)])]),
        201,
        "ties away from zero"
    );
    assert_eq!(dpi_of(&[photoshop(&[photoshop_dpi_block(299.75)])]), 300);
}

#[test]
fn an_unusable_exif_resolution_falls_through_to_jfif() {
    assert_eq!(
        dpi_of(&[exif_resolution(false, 300, 1, Some(1)), jfif(1, 150)]),
        150
    );
    assert_eq!(
        dpi_of(&[jfif(1, 150), exif_resolution(true, 300, 1, Some(7))]),
        150
    );
    // An EXIF segment that declares only an orientation carries no
    // resolution either.
    assert_eq!(
        dpi_of(&[exif(false, &[short(false, ORIENTATION, 6)]), jfif(1, 150)]),
        150
    );
}

#[test]
fn photoshop_blocks_are_skipped_by_their_layout() {
    for name in [&b"ab"[..], b"abc", b"a"] {
        let named = resource(b"8BIM", 0x0400, name, &[1, 2, 3, 4]);
        assert_eq!(
            dpi_of(&[photoshop(&[named, photoshop_dpi_block(240.0)])]),
            240,
            "after a block named {name:?}"
        );
    }
    let foreign = resource(b"MeSa", 0x0400, b"", &[1, 2, 3, 4]);
    assert_eq!(
        dpi_of(&[photoshop(&[foreign, photoshop_dpi_block(240.0)])]),
        240
    );
    // A ResolutionInfo longer than the usual 16 bytes is read from its first 4.
    let mut long_info = resolution_info(240.0, 1);
    long_info.extend_from_slice(&[0; 6]);
    assert_eq!(
        dpi_of(&[photoshop(&[resource(b"8BIM", 0x03ED, b"", &long_info)])]),
        240
    );
}

#[test]
fn implausible_resolutions_below_10_dpi_are_unknown() {
    assert_eq!(dpi_of(&[jfif(1, 1)]), 0);
    assert_eq!(dpi_of(&[jfif(1, 9)]), 0);
    assert_eq!(dpi_of(&[jfif(1, 10)]), 10);
    assert_eq!(dpi_of(&[exif_resolution(false, 5, 1, Some(2))]), 0);
    // Unknown, so the next source is read.
    assert_eq!(
        dpi_of(&[exif_resolution(false, 5, 1, Some(2)), jfif(1, 150)]),
        150
    );
    assert_eq!(placed(&png(Some((236, 236, true)))).dpi, 0, "6 dpi");
}

// ------------------------------------------------------------------ section 7.3

#[test]
fn exif_orientation_and_resolution_in_both_byte_orders() {
    for big in [false, true] {
        let segment = exif(
            big,
            &[
                short(big, ORIENTATION, 6),
                rational(big, X_RESOLUTION, 300, 1),
                short(big, RESOLUTION_UNIT, 2),
            ],
        );
        let image = placed(&jpeg(std::slice::from_ref(&segment)));
        assert_eq!((image.dpi, image.orientation), (300, 6), "big {big}");
        // Every prefix of the segment reads as something, never a panic.
        for end in 0..segment.1.len() {
            let cut = (segment.0, segment.1[..end].to_vec());
            let _ = read(&jpeg(&[cut]), usize::MAX);
        }
    }
    let foreign = (0xE1, b"Exif\0\0XX*\0".to_vec());
    let image = placed(&jpeg(&[foreign]));
    assert_eq!((image.dpi, image.orientation), (0, 1));
    assert_eq!(placed(&jpeg(&[])).orientation, 1, "no EXIF is upright");
}

#[test]
fn invalid_orientations_and_bad_offsets_read_as_none() {
    for value in [0u16, 9, 300] {
        let image = placed(&jpeg(&[exif(false, &[short(false, ORIENTATION, value)])]));
        assert_eq!(image.orientation, 1, "orientation {value}");
    }
    // A value offset past the end: the resolution is unusable, the
    // orientation stays.
    let mut segment = exif(
        false,
        &[
            short(false, ORIENTATION, 6),
            rational(false, X_RESOLUTION, 300, 1),
        ],
    );
    let len = segment.1.len();
    segment.1.truncate(len - 8);
    let image = placed(&jpeg(&[segment]));
    assert_eq!((image.dpi, image.orientation), (0, 6));
    // A signed rational below zero is unusable.
    let image = placed(&jpeg(&[exif(
        true,
        &[
            srational(true, X_RESOLUTION, -300, 1),
            short(true, RESOLUTION_UNIT, 2),
        ],
    )]));
    assert_eq!(image.dpi, 0);
    // An IFD offset far past the end.
    let mut payload = b"Exif\0\0II*\0".to_vec();
    payload.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    payload.extend_from_slice(&[0; 16]);
    let image = placed(&jpeg(&[(0xE1, payload)]));
    assert_eq!((image.dpi, image.orientation), (0, 1));
}

#[test]
fn a_huge_entry_count_in_a_short_payload_returns_quickly() {
    let mut payload = b"Exif\0\0II*\0\x08\0\0\0".to_vec();
    payload.extend_from_slice(&65_535u16.to_le_bytes());
    payload.resize(6 + 40, 0);
    let start = Instant::now();
    let image = placed(&jpeg(&[(0xE1, payload)]));
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!((image.dpi, image.orientation), (0, 1));
}

#[test]
fn exif_reads_long_orientation_and_unit_the_first_tag_and_y_when_x_is_unusable() {
    let image = placed(&jpeg(&[exif(true, &[long(true, ORIENTATION, 6)])]));
    assert_eq!(image.orientation, 6, "Orientation as LONG");
    assert_eq!(
        dpi_of(&[exif(
            false,
            &[
                rational(false, X_RESOLUTION, 300, 1),
                long(false, RESOLUTION_UNIT, 1)
            ],
        )]),
        0,
        "ResolutionUnit 1 as LONG"
    );
    assert_eq!(
        dpi_of(&[exif(
            false,
            &[
                rational(false, X_RESOLUTION, 118, 1),
                long(false, RESOLUTION_UNIT, 3)
            ],
        )]),
        300,
        "ResolutionUnit 3 (cm) as LONG"
    );
    assert_eq!(
        dpi_of(&[exif(
            false,
            &[
                rational(false, X_RESOLUTION, 300, 0),
                rational(false, Y_RESOLUTION, 200, 1),
                short(false, RESOLUTION_UNIT, 2),
            ],
        )]),
        200,
        "YResolution when XResolution is unusable"
    );
    assert_eq!(
        dpi_of(&[exif(
            false,
            &[
                rational(false, X_RESOLUTION, 300, 1),
                rational(false, X_RESOLUTION, 150, 1),
                short(false, RESOLUTION_UNIT, 2),
            ],
        )]),
        300,
        "the first XResolution wins"
    );
    assert_eq!(
        dpi_of(&[exif(
            true,
            &[
                srational(true, X_RESOLUTION, 300, 1),
                short(true, RESOLUTION_UNIT, 2)
            ],
        )]),
        300,
        "a positive SRATIONAL"
    );
}

#[test]
fn photoshop_scans_stop_safely() {
    let segment = photoshop(&[photoshop_dpi_block(150.0)]);
    assert_eq!(dpi_of(std::slice::from_ref(&segment)), 150);
    for end in 0..segment.1.len() {
        let cut = (segment.0, segment.1[..end].to_vec());
        let _ = read(&jpeg(&[cut]), usize::MAX);
    }
    // A data size of 0xFFFF_FFFF ends the scan.
    let mut huge = b"8BIM\x04\x00\0\0".to_vec();
    huge.extend_from_slice(&u32::MAX.to_be_bytes());
    huge.extend_from_slice(&[0; 8]);
    assert_eq!(dpi_of(&[photoshop(&[huge, photoshop_dpi_block(150.0)])]), 0);
    // An odd data size takes its pad byte.
    let odd = resource(b"8BIM", 0x0400, b"", &[1, 2, 3]);
    assert_eq!(
        dpi_of(&[photoshop(&[odd, photoshop_dpi_block(240.0)])]),
        240
    );
    // A ResolutionInfo of 2 bytes is unusable and the scan goes on.
    let short_info = resource(b"8BIM", 0x03ED, b"", &[0, 100]);
    assert_eq!(
        dpi_of(&[photoshop(&[short_info, photoshop_dpi_block(240.0)])]),
        240
    );
    // Not a Photoshop segment.
    assert_eq!(dpi_of(&[(0xED, b"Photoshop 2.5\0".to_vec())]), 0);
}

#[test]
fn a_jfif_header_cut_after_its_units_declares_nothing() {
    let (marker, mut payload) = jfif(1, 300);
    payload.truncate(8);
    assert_eq!(dpi_of(&[(marker, payload)]), 0);
    let (marker, mut payload) = jfif(1, 300);
    payload.truncate(9);
    assert_eq!(dpi_of(&[(marker, payload)]), 0);
}

#[test]
fn the_largest_png_phys_value_fits() {
    let image = read(&png(Some((u32::MAX, u32::MAX, true))), usize::MAX);
    // A decoder may refuse values past 2^31 − 1 (PNG §11.3.5.3); when it
    // reads them, the dpi is the rounded product.
    if let Ok(image) = image {
        assert_eq!(image.dpi, 109_092_169);
    }
    let image = placed(&png(Some((2_147_483_647, 2_147_483_647, true))));
    assert_eq!(image.dpi, 54_546_085);
}
