// SPDX-License-Identifier: AGPL-3.0-or-later
//! Round 3 of the session acceptance review of `open`
//! (engine/design/app-accept-session.md): a dictionary that lost its `>>`
//! no longer hides the object after it, an object header glued to the
//! damaged tail of another object no longer drops an intact xref, an
//! untyped page-tree node is a node everywhere, a damaged `trailer` keyword
//! keeps an encrypted file's identifier, a kid typed as another kind of
//! object that looks like a page stays a page, and a wrong password for a
//! rebuilt locked file is known without a reload. Every file is built here
//! or derived in the test from a fixture.
use std::time::Instant;

use serde_json::{Value, json};
use tandem_pdf_engine::app::document::AppDocument;
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object};

// ------------------------------------------------------------ app sessions

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

fn open(bytes: Vec<u8>) -> (AppEngine, AppSession, Value) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let (reply, _) = engine.dispatch(
        &mut session,
        json!({"op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}}),
        vec![bytes],
    );
    (engine, session, reply)
}

fn text_all(engine: &mut AppEngine, session: &mut AppSession) -> String {
    let reply = dispatch(engine, session, json!({"op": "text-all"}));
    assert_eq!(reply["ok"], true, "{reply}");
    reply["text"].as_str().unwrap().to_string()
}

/// The saved bytes, validated.
fn save(engine: &mut AppEngine, session: &mut AppSession) -> Vec<u8> {
    let (reply, mut outputs) = engine.dispatch(
        session,
        json!({"op": "save", "entropy": {"$buffer": 0}}),
        vec![vec![7; 32]],
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let bytes = outputs.remove(0);
    let (check, _) = engine.dispatch(
        session,
        json!({"op": "validate", "pdf": {"$buffer": 0}}),
        vec![bytes.clone()],
    );
    assert_eq!(check["ok"], true, "{check}");
    bytes
}

/// What a session shows: pages, page ids, text, and links, fields and
/// annotations summed over the pages.
#[derive(Debug, PartialEq)]
struct Shown {
    pages: u64,
    ids: Value,
    text: String,
    links: usize,
    fields: usize,
    annots: usize,
}

fn shown(engine: &mut AppEngine, session: &mut AppSession, reply: &Value) -> Shown {
    assert_eq!(reply["ok"], true, "{reply}");
    let pages = reply["pageCount"].as_u64().unwrap();
    let ids = dispatch(engine, session, json!({"op": "page-ids"}))["ids"].clone();
    let text = text_all(engine, session);
    let mut count = |op: &str| -> usize {
        (0..pages)
            .map(|page| {
                let reply = dispatch(engine, session, json!({"op": op, "page": page}));
                assert_eq!(reply["ok"], true, "{op} {page}: {reply}");
                reply["items"].as_array().map_or(0, Vec::len)
            })
            .sum()
    };
    let (links, fields, annots) = (count("links"), count("fields"), count("annots"));
    Shown {
        pages,
        ids,
        text,
        links,
        fields,
        annots,
    }
}

/// What the file shows, and what its save shows when reopened.
fn shown_and_saved(bytes: Vec<u8>) -> (Shown, Shown) {
    let (mut engine, mut session, reply) = open(bytes);
    let before = shown(&mut engine, &mut session, &reply);
    let saved = save(&mut engine, &mut session);
    let (mut engine, mut session, reply) = open(saved);
    let after = shown(&mut engine, &mut session, &reply);
    (before, after)
}

// ------------------------------------------------------------ file building

/// Objects written in the order given (each `N 0 obj` then `body`, which
/// carries its own `endobj` or none), then a classic xref listing them all.
fn assemble(objects: &[(u32, Vec<u8>)], trailer: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = std::collections::BTreeMap::new();
    for (number, body) in objects {
        offsets.insert(*number, bytes.len());
        bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        bytes.extend_from_slice(body);
    }
    let xref = bytes.len();
    let size = offsets.keys().next_back().unwrap() + 1;
    bytes.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for number in 1..size {
        match offsets.get(&number) {
            Some(offset) => bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes()),
            None => bytes.extend_from_slice(b"0000000000 65535 f \n"),
        }
    }
    bytes.extend_from_slice(
        format!("trailer\n<</Size {size} {trailer}>>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

/// A complete object body: `value` then `endobj`.
fn whole(value: &[u8]) -> Vec<u8> {
    [value, b"\nendobj\n"].concat()
}

fn content(text: &str) -> Vec<u8> {
    let data = format!("BT /F1 12 Tf 20 100 Td ({text}) Tj ET");
    whole(format!("<</Length {}>>\nstream\n{data}\nendstream", data.len()).as_bytes())
}

/// The file with its `startxref` pointing nowhere: the xref is not found.
fn broken_startxref(mut bytes: Vec<u8>) -> Vec<u8> {
    let at = bytes
        .windows(10)
        .rposition(|w| w == b"startxref\n")
        .unwrap()
        + 10;
    let end = at + bytes[at..].iter().position(|&b| b == b'\n').unwrap();
    bytes.splice(at..end, b"9999".iter().copied());
    bytes
}

/// Three pages (Alpha, Beta, Gamma); Beta carries a link and a text field.
/// Object 12, the document information, is written as `damaged` right
/// before object `before`.
fn three_pages(damaged: &[u8], before: u32) -> Vec<u8> {
    let mut ordered = Vec::new();
    for (number, body) in three_page_objects() {
        if number == before {
            ordered.push((12, damaged.to_vec()));
        }
        ordered.push((number, body));
    }
    assemble(&ordered, "/Root 1 0 R/Info 12 0 R")
}

/// The objects of `three_pages` but object 12, in order.
fn three_page_objects() -> Vec<(u32, Vec<u8>)> {
    let page = |contents: u32, extra: &str| {
        whole(format!("<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents {contents} 0 R/Resources<</Font<</F1 9 0 R>>>>{extra}>>").as_bytes())
    };
    let objects = vec![
        (
            1,
            whole(b"<</Type/Catalog/Pages 2 0 R/AcroForm<</Fields[11 0 R]>>>>"),
        ),
        (2, whole(b"<</Type/Pages/Kids[3 0 R 4 0 R 5 0 R]/Count 3>>")),
        (3, page(6, "")),
        (4, page(7, "/Annots[10 0 R 11 0 R]")),
        (5, page(8, "")),
        (6, content("Alpha")),
        (7, content("Beta")),
        (8, content("Gamma")),
        (
            9,
            whole(b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>"),
        ),
        (
            10,
            whole(b"<</Type/Annot/Subtype/Link/Rect[10 10 50 50]/Border[0 0 0]/A<</S/URI/URI(https://example.com/)>>>>"),
        ),
        (
            11,
            whole(b"<</Type/Annot/Subtype/Widget/FT/Tx/T(name)/V(value)/Rect[60 10 150 40]/P 4 0 R/F 4/DA(/Helv 10 Tf 0 g)>>"),
        ),
    ];
    objects
}

fn assert_three_pages(shown: &Shown, label: &str) {
    assert_eq!(shown.pages, 3, "{label}: {shown:?}");
    assert_eq!(shown.ids, json!([3, 4, 5]), "{label}: {shown:?}");
    for word in ["Alpha", "Beta", "Gamma"] {
        assert!(shown.text.contains(word), "{label}: {shown:?}");
    }
    assert_eq!((shown.links, shown.fields), (1, 1), "{label}: {shown:?}");
}

// ------------------------------------------- a dictionary that lost its end

/// A dictionary that lost its `>>` (with or without its `endobj`) right
/// before a page, a content stream, a link or a widget: whatever its last
/// value, the object after it stays, with an intact xref (read at its
/// offset) and with a broken one (the scan does not read the dictionary
/// past a header whose own object parses through `endobj`). Round 2 lost
/// that object in 8 of 9 such shapes (`/Foo /Bar` and `endobj` was kept),
/// and the save wrote the loss.
#[test]
fn a_dictionary_missing_its_end_keeps_the_next_object() {
    let values: [&[u8]; 4] = [b"/Foo 1", b"/Foo /Bar", b"/Foo (abc)", b"/Foo 9 0 R"];
    let ends: [(&str, &[u8]); 2] = [("endobj", b"\nendobj\n"), ("cut", b"\n")];
    for value in values {
        for (end_label, end) in ends {
            let damaged = [b"<</Title(x)".as_slice(), value, end].concat();
            for (before, what) in [(4, "page"), (7, "content"), (10, "link"), (11, "widget")] {
                let source = three_pages(&damaged, before);
                assert!(Document::open(&source, DocumentLimits::default()).is_err());
                for (xref, bytes) in [
                    ("intact", source.clone()),
                    ("broken", broken_startxref(source.clone())),
                ] {
                    let label = format!(
                        "{} {end_label} before the {what}, {xref} xref",
                        String::from_utf8_lossy(value)
                    );
                    let (before, after) = shown_and_saved(bytes);
                    assert_three_pages(&before, &label);
                    assert_three_pages(&after, &format!("{label}, saved"));
                }
            }
        }
    }
}

/// An object header glued to the damaged tail of the object before it
/// (`/Foo 14 0 obj`, `/Bar4 0 obj`, `end4 0 obj`): the scan never sees that
/// header, and an intact xref used to be given up for it (a page or an
/// annotation lost, though MuPDF reads it at its offset). It is now read at
/// its offset. With a broken xref, a header the scan can see (after a
/// delimiter) is kept too.
#[test]
fn a_header_glued_to_a_damaged_object_keeps_an_intact_xref() {
    let tails: [(&[u8], bool); 6] = [
        (b"<</Title(x)/Foo 1", false),
        (b"<</Title(x)/Foo /Bar", false),
        (b"<</Title(x)/Foo (abc)", true),
        (b"<</Title(x)/Foo 9 0 R", false),
        (b"<</Title(x)/Foo 1>>\nend", false),
        (b"<</Title(x)/Foo [1 2", false),
    ];
    for (tail, seen) in tails {
        for before in [4, 7, 10, 11] {
            let source = three_pages(tail, before);
            let label = format!("{} glued to {before}", String::from_utf8_lossy(tail));
            let (shown, saved) = shown_and_saved(source.clone());
            assert_three_pages(&shown, &label);
            assert_three_pages(&saved, &format!("{label}, saved"));
            if seen {
                let (shown, saved) = shown_and_saved(broken_startxref(source));
                assert_three_pages(&shown, &format!("{label}, broken"));
                assert_three_pages(&saved, &format!("{label}, broken, saved"));
            }
        }
    }
    // A damaged page whose tail meets the next page's header: the other
    // two pages and their text stay; the damaged one is a blank page, as
    // MuPDF shows it.
    let mut objects = three_page_objects();
    objects[2].1 = b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 6 0 R".to_vec();
    let bytes = assemble(&objects, "/Root 1 0 R");
    assert!(bytes.windows(12).any(|w| w == b"6 0 R4 0 obj"));
    let (mut engine, mut session, reply) = open(bytes);
    assert_eq!(reply["pageCount"], 3, "{reply}");
    let text = text_all(&mut engine, &mut session);
    assert!(text.contains("Beta") && text.contains("Gamma"), "{text:?}");
    // The same page cut inside its /Annots array, and Gamma's content
    // glued to it (`[108 0 obj`): the page's rereads fail to the end of
    // the input in the scan and again at its offset; the content is still
    // read at its own (the xref's rereads have their own one-pass budget).
    let mut objects = three_page_objects();
    objects[2].1 =
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 6 0 R/Annots[10".to_vec();
    let gamma = objects.remove(7);
    objects.insert(3, gamma);
    let bytes = assemble(&objects, "/Root 1 0 R");
    assert!(bytes.windows(10).any(|w| w == b"[108 0 obj"));
    let (shown, saved) = shown_and_saved(bytes);
    for shown in [shown, saved] {
        assert_eq!(shown.pages, 3, "{shown:?}");
        assert!(
            shown.text.contains("Beta") && shown.text.contains("Gamma"),
            "{shown:?}"
        );
    }
}

/// The review's fixtures with only the two `>>` bytes of one dictionary
/// blanked (an outline root ending in an integer, a link, a widget, a
/// structure tree root): the file shows what the source shows, and so does
/// its save, with the xref intact and broken. Round 2 showed 3 links of 4,
/// 12 fields of 13, and 2 pages of 3.
#[test]
fn fixtures_with_a_dictionary_missing_its_end_keep_their_objects() {
    let sites: [(&str, &[u8], bool); 4] = [
        ("nav.pdf", b"/Count 4 >>", true),
        ("p6/links.pdf", b"/Border [0 0 0] /F 4 >>", false),
        ("p6/tabs.pdf", b"/StructParent 0 >>", false),
        ("p6/tabs.pdf", b"/ParentTreeNextKey 3 >>", true),
    ];
    for (name, needle, whole_kept) in sites {
        let path = format!("{}/../fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let source = std::fs::read(path).unwrap();
        let (mut engine, mut session, reply) = open(source.clone());
        let expected = shown(&mut engine, &mut session, &reply);
        let at = source
            .windows(needle.len())
            .position(|w| w == needle)
            .unwrap()
            + needle.len();
        let mut damaged = source.clone();
        damaged[at - 2] = b' ';
        damaged[at - 1] = b' ';
        for (xref, bytes) in [
            ("intact", damaged.clone()),
            ("broken", broken_startxref(damaged.clone())),
        ] {
            let label = format!("{name} {} {xref}", String::from_utf8_lossy(needle));
            let (shown, saved) = shown_and_saved(bytes);
            for shown in [&shown, &saved] {
                assert_eq!(shown.pages, expected.pages, "{label}: {shown:?}");
                assert_eq!(shown.ids, expected.ids, "{label}: {shown:?}");
                assert_eq!(shown.text, expected.text, "{label}: {shown:?}");
                if whole_kept {
                    // The damaged object is not a link or field.
                    assert_eq!(
                        (shown.links, shown.fields, shown.annots),
                        (expected.links, expected.fields, expected.annots),
                        "{label}: {shown:?}"
                    );
                } else {
                    // The damaged annotation itself may not be read; every
                    // other one is.
                    assert!(
                        shown.links + 1 >= expected.links
                            && shown.fields + 1 >= expected.fields
                            && shown.annots + 1 >= expected.annots,
                        "{label}: {shown:?} vs {expected:?}"
                    );
                }
            }
        }
    }
}

// ------------------------------------------------------- untyped nodes

/// Whether MuPDF (`pdf_lookup_page_loc`) reads `dict` as a node.
fn mupdf_node(doc: &Document<'_>, dict: &Dictionary) -> bool {
    match doc.dict_get(dict, b"Type") {
        Object::Null => {
            !doc.dict_get(dict, b"Kids").is_null() && doc.dict_get(dict, b"MediaBox").is_null()
        }
        kind => kind.as_name() == Some(b"Pages"),
    }
}

/// The page objects MuPDF lists, walking /Kids from the catalog.
fn mupdf_pages(bytes: &[u8]) -> Vec<u32> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    // Depth first, kids in order: pushed in reverse.
    let mut stack = vec![catalog.get(b"Pages".as_slice()).unwrap().clone()];
    let mut pages = Vec::new();
    while let Some(value) = stack.pop() {
        let Some(dict) = doc.resolve(&value).as_dict() else {
            continue;
        };
        if mupdf_node(&doc, dict) {
            if let Some(kids) = doc.dict_get(dict, b"Kids").as_array() {
                stack.extend(kids.iter().rev().cloned());
            }
        } else if let Object::Reference(id) = value {
            pages.push(id.number());
        }
    }
    pages
}

/// An intermediate page-tree node without /Type is a node to the page
/// list, the page ids, the view and the page edits, as it is to the
/// repair (and to MuPDF without a /MediaBox): before, the app showed it as
/// one blank page hiding its kids, and deleting that page deleted them.
#[test]
fn an_untyped_intermediate_node_is_a_node_everywhere() {
    let page = |contents: u32, parent: u32| {
        whole(format!("<</Type/Page/Parent {parent} 0 R/MediaBox[0 0 300 300]/Resources<</Font<</F1 9 0 R>>>>/Contents {contents} 0 R>>").as_bytes())
    };
    let font = whole(b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>");
    let nodes: [(&str, &[u8]); 3] = [
        ("untyped", b"<</Parent 2 0 R/Kids[3 0 R 4 0 R]/Count 2>>"),
        (
            "untyped with a MediaBox",
            b"<</Parent 2 0 R/Kids[3 0 R 4 0 R]/Count 2/MediaBox[0 0 300 300]>>",
        ),
        (
            "null type",
            b"<</Type null/Parent 2 0 R/Kids[3 0 R 4 0 R]/Count 2>>",
        ),
    ];
    let mut sources = Vec::new();
    for (label, node) in nodes {
        let objects = vec![
            (1, whole(b"<</Type/Catalog/Pages 2 0 R>>")),
            (2, whole(b"<</Type/Pages/Kids[10 0 R 5 0 R]/Count 3>>")),
            (3, page(6, 10)),
            (4, page(7, 10)),
            (5, page(8, 2)),
            (6, content("Alpha")),
            (7, content("Beta")),
            (8, content("Gamma")),
            (9, font.clone()),
            (10, whole(node)),
        ];
        sources.push((label.to_string(), assemble(&objects, "/Root 1 0 R")));
    }
    // An untyped node written directly into /Kids: its pages name the root.
    let objects = vec![
        (1, whole(b"<</Type/Catalog/Pages 2 0 R>>")),
        (
            2,
            whole(b"<</Type/Pages/Kids[<</Kids[3 0 R 4 0 R]/Count 2>> 5 0 R]/Count 3>>"),
        ),
        (3, page(6, 2)),
        (4, page(7, 2)),
        (5, page(8, 2)),
        (6, content("Alpha")),
        (7, content("Beta")),
        (8, content("Gamma")),
        (9, font.clone()),
    ];
    sources.push(("direct".to_string(), assemble(&objects, "/Root 1 0 R")));

    for (label, source) in sources {
        // The strict reader opens it: no repair is involved.
        assert!(Document::open(&source, DocumentLimits::default()).is_ok());
        let (mut engine, mut session, reply) = open(source.clone());
        assert_eq!(reply["pageCount"], 3, "{label}: {reply}");
        assert_eq!(reply["info"]["repaired"], false, "{label}: {reply}");
        let ids = dispatch(&mut engine, &mut session, json!({"op": "page-ids"}));
        assert_eq!(ids["ids"], json!([3, 4, 5]), "{label}: {ids}");
        // The view draws three pages in that order (hayro walks the node).
        let text = text_all(&mut engine, &mut session);
        let at = |word: &str| {
            text.find(word)
                .unwrap_or_else(|| panic!("{label}: {text:?}"))
        };
        assert!(
            at("Alpha") < at("Beta") && at("Beta") < at("Gamma"),
            "{label}"
        );
        for page in 0..3 {
            let render = dispatch(
                &mut engine,
                &mut session,
                json!({"op": "render", "page": page, "scale": 0.25}),
            );
            assert_eq!(render["ok"], true, "{label} render {page}: {render}");
        }
        // The save, unedited: MuPDF lists the app's pages.
        let saved = save(&mut engine, &mut session);
        assert_eq!(mupdf_pages(&saved), [3, 4, 5], "{label} saved");

        // page-rotate: the first page turns, the others stay.
        let (mut engine, mut session, _) = open(source.clone());
        let rotated = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "page-rotate", "page": 0, "by": 90}),
        );
        assert_eq!(rotated["ok"], true, "{label}: {rotated}");
        let saved = save(&mut engine, &mut session);
        assert_eq!(mupdf_pages(&saved), [3, 4, 5], "{label} rotated");
        let doc = Document::open(&saved, DocumentLimits::default()).unwrap();
        let rotation = |number: u32| {
            let page = doc
                .resolve_id(tandem_pdf_engine::object::ObjectId::new(number, 0).unwrap())
                .as_dict()
                .unwrap();
            doc.dict_get(page, b"Rotate").as_i64().unwrap_or(0)
        };
        assert_eq!(
            (rotation(3), rotation(4), rotation(5)),
            (90, 0, 0),
            "{label}"
        );
        let (mut engine, mut session, reply) = open(saved);
        assert_eq!(reply["pageCount"], 3, "{label}: {reply}");
        assert!(text_all(&mut engine, &mut session).contains("Beta"));

        // page-delete: only the first page goes.
        let (mut engine, mut session, _) = open(source.clone());
        let deleted = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "page-delete", "page": 0}),
        );
        assert_eq!(deleted["ok"], true, "{label}: {deleted}");
        let ids = dispatch(&mut engine, &mut session, json!({"op": "page-ids"}));
        assert_eq!(ids["ids"], json!([4, 5]), "{label}: {ids}");
        let text = text_all(&mut engine, &mut session);
        assert!(
            !text.contains("Alpha") && text.contains("Beta") && text.contains("Gamma"),
            "{label}: {text:?}"
        );
        let saved = save(&mut engine, &mut session);
        assert_eq!(mupdf_pages(&saved), [4, 5], "{label} deleted");
        let (mut engine, mut session, reply) = open(saved);
        assert_eq!(reply["pageCount"], 2, "{label}: {reply}");
        let text = text_all(&mut engine, &mut session);
        assert!(text.contains("Beta") && text.contains("Gamma"), "{label}");
    }
}

/// The strict page walk and the repair read a tree alike: the untyped node
/// above, with damage elsewhere (a broken `startxref`), gives the same page
/// list as without it.
#[test]
fn an_untyped_node_reads_alike_with_and_without_repair() {
    let page = |contents: u32| {
        whole(format!("<</Type/Page/Parent 10 0 R/MediaBox[0 0 300 300]/Resources<</Font<</F1 9 0 R>>>>/Contents {contents} 0 R>>").as_bytes())
    };
    let objects = vec![
        (1, whole(b"<</Type/Catalog/Pages 2 0 R>>")),
        (2, whole(b"<</Type/Pages/Kids[10 0 R]/Count 2>>")),
        (3, page(6)),
        (4, page(7)),
        (6, content("Alpha")),
        (7, content("Beta")),
        (
            9,
            whole(b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>"),
        ),
        (10, whole(b"<</Parent 2 0 R/Kids[3 0 R 4 0 R]/Count 2>>")),
    ];
    let source = assemble(&objects, "/Root 1 0 R");
    let (strict, _) = shown_and_saved(source.clone());
    let (repaired, _) = shown_and_saved(broken_startxref(source));
    assert_eq!(strict.ids, json!([3, 4]));
    assert_eq!((strict.ids, strict.text), (repaired.ids, repaired.text));
}

// ------------------------------------------- kids typed as other objects

/// A kid typed as another kind of object (/Font, /Annot, /XObject,
/// /Catalog) that has /Contents, or a /MediaBox and no kids, is a page to
/// the repair as it is to the strict walk, MuPDF and engine.ts; it keeps
/// its /Type. One with neither is left out by the repair (MuPDF and the
/// strict walk: a blank page).
#[test]
fn a_kid_typed_as_another_object_that_looks_like_a_page_stays_a_page() {
    for kind in ["/Font", "/Annot", "/XObject", "/Catalog"] {
        for (shape, extra, kept) in [
            ("contents", "/Contents 7 0 R", true),
            ("media box", "/MediaBox[0 0 200 200]", true),
            ("both", "/MediaBox[0 0 200 200]/Contents 7 0 R", true),
            ("neither", "/Contents(a note)", false),
        ] {
            let page = |contents: u32| {
                whole(format!("<</Type/Page/Parent 2 0 R/Contents {contents} 0 R/Resources<</Font<</F1 9 0 R>>>>>>").as_bytes())
            };
            let objects = vec![
                (1, whole(b"<</Type/Catalog/Pages 2 0 R>>")),
                (
                    2,
                    whole(b"<</Type/Pages/Kids[3 0 R 4 0 R 5 0 R]/Count 3/MediaBox[0 0 200 200]>>"),
                ),
                (3, page(6)),
                (
                    4,
                    whole(
                        format!(
                            "<</Type{kind}/Parent 2 0 R/Resources<</Font<</F1 9 0 R>>>>{extra}>>"
                        )
                        .as_bytes(),
                    ),
                ),
                (5, page(8)),
                (6, content("Alpha")),
                (7, content("Beta")),
                (8, content("Gamma")),
                (
                    9,
                    whole(
                        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>",
                    ),
                ),
            ];
            let source = assemble(&objects, "/Root 1 0 R");
            let label = format!("{kind} {shape}");
            // The strict walk shows it as a page whatever it holds.
            let (mut engine, mut session, reply) = open(source.clone());
            assert_eq!(reply["pageCount"], 3, "{label}: {reply}");
            let strict = text_all(&mut engine, &mut session);
            // The repair (xref lost) keeps it when it looks like a page.
            let (repaired, saved) = shown_and_saved(broken_startxref(source));
            let pages = if kept { 3 } else { 2 };
            assert_eq!(repaired.pages, pages, "{label}: {repaired:?}");
            assert_eq!(saved.pages, pages, "{label}: {saved:?}");
            // A page left out leaves out its (empty) text and page break.
            let strict = if kept {
                strict
            } else {
                strict.replacen("\u{c}\u{c}", "\u{c}", 1)
            };
            assert_eq!(repaired.text, strict, "{label}");
            assert_eq!(saved.text, strict, "{label}");
            assert_eq!(
                repaired.ids,
                if kept {
                    json!([3, 4, 5])
                } else {
                    json!([3, 5])
                },
                "{label}"
            );
        }
    }
}

// ------------------------------------------- a damaged trailer keyword

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/../fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// One damaged byte in the `trailer` keyword of an RC4 or AES-128 file
/// (`xrailer`, `8railer`): the repair takes /ID, /Root, /Encrypt and /Info
/// from the dictionary after it, which no keyword introduces, as MuPDF's
/// repair takes them from any dictionary outside an object. The file opens
/// locked, a wrong password is refused and the right one shows the text.
/// Before, the repair found no /ID and refused the file.
#[test]
fn a_damaged_trailer_keyword_keeps_the_file_identifier() {
    for name in [
        "encrypted.pdf",
        "encrypted-noedit.pdf",
        "p3/encrypted-edit-noannot.pdf",
        "encrypted-aes256.pdf",
    ] {
        let source = fixture(name);
        let (mut engine, mut session, reply) = open(source.clone());
        assert_eq!(reply["info"]["needsPassword"], true, "{name}");
        let pages = reply["pageCount"].clone();
        let unlocked = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "authenticate", "password": "pdf4all-user"}),
        );
        assert_eq!(unlocked["authenticated"], true, "{name}: {unlocked}");
        let expected = text_all(&mut engine, &mut session);
        assert!(!expected.trim().is_empty(), "{name}");
        let at = source.windows(7).rposition(|w| w == b"trailer").unwrap();
        for byte in *b"x8" {
            let mut bytes = source.clone();
            bytes[at] = byte;
            let label = format!("{name} {}railer", byte as char);
            let (mut engine, mut session, reply) = open(bytes.clone());
            assert_eq!(reply["ok"], true, "{label}: {reply}");
            assert_eq!(reply["info"]["needsPassword"], true, "{label}: {reply}");
            for password in ["wrong", "", "pdf4all"] {
                let wrong = dispatch(
                    &mut engine,
                    &mut session,
                    json!({"op": "authenticate", "password": password}),
                );
                assert_eq!(wrong["authenticated"], false, "{label}: {wrong}");
            }
            let unlocked = dispatch(
                &mut engine,
                &mut session,
                json!({"op": "authenticate", "password": "pdf4all-user"}),
            );
            assert_eq!(unlocked["authenticated"], true, "{label}: {unlocked}");
            assert_eq!(unlocked["pageCount"], pages, "{label}: {unlocked}");
            assert_eq!(text_all(&mut engine, &mut session), expected, "{label}");
            // The save keeps the encryption and reopens with the password.
            let saved = save(&mut engine, &mut session);
            let reopened =
                Document::open_with_password(&saved, DocumentLimits::default(), b"pdf4all-user")
                    .unwrap();
            assert!(reopened.security().is_some(), "{label}");
            // The owner password unlocks it too.
            let (mut engine, mut session, _) = open(bytes);
            let owner = dispatch(
                &mut engine,
                &mut session,
                json!({"op": "authenticate", "password": "pdf4all-owner"}),
            );
            assert_eq!(owner["authenticated"], true, "{label}: {owner}");
        }
    }
    // Unencrypted files read as before.
    for name in ["hello.pdf", "nav.pdf"] {
        let source = fixture(name);
        let (mut engine, mut session, reply) = open(source.clone());
        let expected = shown(&mut engine, &mut session, &reply);
        let at = source.windows(7).rposition(|w| w == b"trailer").unwrap();
        let mut bytes = source;
        bytes[at] = b'8';
        let (mut engine, mut session, reply) = open(bytes);
        assert_eq!(shown(&mut engine, &mut session, &reply), expected, "{name}");
    }
}

/// Dictionaries outside any object cost the scan at most one pass over the
/// input: a megabyte of unterminated dictionaries after the last object.
#[test]
fn loose_dictionaries_are_read_in_bounded_time() {
    let mut source = fixture("hello.pdf");
    let at = source.windows(4).rposition(|w| w == b"xref").unwrap();
    let junk: Vec<u8> = b"<</A <</B ".repeat(100_000);
    source.splice(at..at, junk);
    let started = Instant::now();
    let (_, _, reply) = open(source);
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
}

// --------------------------------------- wrong passwords on rebuilt files

/// A wrong password for a locked file that was rebuilt is answered from
/// the locked document when the rebuild does not depend on the password
/// (no object streams): the answer is the reload's, for wrong, right,
/// owner and empty passwords, on rebuilt (damaged trailer, a reference to
/// another generation) RC4, AES-128 and AES-256 files.
#[test]
fn a_wrong_password_for_a_rebuilt_locked_file_matches_a_reload() {
    let mut sources = Vec::new();
    for name in [
        "encrypted.pdf",
        "encrypted-noedit.pdf",
        "encrypted-aes256.pdf",
    ] {
        let source = fixture(name);
        let at = source.windows(7).rposition(|w| w == b"trailer").unwrap();
        let mut damaged = source.clone();
        damaged[at] = b'8';
        sources.push((format!("{name} 8railer"), damaged));
        // A reference naming another generation: rebuilt from the xref.
        let doc = Document::open_with_password(&source, DocumentLimits::default(), b"").ok();
        assert!(doc.is_none(), "{name} opens without a password");
        let needle = b"/Pages 2 0 R";
        if let Some(at) = source.windows(needle.len()).position(|w| w == needle) {
            let mut stale = source.clone();
            stale[at + 9] = b'1';
            sources.push((format!("{name} stale"), stale));
        }
    }
    assert!(sources.len() >= 4, "{}", sources.len());
    for (label, bytes) in sources {
        for password in [
            "wrong",
            "",
            "pdf4all-user",
            "pdf4all-owner",
            "pdf4all-userx",
        ] {
            let (mut engine, mut session, reply) = open(bytes.clone());
            assert_eq!(reply["info"]["needsPassword"], true, "{label}: {reply}");
            let answer = dispatch(
                &mut engine,
                &mut session,
                json!({"op": "authenticate", "password": password}),
            );
            let reload =
                AppDocument::load(std::sync::Arc::new(bytes.clone()), password.as_bytes()).unwrap();
            assert_eq!(
                answer["authenticated"],
                !reload.is_locked(),
                "{label} {password:?}: {answer}"
            );
        }
        // Without a reload: with the kept source replaced by bytes that do
        // not open, a wrong password is still answered, and only a right
        // one, which needs the reload, fails.
        let (mut engine, mut session, _) = open(bytes.clone());
        session.locked_bytes = Some(std::sync::Arc::new(b"not a PDF".to_vec()));
        let wrong = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "authenticate", "password": "wrong"}),
        );
        assert_eq!(wrong["authenticated"], false, "{label}: {wrong}");
        let right = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "authenticate", "password": "pdf4all-user"}),
        );
        assert_eq!(right["ok"], false, "{label}: {right}");
    }
}
