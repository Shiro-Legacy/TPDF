// SPDX-License-Identifier: MIT
//! Twelve filter vectors (spec 02 section 6.2): four valid inputs, four
//! malformed ones and four without an end-of-data marker, with this engine's
//! expected results from spec 02 sections 3.3 to 3.6.
use tandem_pdf_engine::stream::{
    Filter, FilterSpec, StreamErrorKind, StreamLimits, decode_filter_specs_lenient, decode_filters,
};

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

/// A zlib stream holding one stored DEFLATE block of the bytes 0..=255.
fn stored_binary() -> Vec<u8> {
    let mut out = vec![0x78, 0x9C, 0x01, 0x00, 0x01, 0xFF, 0xFE];
    out.extend(0..=255u8);
    // Adler-32 of 0..=255 (RFC 1950), big-endian.
    out.extend_from_slice(&[0xAD, 0xF6, 0x7F, 0x81]);
    out
}

fn strict(data: &[u8], filter: Filter) -> Result<Vec<u8>, StreamErrorKind> {
    decode_filters(data, &[filter], StreamLimits::default()).map_err(|error| error.kind)
}

fn lenient(data: &[u8], filter: Filter) -> Result<Vec<u8>, StreamErrorKind> {
    decode_filter_specs_lenient(data, &[FilterSpec::from(filter)], StreamLimits::default())
        .map(|decoded| decoded.bytes)
        .map_err(|error| error.kind)
}

#[test]
fn valid_vectors_decode() {
    let binary: Vec<u8> = (0..=255).collect();
    for (filter, input, expected) in [
        (Filter::Flate, stored_binary(), binary),
        (Filter::AsciiHex, b"00ff 41>".to_vec(), hex("00ff41")),
        (
            Filter::Ascii85,
            b"87cURD]j7BEbo80~>".to_vec(),
            b"Hello world!".to_vec(),
        ),
        (Filter::RunLength, hex("02414243fe5a80"), b"ABCZZZ".to_vec()),
    ] {
        assert_eq!(strict(&input, filter), Ok(expected.clone()), "{filter:?}");
        assert_eq!(lenient(&input, filter), Ok(expected), "{filter:?} lenient");
    }
}

#[test]
fn malformed_vectors_are_refused() {
    for (filter, input, expected) in [
        (
            Filter::Flate,
            b"broken".as_slice(),
            StreamErrorKind::InvalidFlate,
        ),
        (Filter::AsciiHex, b"GG>", StreamErrorKind::InvalidByte(b'G')),
        (Filter::Ascii85, b"v~>", StreamErrorKind::InvalidByte(b'v')),
        (Filter::RunLength, b"\x03a", StreamErrorKind::TruncatedRun),
    ] {
        assert_eq!(strict(input, filter), Err(expected.clone()), "{filter:?}");
        // The lenient entry point relaxes only missing end markers and
        // Flate damage; these stay refused (Flate gives no output).
        match filter {
            Filter::Flate => assert_eq!(lenient(input, filter), Ok(Vec::new())),
            _ => assert_eq!(lenient(input, filter), Err(expected), "{filter:?}"),
        }
    }
}

#[test]
fn missing_end_markers_are_refused_strictly() {
    for (filter, input, expected) in [
        (
            Filter::Flate,
            hex("789c4b4c4a0600024d"),
            StreamErrorKind::TruncatedFlate,
        ),
        (
            Filter::AsciiHex,
            b"4142".to_vec(),
            StreamErrorKind::MissingEndMarker,
        ),
        (
            Filter::Ascii85,
            b"z".to_vec(),
            StreamErrorKind::MissingEndMarker,
        ),
        (
            Filter::RunLength,
            b"\x00a".to_vec(),
            StreamErrorKind::MissingEndMarker,
        ),
    ] {
        assert_eq!(strict(&input, filter), Err(expected), "{filter:?}");
    }
    // A zlib stream cut before its checksum keeps its data when read
    // leniently (spec 02 section 3.6).
    assert_eq!(
        lenient(&hex("789c4b4c4a0600024d"), Filter::Flate),
        Ok(b"abc".to_vec())
    );
}

/// Owner decision S4: the lenient entry point accepts a missing end marker
/// in ASCIIHex, ASCII85 and RunLength.
#[test]
fn missing_end_markers_are_read_leniently() {
    for (filter, input, expected) in [
        (Filter::AsciiHex, b"4142".as_slice(), b"AB".as_slice()),
        (Filter::Ascii85, b"z", b"\0\0\0\0"),
        (Filter::RunLength, b"\x00a", b"a"),
    ] {
        assert_eq!(lenient(input, filter), Ok(expected.to_vec()), "{filter:?}");
    }
}
