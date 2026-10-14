use miniz_oxide::deflate::compress_to_vec_zlib;
use tandem_pdf_engine::stream::{Filter, StreamErrorKind, StreamLimits, decode_filters};

fn decode(data: &[u8], filter: Filter) -> Vec<u8> {
    decode_filters(data, &[filter], StreamLimits::default()).unwrap()
}
fn error(data: &[u8], filter: Filter) -> StreamErrorKind {
    decode_filters(data, &[filter], StreamLimits::default())
        .unwrap_err()
        .kind
}
fn hex_encode(bytes: &[u8]) -> Vec<u8> {
    let mut output = String::new();
    for byte in bytes {
        use std::fmt::Write;
        write!(output, "{byte:02x}").unwrap();
    }
    output.push('>');
    output.into_bytes()
}
fn ascii85_encode(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for chunk in bytes.chunks(4) {
        let mut padded = [0u8; 4];
        padded[..chunk.len()].copy_from_slice(chunk);
        let mut value = u32::from_be_bytes(padded);
        let mut encoded = [b'!'; 5];
        for byte in encoded.iter_mut().rev() {
            *byte += (value % 85) as u8;
            value /= 85;
        }
        output.extend_from_slice(&encoded[..chunk.len() + 1]);
    }
    output.extend_from_slice(b"~>");
    output
}
fn literal_run_encode(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for chunk in bytes.chunks(128) {
        output.push((chunk.len() - 1) as u8);
        output.extend_from_slice(chunk);
    }
    output.push(128);
    output
}

#[test]
fn identity_preserves_bytes_and_checks_budgets() {
    let bytes: Vec<u8> = (0..=255).collect();
    assert_eq!(
        decode_filters(&bytes, &[], StreamLimits::default()).unwrap(),
        bytes
    );
    assert!(
        decode_filters(
            b"",
            &[],
            StreamLimits {
                max_encoded_bytes: 0,
                max_decoded_bytes: 0,
                max_filters: 0,
                max_total_decoded_bytes: 0
            }
        )
        .unwrap()
        .is_empty()
    );
    let mut limits = StreamLimits {
        max_encoded_bytes: 2,
        ..StreamLimits::default()
    };
    assert_eq!(
        decode_filters(b"abc", &[], limits).unwrap_err().kind,
        StreamErrorKind::EncodedInputLimit { limit: 2 }
    );
    limits.max_encoded_bytes = 3;
    limits.max_decoded_bytes = 2;
    assert_eq!(
        decode_filters(b"abc", &[], limits).unwrap_err().kind,
        StreamErrorKind::DecodedOutputLimit { limit: 2 }
    );
    limits.max_decoded_bytes = 3;
    limits.max_total_decoded_bytes = 2;
    assert_eq!(
        decode_filters(b"abc", &[], limits).unwrap_err().kind,
        StreamErrorKind::TotalOutputLimit { limit: 2 }
    );
}

#[test]
fn ascii_hex_known_vectors_and_odd_padding() {
    assert_eq!(
        decode(b"61 62\0\t\n\x0c\r63 64 65 66>", Filter::AsciiHex),
        b"abcdef"
    );
    assert_eq!(decode(b"fF0>", Filter::AsciiHex), [255, 0]);
    assert_eq!(decode(b">\r\n\0", Filter::AsciiHex), b"");
    assert_eq!(decode(b"A>", Filter::AsciiHex), [160]);
}

#[test]
fn ascii_hex_rejects_bad_bytes_missing_markers_and_trailing_data() {
    for data in [b"".as_slice(), b"01", b"0", b"01 \t"] {
        assert_eq!(
            error(data, Filter::AsciiHex),
            StreamErrorKind::MissingEndMarker
        );
    }
    for byte in [b'G', b'%', 8, 11, 127, 255] {
        assert_eq!(
            error(&[b'0', byte, b'>'], Filter::AsciiHex),
            StreamErrorKind::InvalidByte(byte)
        );
    }
    assert_eq!(
        error(b"00> 00", Filter::AsciiHex),
        StreamErrorKind::TrailingData
    );
    assert_eq!(
        error(b"00>>", Filter::AsciiHex),
        StreamErrorKind::TrailingData
    );
}

#[test]
fn ascii85_known_vectors_and_short_groups() {
    assert_eq!(
        decode(b"87cURD_*#TDfTZ)+T~>", Filter::Ascii85),
        b"Hello, world!"
    );
    assert_eq!(
        decode(b"9jqo^BlbD-BleB1DJ+*+F(f,q~>", Filter::Ascii85),
        b"Man is distinguished"
    );
    assert_eq!(decode(b"z!!!!!~>", Filter::Ascii85), [0; 8]);
    assert_eq!(decode(b"rr~>", Filter::Ascii85), [255]);
    assert_eq!(decode(b"s8N~>", Filter::Ascii85), [255; 2]);
    assert_eq!(decode(b"s8W*~>", Filter::Ascii85), [255; 3]);
    assert_eq!(decode(b"s8W-!~>", Filter::Ascii85), [255; 4]);
    assert_eq!(decode(b" \0\t\n\x0c\r~> \n", Filter::Ascii85), b"");
    assert_eq!(decode(b"! ! ! ! !~>", Filter::Ascii85), [0; 4]);
}

#[test]
fn ascii85_rejects_overflow_bad_groups_and_non_pdf_variants() {
    for data in [
        b"uuuuu~>".as_slice(),
        b"uu~>",
        b"uuu~>",
        b"uuuu~>",
        b"s8W-\"~>",
    ] {
        assert_eq!(
            error(data, Filter::Ascii85),
            StreamErrorKind::Ascii85Overflow
        );
    }
    for data in [b"!~>".as_slice(), b"!z~>"] {
        assert_eq!(
            error(data, Filter::Ascii85),
            StreamErrorKind::InvalidAscii85Group
        );
    }
    for data in [b"~".as_slice(), b"~ ", b"~x", b"~\n>"] {
        assert_eq!(
            error(data, Filter::Ascii85),
            StreamErrorKind::InvalidEndMarker
        );
    }
    for data in [b"".as_slice(), b"!!!!!", b"z"] {
        assert_eq!(
            error(data, Filter::Ascii85),
            StreamErrorKind::MissingEndMarker
        );
    }
    for byte in [b'v', b'y', 8, 11, 127, 255] {
        assert_eq!(
            error(&[byte], Filter::Ascii85),
            StreamErrorKind::InvalidByte(byte)
        );
    }
    assert!(decode_filters(b"<~z~>", &[Filter::Ascii85], StreamLimits::default()).is_err());
    assert_eq!(
        error(b"z~>z", Filter::Ascii85),
        StreamErrorKind::TrailingData
    );
}

#[test]
fn run_length_known_vector_and_packet_boundaries() {
    assert_eq!(
        decode(
            &[2, b'A', b'B', b'C', 254, b'Z', 0, b'!', 128],
            Filter::RunLength
        ),
        b"ABCZZZ!"
    );
    assert_eq!(decode(&[255, b'A', 128], Filter::RunLength), b"AA");
    assert_eq!(
        decode(&[129, b'Z', 128], Filter::RunLength),
        vec![b'Z'; 128]
    );
    assert_eq!(decode(&[128, 13, 10, 0], Filter::RunLength), b"");
    let mut largest_literal = vec![127];
    largest_literal.extend(0..128);
    largest_literal.push(128);
    assert_eq!(
        decode(&largest_literal, Filter::RunLength),
        (0..128).collect::<Vec<u8>>()
    );
    assert_eq!(decode(&[0, 128, 128], Filter::RunLength), [128]);
}

#[test]
fn run_length_rejects_truncations_and_trailing_payload() {
    for data in [b"".as_slice(), &[0, b'a'], &[255, b'a']] {
        assert_eq!(
            error(data, Filter::RunLength),
            StreamErrorKind::MissingEndMarker
        );
    }
    for data in [&[0][..], &[2, b'a', b'b'], &[255]] {
        assert_eq!(
            error(data, Filter::RunLength),
            StreamErrorKind::TruncatedRun
        );
    }
    assert_eq!(
        error(&[128, 1], Filter::RunLength),
        StreamErrorKind::TrailingData
    );
}

#[test]
fn flate_known_zlib_vector_and_all_binary_values() {
    // Published miniz_oxide zlib test vector; independent of test encoder.
    let vector = [
        120, 156, 243, 72, 205, 201, 201, 215, 81, 168, 202, 201, 76, 82, 4, 0, 27, 101, 4, 19,
    ];
    assert_eq!(decode(&vector, Filter::Flate), b"Hello, zlib!");
    let bytes: Vec<u8> = (0..=255).cycle().take(256 * 256).collect();
    for level in [0, 1, 6, 10] {
        assert_eq!(
            decode(&compress_to_vec_zlib(&bytes, level), Filter::Flate),
            bytes
        );
    }
    assert_eq!(decode(&[120, 156, 3, 0, 0, 0, 0, 1], Filter::Flate), b"");
}

#[test]
fn all_filters_preserve_every_byte_and_short_binary_tails() {
    let bytes: Vec<u8> = (0..=255).collect();
    for length in 0..=bytes.len() {
        let data = &bytes[..length];
        assert_eq!(decode(&hex_encode(data), Filter::AsciiHex), data);
        assert_eq!(decode(&ascii85_encode(data), Filter::Ascii85), data);
        assert_eq!(decode(&literal_run_encode(data), Filter::RunLength), data);
    }
}

#[test]
fn flate_rejects_truncation_checksum_failure_and_trailing_bytes() {
    let encoded = compress_to_vec_zlib(b"a complete zlib stream", 6);
    for length in 0..encoded.len() {
        assert!(
            decode_filters(
                &encoded[..length],
                &[Filter::Flate],
                StreamLimits::default()
            )
            .is_err(),
            "prefix {length}"
        );
    }
    let mut corrupt = encoded.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert_eq!(
        error(&corrupt, Filter::Flate),
        StreamErrorKind::FlateChecksumMismatch
    );
    for trailing in [b"\n".as_slice(), b"other", &encoded] {
        let mut data = encoded.clone();
        data.extend_from_slice(trailing);
        assert_eq!(error(&data, Filter::Flate), StreamErrorKind::TrailingData);
    }
    assert_eq!(
        error(b"not zlib", Filter::Flate),
        StreamErrorKind::InvalidFlate
    );
}

#[test]
fn zero_and_exact_output_limits_for_every_filter() {
    let examples = [
        (
            Filter::Flate,
            compress_to_vec_zlib(b"abc", 6),
            compress_to_vec_zlib(b"", 6),
        ),
        (Filter::AsciiHex, b"616263>".to_vec(), b">".to_vec()),
        (Filter::Ascii85, b"@:E^~>".to_vec(), b"~>".to_vec()),
        (Filter::RunLength, vec![2, b'a', b'b', b'c', 128], vec![128]),
    ];
    for (filter, encoded, empty) in examples {
        let mut limits = StreamLimits {
            max_decoded_bytes: 0,
            ..StreamLimits::default()
        };
        assert_eq!(decode_filters(&empty, &[filter], limits).unwrap(), b"");
        assert_eq!(
            decode_filters(&encoded, &[filter], limits)
                .unwrap_err()
                .kind,
            StreamErrorKind::DecodedOutputLimit { limit: 0 }
        );
        limits.max_decoded_bytes = 2;
        assert_eq!(
            decode_filters(&encoded, &[filter], limits)
                .unwrap_err()
                .kind,
            StreamErrorKind::DecodedOutputLimit { limit: 2 }
        );
        limits.max_decoded_bytes = 3;
        assert_eq!(decode_filters(&encoded, &[filter], limits).unwrap(), b"abc");
        limits.max_encoded_bytes = encoded.len();
        assert_eq!(decode_filters(&encoded, &[filter], limits).unwrap(), b"abc");
        limits.max_encoded_bytes -= 1;
        assert_eq!(
            decode_filters(&encoded, &[filter], limits)
                .unwrap_err()
                .kind,
            StreamErrorKind::EncodedInputLimit {
                limit: encoded.len() - 1
            }
        );
    }
}

#[test]
fn flate_exact_output_limits_across_buffer_growth_boundaries() {
    for length in [
        1, 2, 3, 63, 64, 65, 127, 128, 129, 4095, 4096, 4097, 8192, 16_385,
    ] {
        let data = vec![b'x'; length];
        let encoded = compress_to_vec_zlib(&data, 6);
        let mut limits = StreamLimits {
            max_decoded_bytes: length,
            ..StreamLimits::default()
        };
        assert_eq!(
            decode_filters(&encoded, &[Filter::Flate], limits).unwrap(),
            data,
            "length={length}"
        );
        limits.max_decoded_bytes -= 1;
        assert!(matches!(
            decode_filters(&encoded, &[Filter::Flate], limits)
                .unwrap_err()
                .kind,
            StreamErrorKind::DecodedOutputLimit { .. }
        ));
    }
}

#[test]
fn filter_order_and_cumulative_stage_budget() {
    let plain = b"stream content for a multi-filter chain";
    let compressed = compress_to_vec_zlib(plain, 6);
    let ascii = hex_encode(&compressed);
    let chain = [Filter::AsciiHex, Filter::Flate];
    let total = compressed.len() + plain.len();
    let mut limits = StreamLimits {
        max_filters: 2,
        max_total_decoded_bytes: total,
        ..StreamLimits::default()
    };
    assert_eq!(decode_filters(&ascii, &chain, limits).unwrap(), plain);
    limits.max_total_decoded_bytes -= 1;
    let failure = decode_filters(&ascii, &chain, limits).unwrap_err();
    assert_eq!(
        failure.kind,
        StreamErrorKind::TotalOutputLimit { limit: total - 1 }
    );
    assert_eq!(failure.filter_index, Some(1));
    limits.max_filters = 1;
    assert_eq!(
        decode_filters(&ascii, &chain, limits).unwrap_err().kind,
        StreamErrorKind::FilterCountLimit { limit: 1 }
    );
    assert!(
        decode_filters(
            &ascii,
            &[Filter::Flate, Filter::AsciiHex],
            StreamLimits::default()
        )
        .is_err()
    );
}

#[test]
fn zero_filter_count_still_allows_identity() {
    let limits = StreamLimits {
        max_filters: 0,
        ..StreamLimits::default()
    };
    assert_eq!(decode_filters(b"a", &[], limits).unwrap(), b"a");
    assert_eq!(
        decode_filters(b">", &[Filter::AsciiHex], limits)
            .unwrap_err()
            .kind,
        StreamErrorKind::FilterCountLimit { limit: 0 }
    );
}

#[test]
fn bomb_like_expansion_stops_at_stage_and_total_caps() {
    let compressed = compress_to_vec_zlib(&vec![0; 1_000_000], 9);
    let limits = StreamLimits {
        max_decoded_bytes: 1024,
        max_total_decoded_bytes: 2048,
        ..StreamLimits::default()
    };
    assert_eq!(
        decode_filters(&compressed, &[Filter::Flate], limits)
            .unwrap_err()
            .kind,
        StreamErrorKind::DecodedOutputLimit { limit: 1024 }
    );
    let repeated_runs: Vec<u8> = [129, 0].repeat(100);
    let mut runs = repeated_runs;
    runs.push(128);
    assert_eq!(
        decode_filters(&runs, &[Filter::RunLength], limits)
            .unwrap_err()
            .kind,
        StreamErrorKind::DecodedOutputLimit { limit: 1024 }
    );
    let mut zeros = vec![b'z'; 1000];
    zeros.extend_from_slice(b"~>");
    assert_eq!(
        decode_filters(&zeros, &[Filter::Ascii85], limits)
            .unwrap_err()
            .kind,
        StreamErrorKind::DecodedOutputLimit { limit: 1024 }
    );
    let twice = compress_to_vec_zlib(&compressed, 9);
    let total_limits = StreamLimits {
        max_decoded_bytes: 2_000_000,
        max_total_decoded_bytes: 2048,
        ..StreamLimits::default()
    };
    let failure =
        decode_filters(&twice, &[Filter::Flate, Filter::Flate], total_limits).unwrap_err();
    assert_eq!(
        failure.kind,
        StreamErrorKind::TotalOutputLimit { limit: 2048 }
    );
    assert_eq!(failure.filter_index, Some(1));
}

#[test]
fn errors_identify_filter_stage_and_input_offset() {
    let failure =
        decode_filters(b"00> 1", &[Filter::AsciiHex], StreamLimits::default()).unwrap_err();
    assert_eq!(failure.filter_index, Some(0));
    assert_eq!(failure.offset, 4);
    assert!(failure.to_string().contains("filter 0 at byte 4"));
    let nested = hex_encode(b"not zlib");
    let failure = decode_filters(
        &nested,
        &[Filter::AsciiHex, Filter::Flate],
        StreamLimits::default(),
    )
    .unwrap_err();
    assert_eq!(failure.filter_index, Some(1));
    assert!(failure.offset <= b"not zlib".len());
}

#[test]
fn deterministic_encoded_mutations_remain_bounded() {
    let original: Vec<u8> = (0..=255).cycle().take(4096).collect();
    let encodings = [
        (Filter::Flate, compress_to_vec_zlib(&original, 6)),
        (Filter::AsciiHex, hex_encode(&original)),
        (Filter::Ascii85, ascii85_encode(&original)),
        (Filter::RunLength, literal_run_encode(&original)),
        (
            Filter::Lzw {
                early_change: false,
            },
            literal_lzw(&original[..3000], false),
        ),
        (
            Filter::Lzw { early_change: true },
            literal_lzw(&original[..3000], true),
        ),
    ];
    let limits = StreamLimits {
        max_decoded_bytes: 8192,
        max_total_decoded_bytes: 8192,
        ..StreamLimits::default()
    };
    let mut state = 0x12345678u64;
    for (filter, encoded) in encodings {
        for _ in 0..512 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let offset = state as usize % encoded.len();
            let mut mutated = encoded.clone();
            mutated[offset] ^= ((state >> 32) as u8) | 1;
            match decode_filters(&mutated, &[filter], limits) {
                Ok(output) => assert!(output.len() <= limits.max_decoded_bytes),
                Err(error) => assert!(error.offset <= mutated.len()),
            }
        }
    }
}

fn predictor_decode(
    encoded_rows: &[u8],
    parameters: tandem_pdf_engine::stream::PredictorParams,
    limits: StreamLimits,
) -> Result<Vec<u8>, tandem_pdf_engine::stream::StreamError> {
    use tandem_pdf_engine::stream::{FilterSpec, decode_filter_specs};
    decode_filter_specs(
        &compress_to_vec_zlib(encoded_rows, 6),
        &[FilterSpec {
            filter: Filter::Flate,
            predictor: parameters,
        }],
        limits,
    )
}

#[test]
fn png_all_modes_and_declared_predictors_use_row_prefix() {
    use tandem_pdf_engine::stream::PredictorParams;
    let cases: [&[u8]; 5] = [
        &[0, 10, 20, 30, 0, 15, 25, 40],
        &[1, 10, 10, 10, 1, 15, 10, 15],
        &[2, 10, 20, 30, 2, 5, 5, 10],
        &[3, 10, 15, 20, 3, 10, 8, 13],
        &[4, 10, 10, 10, 4, 5, 5, 10],
    ];
    for predictor in 10..=15 {
        let params = PredictorParams {
            predictor,
            columns: 3,
            ..PredictorParams::default()
        };
        for encoded in cases {
            assert_eq!(
                predictor_decode(encoded, params, StreamLimits::default()).unwrap(),
                [10, 20, 30, 15, 25, 40]
            );
        }
    }
    let params = PredictorParams {
        predictor: 15,
        columns: 3,
        ..PredictorParams::default()
    };
    assert_eq!(
        predictor_decode(
            &[1, 10, 10, 10, 2, 5, 5, 10],
            params,
            StreamLimits::default()
        )
        .unwrap(),
        [10, 20, 30, 15, 25, 40]
    );
}

#[test]
fn png_first_row_single_pixel_and_component_byte_distance() {
    use tandem_pdf_engine::stream::PredictorParams;
    for bits in [1, 2, 4, 8, 16] {
        let params = PredictorParams {
            predictor: 15,
            colors: 2,
            bits_per_component: bits,
            columns: 2,
        };
        let row_bytes = (4usize * usize::from(bits)).div_ceil(8);
        let bpp = (2usize * usize::from(bits)).div_ceil(8);
        let mut encoded = vec![1];
        encoded.extend(std::iter::repeat_n(1, row_bytes));
        let expected: Vec<u8> = (0..row_bytes).map(|i| (i / bpp + 1) as u8).collect();
        assert_eq!(
            predictor_decode(&encoded, params, StreamLimits::default()).unwrap(),
            expected
        );
    }
    let params = PredictorParams {
        predictor: 15,
        columns: 1,
        ..PredictorParams::default()
    };
    assert_eq!(
        predictor_decode(&[4, 200, 4, 100], params, StreamLimits::default()).unwrap(),
        [200, 44]
    );
    assert!(
        predictor_decode(&[], params, StreamLimits::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn tiff_all_bit_depths_horizontal_modular_addition() {
    use tandem_pdf_engine::stream::PredictorParams;
    let cases: [(u8, usize, &[u8], &[u8]); 5] = [
        (1, 5, &[0b1011_0000], &[0b1101_1000]),
        (2, 3, &[0x6c], &[0x78]),
        (4, 3, &[0xe3, 0xf0], &[0xe1, 0x00]),
        (8, 3, &[10, 10, 255], &[10, 20, 19]),
        (16, 2, &[0xff, 0xfe, 0, 3], &[0xff, 0xfe, 0, 1]),
    ];
    for (bits, columns, encoded, expected) in cases {
        let params = PredictorParams {
            predictor: 2,
            colors: 1,
            bits_per_component: bits,
            columns,
        };
        assert_eq!(
            predictor_decode(encoded, params, StreamLimits::default()).unwrap(),
            expected
        );
    }
}

#[test]
fn tiff_component_history_resets_per_row_and_padding_is_zero() {
    use tandem_pdf_engine::stream::PredictorParams;
    let mut params = PredictorParams {
        predictor: 2,
        colors: 3,
        bits_per_component: 8,
        columns: 2,
    };
    assert_eq!(
        predictor_decode(
            &[10, 20, 30, 1, 2, 3, 1, 2, 3, 255, 254, 253],
            params,
            StreamLimits::default()
        )
        .unwrap(),
        [10, 20, 30, 11, 22, 33, 1, 2, 3, 0, 0, 0]
    );
    params.bits_per_component = 4;
    assert_eq!(
        predictor_decode(&[0xab, 0xc1, 0x23], params, StreamLimits::default()).unwrap(),
        [0xab, 0xcb, 0xdf]
    );
    params.colors = 1;
    params.columns = 1;
    params.bits_per_component = 1;
    assert_eq!(
        predictor_decode(&[0xff], params, StreamLimits::default()).unwrap(),
        [0x80]
    );
}

#[test]
fn predictor_rejects_partial_rows_unknown_mode_and_invalid_dimensions() {
    use tandem_pdf_engine::stream::PredictorParams;
    let params = PredictorParams {
        predictor: 15,
        columns: 2,
        ..PredictorParams::default()
    };
    for partial in [&[0][..], &[0, 1], &[0, 1, 2, 0]] {
        assert_eq!(
            predictor_decode(partial, params, StreamLimits::default())
                .unwrap_err()
                .kind,
            StreamErrorKind::TruncatedPredictorRow
        );
    }
    assert_eq!(
        predictor_decode(&[5, 1, 2], params, StreamLimits::default())
            .unwrap_err()
            .kind,
        StreamErrorKind::InvalidPngFilter(5)
    );
    let tiff = PredictorParams {
        predictor: 2,
        ..params
    };
    assert_eq!(
        predictor_decode(&[1], tiff, StreamLimits::default())
            .unwrap_err()
            .kind,
        StreamErrorKind::TruncatedPredictorRow
    );
    for invalid in [
        PredictorParams {
            columns: 0,
            ..params
        },
        PredictorParams {
            colors: 0,
            ..params
        },
        PredictorParams {
            colors: 33,
            ..params
        },
        PredictorParams {
            columns: usize::MAX,
            ..params
        },
        PredictorParams {
            columns: usize::MAX / 16,
            bits_per_component: 16,
            colors: 32,
            ..params
        },
    ] {
        assert!(matches!(
            predictor_decode(&[], invalid, StreamLimits::default())
                .unwrap_err()
                .kind,
            StreamErrorKind::InvalidPredictor(_)
        ));
    }
    for predictor in [0, 3, 9, 16, 255] {
        assert_eq!(
            predictor_decode(
                &[],
                PredictorParams {
                    predictor,
                    ..params
                },
                StreamLimits::default()
            )
            .unwrap_err()
            .kind,
            StreamErrorKind::UnsupportedPredictor(predictor)
        );
    }
    for bits in [0, 3, 7, 32, 255] {
        assert_eq!(
            predictor_decode(
                &[],
                PredictorParams {
                    bits_per_component: bits,
                    ..params
                },
                StreamLimits::default()
            )
            .unwrap_err()
            .kind,
            StreamErrorKind::UnsupportedBitsPerComponent(bits)
        );
    }
}

#[test]
fn predictor_charges_inflate_and_transform_output_and_limits_row_dimensions() {
    use tandem_pdf_engine::stream::{FilterSpec, PredictorParams, decode_filter_specs};
    let params = PredictorParams {
        predictor: 12,
        columns: 2,
        ..PredictorParams::default()
    };
    let limits = StreamLimits {
        max_decoded_bytes: 6,
        max_total_decoded_bytes: 10,
        ..StreamLimits::default()
    };
    let rows = [2, 1, 2, 2, 3, 4];
    assert_eq!(
        predictor_decode(&rows, params, limits).unwrap(),
        [1, 2, 4, 6]
    );
    assert_eq!(
        predictor_decode(
            &rows,
            params,
            StreamLimits {
                max_total_decoded_bytes: 9,
                ..limits
            }
        )
        .unwrap_err()
        .kind,
        StreamErrorKind::TotalOutputLimit { limit: 9 }
    );
    assert_eq!(
        predictor_decode(
            &rows,
            params,
            StreamLimits {
                max_decoded_bytes: 5,
                ..limits
            }
        )
        .unwrap_err()
        .kind,
        StreamErrorKind::DecodedOutputLimit { limit: 5 }
    );
    assert_eq!(
        predictor_decode(
            &[],
            PredictorParams {
                columns: 7,
                ..params
            },
            limits
        )
        .unwrap_err()
        .kind,
        StreamErrorKind::DecodedOutputLimit { limit: 6 }
    );
    // Predictor1 does not allocate or charge an identity transform.
    assert_eq!(
        predictor_decode(
            b"abc",
            PredictorParams::default(),
            StreamLimits {
                max_decoded_bytes: 3,
                max_total_decoded_bytes: 3,
                ..limits
            }
        )
        .unwrap(),
        b"abc"
    );
    assert!(
        decode_filter_specs(
            b"00>",
            &[FilterSpec {
                filter: Filter::AsciiHex,
                predictor: params
            }],
            limits
        )
        .is_err()
    );
    // PNG parameters1/1/8/1 require room for one row, even for empty streams.
    assert_eq!(
        predictor_decode(
            &[],
            params,
            StreamLimits {
                max_decoded_bytes: 0,
                ..limits
            }
        )
        .unwrap_err()
        .kind,
        StreamErrorKind::DecodedOutputLimit { limit: 0 }
    );
}

fn lzw_pack(codes: &[(u16, u8)]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut accumulator = 0u32;
    let mut occupied = 0u8;
    for &(code, width) in codes {
        accumulator = (accumulator << width) | u32::from(code);
        occupied += width;
        while occupied >= 8 {
            occupied -= 8;
            output.push((accumulator >> occupied) as u8);
        }
        accumulator &= (1u32 << occupied) - 1;
    }
    if occupied > 0 {
        output.push((accumulator << (8 - occupied)) as u8);
    }
    output
}
fn literal_lzw_width(processed: usize, early: bool) -> u8 {
    let correction = usize::from(early);
    if processed < 255 - correction {
        9
    } else if processed < 767 - correction {
        10
    } else if processed < 1791 - correction {
        11
    } else {
        12
    }
}
fn literal_lzw(data: &[u8], early: bool) -> Vec<u8> {
    let mut codes = vec![(256, 9)];
    for (i, &byte) in data.iter().enumerate() {
        codes.push((u16::from(byte), literal_lzw_width(i, early)));
    }
    codes.push((257, literal_lzw_width(data.len(), early)));
    lzw_pack(&codes)
}

#[test]
fn lzw_hand_codes_empty_literals_kwkwk_and_reset() {
    for early_change in [false, true] {
        let filter = Filter::Lzw { early_change };
        assert_eq!(decode(&lzw_pack(&[(256, 9), (257, 9)]), filter), b"");
        assert_eq!(
            decode(
                &lzw_pack(&[(256, 9), (65, 9), (66, 9), (258, 9), (260, 9), (257, 9)]),
                filter
            ),
            b"ABABABA"
        );
        assert_eq!(
            decode(
                &lzw_pack(&[(256, 9), (65, 9), (258, 9), (259, 9), (260, 9), (257, 9)]),
                filter
            ),
            b"AAAAAAAAAA"
        );
        assert_eq!(
            decode(
                &lzw_pack(&[
                    (256, 9),
                    (65, 9),
                    (258, 9),
                    (256, 9),
                    (256, 9),
                    (66, 9),
                    (258, 9),
                    (257, 9)
                ]),
                filter
            ),
            b"AAABBB"
        );
    }
}

#[test]
fn lzw_all_width_transitions_and_full_dictionary_clear() {
    for early_change in [false, true] {
        let filter = Filter::Lzw { early_change };
        for length in [
            0, 1, 253, 254, 255, 256, 765, 766, 767, 768, 1789, 1790, 1791, 1792, 3838, 3839,
        ] {
            let data: Vec<u8> = (0..=255).cycle().take(length).collect();
            assert_eq!(
                decode(&literal_lzw(&data, early_change), filter),
                data,
                "early={early_change}, length={length}"
            );
        }
        let mut codes = vec![(256, 9)];
        for i in 0..3839 {
            codes.push(((i % 256) as u16, literal_lzw_width(i, early_change)));
        }
        codes.extend([(256, 12), (65, 9), (258, 9), (257, 9)]);
        let mut expected: Vec<u8> = (0..=255).cycle().take(3839).collect();
        expected.extend_from_slice(b"AAA");
        assert_eq!(decode(&lzw_pack(&codes), filter), expected);
        codes.truncate(3840);
        codes.extend([(65, 12), (257, 12)]);
        assert_eq!(
            error(&lzw_pack(&codes), filter),
            StreamErrorKind::LzwDictionaryFull
        );
    }
}

#[test]
fn lzw_strict_invalid_codes_initial_clear_padding_and_truncation() {
    let filter = Filter::Lzw { early_change: true };
    assert_eq!(
        error(&lzw_pack(&[(65, 9), (257, 9)]), filter),
        StreamErrorKind::MissingLzwClear
    );
    assert_eq!(
        error(&lzw_pack(&[(256, 9), (258, 9), (257, 9)]), filter),
        StreamErrorKind::InvalidLzwCode(258)
    );
    assert_eq!(
        error(&lzw_pack(&[(256, 9), (65, 9), (259, 9), (257, 9)]), filter),
        StreamErrorKind::InvalidLzwCode(259)
    );
    let valid = lzw_pack(&[(256, 9), (65, 9), (257, 9)]);
    for len in 0..valid.len() {
        assert!(decode_filters(&valid[..len], &[filter], StreamLimits::default()).is_err());
    }
    let mut padding = valid.clone();
    *padding.last_mut().unwrap() |= 1;
    assert_eq!(error(&padding, filter), StreamErrorKind::NonZeroLzwPadding);
    let mut trailing = valid.clone();
    trailing.push(0);
    assert_eq!(error(&trailing, filter), StreamErrorKind::TrailingData);
    assert_eq!(
        error(&lzw_pack(&[(256, 9), (65, 9)]), filter),
        StreamErrorKind::TruncatedLzw
    );
}

#[test]
fn lzw_expansion_limits_and_predictor_accounting() {
    use tandem_pdf_engine::stream::{FilterSpec, PredictorParams, decode_filter_specs};
    let filter = Filter::Lzw { early_change: true };
    let encoded = lzw_pack(&[(256, 9), (65, 9), (258, 9), (259, 9), (260, 9), (257, 9)]);
    let limits = StreamLimits {
        max_decoded_bytes: 10,
        max_total_decoded_bytes: 10,
        ..StreamLimits::default()
    };
    assert_eq!(
        decode_filters(&encoded, &[filter], limits).unwrap(),
        b"AAAAAAAAAA"
    );
    assert_eq!(
        decode_filters(
            &encoded,
            &[filter],
            StreamLimits {
                max_decoded_bytes: 9,
                ..limits
            }
        )
        .unwrap_err()
        .kind,
        StreamErrorKind::DecodedOutputLimit { limit: 9 }
    );
    let empty = lzw_pack(&[(256, 9), (257, 9)]);
    assert!(
        decode_filters(
            &empty,
            &[filter],
            StreamLimits {
                max_decoded_bytes: 0,
                max_total_decoded_bytes: 0,
                ..limits
            }
        )
        .unwrap()
        .is_empty()
    );
    let parameters = PredictorParams {
        predictor: 12,
        columns: 2,
        ..PredictorParams::default()
    };
    let encoded = literal_lzw(&[2, 1, 2, 2, 3, 4], true);
    assert_eq!(
        decode_filter_specs(
            &encoded,
            &[FilterSpec {
                filter,
                predictor: parameters
            }],
            limits
        )
        .unwrap(),
        [1, 2, 4, 6]
    );
    assert_eq!(
        decode_filter_specs(
            &encoded,
            &[FilterSpec {
                filter,
                predictor: parameters
            }],
            StreamLimits {
                max_total_decoded_bytes: 9,
                ..limits
            }
        )
        .unwrap_err()
        .kind,
        StreamErrorKind::TotalOutputLimit { limit: 9 }
    );
    // Consecutive KwKwK entries grow the decoded length quadratically while
    // compressed codes remain bounded; stop before materializing that output.
    let mut codes = vec![(256, 9), (65, 9)];
    for next in 258..=4095 {
        codes.push((next, literal_lzw_width(usize::from(next) - 257, true)));
    }
    codes.push((257, 12));
    assert_eq!(
        decode_filters(
            &lzw_pack(&codes),
            &[filter],
            StreamLimits {
                max_decoded_bytes: 1024,
                ..StreamLimits::default()
            }
        )
        .unwrap_err()
        .kind,
        StreamErrorKind::DecodedOutputLimit { limit: 1024 }
    );
}

#[test]
fn lzw_maximum_prefix_chain_is_bounded_and_decodes_without_recursion() {
    let mut codes = vec![(256, 9), (65, 9)];
    for next in 258..=4095 {
        codes.push((next, literal_lzw_width(usize::from(next) - 257, true)));
    }
    codes.push((257, 12));
    let expected_len = 3839 * 3840 / 2;
    let decoded = decode_filters(
        &lzw_pack(&codes),
        &[Filter::Lzw { early_change: true }],
        StreamLimits {
            max_decoded_bytes: expected_len,
            max_total_decoded_bytes: expected_len,
            ..StreamLimits::default()
        },
    )
    .unwrap();
    assert_eq!(decoded.len(), expected_len);
    assert!(decoded.iter().all(|&byte| byte == b'A'));
}

#[test]
fn decode_usage_reports_identity_every_filter_and_predictor_intermediate() {
    use tandem_pdf_engine::stream::{
        FilterSpec, PredictorParams, decode_filter_specs_with_usage, decode_filters_with_usage,
    };
    let identity = decode_filters_with_usage(b"abc", &[], StreamLimits::default()).unwrap();
    assert_eq!(identity.bytes, b"abc");
    assert_eq!(identity.usage.encoded_bytes, 3);
    assert_eq!(identity.usage.total_decoded_bytes, 3);
    let filtered = decode_filters_with_usage(
        b"0261626380>",
        &[Filter::AsciiHex, Filter::RunLength],
        StreamLimits::default(),
    )
    .unwrap();
    assert_eq!(filtered.bytes, b"abc");
    assert_eq!(filtered.usage.encoded_bytes, 11);
    assert_eq!(filtered.usage.total_decoded_bytes, 8);
    let params = PredictorParams {
        predictor: 12,
        columns: 2,
        ..PredictorParams::default()
    };
    let compressed = compress_to_vec_zlib(&[2, 1, 2, 2, 3, 4], 6);
    let predicted = decode_filter_specs_with_usage(
        &compressed,
        &[FilterSpec {
            filter: Filter::Flate,
            predictor: params,
        }],
        StreamLimits::default(),
    )
    .unwrap();
    assert_eq!(predicted.bytes, [1, 2, 4, 6]);
    assert_eq!(predicted.usage.encoded_bytes, compressed.len());
    assert_eq!(predicted.usage.total_decoded_bytes, 10);
}
