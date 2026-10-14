// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tolerant rebuild of damaged variants of the fixtures. Each damaged file must
//! fail the strict open, and its repair must open strictly with the same pages
//! and decoded content streams as the undamaged file. Set TPDF_REPAIR_DUMP to a
//! directory to write each damaged and repaired pair for comparison with MuPDF.
use std::time::Instant;

use miniz_oxide::deflate::compress_to_vec_zlib;
use tandem_pdf_engine::document::{Document, DocumentErrorKind, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::pages::{PageLimits, collect_pages};
use tandem_pdf_engine::repair::{RepairError, RepairLimits, RepairReport, open_tolerant, rebuild};
use tandem_pdf_engine::serialize::{WriteLimits, serialize_object};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::stream_object::decode_document_stream;

const USER: &[u8] = b"pdf4all-user";

const FIXTURES: [&str; 14] = [
    "hello.pdf",
    "cjk.pdf",
    "form2.pdf",
    "nav.pdf",
    "rtl.pdf",
    "redact.pdf",
    "large.pdf",
    "layers.pdf",
    "rotated.pdf",
    "p0b/certified-p2.pdf",
    "p0b/certified-p2-annot.pdf",
    "p0b/edit-subset.pdf",
    "p6/quartz.pdf",
    "p6/tabs.pdf",
];

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("../fixtures/{name}")).unwrap()
}

/// Each page's decoded content streams, `None` where a stream does not decode.
fn snapshot(bytes: &[u8], password: &[u8]) -> Vec<Vec<Option<Vec<u8>>>> {
    let doc = Document::open_with_password(bytes, DocumentLimits::default(), password).unwrap();
    collect_pages(&doc, PageLimits::default())
        .unwrap()
        .iter()
        .map(|page| {
            page.contents
                .iter()
                .map(|&id| decode_document_stream(&doc, id, StreamLimits::default()).ok())
                .collect()
        })
        .collect()
}

fn assert_repairs(label: &str, original: &[u8], damaged: Vec<u8>, password: &[u8]) -> RepairReport {
    assert!(
        Document::open_with_password(&damaged, DocumentLimits::default(), password).is_err(),
        "{label}: the damage must break the strict open"
    );
    let repaired =
        rebuild(&damaged, RepairLimits::default()).unwrap_or_else(|e| panic!("{label}: {e}"));
    if let Ok(dir) = std::env::var("TPDF_REPAIR_DUMP") {
        let name: String = label
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        std::fs::write(format!("{dir}/{name}.damaged.pdf"), &damaged).unwrap();
        std::fs::write(format!("{dir}/{name}.repaired.pdf"), &repaired.bytes).unwrap();
    }
    let expected = snapshot(original, password);
    assert!(!expected.is_empty(), "{label}");
    assert_eq!(snapshot(&repaired.bytes, password), expected, "{label}");
    let tolerant = open_tolerant(damaged, DocumentLimits::default(), password).unwrap();
    assert_eq!(tolerant.bytes, repaired.bytes, "{label}");
    let report = tolerant.repair.expect("repaired");
    assert!(report.strict_error.is_some(), "{label}");
    assert_eq!(
        RepairReport {
            strict_error: None,
            ..report.clone()
        },
        repaired.report,
        "{label}"
    );
    report
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}
fn replace(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let at = find(bytes, from).unwrap_or_else(|| panic!("{:?}", String::from_utf8_lossy(from)));
    [&bytes[..at], to, &bytes[at + from.len()..]].concat()
}
fn replace_all(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while let Some(at) = find(rest, from) {
        out.extend_from_slice(&rest[..at]);
        out.extend_from_slice(to);
        rest = &rest[at + from.len()..];
    }
    out.extend_from_slice(rest);
    out
}

/// The offset after the last `startxref`, and where its digits are.
fn startxref(bytes: &[u8]) -> (usize, std::ops::Range<usize>) {
    let marker = rfind(bytes, b"startxref").unwrap() + 9;
    let start = marker + bytes[marker..].iter().position(u8::is_ascii_digit).unwrap();
    let end = start
        + bytes[start..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
    let offset = std::str::from_utf8(&bytes[start..end])
        .unwrap()
        .parse()
        .unwrap();
    (offset, start..end)
}

fn truncated_at_xref(bytes: &[u8]) -> Vec<u8> {
    bytes[..startxref(bytes).0].to_vec()
}
fn shifted_xref_offsets(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let mut changed = false;
    for i in 1..bytes.len().saturating_sub(18) {
        let row = &bytes[i..i + 18];
        if matches!(bytes[i - 1], b'\n' | b'\r')
            && row[..10].iter().all(u8::is_ascii_digit)
            && row[10] == b' '
            && row[11..16].iter().all(u8::is_ascii_digit)
            && &row[16..] == b" n"
        {
            let offset: u64 = std::str::from_utf8(&row[..10]).unwrap().parse().unwrap();
            out[i..i + 10].copy_from_slice(format!("{:010}", offset + 7).as_bytes());
            changed = true;
        }
    }
    assert!(changed);
    out
}
fn garbage_prefix(bytes: &[u8]) -> Vec<u8> {
    [b"\x00\xffjunk 7 0 R trailer\n%%EOF\n".as_slice(), bytes].concat()
}
fn startxref_nowhere(bytes: &[u8]) -> Vec<u8> {
    let (_, digits) = startxref(bytes);
    [&bytes[..digits.start], b"3", &bytes[digits.end..]].concat()
}
fn without_endobj(bytes: &[u8]) -> Vec<u8> {
    replace_all(bytes, b"endobj", b"")
}

#[test]
fn generic_damage_to_fixtures_is_repaired() {
    type Damage = fn(&[u8]) -> Vec<u8>;
    let damages: [(&str, Damage); 5] = [
        ("truncated at xref", truncated_at_xref),
        ("shifted xref offsets", shifted_xref_offsets),
        ("garbage prefix", garbage_prefix),
        ("startxref nowhere", startxref_nowhere),
        ("missing endobj", without_endobj),
    ];
    for name in FIXTURES {
        let original = fixture(name);
        for (damage, apply) in damages {
            let report = assert_repairs(
                &format!("{name} {damage}"),
                &original,
                apply(&original),
                b"",
            );
            assert!(report.objects_recovered > 0);
        }
    }
}

#[test]
fn encrypted_fixtures_with_broken_xref_keep_encryption() {
    for name in ["encrypted.pdf", "encrypted-aes256.pdf"] {
        let original = fixture(name);
        for (damage, damaged) in [
            ("shifted xref offsets", shifted_xref_offsets(&original)),
            ("startxref nowhere", startxref_nowhere(&original)),
            ("garbage prefix", garbage_prefix(&original)),
            ("missing endobj", without_endobj(&original)),
        ] {
            let label = format!("{name} {damage}");
            assert_repairs(&label, &original, damaged.clone(), USER);
            let repaired = rebuild(&damaged, RepairLimits::default()).unwrap();
            let error = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap_err();
            assert!(
                matches!(error.kind, DocumentErrorKind::PasswordRequired),
                "{label}"
            );
            let Err(RepairError::Document(error)) =
                open_tolerant(damaged, DocumentLimits::default(), b"wrong")
            else {
                panic!("{label}: wrong password accepted");
            };
            assert!(matches!(error.kind, DocumentErrorKind::PasswordRequired));
        }
    }
    // An encrypted stream with a wrong /Length is cut at `endstream`.
    let original = fixture("encrypted.pdf");
    let damaged = replace(&original, b"/Length 49", b"/Length 60");
    let report = assert_repairs("encrypted.pdf length", &original, damaged, USER);
    assert_eq!(report.streams_length_recovered, 1);
}

#[test]
fn wrong_and_missing_stream_lengths_are_recovered() {
    // The page content is Flate data followed by an EOL that /Length excludes;
    // the recovered length must exclude it too, or the strict decoder refuses
    // the trailing byte.
    let original = fixture("cjk.pdf");
    let at = find(&original, b"/Length 112/").unwrap();
    let eol = at + find(&original[at..], b"\nendstream").unwrap();
    let crlf = [&original[..eol], b"\r", &original[eol..]].concat();
    for (label, damaged) in [
        ("long", replace(&original, b"/Length 112/", b"/Length 150/")),
        (
            "short",
            replace(&original, b"/Length 112/", b"/Length 100/"),
        ),
        ("missing", replace(&original, b"/Length 112/", b"/")),
        (
            "negative",
            replace(&original, b"/Length 112/", b"/Length -5/"),
        ),
        (
            "unresolvable",
            replace(&original, b"/Length 112/", b"/Length 99 0 R/"),
        ),
        ("CRLF", replace(&crlf, b"/Length 112/", b"/Length 0/")),
    ] {
        let report = assert_repairs(&format!("cjk.pdf {label}"), &original, damaged, b"");
        assert_eq!(report.streams_length_recovered, 1, "{label}");
    }
    // hello.pdf's writer counts its EOL in /Length. Recovered data drops it,
    // as the specification says; the content is unchanged apart from that.
    let original = fixture("hello.pdf");
    let damaged = replace(&original, b"/Length 39", b"/Length 45");
    let repaired = rebuild(&damaged, RepairLimits::default()).unwrap();
    let mut expected = snapshot(&original, b"");
    assert_eq!(expected[0][0].as_mut().unwrap().pop(), Some(b'\n'));
    assert_eq!(snapshot(&repaired.bytes, b""), expected);

    // A resolvable indirect /Length is used even though `endstream` occurs
    // inside the data, as long as it lands on the real `endstream`.
    let content = b"BT /F1 36 Tf 72 700 Td (endstream) Tj ET";
    let file = format!(
        "%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
         2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n\
         3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>\nendobj\n\
         4 0 obj\n<< /Length 5 0 R >>\nstream\n{}\nendstream\nendobj\n5 0 obj\n{}\nendobj\n\
         trailer\n<< /Root 1 0 R >>\n%%EOF\n",
        String::from_utf8_lossy(content),
        content.len()
    );
    let repaired = rebuild(file.as_bytes(), RepairLimits::default()).unwrap();
    assert_eq!(repaired.report.streams_length_recovered, 0);
    assert_eq!(
        snapshot(&repaired.bytes, b""),
        vec![vec![Some(content.to_vec())]]
    );
}

#[test]
fn duplicate_keys_keep_the_last_value() {
    let original = fixture("hello.pdf");
    let damaged = replace(&original, b"/Pages 2 0 R", b"/Pages 9 0 R /Pages 2 0 R");
    let damaged = replace(
        &damaged,
        b"/Type /Page /Parent",
        b"/Type /Pages /Type /Page /Parent",
    );
    assert_repairs("hello.pdf duplicate keys", &original, damaged, b"");
}

/// Append an incremental update that replaces `changes` and points at the
/// file's newest xref section.
fn append_update(bytes: &[u8], changes: &[(u32, &[u8])]) -> Vec<u8> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let root = doc.root();
    let size = doc
        .objects()
        .map(|o| o.id.number())
        .chain(changes.iter().map(|c| c.0))
        .max()
        .unwrap()
        + 1;
    let mut out = bytes.to_vec();
    let mut rows = String::new();
    for &(number, body) in changes {
        rows.push_str(&format!("{number} 1\n{:010} 00000 n \n", out.len()));
        out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!(
            "xref\n{rows}trailer\n<< /Size {size} /Root {} {} R /Prev {} >>\nstartxref\n{xref}\n%%EOF\n",
            root.number(),
            root.generation(),
            startxref(bytes).0
        )
        .as_bytes(),
    );
    out
}

#[test]
fn incremental_update_with_broken_xref_keeps_the_newer_objects() {
    let original = fixture("hello.pdf");
    let content = b"BT /F1 24 Tf 72 600 Td (Updated) Tj ET";
    let stream = [
        format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(),
        content,
        b"\nendstream",
    ]
    .concat();
    let updated = append_update(&original, &[(5, &stream), (6, b"<< /Title (v2) >>")]);
    assert_eq!(snapshot(&updated, b""), vec![vec![Some(content.to_vec())]]);
    let section = rfind(&updated, b"\nxref\n").unwrap() + 1;
    let trailer = rfind(&updated, b"trailer").unwrap();
    for (label, damaged) in [
        (
            "keyword",
            [&updated[..section], b"xrex", &updated[section + 4..]].concat(),
        ),
        ("cut trailer", updated[..trailer + 12].to_vec()),
        ("no section", updated[..section].to_vec()),
    ] {
        let report = assert_repairs(&format!("update {label}"), &updated, damaged, b"");
        assert_eq!(report.objects_overridden, 1, "{label}");
        assert_eq!(report.objects_recovered, 6, "{label}");
    }
}

/// Rewrite an unencrypted file with every non-stream object in one Flate
/// object stream and an xref stream, as MuPDF and Acrobat write them.
fn with_object_stream(bytes: &[u8]) -> Vec<u8> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let largest = doc.objects().map(|o| o.id.number()).max().unwrap();
    let (container, xref) = (largest + 1, largest + 2);
    let mut rows = vec![[0_u64, 0, 0]; xref as usize + 1];
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let (mut header, mut members, mut index) = (Vec::new(), Vec::new(), 0);
    let serialize = |value: &Object| serialize_object(value, WriteLimits::default()).unwrap();
    for object in doc.objects() {
        let number = object.id.number();
        assert_eq!(object.id.generation(), 0);
        if let Some(data) = doc.stream_data(object.id).unwrap() {
            let Object::Dictionary(mut dictionary) = object.value.clone() else {
                panic!("stream dictionary");
            };
            dictionary.insert(b"Length".to_vec(), Object::Integer(data.len() as i64));
            rows[number as usize] = [1, out.len() as u64, 0];
            out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
            out.extend_from_slice(&serialize(&Object::Dictionary(dictionary)));
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(data);
            out.extend_from_slice(b"\nendstream\nendobj\n");
        } else {
            rows[number as usize] = [2, u64::from(container), index];
            header.extend_from_slice(format!("{number} {} ", members.len()).as_bytes());
            members.extend_from_slice(&serialize(&object.value));
            members.push(b'\n');
            index += 1;
        }
    }
    let payload = compress_to_vec_zlib(&[header.as_slice(), &members].concat(), 6);
    rows[container as usize] = [1, out.len() as u64, 0];
    out.extend_from_slice(
        format!(
            "{container} 0 obj\n<< /Type /ObjStm /N {index} /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
            header.len(),
            payload.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&payload);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let offset = out.len();
    rows[xref as usize] = [1, offset as u64, 0];
    let mut data = Vec::new();
    for [kind, field, generation] in rows {
        data.push(kind as u8);
        data.extend_from_slice(&(field as u32).to_be_bytes());
        data.extend_from_slice(&(generation as u16).to_be_bytes());
    }
    let root = doc.root();
    out.extend_from_slice(
        format!(
            "{xref} 0 obj\n<< /Type /XRef /Size {} /W [1 4 2] /Root {} 0 R /Length {} >>\nstream\n",
            xref + 1,
            root.number(),
            data.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{offset}\n%%EOF\n").as_bytes());
    out
}

#[test]
fn objects_only_in_an_object_stream_survive_a_broken_xref_stream() {
    for name in ["hello.pdf", "cjk.pdf", "nav.pdf", "form2.pdf"] {
        let original = fixture(name);
        let packed = with_object_stream(&original);
        assert_eq!(snapshot(&packed, b""), snapshot(&original, b""), "{name}");
        for (damage, damaged) in [
            ("widths", replace(&packed, b"/W [1 4 2]", b"/W [1 4 3]")),
            ("startxref nowhere", startxref_nowhere(&packed)),
            ("truncated at xref", truncated_at_xref(&packed)),
            ("garbage prefix", garbage_prefix(&packed)),
        ] {
            let report =
                assert_repairs(&format!("{name} objstm {damage}"), &original, damaged, b"");
            assert_eq!(report.object_streams_expanded, 1, "{name} {damage}");
            assert_eq!(report.objects_dropped, 0, "{name} {damage}");
        }
    }
}

#[test]
fn object_stream_members_compete_at_the_container_position() {
    let members = b"3 0 4 9 5 18 (member) (member) (member)";
    let file = [
        b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
          2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n\
          3 0 obj\n(before)\nendobj\n"
            .as_slice(),
        format!(
            "10 0 obj\n<< /Type /ObjStm /N 3 /First 13 /Length {} >>\nstream\n",
            members.len()
        )
        .as_bytes(),
        members,
        b"\nendstream\nendobj\n4 1 obj\n(after)\nendobj\n6 0 obj\n[3 0 R 4 0 R 5 2 R 7 0 R 10 0 R]\nendobj\n\
          trailer\n<< /Root 1 0 R /Info 7 0 R >>\n%%EOF\n",
    ]
    .concat();
    let repaired = rebuild(&file, RepairLimits::default()).unwrap();
    let doc = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap();
    let value = |number, generation| doc.object(ObjectId::new(number, generation).unwrap());
    assert_eq!(
        value(3, 0).unwrap().value,
        Object::String(b"member".to_vec())
    );
    assert_eq!(
        value(4, 1).unwrap().value,
        Object::String(b"after".to_vec())
    );
    assert!(value(4, 0).is_err() && value(10, 0).is_err());
    // Stale generations follow the surviving object; dangling references,
    // including the dropped container, become null.
    let reference =
        |number, generation| Object::Reference(ObjectId::new(number, generation).unwrap());
    assert_eq!(
        value(6, 0).unwrap().value,
        Object::Array(vec![
            reference(3, 0),
            reference(4, 1),
            reference(5, 0),
            Object::Null,
            Object::Null,
        ])
    );
    assert!(!doc.trailer().contains_key(b"Info".as_slice()));
    let report = repaired.report;
    assert_eq!(
        (report.objects_recovered, report.objects_overridden),
        (6, 2)
    );
    assert_eq!(
        (report.object_streams_expanded, report.references_repaired),
        (1, 4)
    );
}

#[test]
fn catalog_is_found_by_type_when_no_trailer_names_it() {
    let original = fixture("hello.pdf");
    let no_trailer = replace(&original, b"/Root 1 0 R", b"/Root 4 0 R");
    let report = assert_repairs(
        "hello.pdf wrong root",
        &original,
        without_endobj(&no_trailer),
        b"",
    );
    assert!(report.catalog_from_scan);
    let report = assert_repairs(
        "hello.pdf no trailer",
        &original,
        original[..rfind(&original, b"trailer").unwrap()].to_vec(),
        b"",
    );
    assert!(report.catalog_from_scan);
    let no_catalog = replace(&original, b"/Type /Catalog", b"/Type /Katalog");
    let no_catalog = replace(&no_catalog, b"/Pages 2 0 R >>", b">>");
    assert!(matches!(
        rebuild(&no_catalog, RepairLimits::default()),
        Err(RepairError::Invalid("document catalog"))
    ));
    for input in [b"".as_slice(), b"%PDF-1.7\n", b"not a pdf 1 0 obj"] {
        assert!(rebuild(input, RepairLimits::default()).is_err());
    }
}

#[test]
fn open_tolerant_keeps_valid_files_and_passwords() {
    for name in ["hello.pdf", "p0b/certified-p2-annot.pdf"] {
        let original = fixture(name);
        let opened = open_tolerant(original.clone(), DocumentLimits::default(), b"").unwrap();
        assert!(opened.repair.is_none());
        assert_eq!(opened.bytes, original);
    }
    let encrypted = fixture("encrypted.pdf");
    let Err(RepairError::Document(error)) =
        open_tolerant(encrypted.clone(), DocumentLimits::default(), b"")
    else {
        panic!("opened without a password");
    };
    assert!(matches!(error.kind, DocumentErrorKind::PasswordRequired));
    let opened = open_tolerant(encrypted.clone(), DocumentLimits::default(), USER).unwrap();
    assert!(opened.repair.is_none());
}

#[test]
fn limits_are_enforced() {
    let original = fixture("hello.pdf");
    let damaged = garbage_prefix(&original);
    let cases = [
        (
            RepairLimits {
                max_input_bytes: 100,
                ..RepairLimits::default()
            },
            "input bytes",
        ),
        (
            RepairLimits {
                max_objects: 3,
                ..RepairLimits::default()
            },
            "objects",
        ),
        (
            RepairLimits {
                max_values: 20,
                ..RepairLimits::default()
            },
            "values",
        ),
    ];
    for (limits, reason) in cases {
        let error = rebuild(&damaged, limits).unwrap_err();
        assert!(
            matches!(error, RepairError::LimitExceeded(r) if r == reason),
            "{reason}: {error}"
        );
    }
    let packed = startxref_nowhere(&with_object_stream(&original));
    let error = rebuild(
        &packed,
        RepairLimits {
            max_decoded_bytes: 10,
            ..RepairLimits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RepairError::LimitExceeded("object-stream bytes")
    ));
    let small = RepairLimits {
        max_output_bytes: 200,
        ..RepairLimits::default()
    };
    assert!(matches!(
        rebuild(&damaged, small),
        Err(RepairError::Serialize(_))
    ));
}

#[test]
fn adversarial_inputs_scan_in_linear_time() {
    let repeat = |unit: &[u8], count: usize| unit.repeat(count);
    let mut comment_chain = b"%PDF-1.7\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n".to_vec();
    comment_chain.extend(repeat(b"endstream%9 0 obj<<>>stream ", 100_000));
    let inputs = [
        repeat(b"1 0 obj (", 300_000),
        repeat(b"1 0 obj <</Length 999999999>> stream\n", 100_000),
        [
            repeat(b"1 0 obj <<>> stream\n", 100_000),
            b"endstream".to_vec(),
        ]
        .concat(),
        [repeat(b"1 0 obj [", 100_000), repeat(b" ", 2_000_000)].concat(),
        [b"%PDF-1.7\n".to_vec(), repeat(b"          obj", 300_000)].concat(),
        repeat(b"trailer << /Root ", 200_000),
        comment_chain,
    ];
    for input in inputs {
        let start = Instant::now();
        let _ = rebuild(&input, RepairLimits::default());
        assert!(
            start.elapsed().as_secs() < 10,
            "{} bytes took {:?}",
            input.len(),
            start.elapsed()
        );
    }
}

fn random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn mutated_fixtures_never_panic_and_rebuild_to_strict_files() {
    let bases = [
        fixture("hello.pdf"),
        fixture("form2.pdf"),
        fixture("p0b/certified-p2-annot.pdf"),
        fixture("encrypted.pdf"),
        with_object_stream(&fixture("nav.pdf")),
    ];
    let tokens: [&[u8]; 8] = [
        b"endstream",
        b"endobj",
        b" 0 obj ",
        b"stream\n",
        b"<<",
        b">>",
        b"trailer",
        b"/Length 5 0 R",
    ];
    let mut seed = 0x7265_7061_6972_u64;
    let mut rebuilt = 0;
    for base in &bases {
        for _ in 0..300 {
            let mut bytes = base.clone();
            for _ in 0..1 + random(&mut seed) % 6 {
                let len = bytes.len().max(1);
                let at = (random(&mut seed) as usize) % len;
                match random(&mut seed) % 6 {
                    0 => {
                        if let Some(byte) = bytes.get_mut(at) {
                            *byte = random(&mut seed) as u8;
                        }
                    }
                    1 => {
                        let end = (at + 1 + (random(&mut seed) as usize) % 64).min(bytes.len());
                        bytes.drain(at.min(end)..end);
                    }
                    2 => {
                        let token = tokens[(random(&mut seed) as usize) % tokens.len()];
                        let at = at.min(bytes.len());
                        bytes.splice(at..at, token.iter().copied());
                    }
                    3 => bytes.truncate(at),
                    4 => {
                        let end = (at + (random(&mut seed) as usize) % 256).min(bytes.len());
                        let copy = bytes[at.min(end)..end].to_vec();
                        bytes.extend_from_slice(&copy);
                    }
                    _ => {
                        if let Some(byte) = bytes.get_mut(at) {
                            *byte = b"0123456789 \n/<>[]()R"[(random(&mut seed) as usize) % 20];
                        }
                    }
                }
            }
            if let Ok(repaired) = rebuild(&bytes, RepairLimits::default()) {
                rebuilt += 1;
                if let Err(error) = Document::open(&repaired.bytes, DocumentLimits::default()) {
                    assert!(
                        matches!(error.kind, DocumentErrorKind::PasswordRequired),
                        "{error}"
                    );
                }
            }
            let _ = open_tolerant(bytes, DocumentLimits::default(), USER);
        }
    }
    assert!(rebuilt > 1000, "{rebuilt}");
}
