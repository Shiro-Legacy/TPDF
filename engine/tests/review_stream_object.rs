// SPDX-License-Identifier: MIT
//! Independent checks of dictionary-aware stream decoding and CLI integration.
use std::io::Write;
use std::process::{Command, Stdio};
use tandem_pdf_engine::document::{Document, DocumentErrorKind, DocumentLimits};
use tandem_pdf_engine::object::ObjectId;
use tandem_pdf_engine::stream::{StreamErrorKind, StreamLimits};
use tandem_pdf_engine::stream_object::{StreamObjectErrorKind, decode_document_stream};

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut output = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        output.extend_from_slice(object);
        output.extend_from_slice(b"\nendobj\n");
    }
    let xref = output.len();
    output.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    output
}
fn stream(extra: &str, payload: &[u8]) -> Vec<u8> {
    let mut output = format!("<< /Length {} {extra} >>\nstream\n", payload.len()).into_bytes();
    output.extend_from_slice(payload);
    output.extend_from_slice(b"\nendstream");
    output
}
fn root() -> Vec<u8> {
    b"<< /Type /Catalog >>".to_vec()
}
fn id() -> ObjectId {
    ObjectId::new(2, 0).unwrap()
}

#[test]
fn indirect_filter_and_parameter_arrays_preserve_chain_order_and_budgets() {
    let input = pdf(&[
        root(),
        stream("/Filter 3 0 R /DecodeParms 6 0 R", b"\x81f\0>\x80"),
        b"[4 0 R 5 0 R]".to_vec(),
        b"/RunLengthDecode".to_vec(),
        b"/ASCIIHexDecode".to_vec(),
        b"[7 0 R 8 0 R]".to_vec(),
        b"<< /Ignored null >>".to_vec(),
        b"null".to_vec(),
    ]);
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    let limits = StreamLimits {
        max_encoded_bytes: 5,
        max_decoded_bytes: 129,
        max_filters: 2,
        max_total_decoded_bytes: 193,
    };
    assert_eq!(
        decode_document_stream(&doc, id(), limits).unwrap(),
        vec![255; 64]
    );
    let error = decode_document_stream(
        &doc,
        id(),
        StreamLimits {
            max_total_decoded_bytes: 192,
            ..limits
        },
    )
    .unwrap_err();
    let StreamObjectErrorKind::Decode(error) = error.kind else {
        panic!("wrong error kind")
    };
    assert_eq!(error.filter_index, Some(1));
    assert!(matches!(
        error.kind,
        StreamErrorKind::TotalOutputLimit { .. }
    ));
}

#[test]
fn metadata_reference_hops_have_exact_64_limit() {
    for hops in [64usize, 65] {
        let mut objects = vec![root(), stream("/Filter 3 0 R", b"00ff>")];
        for index in 0..hops {
            objects.push(if index + 1 == hops {
                b"/ASCIIHexDecode".to_vec()
            } else {
                format!("{} 0 R", index + 4).into_bytes()
            });
        }
        let input = pdf(&objects);
        let doc = Document::open(&input, DocumentLimits::default()).unwrap();
        let result = decode_document_stream(&doc, id(), StreamLimits::default());
        if hops == 64 {
            assert_eq!(result.unwrap(), [0, 255]);
        } else {
            assert!(matches!(
                result.unwrap_err().kind,
                StreamObjectErrorKind::ReferenceLimit
            ));
        }
    }
}

#[test]
fn cycles_in_decode_parameters_are_reported_before_decoding() {
    let input = pdf(&[
        root(),
        stream("/Filter /ASCIIHexDecode /DecodeParms 3 0 R", b"malformed"),
        b"4 0 R".to_vec(),
        b"3 0 R".to_vec(),
    ]);
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    assert!(matches!(
        decode_document_stream(&doc, id(), StreamLimits::default())
            .unwrap_err()
            .kind,
        StreamObjectErrorKind::ReferenceCycle(_)
    ));
}

#[test]
fn external_sources_and_stream_valued_metadata_are_refused() {
    let input = pdf(&[
        root(),
        stream(
            "/F (file:///never-open-this) /Filter /ASCIIHexDecode",
            b"ff>",
        ),
    ]);
    assert!(matches!(
        Document::open(&input, DocumentLimits::default())
            .unwrap_err()
            .kind,
        DocumentErrorKind::Unsupported("external-file streams")
    ));
    let input = pdf(&[
        root(),
        stream("/Filter 3 0 R", b"ff>"),
        stream("", b"/ASCIIHexDecode"),
    ]);
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    assert!(matches!(
        decode_document_stream(&doc, id(), StreamLimits::default())
            .unwrap_err()
            .kind,
        StreamObjectErrorKind::Invalid("metadata reference targets a stream")
    ));
}

#[test]
fn filter_count_is_checked_before_resolving_array_members() {
    let input = pdf(&[root(), stream("/Filter [999 0 R 999 0 R]", b"ff>")]);
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    let limits = StreamLimits {
        max_filters: 1,
        ..StreamLimits::default()
    };
    assert!(matches!(
        decode_document_stream(&doc, id(), limits).unwrap_err().kind,
        StreamObjectErrorKind::FilterLimit { limit: 1 }
    ));
}

#[test]
fn null_metadata_is_identity_and_still_charges_output_budget() {
    let input = pdf(&[
        root(),
        stream("/Filter 3 0 R /DecodeParms null /F null", b"\0\xffA"),
        b"null".to_vec(),
    ]);
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    assert_eq!(
        decode_document_stream(&doc, id(), StreamLimits::default()).unwrap(),
        b"\0\xffA"
    );
    let error = decode_document_stream(
        &doc,
        id(),
        StreamLimits {
            max_total_decoded_bytes: 2,
            ..StreamLimits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error.kind, StreamObjectErrorKind::Decode(_)));
}

#[test]
fn stream_cli_emits_raw_binary_and_refuses_predictors() {
    for (extra, payload, expected_success) in [
        ("/Filter /ASCIIHexDecode", b"00ff41>".as_slice(), true),
        (
            "/Filter /FlateDecode /DecodeParms << /Predictor 3 >>",
            b"not compressed",
            false,
        ),
    ] {
        let input = pdf(&[root(), stream(extra, payload)]);
        let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
            .arg("--stream=2:0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&input).unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.success(), expected_success);
        if expected_success {
            assert_eq!(output.stdout, b"\0\xffA");
        } else {
            assert!(output.stdout.is_empty());
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .contains("unsupported predictor 3")
            );
        }
    }
}
