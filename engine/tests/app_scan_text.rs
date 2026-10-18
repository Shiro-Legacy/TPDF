// SPDX-License-Identifier: AGPL-3.0-or-later
//! `scan-compose` (app/ops/scan.rs), after review: right-to-left runs drawn
//! in visual order, NBSP and soft hyphen, the embedded CFF
//! widths and notices, the CJK subset's caps and name table, JPEG frame
//! checks, the request's item cap and each too-large refusal's detail.
//! Every test installs the CJK font first (process-wide, idempotent).
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession, info};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::view::set_cjk_font;

fn install() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/fonts/DroidSansFallback.ttf"
    );
    set_cjk_font(std::fs::read(path).unwrap()).unwrap();
}

fn compose(request: Value, images: Vec<Vec<u8>>) -> Result<Vec<u8>, Value> {
    install();
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let (reply, mut outputs) = engine.dispatch(&mut session, request, images);
    if reply["ok"] != true {
        return Err(reply);
    }
    Ok(outputs.remove(0))
}

fn page(runs: Value) -> Value {
    json!({"op": "scan-compose", "pages": [{"width": 612, "height": 792, "runs": runs}]})
}

/// A session with `bytes` open, for `text-all` and `search`.
fn reader(bytes: &[u8]) -> impl FnMut(Value) -> Value {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let open = json!({"id": 1, "op": "open", "name": "scan.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, open, vec![bytes.to_vec()]);
    assert_eq!(reply["ok"], true, "{reply}");
    move |request| engine.dispatch(&mut session, request, vec![]).0
}

fn font<'d>(doc: &'d Document<'_>, name: &str) -> &'d Dictionary {
    let page = info::pages(doc)[0].dict;
    let resources = doc.dict_get(page, b"Resources").as_dict().unwrap();
    let fonts = doc.dict_get(resources, b"Font").as_dict().unwrap();
    doc.dict_get(fonts, name.as_bytes()).as_dict().unwrap()
}

fn stream(doc: &Document<'_>, dict: &Dictionary, key: &[u8]) -> Vec<u8> {
    let id = dict.get(key).and_then(Object::as_reference).unwrap();
    doc.decoded_stream(id, StreamLimits::default()).unwrap()
}

fn content(doc: &Document<'_>) -> String {
    String::from_utf8(stream(doc, info::pages(doc)[0].dict, b"Contents")).unwrap()
}

#[test]
fn right_to_left_runs_draw_in_visual_order_and_extract_as_typed() {
    let lines = ["שלום עולם", "مرحبا بالعالم", "שנת 2026 (חדש) abc"];
    let runs: Vec<Value> = (lines.iter().enumerate())
        .map(|(i, text)| json!({"text": text, "x": 50, "y": 100 + 40 * i, "size": 20, "invisible": i == 0}))
        .collect();
    let bytes = compose(page(json!(runs)), vec![]).unwrap();
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let ops = content(&doc);
    // No /ActualText: a reader would reverse the visual-order glyphs again.
    assert!(!ops.contains("ActualText"), "{ops}");
    // The glyphs are in visual order: the CID of the last letter typed comes first.
    let cmap = String::from_utf8(stream(&doc, font(&doc, "F0"), b"ToUnicode")).unwrap();
    let first = cmap
        .lines()
        .find(|line| line.starts_with("<0001>"))
        .unwrap();
    assert_eq!(first, "<0001> <05DD>", "ם (U+05DD) is drawn first");
    let mut read = reader(&bytes);
    let text = read(json!({"op": "text-all"}))["text"]
        .as_str()
        .unwrap()
        .to_string();
    // A run of one direction extracts as typed; a mixed one keeps each
    // word's letters in order (the readers' own line order differs).
    for line in &lines[..2] {
        assert!(text.contains(line), "{line:?} in {text:?}");
    }
    for word in ["שלום", "עולם", "مرحبا", "חדש", "2026"] {
        let hits = read(json!({"op": "search", "query": word}))["hits"]
            .as_array()
            .unwrap()
            .len();
        assert!(hits >= 1, "{word}");
    }
}

#[test]
fn nbsp_extracts_as_itself_and_soft_hyphens_are_not_drawn() {
    let bytes = compose(
        page(json!([{"text": "a\u{A0}b\u{AD}c", "x": 50, "y": 100, "size": 20}])),
        vec![],
    )
    .unwrap();
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert!(
        content(&doc).contains("/F1 1 Tf <61a06263> Tj"),
        "{}",
        content(&doc)
    );
    let cmap = String::from_utf8(stream(&doc, font(&doc, "F1"), b"ToUnicode")).unwrap();
    assert!(cmap.contains("<A0> <00A0>"), "{cmap}");
    let mut read = reader(&bytes);
    let text = read(json!({"op": "text-all"}))["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("a\u{A0}bc"), "{text:?}");
    // scan-measure leaves the soft hyphen out as well.
    let (reply, _) = AppEngine::new().dispatch(
        &mut AppSession::default(),
        json!({"op": "scan-measure", "text": "b\u{AD}c"}),
        vec![],
    );
    assert_eq!(reply["chars"], json!(["b", "c"]));
}

/// From a bare CFF: defaultWidthX, nominalWidthX, each charstring's width
/// (the operand before its first rmoveto or endchar, if there is one more
/// than the operator takes) and the Top DICT's string operands.
struct Cff {
    widths: Vec<i32>,
    strings: Vec<Vec<u8>>,
    top: Vec<(Vec<i32>, Vec<u8>)>,
}

fn index(bytes: &[u8], at: usize) -> (Vec<&[u8]>, usize) {
    let count = usize::from(u16::from_be_bytes([bytes[at], bytes[at + 1]]));
    if count == 0 {
        return (vec![], at + 2);
    }
    let size = usize::from(bytes[at + 2]);
    let offset = |i: usize| {
        let start = at + 3 + i * size;
        bytes[start..start + size]
            .iter()
            .fold(0usize, |v, b| v << 8 | usize::from(*b))
    };
    let data = at + 3 + (count + 1) * size - 1;
    let items = (0..count)
        .map(|i| &bytes[data + offset(i)..data + offset(i + 1)])
        .collect();
    (items, data + offset(count))
}

/// A DICT's (operands, operator) pairs (integers only).
fn dict(bytes: &[u8]) -> Vec<(Vec<i32>, Vec<u8>)> {
    let (mut out, mut operands, mut at) = (Vec::new(), Vec::new(), 0);
    while at < bytes.len() {
        let b = bytes[at];
        match b {
            28 => {
                operands.push(i32::from(i16::from_be_bytes([
                    bytes[at + 1],
                    bytes[at + 2],
                ])));
                at += 3;
            }
            29 => {
                operands.push(i32::from_be_bytes(
                    bytes[at + 1..at + 5].try_into().unwrap(),
                ));
                at += 5;
            }
            32..=246 => {
                operands.push(i32::from(b) - 139);
                at += 1;
            }
            247..=250 => {
                operands.push((i32::from(b) - 247) * 256 + i32::from(bytes[at + 1]) + 108);
                at += 2;
            }
            251..=254 => {
                operands.push(-(i32::from(b) - 251) * 256 - i32::from(bytes[at + 1]) - 108);
                at += 2;
            }
            12 => {
                out.push((std::mem::take(&mut operands), vec![12, bytes[at + 1]]));
                at += 2;
            }
            _ => {
                out.push((std::mem::take(&mut operands), vec![b]));
                at += 1;
            }
        }
    }
    out
}

fn parse_cff(bytes: &[u8]) -> Cff {
    let (_, at) = index(bytes, usize::from(bytes[2]));
    let (tops, at) = index(bytes, at);
    let (strings, _) = index(bytes, at);
    let top = dict(tops[0]);
    let find = |op: u8| {
        top.iter()
            .find(|(_, o)| o == &[op])
            .map(|(v, _)| v.clone())
            .unwrap()
    };
    let charstrings = usize::try_from(find(17)[0]).unwrap();
    let private = find(18);
    let (size, offset) = (private[0] as usize, private[1] as usize);
    let private = dict(&bytes[offset..offset + size]);
    let value = |op: u8| {
        private
            .iter()
            .find(|(_, o)| o == &[op])
            .map_or(0, |(v, _)| v[0])
    };
    let (default, nominal) = (value(20), value(21));
    let (glyphs, _) = index(bytes, charstrings);
    let widths = glyphs
        .iter()
        .map(|glyph| {
            // Type 2 operands share the DICT integer forms used here but 28.
            let (pairs, mut operands) = (dict(glyph), 0);
            let (first, op) = pairs
                .iter()
                .find(|(_, op)| op == &[21] || op == &[14])
                .unwrap();
            operands += first.len();
            let takes = if op == &[21] { 2 } else { 0 };
            if operands > takes {
                nominal + first[0]
            } else {
                default
            }
        })
        .collect();
    Cff {
        widths,
        strings: strings.iter().map(|s| s.to_vec()).collect(),
        top,
    }
}

#[test]
fn the_nimbus_subsets_carry_their_widths_and_urw_notices() {
    let text = "Ωμέγα Привет ≤ ≥ → ∑ ‰ Ж";
    let bytes = compose(
        page(json!([
            {"text": text, "x": 50, "y": 100, "size": 20},
            {"text": text, "x": 50, "y": 140, "size": 20, "bold": true},
        ])),
        vec![],
    )
    .unwrap();
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    for name in ["N1_0", "N2_0"] {
        let font = font(&doc, name);
        let widths: Vec<i32> = (doc.dict_get(font, b"Widths").as_array().unwrap().iter())
            .map(|w| w.as_f64().unwrap() as i32)
            .collect();
        let descriptor = doc.dict_get(font, b"FontDescriptor").as_dict().unwrap();
        let cff = parse_cff(&stream(&doc, descriptor, b"FontFile3"));
        // Glyph 0 is .notdef; code i + 1 draws glyph i + 1, as /Widths says.
        assert_eq!(cff.widths[1..], widths[..], "{name}");
        assert!(widths.iter().any(|&w| w != widths[0]), "the widths differ");
        let copyright = cff.top.iter().find(|(_, op)| op == &[12, 0]).unwrap().0[0];
        let notice = cff.top.iter().find(|(_, op)| op == &[1]).unwrap().0[0];
        let string = |sid: i32| cff.strings[usize::try_from(sid - 391).unwrap()].clone();
        assert_eq!(
            string(copyright),
            b"Copyright 2016 by (URW)++ Design & Development."
        );
        assert!(string(notice).ends_with(b"SIL Open Font License, Version 1.1."));
    }
}

#[test]
fn the_cjk_subset_keeps_composites_licence_names_and_its_caps() {
    // Every CJK Unified (BMP) character: 20,992 glyphs, which the flattened
    // subset could not hold.
    let all: Vec<char> = (0x4E00..=0x9FFF).filter_map(char::from_u32).collect();
    let runs = |chars: &[char]| -> Value {
        let runs: Vec<Value> = chars
            .chunks(4000)
            .map(|chunk| json!({"text": chunk.iter().collect::<String>(), "x": 0, "y": 10, "size": 8, "invisible": true}))
            .collect();
        json!(runs)
    };
    let started = std::time::Instant::now();
    let bytes = compose(page(runs(&all)), vec![]).unwrap();
    assert!(bytes.len() < 4_000_000, "{}", bytes.len());
    eprintln!(
        "20,992 CJK characters: {} bytes in {:?}",
        bytes.len(),
        started.elapsed()
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let cjk = font(&doc, "F0");
    let descendant = doc.dict_get(cjk, b"DescendantFonts").as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let descendant = doc.object(descendant).unwrap().value.as_dict().unwrap();
    let descriptor = doc
        .dict_get(descendant, b"FontDescriptor")
        .as_dict()
        .unwrap();
    let program = stream(&doc, descriptor, b"FontFile2");
    // The name table holds the copyright and licence records only.
    let count = usize::from(u16::from_be_bytes([program[4], program[5]]));
    let name = (0..count)
        .map(|i| 12 + 16 * i)
        .find(|&r| &program[r..r + 4] == b"name")
        .map(|r| u32::from_be_bytes(program[r + 8..r + 12].try_into().unwrap()) as usize)
        .expect("a name table");
    let records = usize::from(u16::from_be_bytes([program[name + 2], program[name + 3]]));
    let ids: Vec<u16> = (0..records)
        .map(|i| {
            u16::from_be_bytes([
                program[name + 6 + 12 * i + 6],
                program[name + 6 + 12 * i + 7],
            ])
        })
        .collect();
    assert!(
        !ids.is_empty() && ids.iter().all(|id| matches!(id, 0 | 13 | 14)),
        "{ids:?}"
    );
    assert!(ids.contains(&0), "the copyright");

    // 60,000 distinct characters (here ones the font lacks, so each is a
    // 16-byte repeat of .notdef) are the cap; one more is refused.
    let astral: Vec<char> = (0x2_0000..)
        .filter_map(char::from_u32)
        .take(60_001)
        .collect();
    let pages = |chars: &[char]| -> Value {
        let pages: Vec<Value> = chars
            .chunks(40_000)
            .map(|chunk| json!({"width": 612, "height": 792, "runs": runs(chunk)}))
            .collect();
        json!({"op": "scan-compose", "pages": pages})
    };
    let at_cap = compose(pages(&astral[..60_000]), vec![]).unwrap();
    assert!(at_cap.len() < 3_000_000, "{}", at_cap.len());
    let over = compose(pages(&astral), vec![]).unwrap_err();
    assert_eq!(
        (over["reason"].as_str(), over["detail"].as_str()),
        (Some("too-large"), Some("characters")),
        "{over}"
    );
}

#[test]
fn jpeg_frames_with_no_size_huge_sizes_or_arithmetic_coding_are_refused() {
    let jpeg = |marker: u8, width: u16, height: u16| {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, marker, 0, 17, 8];
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.push(3);
        for c in 1..=3u8 {
            bytes.extend_from_slice(&[c, 0x11, 0]);
        }
        bytes.extend_from_slice(&[0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);
        bytes
    };
    let request = json!({"op": "scan-compose", "images": [{"$buffer": 0}],
        "pages": [{"width": 612, "height": 792, "image": 0}]});
    assert!(compose(request.clone(), vec![jpeg(0xC0, 16_384, 16_384)]).is_ok());
    assert!(
        compose(request.clone(), vec![jpeg(0xC2, 10, 10)]).is_ok(),
        "progressive is fine"
    );
    for (image, why) in [
        (jpeg(0xC0, 0, 10), "1 to 16384 a side"),
        (jpeg(0xC0, 10, 0), "1 to 16384 a side"),
        (jpeg(0xC0, 16_385, 10), "1 to 16384 a side"),
        (jpeg(0xC9, 10, 10), "arithmetic-coded"),
        (jpeg(0xCA, 10, 10), "arithmetic-coded"),
    ] {
        let reply = compose(request.clone(), vec![image]).unwrap_err();
        assert_eq!(reply["reason"], "invalid");
        assert!(reply["error"].as_str().unwrap().contains(why), "{reply}");
    }
}

#[test]
fn every_too_large_refusal_names_its_limit() {
    let refused = |request: Value, images: Vec<Vec<u8>>| -> String {
        let reply = compose(request, images).unwrap_err();
        assert_eq!(reply["reason"], "too-large", "{reply}");
        reply["detail"].as_str().unwrap().to_string()
    };
    let blank = json!({"width": 10, "height": 10});
    assert_eq!(
        refused(
            json!({"op": "scan-compose", "pages": vec![blank.clone(); 101]}),
            vec![]
        ),
        "pages"
    );
    let image = |n: usize| json!({"op": "scan-compose", "images": (0..n).map(|i| json!({"$buffer": i})).collect::<Vec<_>>(), "pages": [blank]});
    assert_eq!(
        refused(image(1), vec![vec![0xFF; 32 * 1024 * 1024 + 1]]),
        "image-size"
    );
    assert_eq!(
        refused(image(2), vec![vec![0xFF; 26 * 1024 * 1024]; 2]),
        "images"
    );
    let run = json!({"text": "x", "x": 1, "y": 1, "size": 1});
    let many = |pages: usize, per: usize| json!({"op": "scan-compose", "pages": vec![json!({"width": 10, "height": 10, "runs": vec![run.clone(); per]}); pages]});
    assert_eq!(refused(many(1, 4001), vec![]), "runs");
    // 40,000 runs and boxes in a request; 10 x 4,000 is the cap.
    assert!(compose(many(10, 4000), vec![]).is_ok());
    assert_eq!(refused(many(11, 4000), vec![]), "runs");
    let boxes = json!({"op": "scan-compose", "pages": [
        {"width": 10, "height": 10, "runs": vec![run.clone(); 4000], "boxes": vec![json!({"x": 1, "y": 1, "width": 1, "height": 1}); 4000]}
    ]});
    assert!(
        compose(boxes, vec![]).is_ok(),
        "boxes count apart from runs on a page"
    );
    let long = json!({"text": "x".repeat(4001), "x": 1, "y": 1, "size": 1});
    assert_eq!(refused(page(json!([long])), vec![]), "text");
}

#[test]
fn numbers_and_pointed_letters_in_right_to_left_runs_extract_as_typed() {
    let lines = [
        "מחיר 3.50 שקל",
        "תאריך 12/05/2026",
        "סך הכל 1,000 יחידות",
        "הנחה 50% היום",
        "מחיר ₪25 או $30",
        "שָׁלוֹם עֲלֵיכֶם",
        "مَرْحَبًا بِالْعَالَمِ",
    ];
    let runs: Vec<Value> = (lines.iter().enumerate())
        .map(|(i, text)| json!({"text": text, "x": 50, "y": 60 + 40 * i, "size": 20, "invisible": i % 2 == 0}))
        .collect();
    let bytes = compose(page(json!(runs)), vec![]).unwrap();
    let mut read = reader(&bytes);
    let text = read(json!({"op": "text-all"}))["text"]
        .as_str()
        .unwrap()
        .to_string();
    // Pointed words come back with each mark after its letter, a line-final
    // mark too (behind the RLM drawn before it).
    for line in &lines[5..] {
        assert!(text.contains(line), "{line:?} in {text:?}");
    }
    assert!(text.contains("\u{650}\u{200F}"), "{text:?}");
    // Numbers keep their typed order; the reader orders a line's segments its
    // own way ("שקל3.50 מחיר"), so the search is by word.
    for number in ["3.50", "12/05/2026", "1,000", "50%"] {
        assert!(text.contains(number), "{number} in {text:?}");
    }
    for word in [
        "3.50",
        "12/05/2026",
        "1,000",
        "50%",
        "₪25",
        "$30",
        "שָׁלוֹם",
        "עֲלֵיכֶם",
        "مَرْحَبًا",
    ] {
        let hits = read(json!({"op": "search", "query": word}))["hits"]
            .as_array()
            .unwrap()
            .len();
        assert!(hits >= 1, "{word}");
    }
    // Combining marks the CJK font lacks take no room: one line each, the
    // marks' CIDs 0 wide.
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let cjk = font(&doc, "F0");
    let descendant = doc.dict_get(cjk, b"DescendantFonts").as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let descendant = doc.object(descendant).unwrap().value.as_dict().unwrap();
    let widths = doc.dict_get(descendant, b"W").as_array().unwrap()[1]
        .as_array()
        .unwrap();
    let cmap = String::from_utf8(stream(&doc, cjk, b"ToUnicode")).unwrap();
    let qamats = cmap.lines().find(|line| line.ends_with("<05B8>")).unwrap();
    let cid = usize::from_str_radix(&qamats[1..5], 16).unwrap();
    assert_eq!(widths[cid - 1].as_f64(), Some(0.0), "{qamats}");
    let measure = AppEngine::new()
        .dispatch(
            &mut AppSession::default(),
            json!({"op": "scan-measure", "text": "\u{5B8}"}),
            vec![],
        )
        .0;
    assert_eq!(measure["regular"][0].as_f64(), Some(0.0), "{measure}");
}
