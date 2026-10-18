// SPDX-License-Identifier: MIT
//! Independent hand-computed predictor vectors and budget regressions.
use miniz_oxide::deflate::compress_to_vec_zlib;
use tandem_pdf_engine::stream::{
    Filter, FilterSpec, PredictorParams, StreamErrorKind, StreamLimits, decode_filter_specs,
};
fn spec(predictor: u8, colors: usize, bits: u8, columns: usize) -> FilterSpec {
    FilterSpec {
        filter: Filter::Flate,
        predictor: PredictorParams {
            predictor,
            colors,
            bits_per_component: bits,
            columns,
        },
    }
}
fn decode(raw: &[u8], filter: FilterSpec) -> Vec<u8> {
    decode_filter_specs(
        &compress_to_vec_zlib(raw, 6),
        &[filter],
        StreamLimits::default(),
    )
    .unwrap()
}

#[test]
fn tiff_packed_components_keep_color_history_and_reset_each_row() {
    assert_eq!(
        decode(&[0x12, 0x34, 0xfe], spec(2, 2, 4, 3)),
        [0x12, 0x46, 0x34]
    );
    // Nine one-bit samples; padding bits are deliberately set in input.
    assert_eq!(
        decode(&[0xbd, 0xff, 0xbd, 0xff], spec(2, 3, 1, 3)),
        [0xa8, 0x80, 0xa8, 0x80]
    );
    assert_eq!(
        decode(&[0xff, 0xff, 0, 1, 0, 2, 0xff, 0xff], spec(2, 2, 16, 2)),
        [0xff, 0xff, 0, 1, 0, 1, 0, 0]
    );
}

#[test]
fn png_hand_computed_mixed_rows_use_actual_tags_and_previous_decoded_row() {
    let raw = [
        0, 10, 20, 30, 1, 5, 5, 5, 2, 1, 2, 3, 3, 1, 2, 3, 4, 1, 2, 3,
    ];
    let expected = [10, 20, 30, 5, 10, 15, 6, 12, 18, 4, 10, 17, 5, 12, 20];
    for declaration in 10..=15 {
        assert_eq!(decode(&raw, spec(declaration, 1, 8, 3)), expected);
    }
    // PNG arithmetic operates on bytes, including packed component padding.
    assert_eq!(decode(&[1, 0xff, 2, 3], spec(15, 3, 2, 3)), [0xff, 1, 4]);
}

#[test]
fn partial_rows_and_invalid_tags_never_return_partial_output() {
    for (raw, filter) in [
        (&[1, 2][..], spec(2, 1, 8, 3)),
        (&[0, 1, 2][..], spec(15, 1, 8, 3)),
    ] {
        assert!(matches!(
            decode_filter_specs(
                &compress_to_vec_zlib(raw, 6),
                &[filter],
                StreamLimits::default()
            )
            .unwrap_err()
            .kind,
            StreamErrorKind::TruncatedPredictorRow
        ));
    }
    assert!(matches!(
        decode_filter_specs(
            &compress_to_vec_zlib(&[0, 1, 2, 3, 5, 4, 5, 6], 6),
            &[spec(15, 1, 8, 3)],
            StreamLimits::default()
        )
        .unwrap_err()
        .kind,
        StreamErrorKind::InvalidPngFilter(5)
    ));
}

#[test]
fn predictor_and_later_filter_outputs_share_one_cumulative_budget() {
    let encoded = compress_to_vec_zlib(b"\x004142>", 6);
    let filters = [spec(15, 1, 8, 5), Filter::AsciiHex.into()];
    let exact = StreamLimits {
        max_encoded_bytes: 128,
        max_decoded_bytes: 6,
        max_filters: 2,
        max_total_decoded_bytes: 13,
    };
    assert_eq!(
        decode_filter_specs(&encoded, &filters, exact).unwrap(),
        b"AB"
    );
    let error = decode_filter_specs(
        &encoded,
        &filters,
        StreamLimits {
            max_total_decoded_bytes: 12,
            ..exact
        },
    )
    .unwrap_err();
    assert_eq!(error.filter_index, Some(1));
    assert!(matches!(
        error.kind,
        StreamErrorKind::TotalOutputLimit { .. }
    ));
}

#[test]
fn invalid_later_predictor_is_preflighted_before_decompression() {
    let filters = [Filter::Flate.into(), spec(15, 32, 16, usize::MAX)];
    let error =
        decode_filter_specs(b"malformed zlib", &filters, StreamLimits::default()).unwrap_err();
    assert_eq!(error.filter_index, Some(1));
    assert!(matches!(error.kind, StreamErrorKind::InvalidPredictor(_)));
}
