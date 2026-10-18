// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tolerant rebuild of damaged variants of the fixtures. Each damaged file must
//! fail the strict open, and its repair must open strictly with the same pages
//! and decoded content streams as the undamaged file, and be reported damaged.
//! Valid layouts the strict reader refuses (linearized, hybrid) are rebuilt
//! from their own xref and not reported damaged. Set TPDF_REPAIR_DUMP to a
//! directory to write each damaged and repaired pair for comparison with
//! another reader.
#[path = "support/pages.rs"]
mod pages;
use std::collections::BTreeMap;
use std::time::Instant;

use miniz_oxide::deflate::compress_to_vec_zlib;
use tandem_pdf_engine::document::{Document, DocumentErrorKind, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::repair::{RepairError, RepairLimits, RepairReport, open_tolerant, rebuild};
use tandem_pdf_engine::serialize::{WriteLimits, serialize_object};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::stream_object::decode_document_stream;
use tandem_pdf_engine::writer::{DocumentWriteLimits, rewrite_reachable};

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
    pages::page_ids(&doc)
        .into_iter()
        .map(|page| {
            pages::contents(&doc, page)
                .into_iter()
                .map(|id| decode_document_stream(&doc, id, StreamLimits::default()).ok())
                .collect()
        })
        .collect()
}

fn assert_repairs(label: &str, original: &[u8], damaged: Vec<u8>, password: &[u8]) -> RepairReport {
    let report = assert_rebuilds(label, original, damaged, password);
    assert!(report.damaged, "{label}");
    report
}

/// As `assert_repairs`, without the `damaged` flag (spec 03 section 5.2
/// decides it per case).
fn assert_rebuilds(
    label: &str,
    original: &[u8],
    damaged: Vec<u8>,
    password: &[u8],
) -> RepairReport {
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
    let tolerant = open_tolerant(damaged.clone(), DocumentLimits::default(), password).unwrap();
    assert_eq!(tolerant.bytes, repaired.bytes, "{label}");
    assert!(!tolerant.needs_password, "{label}");
    assert_eq!(tolerant.original, Some(damaged), "{label}");
    assert!(tolerant.strict_error.is_some(), "{label}");
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

/// Where the last complete revision of `bytes` ends (after its %%EOF),
/// when objects follow it and the incremental writer's locator finds its
/// `startxref` (spec 03 section 4.3).
fn complete_revision(bytes: &[u8]) -> Option<usize> {
    tandem_pdf_engine::incremental::startxref(bytes)?;
    let marker = rfind(bytes, b"startxref")?;
    let end = marker + find(&bytes[marker..], b"%%EOF")? + 5;
    bytes[end..].windows(3).any(|w| w == b"obj").then_some(end)
}

fn truncated_at_xref(bytes: &[u8]) -> Vec<u8> {
    bytes[..startxref(bytes).0].to_vec()
}
fn shifted_xref_offsets(bytes: &[u8]) -> Vec<u8> {
    xref_offsets_moved(bytes, 7)
}
fn xref_offsets_moved(bytes: &[u8], delta: i64) -> Vec<u8> {
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
            let offset: i64 = std::str::from_utf8(&row[..10]).unwrap().parse().unwrap();
            out[i..i + 10].copy_from_slice(format!("{:010}", offset + delta).as_bytes());
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
fn patch(bytes: &mut [u8], at: usize, with: &[u8]) {
    bytes[at..at + with.len()].copy_from_slice(with);
}

/// `bytes` with its last xref section rebuilt from the object headers at line
/// starts, so an edit that moves objects leaves an intact file.
fn fresh_xref(bytes: &[u8]) -> Vec<u8> {
    let section = rfind(bytes, b"\nxref\n").unwrap() + 1;
    let trailer = rfind(bytes, b"trailer").unwrap() + 7;
    let trailer = bytes[trailer..rfind(bytes, b"startxref").unwrap()].to_vec();
    let mut out = bytes[..section].to_vec();
    let mut offsets = BTreeMap::new();
    let mut line = 0;
    for (at, &byte) in out.iter().enumerate() {
        if byte != b'\n' {
            continue;
        }
        let words: Vec<&str> = std::str::from_utf8(&out[line..at])
            .map_or(Vec::new(), |text| text.split_whitespace().collect());
        if let [number, "0", "obj"] = words.as_slice() {
            if let Ok(number) = number.parse::<u32>() {
                offsets.insert(number, line);
            }
        }
        line = at + 1;
    }
    let size = offsets.keys().next_back().unwrap() + 1;
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for number in 1..size {
        out.extend_from_slice(
            match offsets.get(&number) {
                Some(offset) => format!("{offset:010} 00000 n \n").into_bytes(),
                None => b"0000000000 00000 f \n".to_vec(),
            }
            .as_slice(),
        );
    }
    out.extend_from_slice(b"trailer");
    out.extend_from_slice(&trailer);
    out.extend_from_slice(format!("startxref\n{xref}\n%%EOF\n").as_bytes());
    out
}

type Damage = fn(&[u8]) -> Vec<u8>;

const DAMAGES: [(&str, Damage); 5] = [
    ("truncated at xref", truncated_at_xref),
    ("shifted xref offsets", shifted_xref_offsets),
    ("garbage prefix", garbage_prefix),
    ("startxref nowhere", startxref_nowhere),
    ("missing endobj", without_endobj),
];

#[test]
fn generic_damage_to_fixtures_is_repaired() {
    for name in FIXTURES {
        let original = fixture(name);
        for (damage, apply) in DAMAGES {
            let damaged = apply(&original);
            // An earlier revision the locator finds is covered by
            // `damage_leaving_an_earlier_revision_reads_that_revision`.
            if complete_revision(&damaged).is_some() {
                continue;
            }
            let report = assert_repairs(&format!("{name} {damage}"), &original, damaged, b"");
            assert!(report.objects_recovered > 0);
        }
    }
}

/// Cut at its last xref section, a file with an earlier revision ends with
/// that revision's `startxref` and the update's objects: the earlier
/// revision is read (spec 03 section 4.3).
#[test]
fn damage_leaving_an_earlier_revision_reads_that_revision() {
    for name in FIXTURES {
        let original = fixture(name);
        for (damage, apply) in DAMAGES {
            let damaged = apply(&original);
            if let Some(end) = complete_revision(&damaged) {
                let repaired = rebuild(&damaged, RepairLimits::default()).unwrap();
                let previous = snapshot(&damaged[..end], b"");
                assert_eq!(snapshot(&repaired.bytes, b""), previous, "{name} {damage}");
                assert!(!repaired.report.damaged, "{name} {damage}");
            }
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
            // A wrong password still yields the rebuilt bytes, locked.
            let locked = open_tolerant(damaged, DocumentLimits::default(), b"wrong").unwrap();
            assert!(locked.needs_password, "{label}");
            assert_eq!(locked.bytes, repaired.bytes, "{label}");
            assert!(locked.repair.is_some_and(|r| r.damaged), "{label}");
            Document::open_with_password(&locked.bytes, DocumentLimits::default(), USER).unwrap();
        }
    }
    // An encrypted stream with a wrong /Length is cut at `endstream`. The
    // xref is intact; whether that is damage is checked in
    // `a_wrong_length_on_an_intact_xref_is_not_damage`.
    let original = fixture("encrypted.pdf");
    let damaged = replace(&original, b"/Length 49", b"/Length 60");
    let report = assert_rebuilds("encrypted.pdf length", &original, damaged, USER);
    assert_eq!(report.streams_length_recovered, 1);

    // Without any trailer, the encryption dictionary is found by the scan.
    // AES-256 does not hash the file identifier, so the file is recovered.
    let original = fixture("encrypted-aes256.pdf");
    let damaged = truncated_at_xref(&original);
    assert_repairs("encrypted-aes256.pdf truncated", &original, damaged, USER);
    // RC4 keys hash the identifier, which was lost with the trailer; a rebuild
    // would only ask for a password forever, or pass ciphertext as plain text.
    let damaged = truncated_at_xref(&fixture("encrypted.pdf"));
    assert!(matches!(
        rebuild(&damaged, RepairLimits::default()),
        Err(RepairError::Invalid(_))
    ));
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
        // "long" and "short" keep every offset, so the xref stays intact;
        // the others shift offsets and are damage (spec 03 section 5.2).
        let report = assert_rebuilds(&format!("cjk.pdf {label}"), &original, damaged, b"");
        assert_eq!(report.streams_length_recovered, 1, "{label}");
        if !matches!(label, "long" | "short") {
            assert!(report.damaged, "{label}");
        }
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

/// An update whose own xref section is damaged (its `startxref` names it)
/// is scanned, and its objects win. One whose section or trailer was cut
/// off, so that the last `startxref` names the previous section, leaves
/// that last complete revision: the objects after it are in no revision
/// (spec 03 section 4.3, experiments 22 and 24).
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
    let damaged = [&updated[..section], b"xrex", &updated[section + 4..]].concat();
    let report = assert_repairs("update keyword", &updated, damaged, b"");
    assert_eq!(report.objects_overridden, 1);
    assert_eq!(report.objects_recovered, 6);
    for (label, damaged) in [
        ("cut trailer", updated[..trailer + 12].to_vec()),
        ("no section", updated[..section].to_vec()),
    ] {
        assert!(Document::open(&damaged, DocumentLimits::default()).is_err());
        let repaired = rebuild(&damaged, RepairLimits::default()).unwrap();
        assert_eq!(
            snapshot(&repaired.bytes, b""),
            snapshot(&original, b""),
            "{label}"
        );
        let report = &repaired.report;
        assert!(report.xref_intact && !report.damaged, "{label}");
        assert_eq!(report.xref_sections, 1, "{label}");
        // Object 5's update and the new object 6 are in no revision.
        assert_eq!(report.objects_overridden, 2, "{label}");
    }
}

/// Append `doc`'s streams as objects and every other object to one Flate
/// object stream numbered `container`, filling `rows` ([type, field, index]).
/// The member text is padded until the compressed payload ends in `last`.
fn pack_into(
    doc: &Document<'_>,
    out: &mut Vec<u8>,
    rows: &mut [[u64; 3]],
    container: u32,
    last: Option<u8>,
) {
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
    let mut payload = compress_to_vec_zlib(&[header.as_slice(), &members].concat(), 6);
    // A tab moves the Adler-32 low byte by 9, reaching every value.
    while last.is_some_and(|last| payload.last() != Some(&last)) {
        members.push(b'\t');
        payload = compress_to_vec_zlib(&[header.as_slice(), &members].concat(), 6);
    }
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
}

/// Xref-stream rows with /W [1 4 2].
fn xref_rows(rows: &[[u64; 3]]) -> Vec<u8> {
    let mut data = Vec::new();
    for &[kind, field, generation] in rows {
        data.push(kind as u8);
        data.extend_from_slice(&(field as u32).to_be_bytes());
        data.extend_from_slice(&(generation as u16).to_be_bytes());
    }
    data
}

/// Rewrite an unencrypted file with every non-stream object in one Flate
/// object stream and an xref stream, as common writers lay them out.
fn with_object_stream(bytes: &[u8]) -> Vec<u8> {
    packed(bytes, None)
}

fn packed(bytes: &[u8], last: Option<u8>) -> Vec<u8> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let largest = doc.objects().map(|o| o.id.number()).max().unwrap();
    let (container, xref) = (largest + 1, largest + 2);
    let mut rows = vec![[0_u64, 0, 0]; xref as usize + 1];
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    pack_into(&doc, &mut out, &mut rows, container, last);
    let offset = out.len();
    rows[xref as usize] = [1, offset as u64, 0];
    let data = xref_rows(&rows);
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

/// Rewrite an unencrypted file in a linearized layout: the parameter
/// dictionary, a first-page xref stream whose /Prev points forward to the
/// main xref stream, the streams, a hint stream, and an object stream.
fn linearized(bytes: &[u8]) -> Vec<u8> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let n = doc.objects().map(|o| o.id.number()).max().unwrap();
    let (container, main, lin, first, hint) = (n + 1, n + 2, n + 3, n + 4, n + 5);
    let mut rows = vec![[0_u64, 0, 0]; hint as usize + 1];
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    rows[lin as usize] = [1, out.len() as u64, 0];
    out.extend_from_slice(
        format!(
            "{lin} 0 obj\n<< /Linearized 1 /L {} /H [0000000000 16] /O 3 /E 0 /N 1 /T 0 >>\nendobj\n",
            bytes.len()
        )
        .as_bytes(),
    );
    let hint_at = rfind(&out, b"0000000000").unwrap();
    let first_at = out.len();
    rows[first as usize] = [1, first_at as u64, 0];
    out.extend_from_slice(
        format!(
            "{first} 0 obj\n<< /Type /XRef /Size {} /Index [{lin} 3] /W [1 4 2] /Root {} 0 R /Prev 0000000000 /Length 21 >>\nstream\n",
            hint + 1,
            doc.root().number()
        )
        .as_bytes(),
    );
    let prev_at = rfind(&out, b"0000000000").unwrap();
    let first_rows = out.len();
    out.extend_from_slice(&[0; 21]);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let hint_offset = out.len();
    rows[hint as usize] = [1, hint_offset as u64, 0];
    out.extend_from_slice(format!("{hint} 0 obj\n<< /S 0 /Length 16 >>\nstream\n").as_bytes());
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    pack_into(&doc, &mut out, &mut rows, container, None);
    let main_offset = out.len();
    rows[main as usize] = [1, main_offset as u64, 0];
    let data = xref_rows(&rows[..lin as usize]);
    out.extend_from_slice(
        format!(
            "{main} 0 obj\n<< /Type /XRef /Size {lin} /W [1 4 2] /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    out.extend_from_slice(
        format!("\nendstream\nendobj\nstartxref\n{first_at}\n%%EOF\n").as_bytes(),
    );
    patch(&mut out, hint_at, format!("{hint_offset:010}").as_bytes());
    patch(&mut out, prev_at, format!("{main_offset:010}").as_bytes());
    let first_data = xref_rows(&rows[lin as usize..]);
    patch(&mut out, first_rows, &first_data);
    out
}

/// Rewrite an unencrypted file as a hybrid: a classic table lists the
/// streams and marks the compressed objects free, and its /XRefStm names an
/// xref stream that locates them in an object stream.
fn hybrid(bytes: &[u8]) -> Vec<u8> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let n = doc.objects().map(|o| o.id.number()).max().unwrap();
    let (container, stream) = (n + 1, n + 2);
    let mut rows = vec![[0_u64, 0, 0]; stream as usize + 1];
    let mut out = b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n".to_vec();
    pack_into(&doc, &mut out, &mut rows, container, None);
    let stream_offset = out.len();
    rows[stream as usize] = [1, stream_offset as u64, 0];
    let compressed: Vec<[u64; 3]> = rows
        .iter()
        .map(|&row| if row[0] == 2 { row } else { [0, 0, 0] })
        .collect();
    let data = xref_rows(&compressed);
    out.extend_from_slice(
        format!(
            "{stream} 0 obj\n<< /Type /XRef /Size {} /W [1 4 2] /Length {} >>\nstream\n",
            stream + 1,
            data.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", stream + 1).as_bytes());
    for row in &rows[1..] {
        out.extend_from_slice(
            match row[0] {
                1 => format!("{:010} 00000 n \n", row[1]).into_bytes(),
                _ => b"0000000000 00000 f \n".to_vec(),
            }
            .as_slice(),
        );
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root {} 0 R /XRefStm {stream_offset} >>\nstartxref\n{xref}\n%%EOF\n",
            stream + 1,
            doc.root().number()
        )
        .as_bytes(),
    );
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
    // A file without any catalog: `a_catalog_is_synthesized_when_none_exists`.
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
    let locked = open_tolerant(encrypted.clone(), DocumentLimits::default(), b"").unwrap();
    assert!(locked.needs_password && locked.repair.is_none());
    assert_eq!(locked.bytes, encrypted);
    let opened = open_tolerant(encrypted.clone(), DocumentLimits::default(), USER).unwrap();
    assert!(!opened.needs_password && opened.repair.is_none() && opened.original.is_none());

    // A strict limit is the caller's decision, not damage: the file is not
    // flattened into one revision behind the caller's back.
    let original = fixture("hello.pdf");
    let once = append_update(&original, &[(6, b"<< /Title (a) >>")]);
    let twice = append_update(&once, &[(6, b"<< /Title (b) >>")]);
    let limits = DocumentLimits {
        max_revisions: 2,
        ..DocumentLimits::default()
    };
    let Err(RepairError::Document(error)) = open_tolerant(twice.clone(), limits, b"") else {
        panic!("a limit refusal was repaired");
    };
    assert!(matches!(error.kind, DocumentErrorKind::LimitExceeded(_)));
    let opened = open_tolerant(twice.clone(), DocumentLimits::default(), b"").unwrap();
    assert!(opened.repair.is_none() && opened.bytes == twice);
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
        // Every parse stops inside a string at the next keyword and is retried
        // past it; the retries share one pass over the input.
        repeat(b"1 0 obj (trailer << ", 150_000),
        [
            b"%PDF-1.7\n1 0 obj (".to_vec(),
            repeat(b"2 0 obj (x) endobj ", 150_000),
        ]
        .concat(),
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

#[test]
fn object_stream_members_sharing_an_offset_are_parsed_once() {
    // Every member points at one region: a megabyte-long unterminated string,
    // or fifteen one-megabyte strings. Parsing the region per member took
    // seconds per thousand members and a gigabyte per hundred.
    let members = 200_000;
    let header = b"9 0 ".repeat(members);
    let string = |byte: u8| [b"(".as_slice(), &vec![byte; 1_000_000], b")"].concat();
    let unterminated = [b"(".as_slice(), &vec![b'A'; 1 << 20]].concat();
    let strings = [
        b"[".to_vec(),
        (0..15).flat_map(|i| string(b'a' + i)).collect(),
        b"]".to_vec(),
    ]
    .concat();
    let original = fixture("hello.pdf");
    for (label, region) in [("unterminated", unterminated), ("strings", strings)] {
        let payload = compress_to_vec_zlib(&[header.as_slice(), &region].concat(), 6);
        let at = rfind(&original, b"\nxref\n").unwrap() + 1;
        let file = [
            &original[..at],
            format!(
                "10 0 obj\n<< /Type /ObjStm /N {members} /First {} /Filter /FlateDecode /Length {} >>\nstream\n",
                header.len(),
                payload.len()
            )
            .as_bytes(),
            &payload,
            b"\nendstream\nendobj\n",
        ]
        .concat();
        let start = Instant::now();
        let repaired = rebuild(&file, RepairLimits::default()).unwrap();
        assert!(
            start.elapsed().as_secs() < 10,
            "{label}: {:?}",
            start.elapsed()
        );
        let report = repaired.report;
        assert_eq!(report.object_streams_expanded, 1, "{label}");
        assert!(report.objects_dropped >= members - 1, "{label}");
        let doc = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap();
        assert_eq!(
            doc.object(ObjectId::new(9, 0).unwrap()).is_ok(),
            label == "strings"
        );
    }

    // Offsets out of order are still separate regions.
    let members = b"3 7 4 0 (four) [(three)]";
    let file = [
        &original[..rfind(&original, b"\nxref\n").unwrap() + 1],
        format!(
            "10 0 obj\n<< /Type /ObjStm /N 2 /First 8 /Length {} >>\nstream\n",
            members.len()
        )
        .as_bytes(),
        members,
        b"\nendstream\nendobj\n",
    ]
    .concat();
    let repaired = rebuild(&file, RepairLimits::default()).unwrap();
    let doc = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap();
    let value = |number| {
        doc.object(ObjectId::new(number, 0).unwrap())
            .unwrap()
            .value
            .clone()
    };
    assert_eq!(value(4), Object::String(b"four".to_vec()));
    assert_eq!(
        value(3),
        Object::Array(vec![Object::String(b"three".to_vec())])
    );
}

/// Each page's decoded content streams by page object.
fn contents_by_page(bytes: &[u8]) -> BTreeMap<ObjectId, Vec<Option<Vec<u8>>>> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    pages::page_ids(&doc)
        .into_iter()
        .map(|page| {
            let contents = pages::contents(&doc, page)
                .into_iter()
                .map(|id| decode_document_stream(&doc, id, StreamLimits::default()).ok())
                .collect();
            (page, contents)
        })
        .collect()
}

#[test]
fn truncated_bodies_keep_their_surviving_pages() {
    // A cut download loses pages and content streams; the rest of the page
    // tree must still pass the strict page walk.
    let original = fixture("large.pdf");
    let expected = contents_by_page(&original);
    for percent in [50, 75, 90] {
        let damaged = original[..original.len() * percent / 100].to_vec();
        let tolerant = open_tolerant(damaged, DocumentLimits::default(), b"").unwrap();
        let report = tolerant.repair.unwrap();
        assert!(report.page_tree_repaired && report.damaged, "{percent}%");
        let pages = contents_by_page(&tolerant.bytes);
        assert!(pages.len() > 200 && pages.len() < 500, "{percent}%");
        // Only the stream the cut went through may differ.
        let differing = pages
            .iter()
            .filter(|(id, contents)| expected[id] != **contents)
            .count();
        assert!(differing <= 1, "{percent}%: {differing}");
    }

    // A dangling content stream is dropped from the /Contents list.
    let hello = fixture("hello.pdf");
    let damaged = startxref_nowhere(&replace(
        &hello,
        b"/Contents 5 0 R",
        b"/Contents [5 0 R 9 0 R]",
    ));
    let report = assert_repairs("hello.pdf dangling contents", &hello, damaged, b"");
    assert_eq!(report.references_repaired, 1);

    // Without the root Pages node, the pages hang off a new one.
    let damaged = replace(&hello, b"2 0 obj", b"2 0 xbj");
    let report = assert_repairs("hello.pdf lost page tree", &hello, damaged.clone(), b"");
    assert!(report.page_tree_repaired);
    // A page with no MediaBox to inherit gets US Letter (spec 03 section
    // 4.10 rule 5).
    let damaged = replace(&damaged, b" /MediaBox [0 0 612 792]", b"");
    let repaired = rebuild(&damaged, RepairLimits::default()).unwrap();
    let doc = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap();
    assert_eq!(
        pages::media_box(&doc, pages::page_ids(&doc)[0]),
        Some(&Object::Array(
            [0, 0, 612, 792].map(Object::Integer).to_vec()
        ))
    );
}

/// A content stream whose Flate data ends in `last`. Each trailing tab moves
/// the Adler-32 low byte by 9, reaching every value.
fn flate_ending_with(last: u8) -> (Vec<u8>, Vec<u8>) {
    (0..256)
        .map(|pad| format!("BT /F1 24 Tf 72 700 Td (Ends) Tj ET{}", "\t".repeat(pad)).into_bytes())
        .map(|content| {
            let data = compress_to_vec_zlib(&content, 6);
            (content, data)
        })
        .find(|(_, data)| data.last() == Some(&last))
        .unwrap()
}

#[test]
fn recovered_lengths_keep_eol_bytes_that_are_data() {
    // A CR or LF before `endstream` may end the data (a Flate checksum byte)
    // rather than belong to the EOL; only the right length inflates.
    let original = fixture("hello.pdf");
    let at = find(&original, b"5 0 obj\n").unwrap() + 8;
    let end = find(&original, b"endstream").unwrap() + 9;
    for (last, eol) in [
        (b'\r', b"\n".as_slice()),
        (b'\r', b""),
        (b'\n', b""),
        (b'\n', b"\r\n"),
    ] {
        let label = format!("{last} before {eol:?}");
        let (content, data) = flate_ending_with(last);
        let file = |length: usize| {
            let stream = [
                format!("<< /Filter /FlateDecode /Length {length} >>\nstream\n").as_bytes(),
                &data,
                eol,
                b"endstream",
            ]
            .concat();
            fresh_xref(&[&original[..at], &stream, &original[end..]].concat())
        };
        let intact = file(data.len());
        assert_eq!(snapshot(&intact, b""), vec![vec![Some(content)]], "{label}");
        // Each file has its own intact xref; the damage flag is checked in
        // `a_wrong_length_on_an_intact_xref_is_not_damage`.
        let report = assert_rebuilds(&label, &intact, file(data.len() + 7), b"");
        assert_eq!(report.streams_length_recovered, 1, "{label}");
    }

    // The same for an object stream holding the catalog.
    let packed = packed(&original, Some(b'\r'));
    let length = find(&packed, b"/FlateDecode /Length ").unwrap() + 21;
    let digits = packed[length..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    let damaged = [&packed[..length], b"1", &packed[length + digits..]].concat();
    let report = assert_repairs("objstm ending in CR", &original, damaged, b"");
    assert_eq!(report.object_streams_expanded, 1);
}

#[test]
fn keywords_inside_strings_do_not_cut_objects() {
    let original = fixture("hello.pdf");
    for note in [
        b"/Note (see the trailer << here) ".as_slice(),
        b"/Note (see 12 0 obj above) ",
        b"/Note (Hitch the trailer to the truck) ",
    ] {
        let label = String::from_utf8_lossy(note).into_owned();
        let edited = fresh_xref(&replace(
            &original,
            b"/Type /Font ",
            &[b"/Type /Font ", note].concat(),
        ));
        let damaged = startxref_nowhere(&edited);
        let report = assert_repairs(&label, &edited, damaged.clone(), b"");
        assert_eq!(report.objects_dropped, 0, "{label}");
        let repaired = rebuild(&damaged, RepairLimits::default()).unwrap();
        let doc = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap();
        let Object::Dictionary(font) = &doc.object(ObjectId::new(4, 0).unwrap()).unwrap().value
        else {
            panic!("{label}: font");
        };
        let text = &note[7..note.len() - 2];
        assert_eq!(
            font.get(b"Note".as_slice()),
            Some(&Object::String(text.to_vec())),
            "{label}"
        );
    }
}

#[test]
fn long_strings_stay_within_the_token_limit() {
    // Hex would double a 600 KB title past the 1 MiB token limit.
    let unit = b"Long (title) \\r\\\\ ";
    let title = [b"\\)".as_slice(), &unit.repeat(40_000)].concat();
    let original = fixture("hello.pdf");
    let at = rfind(&original, b"\nxref\n").unwrap() + 1;
    let file = [
        &original[..at],
        b"6 0 obj\n<< /Title (",
        &title,
        b") >>\nendobj\n",
        &original[at..],
    ]
    .concat();
    let file = fresh_xref(&replace(
        &file,
        b"/Size 6 /Root 1 0 R",
        b"/Size 7 /Root 1 0 R /Info 6 0 R",
    ));
    let doc = Document::open(&file, DocumentLimits::default()).unwrap();
    let expected = doc
        .object(ObjectId::new(6, 0).unwrap())
        .unwrap()
        .value
        .clone();
    let Object::Dictionary(info) = &expected else {
        panic!("info");
    };
    assert!(matches!(info.get(b"Title".as_slice()), Some(Object::String(s)) if s.len() > 590_000));
    let damaged = truncated_at_xref(&file);
    let repaired = rebuild(&damaged, RepairLimits::default()).unwrap();
    let doc = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap();
    assert_eq!(
        doc.object(ObjectId::new(6, 0).unwrap()).unwrap().value,
        expected
    );
    assert!(repaired.bytes.len() < file.len());
}

#[test]
fn valid_structures_the_strict_reader_refuses_are_not_damaged() {
    // Rebuilt, but not damaged: other viewers read these without repair, and
    // the app must not mark them repaired (spec 03 section 5.2).
    let hello = fixture("hello.pdf");
    let cases = [
        (
            "duplicate keys",
            hello.clone(),
            fresh_xref(&replace(
                &hello,
                b"/Type /Page /Parent",
                b"/Type /Pages /Type /Page /Parent",
            )),
        ),
        (
            "xref offsets at the EOL before each object",
            hello.clone(),
            xref_offsets_moved(&hello, -1),
        ),
        ("linearized hello", hello.clone(), linearized(&hello)),
        (
            "linearized nav",
            fixture("nav.pdf"),
            linearized(&fixture("nav.pdf")),
        ),
        (
            "linearized cjk",
            fixture("cjk.pdf"),
            linearized(&fixture("cjk.pdf")),
        ),
        (
            "hybrid cjk",
            fixture("cjk.pdf"),
            hybrid(&fixture("cjk.pdf")),
        ),
        (
            "hybrid nav",
            fixture("nav.pdf"),
            hybrid(&fixture("nav.pdf")),
        ),
    ];
    for (label, original, input) in cases {
        assert!(
            Document::open(&input, DocumentLimits::default()).is_err(),
            "{label}: the strict reader accepts it"
        );
        let opened = open_tolerant(input, DocumentLimits::default(), b"").unwrap();
        let report = opened.repair.unwrap();
        assert!(report.xref_intact && !report.damaged, "{label}: {report:?}");
        assert_eq!(
            report.linearized,
            label.starts_with("linearized"),
            "{label}"
        );
        assert_eq!(
            snapshot(&opened.bytes, b""),
            snapshot(&original, b""),
            "{label}"
        );
        let doc = Document::open(&opened.bytes, DocumentLimits::default()).unwrap();
        rewrite_reachable(&doc, DocumentWriteLimits::default()).unwrap();
        // Exactly the original objects: no parameter dictionary, hint stream,
        // object stream or xref stream.
        let expected = Document::open(&original, DocumentLimits::default()).unwrap();
        assert_eq!(doc.object_count(), expected.object_count(), "{label}");
    }
    // Damage on top of a valid structure is still damage.
    let damaged = startxref_nowhere(&linearized(&hello));
    let report = assert_repairs("linearized hello truncated", &hello, damaged, b"");
    assert!(report.linearized && !report.xref_intact);
}

/// Owner decision R1 (spec 03 section 5.2, Q1): a wrong `/Length`, or a data
/// byte before `endstream` that the EOL rule must keep, on a file whose xref
/// is intact is not damage; the xref is followed and the original bytes can
/// still be extended.
#[test]
fn a_wrong_length_on_an_intact_xref_is_not_damage() {
    let intact = |label: &str, original: &[u8], damaged: Vec<u8>, password: &[u8]| {
        let report = assert_rebuilds(label, original, damaged, password);
        assert!(!report.damaged && report.xref_intact, "{label}: {report:?}");
        assert_eq!(report.streams_length_recovered, 1, "{label}");
    };
    let original = fixture("encrypted.pdf");
    let damaged = replace(&original, b"/Length 49", b"/Length 60");
    intact("encrypted.pdf length", &original, damaged, USER);

    let original = fixture("cjk.pdf");
    for (label, length) in [("long", b"/Length 150/"), ("short", b"/Length 100/")] {
        let damaged = replace(&original, b"/Length 112/", length);
        intact(&format!("cjk.pdf {label}"), &original, damaged, b"");
    }

    let original = fixture("hello.pdf");
    let damaged = replace(&original, b"/Length 39", b"/Length 45");
    let repaired = rebuild(&damaged, RepairLimits::default()).unwrap();
    assert!(!repaired.report.damaged && repaired.report.xref_intact);
    assert_eq!(repaired.report.streams_length_recovered, 1);

    let at = find(&original, b"5 0 obj\n").unwrap() + 8;
    let end = find(&original, b"endstream").unwrap() + 9;
    for (last, eol) in [
        (b'\r', b"\n".as_slice()),
        (b'\r', b""),
        (b'\n', b""),
        (b'\n', b"\r\n"),
    ] {
        let label = format!("{last} before {eol:?}");
        let (_, data) = flate_ending_with(last);
        let file = |length: usize| {
            let stream = [
                format!("<< /Filter /FlateDecode /Length {length} >>\nstream\n").as_bytes(),
                &data,
                eol,
                b"endstream",
            ]
            .concat();
            fresh_xref(&[&original[..at], &stream, &original[end..]].concat())
        };
        intact(&label, &file(data.len()), file(data.len() + 7), b"");
    }
}

/// Owner decision R4 (spec 03 section 4.8 rule 2): a file whose catalog is
/// gone but whose pages survive opens with those pages under a synthesized
/// catalog.
#[test]
fn a_catalog_is_synthesized_when_none_exists() {
    let original = fixture("hello.pdf");
    let no_catalog = replace(&original, b"/Type /Catalog", b"/Type /Katalog");
    let no_catalog = replace(&no_catalog, b"/Pages 2 0 R >>", b">>");
    let repaired = rebuild(&no_catalog, RepairLimits::default()).unwrap();
    let report = &repaired.report;
    assert!(report.catalog_from_scan && report.damaged, "{report:?}");
    assert_eq!(snapshot(&repaired.bytes, b""), snapshot(&original, b""));
    let doc = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap();
    let catalog = doc.resolve_id(doc.root());
    assert_eq!(
        catalog.as_dict().unwrap().get(b"Type".as_slice()),
        Some(&Object::Name(b"Catalog".to_vec()))
    );
    // The new catalog takes a number above every number in use.
    assert!(doc.root().number() > 5, "{:?}", doc.root());
    for input in [b"".as_slice(), b"%PDF-1.7\n", b"not a pdf 1 0 obj"] {
        assert!(rebuild(input, RepairLimits::default()).is_err());
    }
}
