// SPDX-License-Identifier: AGPL-3.0-or-later
//! Resolution and orientation an image declares in its metadata: the 0th
//! IFD of an EXIF APP1 segment (TIFF 6.0, EXIF 2.3), Photoshop
//! ResolutionInfo in an APP13 segment, the JFIF APP0 density and the PNG
//! `pHYs` chunk.
//!
//! Every conversion to pixels per inch works on exact fractions and rounds
//! to the nearest integer, ties away from zero. A result below [`MIN_DPI`]
//! is implausible and reads as none, as does one that does not fit in
//! `u32`; so no function here returns `Some(0)`.
//!
//! The readers only index into the slice they are given, with checked
//! arithmetic, allocate nothing and do work linear in its length.

/// Resolutions below this many pixels per inch are treated as unknown
/// (some broken encoders write a density of 1).
const MIN_DPI: u64 = 10;

/// Inches per centimetre as the fraction 254 / 100 (exact).
const CM_NUMERATOR: u128 = 254;
const CM_DENOMINATOR: u128 = 100;

/// What the 0th IFD of one EXIF APP1 segment declares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Exif {
    /// Pixels per inch, when the segment declares a usable resolution.
    pub dpi: Option<u32>,
    /// 1..8, when the segment declares a valid orientation.
    pub orientation: Option<u8>,
}

/// `numerator / denominator` pixels per inch, rounded half away from zero;
/// `None` when the denominator is zero, or the result is below [`MIN_DPI`]
/// or does not fit in `u32`.
fn dpi_of(numerator: u128, denominator: u128) -> Option<u32> {
    if denominator == 0 {
        return None;
    }
    // For non-negative values, round(n / d) = floor((2n + d) / 2d).
    let twice = numerator.checked_mul(2)?.checked_add(denominator)?;
    let rounded = u64::try_from(twice / denominator.checked_mul(2)?).ok()?;
    if rounded < MIN_DPI {
        return None;
    }
    u32::try_from(rounded).ok()
}

// ------------------------------------------------------------------ EXIF

const TAG_ORIENTATION: u16 = 274;
const TAG_X_RESOLUTION: u16 = 282;
const TAG_Y_RESOLUTION: u16 = 283;
const TAG_RESOLUTION_UNIT: u16 = 296;

const TYPE_SHORT: u16 = 3;
const TYPE_LONG: u16 = 4;
const TYPE_RATIONAL: u16 = 5;
const TYPE_SRATIONAL: u16 = 10;

/// ResolutionUnit values (TIFF 6.0).
const UNIT_INCH: u32 = 2;
const UNIT_CENTIMETRE: u32 = 3;

/// A TIFF stream (the bytes from its header on) and its byte order.
struct Tiff<'a> {
    bytes: &'a [u8],
    big: bool,
}

/// One 12-byte IFD entry as stored.
#[derive(Clone, Copy)]
struct Field {
    kind: u16,
    count: u32,
    /// The value field's four bytes, as stored.
    value: [u8; 4],
}

impl Tiff<'_> {
    fn u16_of(&self, raw: [u8; 2]) -> u16 {
        if self.big {
            u16::from_be_bytes(raw)
        } else {
            u16::from_le_bytes(raw)
        }
    }

    fn u32_of(&self, raw: [u8; 4]) -> u32 {
        if self.big {
            u32::from_be_bytes(raw)
        } else {
            u32::from_le_bytes(raw)
        }
    }

    fn array<const N: usize>(&self, at: usize) -> Option<[u8; N]> {
        self.bytes.get(at..at.checked_add(N)?)?.try_into().ok()
    }

    fn u16_at(&self, at: usize) -> Option<u16> {
        Some(self.u16_of(self.array(at)?))
    }

    fn u32_at(&self, at: usize) -> Option<u32> {
        Some(self.u32_of(self.array(at)?))
    }

    /// The bytes of the field's first value, `N` bytes wide: in the value
    /// field when all its values fit in four bytes (left-justified),
    /// otherwise at the offset the value field holds. `None` when the
    /// count is zero or the value lies outside the stream.
    fn first_value<const N: usize>(&self, field: Field) -> Option<[u8; N]> {
        if field.count == 0 {
            return None;
        }
        let total = u64::from(field.count).checked_mul(u64::try_from(N).ok()?)?;
        if total <= 4 {
            return field.value.get(..N)?.try_into().ok();
        }
        let at = usize::try_from(self.u32_of(field.value)).ok()?;
        self.array(at)
    }

    /// A SHORT or LONG field's first value.
    fn unsigned(&self, field: Field) -> Option<u32> {
        match field.kind {
            TYPE_SHORT => Some(u32::from(self.u16_of(self.first_value(field)?))),
            TYPE_LONG => Some(self.u32_of(self.first_value(field)?)),
            _ => None,
        }
    }

    /// A RATIONAL or SRATIONAL field's first value as a positive fraction
    /// (numerator, denominator); `None` when it is zero or negative, its
    /// denominator is zero, or it lies outside the stream.
    fn positive_rational(&self, field: Field) -> Option<(u128, u128)> {
        if !matches!(field.kind, TYPE_RATIONAL | TYPE_SRATIONAL) {
            return None;
        }
        let raw: [u8; 8] = self.first_value(field)?;
        let numerator = self.u32_of(raw.get(..4)?.try_into().ok()?);
        let denominator = self.u32_of(raw.get(4..)?.try_into().ok()?);
        let (numerator, denominator) = if field.kind == TYPE_RATIONAL {
            (i64::from(numerator), i64::from(denominator))
        } else {
            (
                i64::from(numerator.cast_signed()),
                i64::from(denominator.cast_signed()),
            )
        };
        if numerator == 0 || denominator == 0 || (numerator < 0) != (denominator < 0) {
            return None;
        }
        Some((
            u128::from(numerator.unsigned_abs()),
            u128::from(denominator.unsigned_abs()),
        ))
    }
}

/// `payload`: an APP1 segment's bytes after its two length bytes
/// (so it starts with the identifier `Exif\0`).
pub(crate) fn exif(payload: &[u8]) -> Exif {
    read_exif(payload).unwrap_or_default()
}

fn read_exif(payload: &[u8]) -> Option<Exif> {
    // `Exif`, a zero byte, and one more byte the standard sets to zero
    // (any value is accepted).
    if !payload.starts_with(b"Exif\0") {
        return None;
    }
    let bytes = payload.get(6..)?;
    let big = match bytes.get(..2)? {
        b"II" => false,
        b"MM" => true,
        _ => return None,
    };
    let tiff = Tiff { bytes, big };
    if tiff.u16_at(2)? != 42 {
        return None;
    }
    let ifd = usize::try_from(tiff.u32_at(4)?).ok()?;
    let count = tiff.u16_at(ifd)?;

    // The first occurrence of each tag read here.
    let mut orientation = None;
    let mut x = None;
    let mut y = None;
    let mut unit = None;
    let mut at = ifd.checked_add(2)?;
    for _ in 0..count {
        // The walk stops at the first entry that runs past the stream.
        let (Some(tag), Some(kind), Some(count), Some(value)) = (
            tiff.u16_at(at),
            at.checked_add(2).and_then(|at| tiff.u16_at(at)),
            at.checked_add(4).and_then(|at| tiff.u32_at(at)),
            at.checked_add(8).and_then(|at| tiff.array::<4>(at)),
        ) else {
            break;
        };
        let slot = match tag {
            TAG_ORIENTATION => Some(&mut orientation),
            TAG_X_RESOLUTION => Some(&mut x),
            TAG_Y_RESOLUTION => Some(&mut y),
            TAG_RESOLUTION_UNIT => Some(&mut unit),
            _ => None,
        };
        if let Some(slot) = slot
            && slot.is_none()
        {
            *slot = Some(Field { kind, count, value });
        }
        let Some(next) = at.checked_add(12) else {
            break;
        };
        at = next;
    }

    let orientation = orientation
        .and_then(|field| tiff.unsigned(field))
        .filter(|value| (1..=8).contains(value))
        .and_then(|value| u8::try_from(value).ok());

    // An absent ResolutionUnit means inches (TIFF 6.0 default); one that
    // is present but cannot be read declares no unit.
    let unit = match unit {
        None => Some(UNIT_INCH),
        Some(field) => tiff.unsigned(field),
    };
    let dpi_of_field = |field: Option<Field>| -> Option<u32> {
        let (numerator, denominator) = tiff.positive_rational(field?)?;
        match unit? {
            UNIT_INCH => dpi_of(numerator, denominator),
            UNIT_CENTIMETRE => dpi_of(
                numerator.checked_mul(CM_NUMERATOR)?,
                denominator.checked_mul(CM_DENOMINATOR)?,
            ),
            _ => None,
        }
    };
    // YResolution only stands in when XResolution gives nothing usable.
    let dpi = dpi_of_field(x).or_else(|| dpi_of_field(y));
    Some(Exif { dpi, orientation })
}

// ------------------------------------------------------------------ Photoshop

/// Image resource ID of ResolutionInfo.
const RESOLUTION_INFO: u16 = 0x03ED;

/// `payload`: an APP13 segment's bytes after its two length bytes
/// (starting `Photoshop 3.0\0`). Pixels per inch from ResolutionInfo.
pub(crate) fn photoshop_dpi(payload: &[u8]) -> Option<u32> {
    let blocks = payload.strip_prefix(b"Photoshop 3.0\0".as_slice())?;
    let be = |at: usize, len: usize| -> Option<&[u8]> { blocks.get(at..at.checked_add(len)?) };
    let mut at = 0usize;
    while at < blocks.len() {
        // A four-byte signature (any: writers other than Photoshop use
        // their own), the resource ID, then the name's length byte. A block
        // that runs past the payload ends the scan.
        let id_at = at.checked_add(4)?;
        let id = u16::from_be_bytes(be(id_at, 2)?.try_into().ok()?);
        let name_at = id_at.checked_add(2)?;
        let name_len = usize::from(*blocks.get(name_at)?);
        // The Pascal string, length byte included, padded to an even size.
        let size_at = name_at.checked_add((name_len + 1).next_multiple_of(2))?;
        let size = u32::from_be_bytes(be(size_at, 4)?.try_into().ok()?);
        let size = usize::try_from(size).ok()?;
        let data_at = size_at.checked_add(4)?;
        let data = be(data_at, size)?;
        if id == RESOLUTION_INFO
            && let Some(dpi) = resolution_info_dpi(data)
        {
            return Some(dpi);
        }
        // The data is padded to an even size.
        at = data_at.checked_add(size)?.checked_add(size % 2)?;
    }
    None
}

/// ResolutionInfo's `hRes`: signed 16.16 fixed-point pixels per inch,
/// whatever unit it is displayed in. Shorter than 4 bytes, or negative,
/// is unusable.
fn resolution_info_dpi(data: &[u8]) -> Option<u32> {
    let raw: [u8; 4] = data.get(..4)?.try_into().ok()?;
    let fixed = u32::try_from(i32::from_be_bytes(raw)).ok()?;
    dpi_of(u128::from(fixed), 1 << 16)
}

// ------------------------------------------------------------------ JFIF

/// `payload`: an APP0 segment's bytes after its two length bytes
/// (starting `JFIF\0`). Pixels per inch from the density fields.
pub(crate) fn jfif_dpi(payload: &[u8]) -> Option<u32> {
    if !payload.starts_with(b"JFIF\0") {
        return None;
    }
    // Identifier (5 bytes), version (2), units (1), Xdensity (2).
    let units = *payload.get(7)?;
    let density: [u8; 2] = payload.get(8..10)?.try_into().ok()?;
    let density = u128::from(u16::from_be_bytes(density));
    match units {
        1 => dpi_of(density, 1),
        2 => dpi_of(density * CM_NUMERATOR, CM_DENOMINATOR),
        _ => None,
    }
}

// ------------------------------------------------------------------ PNG

/// A PNG pHYs x value and whether its unit is the metre.
pub(crate) fn png_dpi(pixels_per_unit_x: u32, unit_is_metre: bool) -> Option<u32> {
    if !unit_is_metre {
        return None;
    }
    // One metre is 10000 / 254 inches.
    dpi_of(u128::from(pixels_per_unit_x) * CM_NUMERATOR, 10_000)
}

// ------------------------------------------------------------------ precedence

/// The JPEG's resolution: the first of the three that is Some and at least
/// `MIN_DPI`, else 0.
pub(crate) fn jpeg_dpi(exif: Option<u32>, photoshop: Option<u32>, jfif: Option<u32>) -> u32 {
    [exif, photoshop, jfif]
        .into_iter()
        .flatten()
        .find(|&dpi| u64::from(dpi) >= MIN_DPI)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------- builders

    /// One IFD entry: tag, type, count, and its values as (width in
    /// bytes, value) pairs written in the file's byte order.
    struct Entry {
        tag: u16,
        kind: u16,
        count: u32,
        values: Vec<(usize, u64)>,
    }

    fn short(tag: u16, value: u16) -> Entry {
        Entry {
            tag,
            kind: TYPE_SHORT,
            count: 1,
            values: vec![(2, value.into())],
        }
    }

    fn long(tag: u16, value: u32) -> Entry {
        Entry {
            tag,
            kind: TYPE_LONG,
            count: 1,
            values: vec![(4, value.into())],
        }
    }

    fn rational(tag: u16, n: u32, d: u32) -> Entry {
        Entry {
            tag,
            kind: TYPE_RATIONAL,
            count: 1,
            values: vec![(4, n.into()), (4, d.into())],
        }
    }

    fn srational(tag: u16, n: i32, d: i32) -> Entry {
        Entry {
            tag,
            kind: TYPE_SRATIONAL,
            count: 1,
            values: vec![(4, n.cast_unsigned().into()), (4, d.cast_unsigned().into())],
        }
    }

    fn put(out: &mut Vec<u8>, big: bool, width: usize, value: u64) {
        let bytes = value.to_be_bytes();
        let mut part = bytes[8 - width..].to_vec();
        if !big {
            part.reverse();
        }
        out.extend_from_slice(&part);
    }

    /// An APP1 payload whose 0th IFD, at offset 8, holds `entries`.
    fn exif_payload(big: bool, entries: &[Entry]) -> Vec<u8> {
        let mut tiff = if big {
            b"MM\0*".to_vec()
        } else {
            b"II*\0".to_vec()
        };
        put(&mut tiff, big, 4, 8);
        put(&mut tiff, big, 2, entries.len() as u64);
        let mut data_at = 8 + 2 + 12 * entries.len() + 4;
        let mut data = Vec::new();
        for entry in entries {
            put(&mut tiff, big, 2, entry.tag.into());
            put(&mut tiff, big, 2, entry.kind.into());
            put(&mut tiff, big, 4, entry.count.into());
            let mut value = Vec::new();
            for &(width, v) in &entry.values {
                put(&mut value, big, width, v);
            }
            if value.len() <= 4 {
                value.resize(4, 0);
                tiff.extend_from_slice(&value);
            } else {
                put(&mut tiff, big, 4, data_at as u64);
                data_at += value.len();
                data.extend_from_slice(&value);
            }
        }
        tiff.extend_from_slice(&[0; 4]);
        tiff.extend_from_slice(&data);
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(&tiff);
        payload
    }

    fn exif_xy(big: bool, n: u32, d: u32, unit: Option<u16>) -> Vec<u8> {
        let mut entries = vec![
            rational(TAG_X_RESOLUTION, n, d),
            rational(TAG_Y_RESOLUTION, n, d),
        ];
        if let Some(unit) = unit {
            entries.push(short(TAG_RESOLUTION_UNIT, unit));
        }
        exif_payload(big, &entries)
    }

    fn jfif(units: u8, density: u16) -> Vec<u8> {
        let mut payload = b"JFIF\0\x01\x02".to_vec();
        payload.push(units);
        payload.extend_from_slice(&density.to_be_bytes());
        payload.extend_from_slice(&density.to_be_bytes());
        payload.extend_from_slice(&[0, 0]);
        payload
    }

    fn block(signature: &[u8; 4], id: u16, name: &[u8], data: &[u8]) -> Vec<u8> {
        let mut out = signature.to_vec();
        out.extend_from_slice(&id.to_be_bytes());
        out.push(name.len() as u8);
        out.extend_from_slice(name);
        if name.len() % 2 == 0 {
            out.push(0);
        }
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn resolution_info(h_res: f64, unit: u16) -> Vec<u8> {
        let fixed = ((h_res * 65536.0).round() as u32).to_be_bytes();
        let mut data = fixed.to_vec();
        data.extend_from_slice(&unit.to_be_bytes());
        data.extend_from_slice(&1u16.to_be_bytes());
        data.extend_from_slice(&fixed);
        data.extend_from_slice(&unit.to_be_bytes());
        data.extend_from_slice(&1u16.to_be_bytes());
        data
    }

    fn photoshop(blocks: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = b"Photoshop 3.0\0".to_vec();
        for b in blocks {
            payload.extend_from_slice(b);
        }
        payload
    }

    fn ps_dpi(h_res: f64) -> Vec<u8> {
        block(b"8BIM", RESOLUTION_INFO, b"", &resolution_info(h_res, 1))
    }

    /// The resolution the JPEG path computes from these segments.
    fn jpeg(exif_segment: Option<&[u8]>, ps: Option<&[u8]>, jfif_segment: Option<&[u8]>) -> u32 {
        jpeg_dpi(
            exif_segment.and_then(|p| exif(p).dpi),
            ps.and_then(photoshop_dpi),
            jfif_segment.and_then(jfif_dpi),
        )
    }

    // ---------------------------------------------------------- section 6 rows

    #[test]
    fn jfif_rows() {
        assert_eq!(jpeg(None, None, None), 0);
        assert_eq!(jfif_dpi(&jfif(1, 300)), Some(300));
        assert_eq!(jfif_dpi(&jfif(2, 118)), Some(300));
        assert_eq!(jfif_dpi(&jfif(0, 300)), None);
        // Below 10 dpi reads as unknown (owner decision J1).
        assert_eq!(jfif_dpi(&jfif(1, 1)), None);
        assert_eq!(jfif_dpi(&jfif(1, 0)), None);
        assert_eq!(jfif_dpi(&jfif(7, 300)), None);
        assert_eq!(jfif_dpi(b"JFXX\0\x10\x01\x01\x01\x2C"), None);
    }

    #[test]
    fn exif_rows() {
        for big in [false, true] {
            let x_only = exif_payload(
                big,
                &[
                    rational(TAG_X_RESOLUTION, 300, 1),
                    short(TAG_RESOLUTION_UNIT, 2),
                ],
            );
            assert_eq!(exif(&x_only).dpi, Some(300));
            assert_eq!(exif(&exif_xy(big, 300, 1, Some(2))).dpi, Some(300));
            assert_eq!(exif(&exif_xy(big, 300, 1, None)).dpi, Some(300));
            assert_eq!(exif(&exif_xy(big, 118, 1, Some(3))).dpi, Some(300));
            assert_eq!(exif(&exif_xy(big, 2997, 10, Some(2))).dpi, Some(300));
            assert_eq!(exif(&exif_xy(big, 300, 1, Some(1))).dpi, None);
            let x_and_y = exif_payload(
                big,
                &[
                    rational(TAG_X_RESOLUTION, 300, 1),
                    rational(TAG_Y_RESOLUTION, 150, 1),
                    short(TAG_RESOLUTION_UNIT, 2),
                ],
            );
            assert_eq!(exif(&x_and_y).dpi, Some(300));
        }
    }

    #[test]
    fn precedence_rows() {
        let exif300 = exif_xy(false, 300, 1, Some(2));
        let jfif150 = jfif(1, 150);
        let ps200 = photoshop(&[ps_dpi(200.0)]);
        assert_eq!(jpeg(Some(&exif300), None, Some(&jfif150)), 300);
        let unit1 = exif_xy(false, 300, 1, Some(1));
        assert_eq!(jpeg(Some(&unit1), None, Some(&jfif150)), 150);
        let zero = exif_xy(false, 300, 0, Some(2));
        assert_eq!(jpeg(Some(&zero), None, Some(&jfif150)), 150);
        assert_eq!(jpeg(Some(&exif300), Some(&ps200), None), 300);
        assert_eq!(jpeg(None, Some(&ps200), Some(&jfif150)), 200);
    }

    #[test]
    fn photoshop_rows() {
        assert_eq!(photoshop_dpi(&photoshop(&[ps_dpi(200.0)])), Some(200));
        assert_eq!(photoshop_dpi(&photoshop(&[ps_dpi(200.5)])), Some(201));
        let cm = block(b"8BIM", RESOLUTION_INFO, b"", &resolution_info(200.0, 2));
        assert_eq!(photoshop_dpi(&photoshop(&[cm])), Some(200));
        let unnamed = block(b"8BIM", 0x0400, b"", &[1, 2, 3, 4]);
        let payload = photoshop(&[unnamed, ps_dpi(240.0)]);
        assert_eq!(photoshop_dpi(&payload), Some(240));
        for name in [&b"ab"[..], b"abc"] {
            let named = block(b"8BIM", 0x0400, name, &[1, 2, 3, 4]);
            let payload = photoshop(&[named, ps_dpi(240.0)]);
            assert_eq!(photoshop_dpi(&payload), Some(240), "name {name:?}");
        }
        let foreign = block(b"MeSa", 0x0400, b"", &[1, 2, 3, 4]);
        let payload = photoshop(&[foreign, ps_dpi(240.0)]);
        assert_eq!(photoshop_dpi(&payload), Some(240));
    }

    #[test]
    fn png_rows() {
        assert_eq!(png_dpi(11811, true), Some(300));
        assert_eq!(png_dpi(3780, true), Some(96));
        assert_eq!(png_dpi(2834, true), Some(72));
        assert_eq!(png_dpi(11811, false), None);
    }

    // ---------------------------------------------------------- section 7.2

    #[test]
    fn exif_reads_resolution_and_orientation_in_both_byte_orders() {
        for big in [false, true] {
            let payload = exif_payload(
                big,
                &[
                    short(TAG_ORIENTATION, 6),
                    rational(TAG_X_RESOLUTION, 300, 1),
                    short(TAG_RESOLUTION_UNIT, 2),
                ],
            );
            let expected = Exif {
                dpi: Some(300),
                orientation: Some(6),
            };
            assert_eq!(exif(&payload), expected);
            for end in 0..payload.len() {
                let _ = exif(&payload[..end]);
            }
        }
        assert_eq!(exif(b"Exif\0\0XX*\0"), Exif::default());
        assert_eq!(exif(b"Exif"), Exif::default());
        assert_eq!(exif(b"Exix\0\0II*\0\x08\0\0\0"), Exif::default());
    }

    #[test]
    fn photoshop_resolution_info() {
        let payload = photoshop(&[ps_dpi(150.0)]);
        assert_eq!(photoshop_dpi(&payload), Some(150));
        for end in 0..payload.len() {
            let _ = photoshop_dpi(&payload[..end]);
        }
        assert_eq!(photoshop_dpi(b"Photoshop 2.5\0"), None);
    }

    // ---------------------------------------------------------- section 7.3

    #[test]
    fn exif_tag_types_duplicates_and_bounds() {
        let orientation = |entries: &[Entry]| exif(&exif_payload(false, entries)).orientation;
        assert_eq!(orientation(&[long(TAG_ORIENTATION, 6)]), Some(6));
        assert_eq!(orientation(&[short(TAG_ORIENTATION, 0)]), None);
        assert_eq!(orientation(&[short(TAG_ORIENTATION, 9)]), None);
        assert_eq!(orientation(&[rational(TAG_ORIENTATION, 6, 1)]), None);
        let dpi = |entries: &[Entry]| exif(&exif_payload(true, entries)).dpi;
        let cm = [
            rational(TAG_X_RESOLUTION, 118, 1),
            long(TAG_RESOLUTION_UNIT, 3),
        ];
        assert_eq!(dpi(&cm), Some(300));
        let none = [
            rational(TAG_X_RESOLUTION, 300, 1),
            long(TAG_RESOLUTION_UNIT, 1),
        ];
        assert_eq!(dpi(&none), None);
        let negative = [
            srational(TAG_X_RESOLUTION, -300, 1),
            short(TAG_RESOLUTION_UNIT, 2),
        ];
        assert_eq!(dpi(&negative), None);
        assert_eq!(dpi(&[srational(TAG_X_RESOLUTION, 300, 1)]), Some(300));
        assert_eq!(dpi(&[srational(TAG_X_RESOLUTION, -300, -1)]), Some(300));
        let y = [
            rational(TAG_X_RESOLUTION, 300, 0),
            rational(TAG_Y_RESOLUTION, 200, 1),
        ];
        assert_eq!(dpi(&y), Some(200));
        let twice = [
            rational(TAG_X_RESOLUTION, 300, 1),
            rational(TAG_X_RESOLUTION, 150, 1),
        ];
        assert_eq!(dpi(&twice), Some(300));
        // A huge entry count in a 40-byte payload.
        let mut payload = b"Exif\0\0II*\0\x08\0\0\0".to_vec();
        payload.extend_from_slice(&65_535u16.to_le_bytes());
        payload.resize(40, 0);
        assert_eq!(exif(&payload), Exif::default());
        // An IFD offset far past the end.
        let mut payload = b"Exif\0\0II*\0".to_vec();
        payload.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        payload.extend_from_slice(&[0; 16]);
        assert_eq!(exif(&payload), Exif::default());
        // A value offset past the end keeps the orientation.
        let mut payload = exif_payload(
            false,
            &[
                short(TAG_ORIENTATION, 6),
                rational(TAG_X_RESOLUTION, 300, 1),
            ],
        );
        payload.truncate(payload.len() - 8);
        let expected = Exif {
            dpi: None,
            orientation: Some(6),
        };
        assert_eq!(exif(&payload), expected);
        // Several SHORT values stored at an offset: the first is read.
        let entry = Entry {
            tag: TAG_ORIENTATION,
            kind: TYPE_SHORT,
            count: 3,
            values: vec![(2, 3), (2, 1), (2, 1)],
        };
        assert_eq!(exif(&exif_payload(true, &[entry])).orientation, Some(3));
        // A count of zero declares nothing.
        let entry = Entry {
            tag: TAG_ORIENTATION,
            kind: TYPE_SHORT,
            count: 0,
            values: vec![(2, 6)],
        };
        assert_eq!(exif(&exif_payload(true, &[entry])).orientation, None);
    }

    #[test]
    fn photoshop_scan_limits() {
        let mut huge = b"8BIM\x04\x00\0\0".to_vec();
        huge.extend_from_slice(&u32::MAX.to_be_bytes());
        huge.extend_from_slice(&[0; 8]);
        assert_eq!(photoshop_dpi(&photoshop(&[huge, ps_dpi(150.0)])), None);
        // A zero-length name takes two bytes.
        let empty = block(b"8BIM", 0x0400, b"", &[9, 9]);
        assert_eq!(empty.len(), 4 + 2 + 2 + 4 + 2);
        let payload = photoshop(&[empty, ps_dpi(150.0)]);
        assert_eq!(photoshop_dpi(&payload), Some(150));
        let odd = block(b"8BIM", 0x0400, b"", &[1, 2, 3]);
        let payload = photoshop(&[odd, ps_dpi(240.0)]);
        assert_eq!(photoshop_dpi(&payload), Some(240));
        let short_info = block(b"8BIM", RESOLUTION_INFO, b"", &[0, 100]);
        let payload = photoshop(&[short_info, ps_dpi(240.0)]);
        assert_eq!(photoshop_dpi(&payload), Some(240));
        // A negative fixed-point hRes is no resolution.
        let negative = block(b"8BIM", RESOLUTION_INFO, b"", &[0xFF, 0, 0, 0]);
        assert_eq!(photoshop_dpi(&photoshop(&[negative])), None);
    }

    #[test]
    fn jfif_cut_short() {
        assert_eq!(jfif_dpi(&jfif(1, 300)[..8]), None);
        assert_eq!(jfif_dpi(&jfif(1, 300)[..9]), None);
        assert_eq!(jfif_dpi(&jfif(1, 300)[..10]), Some(300));
    }

    #[test]
    fn png_extremes() {
        assert_eq!(png_dpi(u32::MAX, true), Some(109_092_169));
        assert_eq!(png_dpi(0, true), None);
        assert_eq!(png_dpi(19, true), None, "0.48 dpi");
        assert_eq!(png_dpi(236, true), None, "6 dpi is implausible");
        assert_eq!(png_dpi(394, true), Some(10));
    }

    #[test]
    fn jpeg_precedence() {
        assert_eq!(jpeg_dpi(None, None, None), 0);
        assert_eq!(jpeg_dpi(None, Some(200), Some(150)), 200);
        assert_eq!(jpeg_dpi(Some(300), Some(200), Some(150)), 300);
    }

    #[test]
    fn rounding_is_exact_half_away_from_zero() {
        assert_eq!(dpi_of(401, 2), Some(201));
        assert_eq!(dpi_of(399, 2), Some(200));
        assert_eq!(dpi_of(19, 2), Some(10));
        assert_eq!(dpi_of(18, 2), None);
        // 25 per cm is exactly 63.5 per inch.
        assert_eq!(dpi_of(25 * CM_NUMERATOR, CM_DENOMINATOR), Some(64));
        assert_eq!(dpi_of(1, 0), None);
        assert_eq!(dpi_of(u128::from(u32::MAX) + 1, 1), None);
        assert_eq!(dpi_of(u128::MAX, 1), None);
    }

    // ---------------------------------------------------------- end to end

    /// A 1 × 1 greyscale JPEG's headers with `segments` (marker, payload)
    /// in front of the frame.
    fn jpeg_file(segments: &[(u8, &[u8])]) -> Vec<u8> {
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

    /// (dpi, orientation) through the JPEG marker walk.
    fn walk(segments: &[(u8, &[u8])]) -> (u32, u8) {
        super::super::jpeg_info(&jpeg_file(segments))
            .expect("headers")
            .resolution()
    }

    #[test]
    fn jpeg_segments_through_the_marker_walk() {
        assert_eq!(walk(&[]), (0, 1));
        let unit1 = exif_xy(false, 300, 1, Some(1));
        let jfif150 = jfif(1, 150);
        assert_eq!(walk(&[(0xE1, &unit1), (0xE0, &jfif150)]), (150, 1));
        assert_eq!(walk(&[(0xE0, &jfif(2, 118))]), (300, 1));
        // The first JFIF segment is the one read, even when it declares
        // nothing.
        assert_eq!(walk(&[(0xE0, &jfif(0, 1)), (0xE0, &jfif150)]), (0, 1));
        // The first EXIF segment is the one read; XMP in APP1 is skipped.
        let xmp = b"http://ns.adobe.com/xap/1.0/\0<x/>".as_slice();
        let turned = exif_payload(true, &[short(TAG_ORIENTATION, 6)]);
        let upright = exif_payload(true, &[short(TAG_ORIENTATION, 1)]);
        assert_eq!(
            walk(&[(0xE1, xmp), (0xE1, &turned), (0xE1, &upright)]),
            (0, 6)
        );
        // The first APP13 that yields a resolution.
        let none = photoshop(&[block(b"8BIM", 0x0400, b"", &[0; 4])]);
        let ps200 = photoshop(&[ps_dpi(200.0)]);
        let ps300 = photoshop(&[ps_dpi(300.0)]);
        assert_eq!(
            walk(&[
                (0xED, &none),
                (0xED, &ps200),
                (0xED, &ps300),
                (0xE0, &jfif150)
            ]),
            (200, 1)
        );
        // EXIF wins over both, wherever it stands.
        let exif300 = exif_xy(true, 300, 1, Some(2));
        assert_eq!(
            walk(&[(0xE0, &jfif150), (0xED, &ps200), (0xE1, &exif300)]),
            (300, 1)
        );
        // A JFXX extension is not a JFIF header.
        let jfxx = b"JFXX\0\x10\x01\x01\x01\x2C".as_slice();
        assert_eq!(walk(&[(0xE0, jfxx), (0xE0, &jfif150)]), (150, 1));
    }

    #[test]
    fn png_phys_through_the_decoder() {
        let dpi = |phys: Option<(u32, png::Unit)>| {
            let mut out = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut out, 1, 1);
                encoder.set_color(png::ColorType::Grayscale);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.set_pixel_dims(phys.map(|(ppu, unit)| png::PixelDimensions {
                    xppu: ppu,
                    yppu: ppu / 2,
                    unit,
                }));
                let mut writer = encoder.write_header().expect("header");
                writer.write_image_data(&[128]).expect("data");
            }
            let decoder = png::Decoder::new(std::io::Cursor::new(out));
            let reader = decoder.read_info().expect("info");
            super::super::png_resolution(reader.info())
        };
        assert_eq!(dpi(None), 0);
        assert_eq!(dpi(Some((11811, png::Unit::Meter))), 300);
        assert_eq!(dpi(Some((5906, png::Unit::Meter))), 150);
        assert_eq!(dpi(Some((11811, png::Unit::Unspecified))), 0);
        assert_eq!(dpi(Some((236, png::Unit::Meter))), 0);
    }
}
