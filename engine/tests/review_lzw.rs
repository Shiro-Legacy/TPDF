// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independently confirmed LZW vectors and cumulative allocation budgets.
use tandem_pdf_engine::stream::{
    Filter, FilterSpec, PredictorParams, StreamLimits, decode_filter_specs, decode_filters,
};

fn literal_codes(bytes: &[u8]) -> Vec<u8> {
    // This fixture intentionally remains below any 9-to-10-bit transition.
    assert!(bytes.len() < 250);
    let mut bits = Vec::new();
    for code in std::iter::once(256)
        .chain(bytes.iter().map(|&b| u16::from(b)))
        .chain(std::iter::once(257))
    {
        bits.extend((0..9).rev().map(|bit| ((code >> bit) & 1) as u8));
    }
    while bits.len() % 8 != 0 {
        bits.push(0);
    }
    bits.chunks_exact(8)
        .map(|chunk| chunk.iter().fold(0, |byte, bit| byte * 2 + bit))
        .collect()
}

#[test]
fn vectors_cover_kwkwk_and_clear_replacing_old_dictionary_entries() {
    for early_change in [false, true] {
        for (encoded, expected) in [
            (
                b"\x80\x10\x48\x50\x28\x24\x04".as_slice(),
                b"ABABABA".as_slice(),
            ),
            (b"\x80\x10\x48\x50\x28\x01\x0e\x05\x01", b"ABABCCC"),
        ] {
            assert_eq!(
                decode_filters(
                    encoded,
                    &[Filter::Lzw { early_change }],
                    StreamLimits::default()
                )
                .unwrap(),
                expected
            );
        }
    }
}

#[test]
fn nested_lzw_charges_both_intermediate_outputs() {
    let original = b"nested LZW output\0\xff";
    let inner = literal_codes(original);
    let outer = literal_codes(&inner);
    let filter = Filter::Lzw { early_change: true };
    let limits = StreamLimits {
        max_total_decoded_bytes: inner.len() + original.len(),
        ..StreamLimits::default()
    };
    assert_eq!(
        decode_filters(&outer, &[filter, filter], limits).unwrap(),
        original
    );
    assert!(
        decode_filters(
            &outer,
            &[filter, filter],
            StreamLimits {
                max_total_decoded_bytes: limits.max_total_decoded_bytes - 1,
                ..limits
            }
        )
        .is_err()
    );
}

#[test]
fn png_after_lzw_charges_the_row_tag_as_intermediate_output() {
    // LZW gives [0,1,2,3] (independently confirmed); PNG filter 0 removes
    // the row tag.
    let encoded = b"\x80\x00\x00\x20\x20\x1c\x04";
    let filters = [FilterSpec {
        filter: Filter::Lzw { early_change: true },
        predictor: PredictorParams {
            predictor: 15,
            columns: 3,
            ..PredictorParams::default()
        },
    }];
    let limits = StreamLimits {
        max_decoded_bytes: 4,
        max_total_decoded_bytes: 7,
        ..StreamLimits::default()
    };
    assert_eq!(
        decode_filter_specs(encoded, &filters, limits).unwrap(),
        [1, 2, 3]
    );
    for limited in [
        StreamLimits {
            max_total_decoded_bytes: 6,
            ..limits
        },
        StreamLimits {
            max_decoded_bytes: 3,
            ..limits
        },
    ] {
        assert!(decode_filter_specs(encoded, &filters, limited).is_err());
    }
}

#[test]
fn zero_output_cap_accepts_clear_eod_but_refuses_first_literal() {
    let limits = StreamLimits {
        max_decoded_bytes: 0,
        max_total_decoded_bytes: 0,
        ..StreamLimits::default()
    };
    let filter = [Filter::Lzw { early_change: true }];
    assert!(
        decode_filters(&literal_codes(&[]), &filter, limits)
            .unwrap()
            .is_empty()
    );
    assert!(decode_filters(&literal_codes(b"A"), &filter, limits).is_err());
    let mut padded = literal_codes(&[]);
    *padded.last_mut().unwrap() |= 1;
    assert!(decode_filters(&padded, &filter, limits).is_err());
}
