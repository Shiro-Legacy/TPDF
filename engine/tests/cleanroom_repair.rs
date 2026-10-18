// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acceptance cases for spec 03 (tolerant open and repair), section 8.2
//! (N1 to N17), and the owner decisions R1, R4, R6 and R8. Each input is
//! built here from the experiment of the same number in spec 03 section 7.3,
//! on the base document of section 7.2: catalog 1, page tree 2 over pages 3,
//! 4 and 5, whose contents 6, 7 and 8 draw "Alpha", "Beta" and "Gamma", font
//! 9, and /Info 10 titled "Original".
#[path = "support/pages.rs"]
mod pages;

use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::repair::{
    RepairLimits, RepairReport, RepairedPdf, rebuild, rebuild_with_passwords,
};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::stream_object::decode_document_stream;

// ------------------------------------------------------------------ inputs

#[derive(Clone)]
struct Obj {
    number: u32,
    generation: u16,
    body: Vec<u8>,
}

fn obj(number: u32, body: &str) -> Obj {
    Obj {
        number,
        generation: 0,
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

const FONT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";

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
        obj(9, FONT),
        obj(10, "<< /Title (Original) >>"),
    ]
}

/// `objects` with object `number`'s body replaced.
fn with(mut objects: Vec<Obj>, number: u32, body: &str) -> Vec<Obj> {
    let at = objects.iter().position(|o| o.number == number).unwrap();
    objects[at].body = body.as_bytes().to_vec();
    objects
}

fn without(objects: Vec<Obj>, number: u32) -> Vec<Obj> {
    objects.into_iter().filter(|o| o.number != number).collect()
}

const HEADER: &[u8] = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n";

/// Appends the objects; returns (number, generation, offset) for each.
fn write_objects(out: &mut Vec<u8>, objects: &[Obj]) -> Vec<(u32, u16, usize)> {
    let mut offsets = Vec::new();
    for o in objects {
        offsets.push((o.number, o.generation, out.len()));
        out.extend_from_slice(format!("{} {} obj\n", o.number, o.generation).as_bytes());
        out.extend_from_slice(&o.body);
        out.extend_from_slice(b"\nendobj\n");
    }
    offsets
}

/// A classic xref table for numbers 0..size, one subsection.
fn table(offsets: &[(u32, u16, usize)], size: u32) -> String {
    let mut rows = format!("xref\n0 {size}\n0000000000 65535 f \n");
    for number in 1..size {
        match offsets.iter().rev().find(|(n, _, _)| *n == number) {
            Some((_, generation, offset)) => {
                rows.push_str(&format!("{offset:010} {generation:05} n \n"));
            }
            None => rows.push_str("0000000000 00000 f \n"),
        }
    }
    rows
}

/// A classic-xref file; `trailer` is the trailer's entries after /Size.
fn classic_with(objects: &[Obj], trailer: &str) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    let offsets = write_objects(&mut out, objects);
    let size = objects.iter().map(|o| o.number).max().unwrap() + 1;
    let xref = out.len();
    out.extend_from_slice(table(&offsets, size).as_bytes());
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} {trailer} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

fn classic(objects: &[Obj]) -> Vec<u8> {
    classic_with(objects, "/Root 1 0 R /Info 10 0 R")
}

/// Objects only: no xref and no `startxref`, then `tail`.
fn scan_only(objects: &[Obj], tail: &str) -> Vec<u8> {
    let mut out = HEADER.to_vec();
    write_objects(&mut out, objects);
    out.extend_from_slice(tail.as_bytes());
    out
}

fn find(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes.windows(needle.len()).position(|w| w == needle)
}

fn rfind(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes.windows(needle.len()).rposition(|w| w == needle)
}

fn replace(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let at = find(bytes, from).unwrap_or_else(|| panic!("{}", String::from_utf8_lossy(from)));
    [&bytes[..at], to, &bytes[at + from.len()..]].concat()
}

/// The digits after the last `startxref`.
fn startxref_digits(bytes: &[u8]) -> std::ops::Range<usize> {
    let at = rfind(bytes, b"startxref").unwrap() + 9;
    let start = at + bytes[at..].iter().position(u8::is_ascii_digit).unwrap();
    let end = start
        + bytes[start..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
    start..end
}

/// The last `startxref` names an offset past the end of the file.
fn broken_startxref(bytes: &[u8]) -> Vec<u8> {
    let digits = startxref_digits(bytes);
    [&bytes[..digits.start], b"99999999", &bytes[digits.end..]].concat()
}

/// Appends an incremental update replacing `objects`, with a trailer of
/// /Size, /Root, `extra` and /Prev.
fn append_update(bytes: &[u8], objects: &[Obj], extra: &str) -> Vec<u8> {
    let digits = startxref_digits(bytes);
    let prev = std::str::from_utf8(&bytes[digits]).unwrap().to_string();
    let mut out = bytes.to_vec();
    let offsets = write_objects(&mut out, objects);
    let size = objects.iter().map(|o| o.number).max().unwrap().max(10) + 1;
    let xref = out.len();
    out.extend_from_slice(b"xref\n");
    for (number, generation, offset) in offsets {
        out.extend_from_slice(format!("{number} 1\n{offset:010} {generation:05} n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {size} /Root 1 0 R {extra} /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    out
}

/// An object stream (uncompressed) holding `members`.
fn object_stream(number: u32, members: &[Obj]) -> Obj {
    let mut header = String::new();
    let mut data = Vec::new();
    for member in members {
        header.push_str(&format!("{} {} ", member.number, data.len()));
        data.extend_from_slice(&member.body);
        data.push(b'\n');
    }
    let body = [
        format!(
            "<< /Type /ObjStm /N {} /First {} /Length {} >>\nstream\n",
            members.len(),
            header.len(),
            header.len() + data.len()
        )
        .as_bytes(),
        header.as_bytes(),
        &data,
        b"\nendstream",
    ]
    .concat();
    Obj {
        number,
        generation: 0,
        body,
    }
}

/// Xref-stream rows (/W [1 4 2]) for 0..size: top-level offsets, members
/// of `container` by index, the rest free.
fn xref_stream_rows(
    size: u32,
    offsets: &[(u32, u16, usize)],
    container: u32,
    members: &[Obj],
    top_level: bool,
) -> Vec<u8> {
    let mut data = Vec::new();
    for number in 0..size {
        let row: (u8, u32, u16) = if let Some(i) = members.iter().position(|m| m.number == number) {
            (2, container, i as u16)
        } else if let Some((_, g, offset)) = offsets.iter().find(|(n, _, _)| *n == number) {
            if top_level {
                (1, *offset as u32, *g)
            } else {
                (0, 0, 0)
            }
        } else {
            (0, 0, if number == 0 { 65535 } else { 0 })
        };
        data.push(row.0);
        data.extend_from_slice(&row.1.to_be_bytes());
        data.extend_from_slice(&row.2.to_be_bytes());
    }
    data
}

/// The base document with its non-stream objects in object stream 11 and an
/// xref stream 12 whose dictionary carries /Root and /Info (experiment 80).
fn with_xref_stream() -> Vec<u8> {
    let objects = base();
    let (streams, members): (Vec<Obj>, Vec<Obj>) = objects
        .into_iter()
        .partition(|o| (6..=8).contains(&o.number));
    let mut out = HEADER.to_vec();
    let mut top = streams.clone();
    top.push(object_stream(11, &members));
    let mut offsets = write_objects(&mut out, &top);
    let xref = out.len();
    offsets.push((12, 0, xref));
    let data = xref_stream_rows(13, &offsets, 11, &members, true);
    out.extend_from_slice(
        format!(
            "12 0 obj\n<< /Type /XRef /Size 13 /W [1 4 2] /Root 1 0 R /Info 10 0 R /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    out.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    out
}

/// The base document as a hybrid file (experiment e0): a classic table
/// lists the streams and marks the members of object stream 11 free; its
/// /XRefStm names xref stream 12, which locates them.
fn hybrid() -> Vec<u8> {
    let objects = base();
    let (streams, members): (Vec<Obj>, Vec<Obj>) = objects
        .into_iter()
        .partition(|o| (6..=8).contains(&o.number));
    let mut out = b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut top = streams.clone();
    top.push(object_stream(11, &members));
    let mut offsets = write_objects(&mut out, &top);
    let stream_at = out.len();
    let data = xref_stream_rows(13, &offsets, 11, &members, false);
    out.extend_from_slice(
        format!(
            "12 0 obj\n<< /Type /XRef /Size 13 /W [1 4 2] /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&data);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    offsets.push((12, 0, stream_at));
    let xref = out.len();
    out.extend_from_slice(table(&offsets, 13).as_bytes());
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size 13 /Root 1 0 R /Info 10 0 R /XRefStm {stream_at:010} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    out
}

/// The base document linearized (experiment f0): parameter dictionary 11,
/// a first-page section whose /Prev points forward to the main section, a
/// hint stream 12, the objects, the main section; `startxref` names the
/// first-page section.
fn linearized() -> Vec<u8> {
    let pad = |n: usize| format!("{n:010}");
    let mut out = HEADER.to_vec();
    let lin_at = out.len();
    out.extend_from_slice(
        b"11 0 obj\n<< /Linearized 1 /L 0000000000 /H [0000000000 16] /O 3 /E 0000000000 /N 3 /T 0000000000 >>\nendobj\n",
    );
    let first_at = out.len();
    out.extend_from_slice(
        format!(
            "xref\n11 2\n{} 00000 n \n0000000000 00000 n \ntrailer\n<< /Size 13 /Root 1 0 R /Info 10 0 R /Prev 0000000000 >>\nstartxref\n0\n%%EOF\n",
            pad(lin_at)
        )
        .as_bytes(),
    );
    let hint_at = out.len();
    out.extend_from_slice(b"12 0 obj\n<< /Length 16 >>\nstream\n");
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(b"\nendstream\nendobj\n");
    let end_of_first_page = out.len();
    let offsets = write_objects(&mut out, &base());
    let main_at = out.len();
    out.extend_from_slice(table(&offsets, 11).as_bytes());
    out.extend_from_slice(
        format!("trailer\n<< /Size 11 >>\nstartxref\n{first_at}\n%%EOF\n").as_bytes(),
    );
    let length = out.len();
    let patch = |out: &mut Vec<u8>, after: &[u8], value: usize| {
        let at = find(out, after).unwrap() + after.len();
        out[at..at + 10].copy_from_slice(pad(value).as_bytes());
    };
    patch(&mut out, b"/L ", length);
    patch(&mut out, b"/H [", hint_at);
    patch(&mut out, b"/E ", end_of_first_page);
    patch(&mut out, b"/T ", main_at);
    patch(&mut out, b"/Prev ", main_at);
    let row = find(&out, b"\n0000000000 00000 n \ntrailer").unwrap() + 1;
    out[row..row + 10].copy_from_slice(pad(hint_at).as_bytes());
    out
}

// ---------------------------------------------------------------- reading

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

/// The page texts and /Info /Title of a strictly opened file.
fn shown(bytes: &[u8], password: &[u8]) -> (Vec<String>, Option<String>) {
    let doc = Document::open_with_password(bytes, DocumentLimits::default(), password)
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
    (texts, title)
}

fn repaired(bytes: &[u8]) -> RepairedPdf {
    rebuild(bytes, RepairLimits::default()).unwrap_or_else(|e| panic!("rebuild: {e}"))
}

/// Rebuilds `bytes` and returns the report with what the output shows.
fn read(bytes: &[u8]) -> (RepairReport, Vec<String>, Option<String>) {
    let output = repaired(bytes);
    let (texts, title) = shown(&output.bytes, b"");
    (output.report, texts, title)
}

const ALL: [&str; 3] = ["Alpha", "Beta", "Gamma"];

fn id(number: u32, generation: u16) -> ObjectId {
    ObjectId::new(number, generation).unwrap()
}

// ------------------------------------------------------------------ cases

/// The builders above give files the strict reader opens as they should.
#[test]
fn the_base_documents_open() {
    let (texts, title) = shown(&classic(&base()), b"");
    assert_eq!(
        (texts, title.as_deref()),
        (ALL.map(String::from).to_vec(), Some("Original"))
    );
    let (texts, title) = shown(&with_xref_stream(), b"");
    assert_eq!(
        (texts, title.as_deref()),
        (ALL.map(String::from).to_vec(), Some("Original"))
    );
}

/// N1 (experiments 45, 46): two catalogs and no trailer; the later catalog
/// in the file wins.
#[test]
fn n1_the_later_of_two_catalogs_wins() {
    let small = [
        obj(12, "<< /Type /Catalog /Pages 13 0 R >>"),
        obj(13, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
    ];
    let earlier = [small.to_vec(), base()].concat();
    let later = [base(), small.to_vec()].concat();
    for (label, objects, expected) in [("45", earlier, 3), ("46", later, 1)] {
        let (report, texts, _) = read(&scan_only(&objects, "%%EOF\n"));
        assert_eq!(texts.len(), expected, "{label}: {texts:?}");
        assert_eq!(texts[0], "Alpha", "{label}");
        assert!(report.catalog_from_scan && report.damaged, "{label}");
    }
}

/// N2 (experiments 43, 44): a catalog typed wrongly, or none at all, with
/// page objects present: the pages open in file order (spec 03 section 4.8
/// rules 1 and 2).
#[test]
fn n2_pages_open_without_a_usable_catalog() {
    let katalog = with(base(), 1, "<< /Type /Katalog /Pages 2 0 R >>");
    let none = without(base(), 1);
    for (label, objects) in [("43", katalog), ("44", none)] {
        let bytes = scan_only(&objects, "%%EOF\n");
        let (report, texts, _) = read(&bytes);
        assert_eq!(texts, ALL, "{label}");
        assert!(report.catalog_from_scan && report.damaged, "{label}");
    }
    // No tree either: the pages in file order under a new tree.
    let bare = without(without(base(), 1), 2);
    let (report, texts, _) = read(&scan_only(&bare, "%%EOF\n"));
    assert_eq!(texts, ALL);
    assert!(report.catalog_from_scan && report.page_tree_repaired && report.damaged);
}

/// N3 (experiment 54): /Count larger than the kids, broken xref: only the
/// real pages.
#[test]
fn n3_count_beyond_the_kids_gives_the_real_pages() {
    let objects = with(
        base(),
        2,
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 7 >>",
    );
    let (_, texts, _) = read(&broken_startxref(&classic(&objects)));
    assert_eq!(texts, ALL);
}

/// N4 (experiments 50, 51, 58): the /Pages node is lost (`2 0 xbj`), xref
/// intact or broken: three pages in file order, a rebuilt tree, damaged;
/// US Letter where no /MediaBox applies.
#[test]
fn n4_a_lost_page_tree_is_rebuilt_in_file_order() {
    let intact = replace(&classic(&base()), b"2 0 obj", b"2 0 xbj");
    for (label, bytes) in [("50", intact.clone()), ("51", broken_startxref(&intact))] {
        let (report, texts, _) = read(&bytes);
        assert_eq!(texts, ALL, "{label}");
        assert!(report.page_tree_repaired && report.damaged, "{label}");
    }
    let no_box = with(base(), 3, &page(6).replace(" /MediaBox [0 0 200 200]", ""));
    let bytes = replace(&classic(&no_box), b"2 0 obj", b"2 0 xbj");
    let output = repaired(&bytes);
    let doc = Document::open(&output.bytes, DocumentLimits::default()).unwrap();
    let first = pages::page_ids(&doc)[0];
    let media = pages::media_box(&doc, first).unwrap();
    let values: Vec<f64> = media
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Object::as_f64)
        .collect();
    assert_eq!(values, [0.0, 0.0, 612.0, 792.0]);
}

/// N5 (experiments 52, 53): a dangling kid gives the same page list on the
/// intact and the broken file.
#[test]
fn n5_a_dangling_kid_reads_alike_intact_and_broken() {
    let objects = with(
        base(),
        2,
        "<< /Type /Pages /Kids [3 0 R 99 0 R 4 0 R 5 0 R] /Count 4 >>",
    );
    let intact = classic(&objects);
    let (_, intact_texts, _) = read(&intact);
    let (_, broken_texts, _) = read(&broken_startxref(&intact));
    assert_eq!(intact_texts, ALL);
    assert_eq!(broken_texts, ALL);
    // The app's own page walk agrees on the intact file.
    let doc = Document::open(&intact, DocumentLimits::default()).unwrap();
    assert_eq!(tandem_pdf_engine::app::info::pages(&doc).len(), 3);
}

/// N6 (experiments 31, 32): the later definition wins whatever the
/// generations, and keeps its own generation.
#[test]
fn n6_the_later_definition_wins_and_keeps_its_generation() {
    let gen_one = Obj {
        generation: 1,
        ..stream(7, &text("GenOne"))
    };
    let objects = [base(), vec![gen_one]].concat();
    let tail = "trailer\n<< /Root 1 0 R /Info 10 0 R >>\n%%EOF\n";
    let output = repaired(&scan_only(&objects, tail));
    assert_eq!(shown(&output.bytes, b"").0, ["Alpha", "GenOne", "Gamma"]);
    let doc = Document::open(&output.bytes, DocumentLimits::default()).unwrap();
    assert!(doc.object(id(7, 1)).is_ok() && doc.object(id(7, 0)).is_err());

    let mut objects = base();
    let at = objects.iter().position(|o| o.number == 7).unwrap();
    objects[at].generation = 3;
    objects.push(stream(7, &text("GenZeroLater")));
    let output = repaired(&scan_only(&objects, tail));
    assert_eq!(
        shown(&output.bytes, b"").0,
        ["Alpha", "GenZeroLater", "Gamma"]
    );
    let doc = Document::open(&output.bytes, DocumentLimits::default()).unwrap();
    assert!(doc.object(id(7, 0)).is_ok() && doc.object(id(7, 3)).is_err());
}

/// N7 (experiment d1): broken xref, the newest trailer without /Info: /Info
/// comes from the older trailer.
#[test]
fn n7_info_comes_from_an_older_trailer() {
    let updated = append_update(&classic(&base()), &[stream(7, &text("Updated"))], "");
    let (texts, title) = shown(&updated, b"");
    assert_eq!((texts.len(), title), (3, None));
    let (report, texts, title) = read(&broken_startxref(&updated));
    assert_eq!(texts, ["Alpha", "Updated", "Gamma"]);
    assert_eq!(title.as_deref(), Some("Original"));
    assert!(report.damaged);
}

/// N8 (experiments 12, 13 and c-space; owner decision R1): an intact xref
/// with a wrong /Length or a space after `stream`: the content of the intact
/// file, not damaged, read through the xref, one length recovered.
#[test]
fn n8_stream_extent_quirks_on_an_intact_xref_are_not_damage() {
    let intact = classic(&base());
    let length = format!("/Length {}", text("Beta").len());
    let long = format!("/Length {}", text("Beta").len() + 9);
    let short = format!("/Length {}", text("Beta").len() - 9);
    let beta = find(&intact, b"7 0 obj").unwrap();
    let at = |bytes: &[u8], from: &[u8], to: &[u8]| {
        let offset = beta + find(&bytes[beta..], from).unwrap();
        [&bytes[..offset], to, &bytes[offset + from.len()..]].concat()
    };
    for (label, bytes) in [
        ("12 long", at(&intact, length.as_bytes(), long.as_bytes())),
        ("13 short", at(&intact, length.as_bytes(), short.as_bytes())),
        ("c-space", at(&intact, b"stream\n", b"stream ")),
    ] {
        assert_eq!(bytes.len(), intact.len(), "{label}: offsets kept");
        assert!(
            Document::open(&bytes, DocumentLimits::default()).is_err(),
            "{label}"
        );
        let (report, texts, title) = read(&bytes);
        assert_eq!(texts, ALL, "{label}");
        assert_eq!(title.as_deref(), Some("Original"), "{label}");
        assert!(!report.damaged && report.xref_intact, "{label}: {report:?}");
        assert_eq!(report.streams_length_recovered, 1, "{label}");
    }

    // sign-prepare on a signed file whose content /Length is wrong appends
    // an update after the original bytes.
    let source = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/p0b/signed-intact.pdf"
    ))
    .unwrap();
    let source = replace(&source, b"<< /Length 46 >>", b"<< /Length 40 >>");
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let open = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, open, vec![source.clone()]);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["info"]["repaired"], false, "{reply}");
    let prepare = json!({"id": 2, "op": "sign-prepare", "name": "Signer",
        "reason": "test", "page": 0, "rect": null, "certify": false,
        "when": 1_791_979_200_000_i64});
    let (reply, outputs): (Value, Vec<Vec<u8>>) =
        engine.dispatch(&mut session, prepare, Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    let index = reply["bytes"]["$buffer"].as_u64().unwrap() as usize;
    assert_eq!(&outputs[index][..source.len()], source.as_slice());
}

/// N9 (experiments h-*-nowhere): a broken xref over encrypted object
/// streams: every page with the user password, locked without it.
#[test]
fn n9_encrypted_object_streams_survive_a_broken_xref() {
    for name in ["encrypted-aes128-objstm.pdf", "encrypted-aes256-objstm.pdf"] {
        let source =
            std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let expected = {
            let doc =
                Document::open_with_password(&source, DocumentLimits::default(), b"u").unwrap();
            pages::page_ids(&doc).len()
        };
        assert!(expected > 0, "{name}");
        let damaged = broken_startxref(&source);
        let output = rebuild_with_passwords(&damaged, RepairLimits::default(), &[b"u"]).unwrap();
        assert!(!output.report.locked && output.report.damaged, "{name}");
        assert!(output.report.object_streams_expanded > 0, "{name}");
        let doc =
            Document::open_with_password(&output.bytes, DocumentLimits::default(), b"u").unwrap();
        assert_eq!(pages::page_ids(&doc).len(), expected, "{name}");
        let locked = rebuild_with_passwords(&damaged, RepairLimits::default(), &[]).unwrap();
        assert!(
            locked.report.locked && locked.report.password_dependent,
            "{name}"
        );
    }
}

/// N10 (experiment a4): one xref entry pointing at another object's header:
/// reconstructed, all pages, damaged.
#[test]
fn n10_an_entry_at_another_objects_header_is_reconstructed() {
    let bytes = classic(&base());
    let five = find(&bytes, b"5 0 obj").unwrap();
    let rows = find(&bytes, b"xref\n0 11\n").unwrap() + 10;
    let four = rows + 4 * 20;
    let bytes = [
        &bytes[..four],
        format!("{five:010}").as_bytes(),
        &bytes[four + 10..],
    ]
    .concat();
    assert!(Document::open(&bytes, DocumentLimits::default()).is_err());
    let (report, texts, title) = read(&bytes);
    assert_eq!(texts, ALL);
    assert_eq!(title.as_deref(), Some("Original"));
    assert!(report.damaged && !report.xref_intact);
}

/// N11 (experiment a5): an entry free while the object is present: the
/// object is not in the output (spec 03 section 4.3), and its kid drops.
#[test]
fn n11_a_freed_object_stays_out() {
    let bytes = classic(&base());
    let rows = find(&bytes, b"xref\n0 11\n").unwrap() + 10;
    let four = rows + 4 * 20;
    let bytes = [&bytes[..four], b"0000000000 00000 f ", &bytes[four + 19..]].concat();
    let output = repaired(&bytes);
    assert!(output.report.xref_intact, "{:?}", output.report);
    let doc = Document::open(&output.bytes, DocumentLimits::default()).unwrap();
    assert!(doc.object(id(4, 0)).is_err());
    assert_eq!(shown(&output.bytes, b"").0, ["Alpha", "Gamma"]);
}

/// N12 (experiments e1, f1): a hybrid file with a wrong /XRefStm and a
/// linearized file with a wrong main /Prev: reconstructed, all pages, the
/// title kept. Intact (e0, f0) they are read through their own xref.
#[test]
fn n12_broken_hybrid_and_linearized_files_are_reconstructed() {
    for (label, intact) in [("hybrid", hybrid()), ("linearized", linearized())] {
        let (report, texts, title) = read(&intact);
        assert_eq!(texts, ALL, "{label} intact");
        assert_eq!(title.as_deref(), Some("Original"), "{label} intact");
        assert!(report.xref_intact && !report.damaged, "{label}: {report:?}");
        assert_eq!(report.linearized, label == "linearized", "{label}");
    }
    let hybrid = hybrid();
    let at = find(&hybrid, b"/XRefStm ").unwrap() + 9;
    let wrong: usize = std::str::from_utf8(&hybrid[at..at + 10])
        .unwrap()
        .parse()
        .unwrap();
    let e1 = [
        &hybrid[..at],
        format!("{:010}", wrong + 3).as_bytes(),
        &hybrid[at + 10..],
    ]
    .concat();
    let linear = linearized();
    let at = find(&linear, b"/Prev ").unwrap() + 6;
    let f1 = [&linear[..at], b"0000000017", &linear[at + 10..]].concat();
    for (label, bytes) in [("e1", e1), ("f1", f1)] {
        let (report, texts, title) = read(&bytes);
        assert_eq!(texts, ALL, "{label}");
        assert_eq!(title.as_deref(), Some("Original"), "{label}");
        assert!(report.damaged && !report.xref_intact, "{label}: {report:?}");
    }
}

/// N13 (experiments 81, 83): broken xref, the xref stream unreachable or
/// unreadable: the /Info title comes from the xref stream's dictionary.
#[test]
fn n13_trailer_entries_come_from_an_xref_stream_dictionary() {
    let packed = with_xref_stream();
    let w = replace(&packed, b"/W [1 4 2]", b"/W [1 4 3]");
    for (label, bytes) in [("81", broken_startxref(&packed)), ("83", w)] {
        let (report, texts, title) = read(&bytes);
        assert_eq!(texts, ALL, "{label}");
        assert_eq!(title.as_deref(), Some("Original"), "{label}");
        assert!(
            report.damaged && report.object_streams_expanded == 1,
            "{label}"
        );
    }
}

/// N14 (experiment 23): a `startxref` the incremental writer's locator does
/// not find is not followed: the file is reconstructed and the appended
/// object wins. One it finds is followed: the last complete revision.
#[test]
fn n14_reading_follows_the_writers_locator() {
    let mut bytes = classic(&base());
    bytes.extend_from_slice(b"7 0 obj\n");
    bytes.extend_from_slice(&stream(7, &text("Updated")).body);
    bytes.extend_from_slice(b"\nendobj\n");
    let with_tail = |length: usize| [bytes.as_slice(), &vec![b'x'; length]].concat();
    let mut limit = 0;
    while tandem_pdf_engine::incremental::startxref(&with_tail(limit)).is_some() {
        limit += 1;
        assert!(limit < 1 << 16);
    }
    let (report, texts, _) = read(&with_tail(limit - 1));
    assert_eq!(texts, ALL);
    assert!(report.xref_intact && !report.damaged, "{report:?}");
    let (report, texts, _) = read(&with_tail(limit));
    assert_eq!(texts, ["Alpha", "Updated", "Gamma"]);
    assert!(!report.xref_intact && report.damaged, "{report:?}");
}

/// N15 (experiment 72): 3,000 NUL bytes after a layout the strict reader
/// refuses (a hybrid) are not damage: NUL is white space.
#[test]
fn n15_trailing_nul_bytes_are_not_damage() {
    let bytes = [hybrid(), vec![0; 3000]].concat();
    let (report, texts, _) = read(&bytes);
    assert_eq!(texts, ALL);
    assert!(report.xref_intact && !report.damaged, "{report:?}");
}

/// N16 (spec 03 section 5.1): the same input gives the same bytes; an
/// encrypted file without object streams gives the same bytes for every
/// password list.
#[test]
fn n16_rebuilds_are_deterministic() {
    let damaged = broken_startxref(&classic(&base()));
    assert_eq!(repaired(&damaged).bytes, repaired(&damaged).bytes);
    let packed = broken_startxref(&with_xref_stream());
    assert_eq!(repaired(&packed).bytes, repaired(&packed).bytes);

    let source = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/encrypted.pdf"
    ))
    .unwrap();
    let damaged = broken_startxref(&source);
    let outputs: Vec<RepairedPdf> = [
        &[][..],
        &[b"wrong".as_slice()],
        &[b"pdf4all-user".as_slice()],
    ]
    .iter()
    .map(|passwords| rebuild_with_passwords(&damaged, RepairLimits::default(), passwords).unwrap())
    .collect();
    assert!(!outputs[0].report.password_dependent);
    assert_eq!(outputs[0].bytes, outputs[1].bytes);
    assert_eq!(outputs[0].bytes, outputs[2].bytes);
}

/// A chain of `sections` classic sections: the base and updates retitling
/// /Info.
fn chain(sections: usize) -> Vec<u8> {
    let mut bytes = classic(&base());
    for i in 1..sections {
        bytes = append_update(
            &bytes,
            &[obj(10, &format!("<< /Title (Version {i}) >>"))],
            "/Info 10 0 R",
        );
    }
    bytes
}

fn revision_limits(max_revisions: usize) -> RepairLimits {
    RepairLimits::for_document(DocumentLimits {
        max_revisions,
        ..DocumentLimits::default()
    })
}

/// N17: a chain at the `max_revisions` limit is read through.
#[test]
fn n17_a_chain_at_the_revision_limit_is_read() {
    let bytes = chain(4);
    let output = rebuild(&bytes, revision_limits(4)).unwrap();
    assert!(output.report.xref_intact && !output.report.damaged);
    assert_eq!(output.report.xref_sections, 4);
    let (texts, title) = shown(&output.bytes, b"");
    assert_eq!((texts.len(), title.as_deref()), (3, Some("Version 3")));
}

/// N17 (spec 03 section 2.1, `max_revisions`): a chain one section over the
/// limit is reconstructed, and reports the sections it read.
#[test]
fn n17_a_chain_past_the_revision_limit_is_reconstructed() {
    let bytes = chain(5);
    let output = rebuild(&bytes, revision_limits(4)).unwrap();
    assert!(
        !output.report.xref_intact && output.report.damaged,
        "{:?}",
        output.report
    );
    assert_eq!(output.report.xref_sections, 4);
    // The newest definition still wins.
    let (texts, title) = shown(&output.bytes, b"");
    assert_eq!((texts.len(), title.as_deref()), (3, Some("Version 4")));
}

/// Owner decision R6 (spec 03 section 2.1): the default repair limits are
/// the app's.
#[test]
fn r6_default_repair_limits_are_the_apps() {
    let app = RepairLimits::for_document(tandem_pdf_engine::app::document::limits());
    assert_eq!(format!("{:?}", RepairLimits::default()), format!("{app:?}"));
}

fn catalog_with_lists() -> Vec<Obj> {
    let objects = with(
        base(),
        1,
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [11 0 R 99 0 R] >> /OCProperties << /OCGs [12 0 R 98 0 R] /D << /Order [12 0 R 97 0 R] /ON [12 0 R 96 0 R] /OFF [95 0 R 12 0 R] >> >> /Other [94 0 R 12 0 R] /Single 93 0 R >>",
    );
    let objects = with(
        objects,
        3,
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 9 0 R >> >> /Contents 6 0 R /Annots [11 0 R 92 0 R] >>",
    );
    [
        objects,
        vec![
            obj(
                11,
                "<< /Type /Annot /Subtype /Widget /FT /Tx /T (f) /Rect [0 0 10 10] /P 3 0 R >>",
            ),
            obj(12, "<< /Type /OCG /Name (L) >>"),
        ],
    ]
    .concat()
}

fn catalog_after_repair() -> (Vec<u8>, RepairReport) {
    let bytes = broken_startxref(&classic(&catalog_with_lists()));
    let output = repaired(&bytes);
    (output.bytes, output.report)
}

/// Spec 03 section 4.9: dangling references are removed from /Annots, and
/// become null elsewhere.
#[test]
fn dangling_references_drop_from_annots_and_are_null_elsewhere() {
    let (bytes, report) = catalog_after_repair();
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    let reference = |n| Object::Reference(id(n, 0));
    assert_eq!(
        catalog.get(b"Other".as_slice()),
        Some(&Object::Array(vec![Object::Null, reference(12)]))
    );
    assert!(matches!(
        catalog.get(b"Single".as_slice()),
        None | Some(Object::Null)
    ));
    let first = pages::page_ids(&doc)[0];
    assert_eq!(
        pages::page(&doc, first).get(b"Annots".as_slice()),
        Some(&Object::Array(vec![reference(11)]))
    );
    assert!(report.references_repaired >= 3, "{report:?}");
}

/// Owner decision R8: dangling entries are also removed from /Fields,
/// /OCGs, /Order, /ON and /OFF.
#[test]
fn r8_dangling_entries_drop_from_object_lists() {
    let (bytes, _) = catalog_after_repair();
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    let list = |dict: &tandem_pdf_engine::object::Dictionary, key: &[u8]| {
        doc.resolve(dict.get(key).unwrap()).clone()
    };
    let only = |n| Object::Array(vec![Object::Reference(id(n, 0))]);
    let form = list(catalog, b"AcroForm");
    assert_eq!(list(form.as_dict().unwrap(), b"Fields"), only(11));
    let properties = list(catalog, b"OCProperties");
    let properties = properties.as_dict().unwrap();
    assert_eq!(list(properties, b"OCGs"), only(12));
    let config = list(properties, b"D");
    let config = config.as_dict().unwrap();
    for key in [b"Order".as_slice(), b"ON", b"OFF"] {
        assert_eq!(
            list(config, key),
            only(12),
            "{}",
            String::from_utf8_lossy(key)
        );
    }
}
