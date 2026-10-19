// SPDX-License-Identifier: MIT
//! Acceptance for the spec 03 integration addenda A7 to A10, found by the
//! corpus run of the clean-room tree. Each input is built here on the base
//! document of spec 03 section 7.2: catalog 1, page tree 2 over pages 3, 4
//! and 5, whose contents 6, 7 and 8 draw "Alpha", "Beta" and "Gamma", font
//! 9, and /Info 10 titled "Original".
#[path = "support/pages.rs"]
mod pages;

use std::sync::Arc;
use tandem_pdf_engine::app::AppDocument;
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::repair::{RepairLimits, RepairReport, rebuild};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::stream_object::decode_document_stream;

// ------------------------------------------------------------------ inputs

#[derive(Clone)]
struct Obj {
    number: u32,
    body: Vec<u8>,
}

fn obj(number: u32, body: &str) -> Obj {
    Obj {
        number,
        body: body.as_bytes().to_vec(),
    }
}

fn stream(number: u32, data: &str) -> Obj {
    obj(
        number,
        &format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len()),
    )
}

fn text(word: &str) -> String {
    format!("BT /F1 24 Tf 20 100 Td ({word}) Tj ET")
}

fn page(contents: u32) -> String {
    format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 9 0 R >> >> /Contents {contents} 0 R >>"
    )
}

fn base() -> Vec<Obj> {
    vec![
        obj(1, "<< /Type /Catalog /Pages 2 0 R >>"),
        obj(2, "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>"),
        obj(3, &page(6)),
        obj(4, &page(7)),
        obj(5, &page(8)),
        stream(6, &text("Alpha")),
        stream(7, &text("Beta")),
        stream(8, &text("Gamma")),
        obj(9, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
        obj(10, "<< /Title (Original) >>"),
    ]
}

const HEADER: &[u8] = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n";

/// Appends `N 0 obj body endobj` for each object; returns (number, offset).
fn write_objects(out: &mut Vec<u8>, objects: &[Obj]) -> Vec<(u32, usize)> {
    let mut offsets = Vec::new();
    for o in objects {
        offsets.push((o.number, out.len()));
        out.extend_from_slice(format!("{} 0 obj\n", o.number).as_bytes());
        out.extend_from_slice(&o.body);
        out.extend_from_slice(b"\nendobj\n");
    }
    offsets
}

/// A classic table for numbers 0..size, one subsection.
fn table(offsets: &[(u32, usize)], size: u32) -> String {
    let mut rows = format!("xref\n0 {size}\n0000000000 65535 f \n");
    for number in 1..size {
        match offsets.iter().rev().find(|(n, _)| *n == number) {
            Some((_, offset)) => rows.push_str(&format!("{offset:010} 00000 n \n")),
            None => rows.push_str("0000000000 00000 f \n"),
        }
    }
    rows
}

/// A classic-xref file whose body starts with `prefix`; `trailer` follows
/// /Size in the trailer.
fn classic_after(prefix: &[u8], objects: &[Obj], trailer: &str) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    out.extend_from_slice(prefix);
    let offsets = write_objects(&mut out, objects);
    let size = objects.iter().map(|o| o.number).max().unwrap() + 1;
    let xref = out.len();
    out.extend_from_slice(table(&offsets, size).as_bytes());
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} {trailer} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

fn find(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes.windows(needle.len()).position(|w| w == needle)
}

// ------------------------------------------------------------------ reading

/// The literal strings of a page's content streams, or "-" for none.
fn words(doc: &Document<'_>, page: ObjectId) -> String {
    let mut out = String::new();
    for content in pages::contents(doc, page) {
        let Ok(data) = decode_document_stream(doc, content, StreamLimits::default()) else {
            continue;
        };
        let mut rest = data.as_slice();
        while let Some(open) = rest.iter().position(|&b| b == b'(') {
            let close = open + rest[open..].iter().position(|&b| b == b')').unwrap();
            out.push_str(&String::from_utf8_lossy(&rest[open + 1..close]));
            rest = &rest[close + 1..];
        }
    }
    if out.is_empty() { "-".into() } else { out }
}

/// Rebuilds `bytes`; returns the report, each page's words, the /Info
/// /Title and whether the output trailer has /Encrypt.
fn read(bytes: &[u8]) -> (RepairReport, Vec<String>, Option<String>, bool) {
    let output = rebuild(bytes, RepairLimits::default()).unwrap_or_else(|e| panic!("rebuild: {e}"));
    let doc = Document::open_with_password(&output.bytes, DocumentLimits::default(), b"")
        .unwrap_or_else(|e| panic!("the output must open strictly: {e}"));
    let texts = pages::page_ids(&doc)
        .into_iter()
        .map(|page| words(&doc, page))
        .collect();
    let title = doc
        .trailer()
        .get(b"Info".as_slice())
        .map(|info| doc.resolve(info))
        .and_then(Object::as_dict)
        .and_then(|info| info.get(b"Title".as_slice()))
        .map(|title| match doc.resolve(title) {
            Object::String(bytes) => String::from_utf8_lossy(bytes).into_owned(),
            other => format!("{other:?}"),
        });
    let encrypt = doc.trailer().get(b"Encrypt".as_slice()).is_some();
    (output.report, texts, title, encrypt)
}

const ALL: [&str; 3] = ["Alpha", "Beta", "Gamma"];

/// The app's open (strict, else repair): (page count, repaired flag).
fn app_open(bytes: Vec<u8>) -> (usize, bool) {
    let doc = AppDocument::load(Arc::new(bytes), b"")
        .unwrap_or_else(|e| panic!("the app must open the file: {e:?}"));
    (doc.page_count(), doc.repaired())
}

// ------------------------------------------------------------------ A7

/// A7: a linearization dictionary whose /H offset lands inside another
/// object (left over after the file was rewritten) drops no object but the
/// dictionary itself: the content stream holding that offset stays.
#[test]
fn a7_a_hint_offset_inside_another_object_drops_nothing_else() {
    // Object 11, the linearization dictionary, comes first; its /H is
    // patched to point ten bytes into the data of content stream 6, and /L
    // to the file length. The chain is intact.
    let objects = [
        vec![obj(
            11,
            "<< /Linearized 1 /L 0000000000 /H [0000000000 16] /O 3 /E 0000000000 /N 3 /T 0000000000 >>",
        )],
        base(),
    ]
    .concat();
    let mut bytes = classic_after(b"", &objects, "/Root 1 0 R /Info 10 0 R");
    let inside = find(&bytes, b"(Alpha)").unwrap() - 10;
    let field = find(&bytes, b"/H [").unwrap() + 4;
    bytes[field..field + 10].copy_from_slice(format!("{inside:010}").as_bytes());
    let length = bytes.len();
    let field = find(&bytes, b"/L ").unwrap() + 3;
    bytes[field..field + 10].copy_from_slice(format!("{length:010}").as_bytes());

    let (report, texts, title, _) = read(&bytes);
    assert_eq!(texts, ALL, "{report:?}");
    assert_eq!(title.as_deref(), Some("Original"));
    assert!(report.linearized, "{report:?}");
}

// ------------------------------------------------------------------ A8

/// A8: an update whose /Prev points at its own section hides every older
/// section. The section read does not give an entry for every number below
/// /Size, so the chain is incomplete: the file is reconstructed, the
/// update's objects win (P3), and all pages show.
#[test]
fn a8_a_prev_loop_that_hides_the_older_sections_is_reconstructed() {
    let original = classic_after(b"", &base(), "/Root 1 0 R /Info 10 0 R");
    let mut bytes = original.clone();
    let update = [
        obj(1, "<< /Type /Catalog /Pages 2 0 R /PageMode /UseNone >>"),
        obj(10, "<< /Title (Updated) >>"),
    ];
    let offsets = write_objects(&mut bytes, &update);
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 2\n0000000000 65535 f \n");
    bytes.extend_from_slice(format!("{:010} 00000 n \n", offsets[0].1).as_bytes());
    bytes.extend_from_slice(format!("10 1\n{:010} 00000 n \n", offsets[1].1).as_bytes());
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size 11 /Root 1 0 R /Info 10 0 R /Prev {xref} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    assert_eq!(app_open(bytes.clone()), (3, true));
    let (report, texts, title, _) = read(&bytes);
    assert_eq!(texts, ALL, "{report:?}");
    assert_eq!(title.as_deref(), Some("Updated"), "{report:?}");
    assert!(report.damaged, "{report:?}");
}

/// A8 control (holds today, spec 03 4.3): a loop back to a section that
/// covers every number below /Size ends the chain and is not damage.
#[test]
fn a8_a_prev_loop_after_a_complete_section_is_not_damage() {
    let original = classic_after(b"", &base(), "/Root 1 0 R /Info 10 0 R");
    let at = find(&original, b"startxref\n").unwrap() + 10;
    let end = at + original[at..].iter().position(|&b| b == b'\n').unwrap();
    let xref = std::str::from_utf8(&original[at..end]).unwrap().to_string();
    let bytes = [
        &original[..find(&original, b"/Info 10 0 R >>").unwrap()],
        format!("/Info 10 0 R /Prev {xref} >>").as_bytes(),
        &original[find(&original, b"/Info 10 0 R >>").unwrap() + 15..],
    ]
    .concat();
    let (report, texts, title, _) = read(&bytes);
    assert_eq!(texts, ALL, "{report:?}");
    assert_eq!(title.as_deref(), Some("Original"));
    assert!(!report.damaged && report.xref_intact, "{report:?}");
}

// ------------------------------------------------------------------ A9

/// A9 (with A6 and P5): /Encrypt names an object that exists but reads as
/// no dictionary (its opening `<<` is damaged), and no other encryption
/// dictionary is found. Nothing can be decrypted, so the rebuild refuses
/// with the A6 reason rather than showing the file as plaintext.
#[test]
fn a9_an_unreadable_encryption_dictionary_refuses_the_rebuild() {
    let mut objects = base();
    objects.push(obj(
        11,
        "E< /Filter /Standard /V 5 /R 6 /Length 256 /P -4 >>",
    ));
    let bytes = classic_after(
        b"",
        &objects,
        "/Root 1 0 R /Info 10 0 R /Encrypt 11 0 R \
         /ID [<00112233445566778899AABBCCDDEEFF> <00112233445566778899AABBCCDDEEFF>]",
    );
    match rebuild(&bytes, RepairLimits::default()) {
        Err(error) => assert_eq!(
            error.to_string(),
            "PDF repair: invalid encryption dictionary"
        ),
        Ok(output) => panic!("rebuilt as plaintext: {:?}", output.report),
    }
    assert!(AppDocument::load(Arc::new(bytes), b"").is_err());
}

// ------------------------------------------------------------------ A10

/// A10: a content stream whose `/Length` is right but whose `endstream` and
/// `endobj` keywords are both damaged, followed by complete objects. The
/// stream keeps exactly its declared data (page 1 reads "Alpha"), and the
/// objects after it are kept.
#[test]
fn a10_a_stream_with_damaged_end_keywords_keeps_its_declared_data() {
    let data = text("Alpha");
    let damaged = format!(
        "<< /Length {} >>\nstream\n{data}\nenxstream\nenxobj\n\n",
        data.len()
    );
    // Written by hand: object 6 has no `endobj` of its own. Object 9, the
    // font, follows it as in the base document order 1..10 with 9 moved up.
    let mut out = HEADER.to_vec();
    let mut offsets = Vec::new();
    for o in base() {
        if o.number == 6 {
            offsets.push((6, out.len()));
            out.extend_from_slice(b"6 0 obj\n");
            out.extend_from_slice(damaged.as_bytes());
            continue;
        }
        offsets.push((o.number, out.len()));
        out.extend_from_slice(format!("{} 0 obj\n", o.number).as_bytes());
        out.extend_from_slice(&o.body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(table(&offsets, 11).as_bytes());
    out.extend_from_slice(
        format!("trailer\n<< /Size 11 /Root 1 0 R /Info 10 0 R >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    let (report, texts, _, _) = read(&out);
    assert_eq!(texts, ALL, "{report:?}");
    assert!(report.damaged, "{report:?}");
}

// ------------------------------------------------------------------ A10 review cases

/// The base document with object 6 written as `stream6` (header added here,
/// its own end keywords included), the others whole, in `order`; a classic
/// xref unless `xref` is false, its `startxref` wrong when `broken`.
fn with_stream6(stream6: &[u8], order: &[u32], xref: bool, broken: bool) -> Vec<u8> {
    let base = base();
    let mut out = HEADER.to_vec();
    let mut offsets = Vec::new();
    for &number in order {
        offsets.push((number, out.len()));
        out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        if number == 6 {
            out.extend_from_slice(stream6);
        } else {
            let o = base.iter().find(|o| o.number == number).unwrap();
            out.extend_from_slice(&o.body);
            out.extend_from_slice(b"\nendobj\n");
        }
    }
    if !xref {
        return out;
    }
    let at = out.len();
    out.extend_from_slice(table(&offsets, 11).as_bytes());
    let start = if broken { 99_999_999 } else { at };
    out.extend_from_slice(
        format!("trailer\n<< /Size 11 /Root 1 0 R /Info 10 0 R >>\nstartxref\n{start}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

const IN_ORDER: [u32; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

/// Stream 6's decoded data in the rebuilt file, with the report and words.
fn stream6(bytes: &[u8]) -> (RepairReport, Vec<String>, Vec<u8>) {
    let output = rebuild(bytes, RepairLimits::default()).unwrap_or_else(|e| panic!("rebuild: {e}"));
    let doc = Document::open(&output.bytes, DocumentLimits::default()).unwrap();
    let texts = pages::page_ids(&doc)
        .into_iter()
        .map(|page| words(&doc, page))
        .collect();
    let data = decode_document_stream(&doc, ObjectId::new(6, 0).unwrap(), StreamLimits::default())
        .unwrap_or_default();
    (output.report, texts, data)
}

/// A10 (review): a damaged `endstream` with `endobj` missing, the next
/// header right after it, reconstructed; also with a damaged `endobj` and a
/// comment before the next header. The declared data is kept, damaged.
#[test]
fn a10_a_damaged_endstream_without_endobj_keeps_its_data_when_reconstructed() {
    let data = text("Alpha");
    for tail in [
        "\nenxstream\n",
        "\nenxstream\nenxobj\n%c\n",
        "\nenxstream %c\n",
    ] {
        let body = format!("<< /Length {} >>\nstream\n{data}{tail}", data.len());
        for broken in [true, false] {
            let bytes = with_stream6(body.as_bytes(), &IN_ORDER, true, broken);
            let (report, texts, decoded) = stream6(&bytes);
            assert_eq!(texts, ALL, "{tail:?} broken={broken}: {report:?}");
            assert_eq!(decoded, data.as_bytes(), "{tail:?} broken={broken}");
            assert!(report.damaged, "{tail:?} broken={broken}: {report:?}");
        }
    }
}

/// A10 (review): a real `endstream` between the data start and the end of
/// the keywords A10 would consume leaves the ordinary `endstream` search in
/// charge. /Length one byte short with `endobj` missing, and /Length running
/// into the `endobj` after the real `endstream` (LF and CRLF line ends,
/// intact xref, reconstructed, and at the end of the input).
#[test]
fn a10_does_not_apply_across_a_real_endstream() {
    // (a) One byte short, `endobj` missing, the next header right after.
    let data = "BT /F1 24 Tf 72 700 Td (Alpha) Tj";
    let body = format!(
        "<< /Length {} >>\nstream\n{data}\nendstream\n",
        data.len() - 1
    );
    for broken in [false, true] {
        let bytes = with_stream6(body.as_bytes(), &IN_ORDER, true, broken);
        let (report, texts, decoded) = stream6(&bytes);
        assert_eq!(texts, ALL, "short broken={broken}: {report:?}");
        assert_eq!(decoded, data.as_bytes(), "short broken={broken}");
    }
    // (b) /Length running past the real `endstream` into `endobj`.
    let data = text("Alpha");
    for (eol, overshoots) in [("\n", 13..=16), ("\r\n", 14..=18)] {
        for over in overshoots {
            let body = format!(
                "<< /Length {} >>\nstream{eol}{data}{eol}endstream{eol}endobj{eol}",
                data.len() + over
            );
            let last: Vec<u32> = [1, 2, 3, 4, 5, 7, 8, 9, 10, 6].to_vec();
            for (label, bytes) in [
                (
                    "intact",
                    with_stream6(body.as_bytes(), &IN_ORDER, true, false),
                ),
                (
                    "broken",
                    with_stream6(body.as_bytes(), &IN_ORDER, true, true),
                ),
                (
                    "at the end",
                    with_stream6(body.as_bytes(), &last, false, false),
                ),
            ] {
                let (report, texts, decoded) = stream6(&bytes);
                let label = format!("{eol:?} +{over} {label}");
                assert_eq!(texts, ALL, "{label}: {report:?}");
                assert_eq!(decoded, data.as_bytes(), "{label}");
            }
        }
    }
}
