// SPDX-License-Identifier: MIT
//! `scan-compose` and `scan-measure` (app/ops/scan.rs): the scanner's page
//! sizes, the JPEG stored as it is, text that extracts as given at the
//! positions given, the font each script takes, ToUnicode, the CJK font's
//! subset and the limits. The CJK font is process-wide, so one test
//! installs it and every other test sticks to text that does not need it.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession, info};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object, decode_text_string};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::view::{ViewDocument, cjk_font_wanted, set_cjk_font};

fn dispatch(request: Value, buffers: Vec<Vec<u8>>) -> (Value, Vec<Vec<u8>>) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    engine.dispatch(&mut session, request, buffers)
}

/// The composed file, or the failed reply.
fn compose(request: Value, images: Vec<Vec<u8>>) -> Result<Vec<u8>, Value> {
    let (reply, mut outputs) = dispatch(request, images);
    if reply["ok"] != true {
        return Err(reply);
    }
    assert_eq!(reply["bytes"], json!({"$buffer": 0}), "{reply}");
    Ok(outputs.remove(0))
}

/// JPEG headers only (SOI, a baseline frame header, an empty scan, EOI):
/// what the op reads, as pdf-from-images' tests build them.
fn jpeg(width: u16, height: u16, extra: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8];
    bytes.extend_from_slice(extra);
    let mut frame = vec![8];
    frame.extend_from_slice(&height.to_be_bytes());
    frame.extend_from_slice(&width.to_be_bytes());
    frame.push(3);
    for c in 0..3u8 {
        frame.extend_from_slice(&[c + 1, 0x11, 0]);
    }
    bytes.extend_from_slice(&[0xFF, 0xC0]);
    bytes.extend_from_slice(&(frame.len() as u16 + 2).to_be_bytes());
    bytes.extend_from_slice(&frame);
    bytes.extend_from_slice(&[0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);
    bytes
}

fn open(bytes: &[u8]) -> Document<'_> {
    Document::open(bytes, DocumentLimits::default()).unwrap()
}

fn page_dict<'d>(doc: &'d Document<'_>, index: usize) -> &'d Dictionary {
    info::pages(doc)[index].dict
}

fn resource<'d>(doc: &'d Document<'_>, page: &'d Dictionary, kind: &[u8]) -> &'d Dictionary {
    let resources = doc.dict_get(page, b"Resources").as_dict().unwrap();
    doc.dict_get(resources, kind).as_dict().unwrap()
}

fn content(doc: &Document<'_>, page: usize) -> String {
    let id = doc
        .dict_get(page_dict(doc, page), b"Contents")
        .as_reference();
    let id = id
        .or_else(|| {
            page_dict(doc, page)
                .get(b"Contents".as_slice())
                .and_then(Object::as_reference)
        })
        .unwrap();
    String::from_utf8(doc.decoded_stream(id, StreamLimits::default()).unwrap()).unwrap()
}

/// A font resource's dictionary by its name.
fn font<'d>(doc: &'d Document<'_>, page: usize, name: &str) -> &'d Dictionary {
    let fonts = resource(doc, page_dict(doc, page), b"Font");
    doc.dict_get(fonts, name.as_bytes()).as_dict().unwrap()
}

/// Every character a page's text extraction finds, with its baseline
/// origin and right edge (y down).
fn chars(bytes: &[u8], page: usize) -> Vec<(char, f64, f64, f64)> {
    let view = ViewDocument::open(bytes.to_vec(), "").unwrap();
    let text = view.structured_text(page).unwrap();
    (text.blocks().iter())
        .flat_map(|block| &block.lines)
        .flat_map(|line| &line.chars)
        .filter(|c| !c.synthetic)
        .map(|c| (c.c, c.origin[0], c.origin[1], c.quad[2]))
        .collect()
}

/// The open and validate checks the scan worker runs on what it composed.
fn reopens(bytes: &[u8], pages: usize) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let open = json!({"id": 1, "op": "open", "name": "scan.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, open, vec![bytes.to_vec()]);
    assert_eq!(
        (reply["ok"].clone(), reply["pageCount"].clone()),
        (json!(true), json!(pages)),
        "{reply}"
    );
    let validate = json!({"id": 2, "op": "validate", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, validate, vec![bytes.to_vec()]);
    assert_eq!(reply["ok"], true, "{reply}");
}

#[test]
fn a_searchable_scan_keeps_its_jpeg_page_size_and_text() {
    let image = jpeg(1224, 1584, &[]);
    let line = "Hello scan résumé";
    let request = json!({
        "op": "scan-compose",
        "producer": "Tandem PDF",
        "images": [{"$buffer": 0}, {"$buffer": 1}],
        "pages": [
            {"width": 612, "height": 792, "image": 0,
             "runs": [{"text": line, "x": 61.2, "y": 177.44, "size": 23.76, "invisible": true, "width": 306}]},
            {"width": 612, "height": 792, "image": 1},
            {"width": 792, "height": 612, "image": 0, "runs": [{"text": " \t ", "x": 1, "y": 1, "size": 1}]},
        ],
    });
    let bytes = compose(request, vec![image.clone(), image.clone()]).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.7"));
    let doc = open(&bytes);
    assert_eq!(info::pages(&doc).len(), 3);
    let media = |index| {
        (doc.dict_get(page_dict(&doc, index), b"MediaBox")
            .as_array()
            .unwrap()
            .iter())
        .map(|v| v.as_f64().unwrap())
        .collect::<Vec<_>>()
    };
    assert_eq!(media(0), [0.0, 0.0, 612.0, 792.0]);
    assert_eq!(media(2), [0.0, 0.0, 792.0, 612.0]);
    // The JPEG as it is, stored once for three pages and two equal inputs.
    let images: Vec<_> = (0..3)
        .map(|i| {
            resource(&doc, page_dict(&doc, i), b"XObject")
                .get(b"Im0".as_slice())
                .unwrap()
                .clone()
        })
        .collect();
    let id = images[0].as_reference().unwrap();
    assert!(images.iter().all(|i| i.as_reference() == Some(id)));
    assert_eq!(doc.stream_data(id).unwrap().unwrap(), image.as_slice());
    let dict = doc.object(id).unwrap().value.as_dict().unwrap();
    assert_eq!(
        dict.get(b"Filter".as_slice()),
        Some(&Object::name("DCTDecode"))
    );
    assert_eq!(
        dict.get(b"ColorSpace".as_slice()),
        Some(&Object::name("DeviceRGB"))
    );
    let info = doc
        .trailer_entry(b"Info")
        .and_then(Object::as_reference)
        .unwrap();
    let info = doc.object(info).unwrap().value.as_dict().unwrap();
    let producer = info
        .get(b"Producer".as_slice())
        .and_then(Object::as_string)
        .unwrap();
    assert_eq!(decode_text_string(producer), "Tandem PDF");

    // The OCR line reads back as given, from its x and baseline, fitted to
    // its width; invisible, with Helvetica (not embedded) only.
    let found = chars(&bytes, 0);
    assert_eq!(found.iter().map(|c| c.0).collect::<String>(), line);
    let (first, last) = (found.first().unwrap(), found.last().unwrap());
    assert!(
        (first.1 - 61.2).abs() < 0.2 && (first.2 - 177.44).abs() < 0.2,
        "{first:?}"
    );
    assert!((last.3 - (61.2 + 306.0)).abs() < 1.0, "{last:?}");
    let fonts = resource(&doc, page_dict(&doc, 0), b"Font");
    assert_eq!(fonts.len(), 1);
    let helvetica = font(&doc, 0, "F1");
    assert_eq!(
        doc.dict_get(helvetica, b"BaseFont").as_name(),
        Some(b"Helvetica".as_slice())
    );
    assert!(helvetica.get(b"FontDescriptor".as_slice()).is_none());
    let ops = content(&doc, 0);
    assert!(
        ops.starts_with("q 612 0 0 792 0 0 cm /Im0 Do Q\nBT 3 Tr "),
        "{ops}"
    );
    // A run of nothing but white space and controls still draws its spaces;
    // the third page has no text object for the tab (it became a space).
    assert!(
        content(&doc, 2).contains("<202020>"),
        "{}",
        content(&doc, 2)
    );
    reopens(&bytes, 3);
}

#[test]
fn rebuilt_text_takes_helvetica_latin_names_then_nimbus_outlines() {
    let lines = [
        ("Zażółć gęślą jaźń Příliš Ωμέγα", false),
        ("Привет Ђ ≤ ≥ → € ∑ ƒ", true),
        ("Plain WinAnsi café", false),
    ];
    let runs: Vec<Value> = (lines.iter().enumerate())
        .map(|(i, (text, bold))| {
            json!({"text": text, "x": 52, "y": 80 + 30 * i, "size": 14, "bold": bold,
                   "color": [0.12, 0.16, 0.15]})
        })
        .collect();
    let request = json!({
        "op": "scan-compose",
        "pages": [{"width": 595, "height": 842, "runs": runs,
                   "boxes": [{"x": 52, "y": 300, "width": 245.5, "height": 40, "lineWidth": 0.5,
                              "color": [0.75, 0.78, 0.76]}]}],
    });
    let bytes = compose(request, vec![]).unwrap();
    let doc = open(&bytes);
    assert!(
        doc.trailer_entry(b"Info").is_none(),
        "no producer, no /Info"
    );
    // Text extraction gives each line back, at its baseline.
    let found = chars(&bytes, 0);
    let all: String = found.iter().map(|c| c.0).collect();
    let expected: String = lines.iter().map(|(text, _)| *text).collect();
    assert_eq!(all, expected);
    let mut at = 0;
    for (i, (text, _)) in lines.iter().enumerate() {
        let first = found[at];
        assert!(
            (first.1 - 52.0).abs() < 0.2 && (first.2 - (80.0 + 30.0 * i as f64)).abs() < 0.2,
            "{first:?}"
        );
        at += text.chars().count();
    }
    // Which fonts: WinAnsi Helvetica (F1, F2), the Latin glyph names (E1)
    // and Nimbus subsets (N1_0, N2_0) with their programs and ToUnicode.
    let fonts = resource(&doc, page_dict(&doc, 0), b"Font");
    let mut names: Vec<String> = fonts
        .iter()
        .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["E1", "F1", "F2", "N1_0", "N2_0"]);
    let latin = font(&doc, 0, "E1");
    assert_eq!(
        doc.dict_get(latin, b"BaseFont").as_name(),
        Some(b"Helvetica".as_slice())
    );
    assert!(latin.get(b"ToUnicode".as_slice()).is_some());
    for name in ["N1_0", "N2_0"] {
        let nimbus = font(&doc, 0, name);
        let base = doc.dict_get(nimbus, b"BaseFont").as_name().unwrap();
        let style = if name == "N1_0" { "Regular" } else { "Bold" };
        assert!(base.ends_with(format!("+NimbusSans-{style}").as_bytes()) && base[6] == b'+');
        let descriptor = doc.dict_get(nimbus, b"FontDescriptor").as_dict().unwrap();
        let file = descriptor
            .get(b"FontFile3".as_slice())
            .and_then(Object::as_reference)
            .unwrap();
        let program = doc.decoded_stream(file, StreamLimits::default()).unwrap();
        assert_eq!(&program[..4], &[1, 0, 4, 4], "a CFF header");
        assert_eq!(
            doc.dict_get(doc.stream_dict(file).unwrap(), b"Subtype")
                .as_name(),
            Some(b"Type1C".as_slice())
        );
        assert!(nimbus.get(b"ToUnicode".as_slice()).is_some());
    }
    let ops = content(&doc, 0);
    assert!(
        ops.starts_with("q 0.5 w 0 J 0 j 10 M 0.75 0.78 0.76 RG 52 502 245.5 40 re S Q\n"),
        "{ops}"
    );
    assert!(
        ops.contains("BT 0 Tr 0.12 0.16 0.15 rg 14 0 0 14 52 762 Tm /F1 1 Tf <5a61> Tj /E1 1 Tf"),
        "{ops}"
    );
    reopens(&bytes, 1);
}

#[test]
fn measure_gives_the_advances_compose_draws_with() {
    let (reply, _) = dispatch(
        json!({"op": "scan-measure", "text": "AA é\tż Ω\u{7}"}),
        vec![],
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["chars"], json!(["A", " ", "é", "ż", "Ω"]));
    let numbers = |key: &str| -> Vec<f64> {
        reply[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect()
    };
    assert_eq!(numbers("regular"), [667.0, 278.0, 556.0, 500.0, 748.0]);
    assert_eq!(numbers("bold"), [722.0, 278.0, 556.0, 500.0, 802.0]);
    assert_eq!(
        reply["fonts"],
        json!(["helvetica", "helvetica", "helvetica", "latin", "nimbus"])
    );
    // A fitted run's last glyph ends at its width; an unfitted one at the
    // sum of the measured advances.
    let request = json!({"op": "scan-compose", "pages": [{"width": 300, "height": 100, "runs": [
        {"text": "AA éż Ω", "x": 10, "y": 50, "size": 10},
    ]}]});
    let bytes = compose(request, vec![]).unwrap();
    let found = chars(&bytes, 0);
    let width = (667.0 * 2.0 + 278.0 * 2.0 + 556.0 + 500.0 + 748.0) / 1000.0 * 10.0;
    assert!(
        (found.last().unwrap().3 - (10.0 + width)).abs() < 0.2,
        "{found:?}"
    );
    let (reply, _) = dispatch(json!({"op": "scan-measure", "text": 5}), vec![]);
    assert_eq!(
        (reply["ok"].clone(), reply["reason"].clone()),
        (json!(false), json!("invalid"))
    );
}

#[test]
fn an_embedded_icc_profile_becomes_the_colour_space() {
    let profile = b"a stand-in ICC profile, split across two chunks";
    let app2 = |part: u8, data: &[u8]| {
        let mut chunk = vec![0xFF, 0xE2];
        chunk.extend_from_slice(&(16 + data.len() as u16).to_be_bytes());
        chunk.extend_from_slice(b"ICC_PROFILE\0");
        chunk.extend_from_slice(&[part, 2]);
        chunk.extend_from_slice(data);
        chunk
    };
    let extra = [app2(2, &profile[20..]), app2(1, &profile[..20])].concat();
    let image = jpeg(10, 20, &extra);
    let request = json!({"op": "scan-compose", "images": [{"$buffer": 0}],
        "pages": [{"width": 396, "height": 792, "image": 0}]});
    let bytes = compose(request, vec![image.clone()]).unwrap();
    let doc = open(&bytes);
    let xobject = resource(&doc, page_dict(&doc, 0), b"XObject");
    let id = xobject
        .get(b"Im0".as_slice())
        .and_then(Object::as_reference)
        .unwrap();
    assert_eq!(
        doc.stream_data(id).unwrap().unwrap(),
        image.as_slice(),
        "byte for byte, profile included"
    );
    let space = doc
        .dict_get(doc.stream_dict(id).unwrap(), b"ColorSpace")
        .as_array()
        .unwrap();
    assert_eq!(space[0], Object::name("ICCBased"));
    let icc = space[1].as_reference().unwrap();
    assert_eq!(
        doc.dict_get(doc.stream_dict(icc).unwrap(), b"N").as_i64(),
        Some(3)
    );
    assert_eq!(
        doc.decoded_stream(icc, StreamLimits::default()).unwrap(),
        profile
    );
}

#[test]
fn bad_and_oversized_requests_are_refused() {
    let refused = |request: Value, images: Vec<Vec<u8>>| -> (String, String) {
        let reply = compose(request, images).unwrap_err();
        assert_eq!(reply["ok"], false);
        (
            reply["reason"].as_str().unwrap_or("").into(),
            reply["error"].as_str().unwrap().into(),
        )
    };
    let page = json!({"width": 612, "height": 792});
    let pages = |n: usize| vec![page.clone(); n];
    assert!(compose(json!({"op": "scan-compose", "pages": pages(100)}), vec![]).is_ok());
    let (reason, error) = refused(json!({"op": "scan-compose", "pages": pages(101)}), vec![]);
    assert_eq!(
        (reason.as_str(), error.as_str()),
        ("too-large", "scan-compose: a scan has at most 100 pages")
    );
    assert_eq!(
        refused(json!({"op": "scan-compose", "pages": []}), vec![]).0,
        "invalid"
    );
    let (reason, error) = refused(
        json!({"op": "scan-compose", "images": [{"$buffer": 0}], "pages": [{"width": 1, "height": 1, "image": 0}]}),
        vec![b"\x89PNG\r\n\x1a\nnot a jpeg".to_vec()],
    );
    assert_eq!(
        (reason.as_str(), error.as_str()),
        ("invalid", "scan-compose: image 0 is not a JPEG")
    );
    let truncated = jpeg(10, 10, &[])[..12].to_vec();
    let (reason, error) = refused(
        json!({"op": "scan-compose", "images": [{"$buffer": 0}], "pages": [{"width": 1, "height": 1, "image": 0}]}),
        vec![truncated],
    );
    assert_eq!(reason, "invalid");
    assert!(
        error.contains("image 0 is not a JPEG the app can read"),
        "{error}"
    );
    assert_eq!(
        refused(
            json!({"op": "scan-compose", "pages": [{"width": 1, "height": 1, "image": 0}]}),
            vec![]
        )
        .0,
        "invalid",
        "an image index needs an image"
    );
    // Images: 32 MiB each, 50 MiB together, before any is read.
    let big = vec![0xFF; 32 * 1024 * 1024 + 1];
    let (reason, _) = refused(
        json!({"op": "scan-compose", "images": [{"$buffer": 0}], "pages": [page]}),
        vec![big],
    );
    assert_eq!(reason, "too-large");
    let half = vec![0xFF; 26 * 1024 * 1024];
    let (reason, error) = refused(
        json!({"op": "scan-compose", "images": [{"$buffer": 0}, {"$buffer": 1}], "pages": [page]}),
        vec![half.clone(), half],
    );
    assert_eq!(
        (reason.as_str(), error.as_str()),
        (
            "too-large",
            "scan-compose: the images total more than 52428800 bytes"
        )
    );
    // Text: 4,000 runs a page, 4,000 characters a run, 300,000 in all.
    let run = json!({"text": "x", "x": 1, "y": 1, "size": 1});
    let runs = |n: usize| json!({"width": 10, "height": 10, "runs": vec![run.clone(); n]});
    assert!(compose(json!({"op": "scan-compose", "pages": [runs(4000)]}), vec![]).is_ok());
    assert_eq!(
        refused(json!({"op": "scan-compose", "pages": [runs(4001)]}), vec![]).0,
        "too-large"
    );
    let long = |n: usize| json!({"text": "x".repeat(n), "x": 1, "y": 1, "size": 1});
    assert!(compose(json!({"op": "scan-compose", "pages": [{"width": 10, "height": 10, "runs": [long(4000)]}]}), vec![]).is_ok());
    assert_eq!(refused(json!({"op": "scan-compose", "pages": [{"width": 10, "height": 10, "runs": [long(4001)]}]}), vec![]).0, "too-large");
    let full = json!({"width": 10, "height": 10, "runs": vec![long(4000); 76]});
    assert_eq!(
        refused(json!({"op": "scan-compose", "pages": [full]}), vec![]).0,
        "too-large",
        "304,000 characters"
    );
    let (reply, _) = dispatch(
        json!({"op": "scan-measure", "text": "x".repeat(300_001)}),
        vec![],
    );
    assert_eq!(reply["reason"], "too-large");
    // Numbers and kinds.
    for bad in [
        json!({"width": 0, "height": 10}),
        json!({"width": 14_401, "height": 10}),
        json!({"width": "10", "height": 10}),
        json!({"width": 10, "height": 10, "runs": [{"text": "x", "x": 1, "y": 1, "size": 0}]}),
        json!({"width": 10, "height": 10, "runs": [{"text": "x", "x": 1, "y": 1, "size": 1, "color": [2, 0, 0]}]}),
        json!({"width": 10, "height": 10, "runs": [{"text": "x", "x": 1, "y": 1, "size": 1, "bold": "yes"}]}),
        json!({"width": 10, "height": 10, "runs": [{"text": 1, "x": 1, "y": 1, "size": 1}]}),
        json!({"width": 10, "height": 10, "runs": {}}),
        json!({"width": 10, "height": 10, "boxes": [{"x": 1, "y": 1, "width": -1, "height": 1}]}),
    ] {
        assert_eq!(
            refused(
                json!({"op": "scan-compose", "pages": [bad.clone()]}),
                vec![]
            )
            .0,
            "invalid",
            "{bad}"
        );
    }
    assert_eq!(
        refused(
            json!({"op": "scan-compose", "pages": [page], "producer": 5}),
            vec![]
        )
        .0,
        "invalid"
    );
}

#[test]
fn cjk_text_needs_the_installed_font_and_embeds_a_subset_of_it() {
    // Before a font is installed: refused, and the host is asked for it.
    let request = json!({"op": "scan-compose", "pages": [{"width": 612, "height": 792, "runs": [
        {"text": "Hello 中文 日本語 한국어 ✓", "x": 61.2, "y": 177.44, "size": 23.76, "invisible": true, "width": 306},
    ]}]});
    let reply = compose(request.clone(), vec![]).unwrap_err();
    assert_eq!(reply["reason"], "no-font", "{reply}");
    assert!(cjk_font_wanted());
    let (reply, _) = dispatch(json!({"op": "scan-measure", "text": "中"}), vec![]);
    assert_eq!(reply["reason"], "no-font");
    // Latin, Greek and Cyrillic never need it.
    assert!(
        compose(
            json!({"op": "scan-compose", "pages": [{"width": 9, "height": 9, "runs": [
        {"text": "Ωж", "x": 1, "y": 1, "size": 1}]}]}),
            vec![]
        )
        .is_ok()
    );

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../assets/fonts/DroidSansFallback.ttf"
    );
    set_cjk_font(std::fs::read(path).unwrap()).unwrap();
    assert!(!cjk_font_wanted());
    let bytes = compose(request, vec![]).unwrap();
    let found = chars(&bytes, 0);
    assert_eq!(
        found.iter().map(|c| c.0).collect::<String>(),
        "Hello 中文 日本語 한국어 ✓"
    );
    let (first, last) = (found.first().unwrap(), found.last().unwrap());
    assert!(
        (first.1 - 61.2).abs() < 0.2 && (first.2 - 177.44).abs() < 0.2,
        "{first:?}"
    );
    assert!((last.3 - 367.2).abs() < 1.0, "{last:?}");
    let doc = open(&bytes);
    let cjk = font(&doc, 0, "F0");
    assert_eq!(
        doc.dict_get(cjk, b"Subtype").as_name(),
        Some(b"Type0".as_slice())
    );
    assert_eq!(
        doc.dict_get(cjk, b"Encoding").as_name(),
        Some(b"Identity-H".as_slice())
    );
    let base = doc.dict_get(cjk, b"BaseFont").as_name().unwrap();
    assert!(base.ends_with(b"+DroidSansFallback"));
    let cmap = cjk
        .get(b"ToUnicode".as_slice())
        .and_then(Object::as_reference)
        .unwrap();
    let cmap =
        String::from_utf8(doc.decoded_stream(cmap, StreamLimits::default()).unwrap()).unwrap();
    assert!(
        cmap.contains("<0001> <4E2D>")
            && cmap.contains("9 beginbfchar")
            && cmap.contains("<0009> <2713>"),
        "{cmap}"
    );
    let descendant = doc.dict_get(cjk, b"DescendantFonts").as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let descendant = doc.object(descendant).unwrap().value.as_dict().unwrap();
    let descriptor = doc
        .dict_get(descendant, b"FontDescriptor")
        .as_dict()
        .unwrap();
    let file = descriptor
        .get(b"FontFile2".as_slice())
        .and_then(Object::as_reference)
        .unwrap();
    let program = doc.decoded_stream(file, StreamLimits::default()).unwrap();
    assert_eq!(&program[..4], b"\0\x01\0\0");
    assert!(
        program.len() < 20_000,
        "a subset of 10 glyphs, not the 3.4 MB font: {}",
        program.len()
    );
    assert!(bytes.len() < 20_000, "{}", bytes.len());
    reopens(&bytes, 1);

    // The same characters, the same tag; others, another.
    let tag = |text: &str| {
        let request = json!({"op": "scan-compose", "pages": [{"width": 9, "height": 9, "runs": [
            {"text": text, "x": 1, "y": 1, "size": 1}]}]});
        let bytes = compose(request, vec![]).unwrap();
        let doc = open(&bytes);
        doc.dict_get(font(&doc, 0, "F0"), b"BaseFont")
            .as_name()
            .unwrap()[..6]
            .to_vec()
    };
    assert_eq!(tag("中文"), tag("中文"));
    assert_ne!(tag("中文"), tag("日本"));

    // Measured: the CJK font's advances, and boxes for what no font has.
    let (reply, _) = dispatch(json!({"op": "scan-measure", "text": "中✓"}), vec![]);
    assert_eq!(reply["fonts"], json!(["cjk", "missing"]), "{reply}");
    assert_eq!(reply["regular"][0].as_f64(), Some(1000.0));
}
