// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acceptance cases for spec 02 (stream filters and predictors), section 6.3,
//! and the owner decisions S4 and S6. Every input is built here, with
//! encoders written from ISO 32000-2 section 7.4.
use std::collections::HashMap;

use miniz_oxide::deflate::compress_to_vec_zlib;
use tandem_pdf_engine::stream::{
    Filter, FilterSpec, PredictorParams, StreamErrorKind, StreamLimits, decode_filter_specs,
    decode_filter_specs_lenient, decode_filters,
};

fn spec(filter: Filter, predictor: u8, colors: usize, bpc: u8, columns: usize) -> FilterSpec {
    FilterSpec {
        filter,
        predictor: PredictorParams {
            predictor,
            colors,
            bits_per_component: bpc,
            columns,
        },
    }
}

fn strict(data: &[u8], filters: &[FilterSpec]) -> Result<Vec<u8>, StreamErrorKind> {
    decode_filter_specs(data, filters, StreamLimits::default()).map_err(|error| error.kind)
}

fn lenient(data: &[u8], filters: &[FilterSpec]) -> Result<Vec<u8>, StreamErrorKind> {
    decode_filter_specs_lenient(data, filters, StreamLimits::default())
        .map(|decoded| decoded.bytes)
        .map_err(|error| error.kind)
}

fn one(data: &[u8], filter: Filter) -> Result<Vec<u8>, StreamErrorKind> {
    decode_filters(data, &[filter], StreamLimits::default()).map_err(|error| error.kind)
}

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(u8::is_ascii_hexdigit).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn zlib(data: &[u8]) -> Vec<u8> {
    compress_to_vec_zlib(data, 6)
}

/// Packs (code, width) pairs most significant bit first, zero padded.
fn pack(codes: &[(u16, u8)]) -> Vec<u8> {
    let mut bits = Vec::new();
    for &(code, width) in codes {
        bits.extend((0..width).rev().map(|bit| ((code >> bit) & 1) as u8));
    }
    while bits.len() % 8 != 0 {
        bits.push(0);
    }
    bits.chunks_exact(8)
        .map(|chunk| chunk.iter().fold(0, |byte, bit| byte * 2 + bit))
        .collect()
}

/// An LZW encoder written from ISO 32000-2 section 7.4.4.2: it starts with a
/// clear code, widens codes when the decoder's next free entry needs it, and
/// emits a clear code as soon as the decoder's table is full.
fn lzw_encode(data: &[u8], early_change: bool) -> Vec<(u16, u8)> {
    struct Out {
        codes: Vec<(u16, u8)>,
        decoder_next: u16,
        first: bool,
        early: u16,
    }
    impl Out {
        fn emit(&mut self, code: u16) {
            let next = self.decoder_next + self.early;
            let width = if next < 512 {
                9
            } else if next < 1024 {
                10
            } else if next < 2048 {
                11
            } else {
                12
            };
            self.codes.push((code, width));
            if code == 256 {
                self.decoder_next = 258;
                self.first = true;
            } else if code != 257 {
                if self.first {
                    self.first = false;
                } else {
                    self.decoder_next = (self.decoder_next + 1).min(4096);
                }
            }
        }
    }
    let mut out = Out {
        codes: Vec::new(),
        decoder_next: 258,
        first: true,
        early: u16::from(early_change),
    };
    let mut table: HashMap<(u16, u8), u16> = HashMap::new();
    let mut next = 258u16;
    out.emit(256);
    let mut current: Option<u16> = None;
    for &byte in data {
        match current {
            None => current = Some(u16::from(byte)),
            Some(prefix) => {
                if let Some(&code) = table.get(&(prefix, byte)) {
                    current = Some(code);
                } else {
                    out.emit(prefix);
                    if next < 4096 {
                        table.insert((prefix, byte), next);
                        next += 1;
                    } else {
                        out.emit(256);
                        table.clear();
                        next = 258;
                    }
                    current = Some(u16::from(byte));
                }
            }
        }
    }
    if let Some(prefix) = current {
        out.emit(prefix);
    }
    out.emit(257);
    out.codes
}

/// 6.3 case 1: every row of the section 4 table, as this engine reads it.
#[test]
fn recorded_differences_read_as_the_engine_reads_them() {
    use StreamErrorKind::*;
    let hello = b"87cURD]j7BEbo80".to_vec();
    let cat = |a: &[u8], b: &[u8]| [a, b].concat();
    for (filter, input, expected) in [
        (Filter::AsciiHex, b"414".to_vec(), MissingEndMarker),
        (Filter::AsciiHex, b"41>ZZ".to_vec(), TrailingData),
        (Filter::AsciiHex, b"4G>".to_vec(), InvalidByte(b'G')),
        (Filter::AsciiHex, b"41\x0b42>".to_vec(), InvalidByte(0x0b)),
        (Filter::Ascii85, hello.clone(), MissingEndMarker),
        (Filter::Ascii85, cat(&hello, b"~"), InvalidEndMarker),
        (Filter::Ascii85, cat(&hello, b"~>xyz"), TrailingData),
        (Filter::Ascii85, b"!~>".to_vec(), InvalidAscii85Group),
        (Filter::Ascii85, b"s8W-\"~>".to_vec(), Ascii85Overflow),
        (Filter::Ascii85, b"uuuuu~>".to_vec(), Ascii85Overflow),
        (Filter::RunLength, hex("02616263"), MissingEndMarker),
        (Filter::RunLength, hex("0041800042"), TrailingData),
        (Filter::RunLength, hex("0261"), TruncatedRun),
    ] {
        assert_eq!(one(&input, filter), Err(expected), "{filter:?} {input:?}");
    }
    assert_eq!(
        one(&cat(&hello, b"~>"), Filter::Ascii85),
        Ok(b"Hello world!".to_vec())
    );
    // The PostScript `<~` prefix is refused. Section 4 names InvalidByte('<'),
    // but `<` is a base-85 digit under section 3.4, which makes the `~` after
    // it an InvalidEndMarker; either refusal is accepted.
    assert!(matches!(
        one(&cat(b"<~", &cat(&hello, b"~>")), Filter::Ascii85),
        Err(InvalidByte(b'<') | InvalidEndMarker)
    ));

    for early_change in [false, true] {
        let lzw = Filter::Lzw { early_change };
        let w = |codes: &[u16]| pack(&codes.iter().map(|&c| (c, 9)).collect::<Vec<_>>());
        for (input, expected) in [
            (w(&[65, 66, 257]), MissingLzwClear),
            (w(&[256, 65, 66]), TruncatedLzw),
            (w(&[256, 258, 257]), InvalidLzwCode(258)),
            (cat(&w(&[256, 65, 257]), b"\0"), TrailingData),
            (w(&[256, 65, 300, 257]), InvalidLzwCode(300)),
        ] {
            assert_eq!(one(&input, lzw), Err(expected), "{early_change}");
        }
        // 256 65 257 is 27 bits; a set bit in the 5 padding bits.
        let mut padded = w(&[256, 65, 257]);
        *padded.last_mut().unwrap() |= 1;
        assert_eq!(one(&padded, lzw), Err(NonZeroLzwPadding));
    }

    let png = |predictor: u8| spec(Filter::Flate, predictor, 1, 8, 3);
    for tag in [5u8, 255] {
        let rows = zlib(&[0, 1, 2, 3, tag, 1, 1, 1]);
        assert_eq!(strict(&rows, &[png(12)]), Err(InvalidPngFilter(tag)));
    }
    assert_eq!(
        strict(&zlib(&hex("00 010203 01 01")), &[png(12)]),
        Err(TruncatedPredictorRow)
    );
    assert_eq!(
        strict(
            &zlib(&hex("010101010202")),
            &[spec(Filter::Flate, 2, 1, 8, 4)]
        ),
        Err(TruncatedPredictorRow)
    );
    for predictor in [0u8, 3, 9, 16, 255] {
        assert_eq!(
            strict(&zlib(b"abc"), &[png(predictor)]),
            Err(UnsupportedPredictor(predictor))
        );
    }
    for bpc in [0u8, 3, 7, 32] {
        assert_eq!(
            strict(&zlib(b"abc"), &[spec(Filter::Flate, 12, 1, bpc, 3)]),
            Err(UnsupportedBitsPerComponent(bpc))
        );
    }
    for (colors, columns) in [(1, 0), (33, 1), (64, 1), (0, 1)] {
        assert!(
            matches!(
                strict(
                    &zlib(b"abc"),
                    &[spec(Filter::Flate, 12, colors, 8, columns)]
                ),
                Err(InvalidPredictor(_))
            ),
            "{colors} {columns}"
        );
    }
    // 32 colours are accepted.
    let row = [&[0u8][..], &[7u8; 32]].concat();
    assert_eq!(
        strict(&zlib(&row), &[spec(Filter::Flate, 12, 32, 8, 1)]),
        Ok(vec![7; 32])
    );
}

/// 6.3 case 2: the predictor vectors of sections 3.7.2 and 3.7.3.
#[test]
fn predictor_vectors_decode_exactly() {
    let tiff = |colors, bpc, columns, rows: &str, expected: &str| {
        assert_eq!(
            strict(
                &zlib(&hex(rows)),
                &[spec(Filter::Flate, 2, colors, bpc, columns)]
            ),
            Ok(hex(expected)),
            "TIFF {colors} {bpc} {columns} {rows}"
        );
    };
    tiff(1, 8, 4, "01010101 02020202", "01020304 02040608");
    tiff(
        3,
        8,
        2,
        "0a141e 010101 050505 ff0000",
        "0a141e0b151f 050505040505",
    );
    tiff(1, 16, 2, "00ff0002 ffff0001", "00ff0101 ffff0000");
    tiff(1, 1, 5, "a8 ff", "c8 a8");
    tiff(1, 2, 3, "57 ff", "6c e4");
    tiff(1, 4, 3, "111f 222f", "1230 2460");
    tiff(2, 4, 1, "12 34", "12 34");

    let five_rows = "00 010203 01 010101 02 010101 03 010101 04 010101";
    for predictor in 10..=15u8 {
        let png = |colors, bpc, columns, rows: &str, expected: &str| {
            assert_eq!(
                strict(
                    &zlib(&hex(rows)),
                    &[spec(Filter::Flate, predictor, colors, bpc, columns)]
                ),
                Ok(hex(expected)),
                "PNG {predictor} {colors} {bpc} {columns} {rows}"
            );
        };
        png(1, 8, 3, five_rows, "010203 010203 020304 020304 030405");
        png(1, 8, 3, "000a141e 04010203", "0a141e 0b1621");
        png(1, 8, 3, "000a141e 03010203", "0a141e 060f19");
        png(3, 8, 1, "01 0a141e", "0a141e");
        png(3, 8, 2, "01 0a141e010101", "0a141e0b151f");
        png(1, 4, 4, "01 1211", "1223");
        png(1, 16, 2, "01 0102 0101", "0102 0203");
    }
    // LZW then PNG predictor 12.
    let lzw = pack(
        &[256, 2, 1, 1, 1, 2, 1, 1, 1, 257]
            .iter()
            .map(|&c| (c, 9))
            .collect::<Vec<_>>(),
    );
    assert_eq!(
        strict(
            &lzw,
            &[spec(Filter::Lzw { early_change: true }, 12, 1, 8, 3)]
        ),
        Ok(hex("010101020202"))
    );
}

/// 6.3 case 3: LZW round trips for both EarlyChange values, including
/// tables that fill and continue after a clear code.
#[test]
fn lzw_round_trips_with_an_independent_encoder() {
    let mut state = 0x1234_5678_9ABC_DEF0_u64;
    let mut random = |alphabet: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % alphabet) as u8
    };
    let inputs: Vec<Vec<u8>> = vec![
        b"-----A---B".to_vec(),
        (0..20_000).map(|_| random(256)).collect(),
        (0..20_000).map(|_| b'a' + random(4)).collect(),
        (0..20_000).map(|i| (i % 251) as u8 ^ random(2)).collect(),
        vec![b'A'; 20_000],
        Vec::new(),
    ];
    for early_change in [false, true] {
        for (i, data) in inputs.iter().enumerate() {
            let codes = lzw_encode(data, early_change);
            let clears = codes.iter().filter(|(code, _)| *code == 256).count();
            if i == 1 {
                // Random bytes fill the table more than once.
                assert!(clears >= 3, "{clears}");
                assert!(codes.iter().any(|&(_, width)| width == 12));
            }
            let decoded = one(&pack(&codes), Filter::Lzw { early_change });
            assert_eq!(
                decoded.as_deref(),
                Ok(data.as_slice()),
                "{i} {early_change}"
            );
        }
    }
    // A table that fills exactly: after the clear the decoder accepts only
    // literals, and the stream continues.
    let data: Vec<u8> = (0..30_000).map(|_| random(256)).collect();
    let codes = lzw_encode(&data, true);
    let first_clear = codes.iter().skip(1).position(|(c, _)| *c == 256).unwrap() + 1;
    assert_eq!(codes[first_clear].1, 12);
    assert!(codes[first_clear + 1].0 < 256);
    assert_eq!(
        one(&pack(&codes), Filter::Lzw { early_change: true }).as_deref(),
        Ok(data.as_slice())
    );
    // A code after a full table that is not a clear code is refused.
    let mut broken = codes.clone();
    broken[first_clear].0 = 65;
    assert!(one(&pack(&broken), Filter::Lzw { early_change: true }).is_err());
}

/// 6.3 case 4: lenient Flate (section 3.6).
#[test]
fn lenient_flate_keeps_what_decodes() {
    let text: Vec<u8> = (0..20_000u32)
        .flat_map(|i| format!("{i} 0 0 rg\n").into_bytes())
        .collect();
    let flate = [FilterSpec::from(Filter::Flate)];
    let good = zlib(&text);
    // Trailing garbage is ignored.
    let trailing = [&good[..], b"\r\nendstream garbage"].concat();
    assert_eq!(lenient(&trailing, &flate), Ok(text.clone()));
    assert_eq!(
        strict(&trailing, &flate),
        Err(StreamErrorKind::TrailingData)
    );
    // A truncated stream keeps a prefix.
    let stored = compress_to_vec_zlib(&text, 0);
    let cut = &stored[..stored.len() * 3 / 5];
    let prefix = lenient(cut, &flate).unwrap();
    assert!(!prefix.is_empty() && prefix.len() < text.len());
    assert_eq!(prefix[..], text[..prefix.len()]);
    assert_eq!(strict(cut, &flate), Err(StreamErrorKind::TruncatedFlate));
    // A bad checksum keeps the full output.
    let mut bad_sum = good.clone();
    *bad_sum.last_mut().unwrap() ^= 0xFF;
    assert_eq!(lenient(&bad_sum, &flate), Ok(text.clone()));
    assert_eq!(
        strict(&bad_sum, &flate),
        Err(StreamErrorKind::FlateChecksumMismatch)
    );
    // Garbage gives nothing, without an error.
    assert_eq!(lenient(b"this is not zlib", &flate), Ok(Vec::new()));
    assert_eq!(lenient(b"", &flate), Ok(Vec::new()));
    // Limits are still errors.
    let limits = StreamLimits {
        max_decoded_bytes: 1000,
        ..StreamLimits::default()
    };
    let error = decode_filter_specs_lenient(&trailing, &flate, limits).unwrap_err();
    assert_eq!(
        error.kind,
        StreamErrorKind::DecodedOutputLimit { limit: 1000 }
    );
    let limits = StreamLimits {
        max_total_decoded_bytes: 1000,
        ..StreamLimits::default()
    };
    let error = decode_filter_specs_lenient(&bad_sum, &flate, limits).unwrap_err();
    assert_eq!(
        error.kind,
        StreamErrorKind::TotalOutputLimit { limit: 1000 }
    );
    // Other filters and predictors stay strict in the lenient entry point.
    assert_eq!(
        lenient(b"4G>", &[FilterSpec::from(Filter::AsciiHex)]),
        Err(StreamErrorKind::InvalidByte(b'G'))
    );
    assert_eq!(
        lenient(&zlib(&[9, 1, 2, 3]), &[spec(Filter::Flate, 12, 1, 8, 3)]),
        Err(StreamErrorKind::InvalidPngFilter(9))
    );
}

/// 6.3 case 5: `filter_index` and `offset` for an error in each stage of
/// `AsciiHex, Flate + PNG`.
#[test]
fn errors_name_the_stage_and_an_offset_into_its_input() {
    let hex_of = |bytes: &[u8]| -> Vec<u8> {
        let mut out: Vec<u8> = bytes
            .iter()
            .flat_map(|b| format!("{b:02X}").into_bytes())
            .collect();
        out.push(b'>');
        out
    };
    let chain = [
        FilterSpec::from(Filter::AsciiHex),
        spec(Filter::Flate, 12, 1, 8, 3),
    ];
    let rows = hex("00 010203 02 010101 00 040506");
    let good = hex_of(&zlib(&rows));
    let decoded = decode_filter_specs(&good, &chain, StreamLimits::default()).unwrap();
    assert_eq!(decoded, hex("010203 020304 040506"));

    // Stage 0: a bad hex digit, at its own offset.
    let mut bad_hex = good.clone();
    bad_hex[6] = b'G';
    let error = decode_filter_specs(&bad_hex, &chain, StreamLimits::default()).unwrap_err();
    assert_eq!(
        (error.filter_index, error.offset, error.kind),
        (Some(0), 6, StreamErrorKind::InvalidByte(b'G'))
    );
    // Stage 1: not zlib; the offset is within the hex-decoded bytes.
    let not_zlib = hex_of(b"not zlib at all");
    let error = decode_filter_specs(&not_zlib, &chain, StreamLimits::default()).unwrap_err();
    assert_eq!(error.filter_index, Some(1));
    assert_eq!(error.kind, StreamErrorKind::InvalidFlate);
    assert!(error.offset <= 15);
    // Stage 2, the predictor: index 1, offset of the bad row in the
    // inflated bytes.
    let bad_rows = hex("00 010203 07 010101 00 040506");
    let error = decode_filter_specs(&hex_of(&zlib(&bad_rows)), &chain, StreamLimits::default())
        .unwrap_err();
    assert_eq!(
        (error.filter_index, error.offset, error.kind.clone()),
        (Some(1), 4, StreamErrorKind::InvalidPngFilter(7))
    );
    assert!(
        error
            .to_string()
            .starts_with("PDF stream filter 1 at byte 4: ")
    );
    // A partial row: offset is the inflated length.
    let partial = hex("00 010203 00 01");
    let error =
        decode_filter_specs(&hex_of(&zlib(&partial)), &chain, StreamLimits::default()).unwrap_err();
    assert_eq!(
        (error.filter_index, error.offset, error.kind),
        (Some(1), 6, StreamErrorKind::TruncatedPredictorRow)
    );
}

/// Owner decision S6: a predictor other than 1 on a filter other than Flate
/// and LZW is refused; other parameters with predictor 1 are not.
#[test]
fn predictors_only_follow_flate_and_lzw() {
    for filter in [Filter::AsciiHex, Filter::Ascii85, Filter::RunLength] {
        let error = strict(b"", &[spec(filter, 12, 1, 8, 1)]).unwrap_err();
        assert!(
            matches!(error, StreamErrorKind::InvalidPredictor(_)),
            "{filter:?}"
        );
        let error = strict(b"", &[spec(filter, 2, 1, 8, 1)]).unwrap_err();
        assert!(
            matches!(error, StreamErrorKind::InvalidPredictor(_)),
            "{filter:?}"
        );
    }
}

/// Owner decision S6: only a predictor other than 1 makes parameters on a
/// filter other than Flate and LZW an error.
#[test]
fn predictor_one_parameters_are_accepted_on_any_filter() {
    for (filter, input, expected) in [
        (Filter::AsciiHex, b"41>".as_slice(), b"A".as_slice()),
        (Filter::Ascii85, b"5l~>", b"A"),
        (Filter::RunLength, b"\x00A\x80", b"A"),
    ] {
        assert_eq!(
            strict(input, &[spec(filter, 1, 3, 8, 7)]),
            Ok(expected.to_vec()),
            "{filter:?}"
        );
    }
}

/// Owner decision S6: a zlib header asking for a preset dictionary is
/// refused, since PDF defines none.
#[test]
fn a_preset_dictionary_is_refused() {
    // CMF 0x78, FLG with FDICT set and a valid check value, a dictionary id,
    // then ordinary DEFLATE data.
    let plain = zlib(b"abc");
    let mut preset = vec![0x78, 0xBB, 0x00, 0x00, 0x00, 0x01];
    assert_eq!((0x78u16 * 256 + 0xBB) % 31, 0);
    preset.extend_from_slice(&plain[2..]);
    let flate = [FilterSpec::from(Filter::Flate)];
    assert_eq!(strict(&preset, &flate), Err(StreamErrorKind::InvalidFlate));
    match lenient(&preset, &flate) {
        Ok(bytes) => assert!(bytes.is_empty(), "{bytes:?}"),
        Err(kind) => assert_eq!(kind, StreamErrorKind::InvalidFlate),
    }
}

/// Owner decision S4: the lenient entry point reads a stream whose end
/// marker is missing in ASCIIHex, ASCII85, RunLength and LZW; the strict
/// one still refuses it.
#[test]
fn lenient_decoding_accepts_a_missing_end_marker() {
    let data = b"Hello, lenient world! \x00\xff";
    let ahx: Vec<u8> = data
        .iter()
        .flat_map(|b| format!("{b:02x}").into_bytes())
        .collect();
    let mut a85 = Vec::new();
    for chunk in data.chunks(4) {
        let mut padded = [0u8; 4];
        padded[..chunk.len()].copy_from_slice(chunk);
        let mut value = u32::from_be_bytes(padded);
        let mut group = [b'!'; 5];
        for byte in group.iter_mut().rev() {
            *byte += (value % 85) as u8;
            value /= 85;
        }
        a85.extend_from_slice(&group[..chunk.len() + 1]);
    }
    let mut rl = vec![(data.len() - 1) as u8];
    rl.extend_from_slice(data);
    for early_change in [false, true] {
        let mut codes = lzw_encode(data, early_change);
        codes.pop();
        let lzw = pack(&codes);
        for (filter, input) in [
            (Filter::AsciiHex, ahx.clone()),
            (Filter::Ascii85, a85.clone()),
            (Filter::RunLength, rl.clone()),
            (Filter::Lzw { early_change }, lzw),
        ] {
            let specs = [FilterSpec::from(filter)];
            assert!(strict(&input, &specs).is_err(), "{filter:?}");
            assert_eq!(lenient(&input, &specs), Ok(data.to_vec()), "{filter:?}");
        }
    }
    // An odd final hex digit still takes an implied 0.
    assert_eq!(
        lenient(b"414", &[FilterSpec::from(Filter::AsciiHex)]),
        Ok(vec![0x41, 0x40])
    );
}
