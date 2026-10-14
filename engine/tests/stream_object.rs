// SPDX-License-Identifier: AGPL-3.0-or-later
use miniz_oxide::deflate::compress_to_vec_zlib;
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::ObjectId;
use tandem_pdf_engine::stream::{StreamErrorKind, StreamLimits};
use tandem_pdf_engine::stream_object::{StreamObjectErrorKind, decode_document_stream};

fn id(number: u32) -> ObjectId {
    ObjectId::new(number, 0).unwrap()
}
fn pdf(metadata: &str, payload: &[u8], additional: &[Vec<u8>]) -> Vec<u8> {
    let mut bodies = vec![b"<< /Type /Catalog >>".to_vec()];
    let mut stream = format!("<< /Length {} {metadata} >>\nstream\n", payload.len()).into_bytes();
    stream.extend_from_slice(payload);
    stream.extend_from_slice(b"\nendstream");
    bodies.push(stream);
    bodies.extend_from_slice(additional);
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in bodies.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    bytes
}
fn decode(bytes: &[u8]) -> Result<Vec<u8>, tandem_pdf_engine::stream_object::StreamObjectError> {
    let document = Document::open(bytes, DocumentLimits::default()).unwrap();
    decode_document_stream(&document, id(2), StreamLimits::default())
}

#[test]
fn identity_and_all_full_filter_names_decode() {
    let zlib = compress_to_vec_zlib(b"hello", 6);
    for (metadata, payload, expected) in [
        ("", b"abc".as_slice(), b"abc".as_slice()),
        ("/Filter null /DecodeParms null", b"abc", b"abc"),
        ("/Filter []", b"abc", b"abc"),
        ("/Filter /FlateDecode", zlib.as_slice(), b"hello"),
        ("/Filter /ASCIIHexDecode", b"68656c6c6f>", b"hello"),
        ("/Filter /ASCII85Decode", b"z~>", b"\0\0\0\0"),
        ("/Filter /RunLengthDecode", b"\x02abc\x80", b"abc"),
    ] {
        assert_eq!(decode(&pdf(metadata, payload, &[])).unwrap(), expected);
    }
}

#[test]
fn filter_chains_apply_in_declared_order() {
    let bytes = pdf(
        "/Filter [/ASCIIHexDecode /RunLengthDecode] /DecodeParms [null <<>>]",
        b"0261626380>",
        &[],
    );
    assert_eq!(decode(&bytes).unwrap(), b"abc");
    let bytes = pdf(
        "/Filter [/RunLengthDecode /ASCIIHexDecode]",
        b"0261626380>",
        &[],
    );
    assert!(decode(&bytes).is_err());
}

#[test]
fn empty_and_null_parameter_dictionaries_are_accepted() {
    let zlib = compress_to_vec_zlib(b"hello", 6);
    for parameters in [
        "<<>>",
        "<< /Predictor 1 >>",
        "<< /Predictor null /Ignored null >>",
        "[null]",
        "[<< /Predictor 1 >>]",
    ] {
        assert_eq!(
            decode(&pdf(
                &format!("/Filter /FlateDecode /DecodeParms {parameters}"),
                &zlib,
                &[]
            ))
            .unwrap(),
            b"hello"
        );
    }
    assert_eq!(
        decode(&pdf(
            "/Filter [/ASCIIHexDecode] /DecodeParms <<>>",
            b"61>",
            &[]
        ))
        .unwrap(),
        b"a"
    );
}

#[test]
fn unsupported_or_wrong_parameters_are_never_silently_ignored() {
    for metadata in [
        "/DecodeParms <<>>",
        "/Filter null /DecodeParms []",
        "/Filter /ASCIIHexDecode /DecodeParms << /Predictor 1 >>",
        "/Filter /FlateDecode /DecodeParms << /Predictor 256 >>",
        "/Filter /FlateDecode /DecodeParms << /Predictor 1.0 >>",
        "/Filter /FlateDecode /DecodeParms << /Columns -20 >>",
        "/Filter /FlateDecode /DecodeParms << /Colors -3 >>",
        "/Filter /FlateDecode /DecodeParms << /Unknown true >>",
        "/Filter /ASCIIHexDecode /DecodeParms true",
        "/Filter /ASCIIHexDecode /DecodeParms [null null]",
        "/Filter [/ASCIIHexDecode /RunLengthDecode] /DecodeParms <<>>",
        "/Filter /ASCIIHexDecode /DecodeParms [42]",
    ] {
        let error = decode(&pdf(metadata, b"61>", &[])).unwrap_err();
        assert!(
            matches!(
                error.kind,
                StreamObjectErrorKind::Invalid(_) | StreamObjectErrorKind::Unsupported(_)
            ),
            "{metadata}: {error}"
        );
    }
}

#[test]
fn wrong_filter_types_and_unknown_filters_fail_explicitly() {
    for metadata in [
        "/Filter 3",
        "/Filter <<>>",
        "/Filter [null]",
        "/Filter [(FlateDecode)]",
        "/Filter [true]",
    ] {
        assert!(matches!(
            decode(&pdf(metadata, b"", &[])).unwrap_err().kind,
            StreamObjectErrorKind::Invalid(_)
        ));
    }
    for name in ["DCTDecode", "JPXDecode", "Crypt", "Fl", "AHx", "LZW"] {
        assert!(matches!(
            decode(&pdf(&format!("/Filter /{name}"), b"", &[]))
                .unwrap_err()
                .kind,
            StreamObjectErrorKind::UnknownFilter(_)
        ));
    }
}

#[test]
fn indirect_filter_array_names_and_parameters_resolve() {
    let additional = vec![
        b"[4 0 R]".to_vec(),
        b"/ASCIIHexDecode".to_vec(),
        b"[6 0 R]".to_vec(),
        b"<< /Ignored 7 0 R >>".to_vec(),
        b"null".to_vec(),
    ];
    assert_eq!(
        decode(&pdf(
            "/Filter 3 0 R /DecodeParms 5 0 R",
            b"61>",
            &additional
        ))
        .unwrap(),
        b"a"
    );
    let zlib = compress_to_vec_zlib(b"hello", 6);
    let additional = vec![
        b"/FlateDecode".to_vec(),
        b"<< /Predictor 5 0 R >>".to_vec(),
        b"1".to_vec(),
    ];
    assert_eq!(
        decode(&pdf("/Filter 3 0 R /DecodeParms 4 0 R", &zlib, &additional)).unwrap(),
        b"hello"
    );
}

#[test]
fn indirect_null_filter_and_parameters_are_absent() {
    assert_eq!(
        decode(&pdf(
            "/Filter 3 0 R /DecodeParms 3 0 R",
            b"abc",
            &[b"null".to_vec()]
        ))
        .unwrap(),
        b"abc"
    );
}

#[test]
fn missing_cyclic_deep_and_stream_metadata_references_fail() {
    let error = decode(&pdf("/Filter 9 0 R", b"", &[])).unwrap_err();
    assert!(matches!(error.kind, StreamObjectErrorKind::Document(_)));
    let error = decode(&pdf(
        "/Filter 3 0 R",
        b"",
        &[b"4 0 R".to_vec(), b"3 0 R".to_vec()],
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        StreamObjectErrorKind::ReferenceCycle(_)
    ));
    let error = decode(&pdf("/Filter 2 0 R", b"", &[])).unwrap_err();
    assert!(matches!(error.kind, StreamObjectErrorKind::Invalid(_)));
    let mut chain: Vec<Vec<u8>> = (4..=66)
        .map(|number| format!("{number} 0 R").into_bytes())
        .collect();
    chain.push(b"/ASCIIHexDecode".to_vec()); // exactly64 metadata objects, IDs3..66
    assert_eq!(decode(&pdf("/Filter 3 0 R", b"61>", &chain)).unwrap(), b"a");
    *chain.last_mut().unwrap() = b"67 0 R".to_vec();
    chain.push(b"/ASCIIHexDecode".to_vec());
    assert!(matches!(
        decode(&pdf("/Filter 3 0 R", b"61>", &chain))
            .unwrap_err()
            .kind,
        StreamObjectErrorKind::ReferenceLimit
    ));
}

#[test]
fn parameter_reference_cycles_fail() {
    let additional = vec![
        b"<< /Predictor 4 0 R >>".to_vec(),
        b"5 0 R".to_vec(),
        b"4 0 R".to_vec(),
    ];
    assert!(matches!(
        decode(&pdf(
            "/Filter /FlateDecode /DecodeParms 3 0 R",
            b"",
            &additional
        ))
        .unwrap_err()
        .kind,
        StreamObjectErrorKind::ReferenceCycle(_)
    ));
}

#[test]
fn limits_and_decoder_errors_are_preserved() {
    let bytes = pdf(
        "/Filter [/ASCIIHexDecode /RunLengthDecode]",
        b"0261626380>",
        &[],
    );
    let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert!(matches!(
        decode_document_stream(
            &document,
            id(2),
            StreamLimits {
                max_filters: 1,
                ..StreamLimits::default()
            }
        )
        .unwrap_err()
        .kind,
        StreamObjectErrorKind::FilterLimit { limit: 1 }
    ));
    assert!(
        matches!(decode_document_stream(&document, id(2), StreamLimits { max_encoded_bytes: 2, ..StreamLimits::default() }).unwrap_err().kind, StreamObjectErrorKind::Decode(ref source) if matches!(source.kind, StreamErrorKind::EncodedInputLimit { .. }))
    );
    assert!(
        matches!(decode_document_stream(&document, id(2), StreamLimits { max_decoded_bytes: 2, ..StreamLimits::default() }).unwrap_err().kind, StreamObjectErrorKind::Decode(ref source) if matches!(source.kind, StreamErrorKind::DecodedOutputLimit { .. }))
    );
    assert!(
        matches!(decode_document_stream(&document, id(2), StreamLimits { max_total_decoded_bytes: 7, ..StreamLimits::default() }).unwrap_err().kind, StreamObjectErrorKind::Decode(ref source) if matches!(source.kind, StreamErrorKind::TotalOutputLimit { .. }))
    );
    assert!(matches!(
        decode_document_stream(&document, id(1), StreamLimits::default())
            .unwrap_err()
            .kind,
        StreamObjectErrorKind::NotStream
    ));
    assert!(matches!(
        decode_document_stream(&document, id(99), StreamLimits::default())
            .unwrap_err()
            .kind,
        StreamObjectErrorKind::Document(_)
    ));
    assert!(
        matches!(decode(&pdf("/Filter /ASCIIHexDecode", b"61", &[])).unwrap_err().kind, StreamObjectErrorKind::Decode(ref source) if source.kind == StreamErrorKind::MissingEndMarker)
    );
}

#[test]
fn png_and_tiff_parameters_decode_and_resolve_indirect_values() {
    let zlib = compress_to_vec_zlib(&[2, 1, 2, 2, 3, 4], 6);
    assert_eq!(
        decode(&pdf(
            "/Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 2 >>",
            &zlib,
            &[]
        ))
        .unwrap(),
        [1, 2, 4, 6]
    );
    let additional = vec![b"15".to_vec(), b"2".to_vec(), b"8".to_vec(), b"1".to_vec()];
    assert_eq!(decode(&pdf("/Filter /FlateDecode /DecodeParms << /Predictor 3 0 R /Columns 4 0 R /BitsPerComponent 5 0 R /Colors 6 0 R >>",&zlib,&additional)).unwrap(),[1,2,4,6]);
    let zlib = compress_to_vec_zlib(&[0xab, 0xc1, 0x23], 6);
    assert_eq!(decode(&pdf("/Filter /FlateDecode /DecodeParms << /Predictor 2 /Colors 3 /BitsPerComponent 4 /Columns 2 >>",&zlib,&[])).unwrap(),[0xab,0xcb,0xdf]);
}

#[test]
fn predictor_metadata_defaults_nulls_and_identity_dimensions() {
    let zlib = compress_to_vec_zlib(&[0, 1, 2, 2], 6);
    assert_eq!(decode(&pdf("/Filter /FlateDecode /DecodeParms << /Predictor 15 /Columns null /Colors null /BitsPerComponent null >>",&zlib,&[])).unwrap(),[1,3]);
    let zlib = compress_to_vec_zlib(b"hello", 6);
    assert_eq!(
        decode(&pdf(
            "/Filter /FlateDecode /DecodeParms << /Columns 20 /Colors 3 /BitsPerComponent 16 >>",
            &zlib,
            &[]
        ))
        .unwrap(),
        b"hello"
    );
}

#[test]
fn predictor_filter_array_and_intermediate_accounting() {
    let zlib = compress_to_vec_zlib(&[1, 1, 2], 6);
    let mut encoded = String::new();
    for byte in &zlib {
        use std::fmt::Write;
        write!(encoded, "{byte:02x}").unwrap();
    }
    encoded.push('>');
    let bytes = pdf(
        "/Filter [/ASCIIHexDecode /FlateDecode] /DecodeParms [null << /Predictor 11 /Columns 2 >>]",
        encoded.as_bytes(),
        &[],
    );
    assert_eq!(decode(&bytes).unwrap(), [1, 3]);
    let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let total = zlib.len() + 3 + 2;
    assert!(
        decode_document_stream(
            &document,
            id(2),
            StreamLimits {
                max_total_decoded_bytes: total,
                ..StreamLimits::default()
            }
        )
        .is_ok()
    );
    assert!(
        matches!(decode_document_stream(&document,id(2),StreamLimits {max_total_decoded_bytes:total-1,..StreamLimits::default()}).unwrap_err().kind,StreamObjectErrorKind::Decode(ref error) if matches!(error.kind,StreamErrorKind::TotalOutputLimit {..}))
    );
}

#[test]
fn invalid_predictor_metadata_fails_before_decoding() {
    for parameters in [
        "/Colors 0",
        "/Colors 33",
        "/BitsPerComponent 3",
        "/Columns 0",
        "/Predictor 3",
        "/Predictor 15 /Columns 9223372036854775807",
    ] {
        let result = decode(&pdf(
            &format!("/Filter /FlateDecode /DecodeParms << {parameters} >>"),
            b"not zlib",
            &[],
        ))
        .unwrap_err();
        assert!(
            matches!(result.kind,StreamObjectErrorKind::Decode(ref error) if matches!(error.kind,StreamErrorKind::InvalidPredictor(_) | StreamErrorKind::UnsupportedPredictor(_) | StreamErrorKind::UnsupportedBitsPerComponent(_) | StreamErrorKind::DecodedOutputLimit {..})),
            "{parameters}: {result}"
        );
    }
}

#[test]
fn lzw_metadata_earlychange_and_predictor_defaults() {
    // Nine-bit clear, 'A', 'B', dictionary258, KwKwK260, EOD257.
    let encoded = [128, 16, 72, 80, 40, 36, 4];
    for parameters in [
        "",
        " /DecodeParms << /EarlyChange 0 >>",
        " /DecodeParms << /EarlyChange 1 >>",
        " /DecodeParms << /EarlyChange null >>",
    ] {
        assert_eq!(
            decode(&pdf(
                &format!("/Filter /LZWDecode{parameters}"),
                &encoded,
                &[]
            ))
            .unwrap(),
            b"ABABABA"
        );
    }
    assert_eq!(
        decode(&pdf(
            "/Filter /LZWDecode /DecodeParms << /EarlyChange 3 0 R >>",
            &encoded,
            &[b"0".to_vec()]
        ))
        .unwrap(),
        b"ABABABA"
    );
}

#[test]
fn invalid_earlychange_is_explicit_and_not_accepted_for_flate() {
    for parameters in [
        "/EarlyChange -1",
        "/EarlyChange 2",
        "/EarlyChange 0.0",
        "/EarlyChange true",
    ] {
        let error = decode(&pdf(
            &format!("/Filter /LZWDecode /DecodeParms << {parameters} >>"),
            b"",
            &[],
        ))
        .unwrap_err();
        assert!(matches!(
            error.kind,
            StreamObjectErrorKind::Invalid(_) | StreamObjectErrorKind::Unsupported(_)
        ));
    }
    assert!(matches!(
        decode(&pdf(
            "/Filter /FlateDecode /DecodeParms << /EarlyChange 0 >>",
            b"",
            &[]
        ))
        .unwrap_err()
        .kind,
        StreamObjectErrorKind::Unsupported(_)
    ));
}

#[test]
fn document_decode_usage_reports_actual_intermediate_cost() {
    use tandem_pdf_engine::stream_object::decode_document_stream_with_usage;
    let bytes = pdf(
        "/Filter [/ASCIIHexDecode /RunLengthDecode]",
        b"0261626380>",
        &[],
    );
    let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let decoded =
        decode_document_stream_with_usage(&document, id(2), StreamLimits::default()).unwrap();
    assert_eq!(decoded.bytes, b"abc");
    assert_eq!(decoded.usage.encoded_bytes, 11);
    assert_eq!(decoded.usage.total_decoded_bytes, 8);
}
