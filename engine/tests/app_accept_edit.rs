// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acceptance review of the content-edit ops (round 1): /ToUnicode CMaps
//! with Flate damage reference viewers read, content streams no decoder
//! reads, and the scan's total of generated CMap and /W codes.
use std::sync::Arc;

use serde_json::{Value, json};
use tandem_pdf_engine::app::ops::edit::scan_for;
use tandem_pdf_engine::app::{AppEngine, AppSession, info};
use tandem_pdf_engine::document::Document;
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId};
use tandem_pdf_engine::serialize::{WriteLimits, serialize_object};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("../fixtures/{name}")).unwrap()
}

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn stream(dictionary: &str, data: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< {dictionary} /Length {} >>\nstream\n", data.len()).into_bytes();
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

/// The number after the last `key` in `bytes`.
fn last_number(bytes: &[u8], key: &[u8]) -> usize {
    let at = bytes.windows(key.len()).rposition(|w| w == key).unwrap();
    let digits: String = bytes[at + key.len()..]
        .iter()
        .map(|&b| char::from(b))
        .skip_while(|c| c.is_ascii_whitespace())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().unwrap()
}

/// `bytes` with object `number` redefined as `body` by an appended update.
fn update(bytes: &[u8], number: u32, body: &[u8]) -> Vec<u8> {
    let (prev, size) = (
        last_number(bytes, b"startxref"),
        last_number(bytes, b"/Size"),
    );
    let mut out = bytes.to_vec();
    out.push(b'\n');
    let offset = out.len();
    out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    out.extend_from_slice(body);
    out.extend_from_slice(b"\nendobj\n");
    let xref = out.len();
    out.extend_from_slice(
        format!(
            "xref\n{number} 1\n{offset:010} 00000 n \ntrailer\n<< /Size {size} /Root 1 0 R /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    out
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let reply = dispatch(engine, session, request);
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn document(session: &AppSession) -> &Document<'static> {
    session.doc.as_ref().unwrap().document()
}

/// Every object written, for exact before/after comparisons.
fn serialized(session: &AppSession) -> Vec<u8> {
    let options = WriteOptions {
        garbage: false,
        ..WriteOptions::APP
    };
    write_document(document(session), DocumentWriteLimits::default(), options).unwrap()
}

fn scan(engine: &mut AppEngine, session: &mut AppSession) -> Value {
    op(engine, session, json!({"op": "edit-targets", "page": 0}))
}

fn items(reply: &Value) -> &Vec<Value> {
    reply["items"].as_array().unwrap()
}

fn target<'r>(reply: &'r Value, text: &str) -> &'r Value {
    items(reply)
        .iter()
        .find(|item| item["text"] == text)
        .unwrap_or_else(|| panic!("no {text:?} in {reply}"))
}

/// What a scan says of each item: text, support, reason, repertoire.
fn verdicts(reply: &Value) -> Vec<String> {
    items(reply)
        .iter()
        .map(|item| {
            format!(
                "{} {} {} {}",
                item["text"], item["support"], item["reason"], item["repertoire"]
            )
        })
        .collect()
}

fn edit_text(
    engine: &mut AppEngine,
    session: &mut AppSession,
    scan: &Value,
    text: &str,
    next: &str,
) -> Value {
    let request = json!({"op": "edit-text", "page": 0, "targetId": target(scan, text)["targetId"],
        "generation": scan["generation"], "revision": scan["revision"], "text": next});
    dispatch(engine, session, request)
}

fn page_dict(doc: &Document<'_>) -> Dictionary {
    info::pages(doc)[0].dict.clone()
}

/// The object `/Font /<key>` of page 0 names.
fn page_font(doc: &Document<'_>, key: &[u8]) -> ObjectId {
    let page = page_dict(doc);
    let resources = doc.resolve(page.get(b"Resources".as_slice()).unwrap());
    let fonts = doc.resolve(
        resources
            .as_dict()
            .unwrap()
            .get(b"Font".as_slice())
            .unwrap(),
    );
    fonts
        .as_dict()
        .unwrap()
        .get(key)
        .unwrap()
        .as_reference()
        .unwrap()
}

fn entry<'d>(doc: &'d Document<'_>, object: ObjectId, key: &[u8]) -> &'d Object {
    doc.dict_get(doc.resolve_id(object).as_dict().unwrap(), key)
}

fn reference(doc: &Document<'_>, object: ObjectId, key: &[u8]) -> ObjectId {
    doc.resolve_id(object)
        .as_dict()
        .unwrap()
        .get(key)
        .unwrap()
        .as_reference()
        .unwrap()
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// A /ToUnicode CMap packed as producers damage it: the EOL before
/// `endstream` counted in /Length, a wrong or missing Adler-32.
fn damaged(cmap: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(cmap, 6);
    let body = packed.len() - 4;
    assert_eq!(packed[body..], adler32(cmap).to_be_bytes());
    let mut wrong = packed.clone();
    wrong[body..].copy_from_slice(&(adler32(cmap) ^ 1).to_be_bytes());
    vec![
        ("trailing LF", [&packed[..], b"\n"].concat()),
        ("trailing CRLF", [&packed[..], b"\r\n"].concat()),
        ("bad Adler-32", wrong),
        ("no Adler-32", packed[..body].to_vec()),
    ]
}

/// One damaged variant of `fixture` per `damaged` shape: its scan must say
/// what the clean file's says, and an edit, a replace and their undos work.
fn damaged_cmap_round_trips(
    name: &str,
    font_key: &[u8],
    (run, next): (&str, &str),
    (find, replace, count): (&str, &str, usize),
) {
    let clean = fixture(name);
    let (mut engine, mut session) = session_with(clean.clone());
    let expected = verdicts(&scan(&mut engine, &mut session));
    let doc = document(&session);
    let font = page_font(doc, font_key);
    let cmap_id = reference(doc, font, b"ToUnicode");
    let cmap = doc
        .decoded_stream(cmap_id, StreamLimits::default())
        .unwrap();
    for (label, data) in damaged(&cmap) {
        let bytes = update(
            &clean,
            cmap_id.number(),
            &stream("/Filter /FlateDecode", &data),
        );
        let (mut engine, mut session) = session_with(bytes);
        assert!(
            document(&session)
                .decoded_stream(cmap_id, StreamLimits::default())
                .is_err(),
            "{label}: the strict decoder refuses it"
        );
        let label = format!("{name} {label}");
        edits_round_trip(
            &mut engine,
            &mut session,
            &expected,
            &label,
            (run, next),
            (find, replace, count),
        );
    }
}

/// The scan says `expected`; editing `run` to `next` and replacing `find`
/// (`count` matches) work, and each undo restores every byte.
fn edits_round_trip(
    engine: &mut AppEngine,
    session: &mut AppSession,
    expected: &[String],
    label: &str,
    (run, next): (&str, &str),
    (find, replace, count): (&str, &str, usize),
) {
    let reply = scan(engine, session);
    assert_eq!(verdicts(&reply), expected, "{label}");
    assert_eq!(target(&reply, run)["support"], "candidate", "{label}");
    let before = serialized(session);
    let edited = edit_text(engine, session, &reply, run, next);
    assert_eq!(edited["ok"], true, "{label}: {edited}");
    let rescan = scan(engine, session);
    assert_eq!(target(&rescan, next)["support"], "candidate", "{label}");
    op(engine, session, json!({"op": "undo"}));
    assert_eq!(
        serialized(session),
        before,
        "{label}: undo restores the edit"
    );
    let preview = op(
        engine,
        session,
        json!({"op": "edit-replace-preview", "page": 0, "find": find, "replace": replace}),
    );
    assert_eq!(preview["rejected"], json!([]), "{label}: {preview}");
    let ids: Vec<Value> = preview["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["targetId"].clone())
        .collect();
    assert_eq!(ids.len(), count, "{label}: {preview}");
    let applied = op(
        engine,
        session,
        json!({"op": "edit-replace-apply", "page": 0, "find": find, "replace": replace,
            "previewToken": preview["previewToken"], "targetIds": ids}),
    );
    assert_eq!(applied["replaced"], count as u64, "{label}: {applied}");
    op(engine, session, json!({"op": "undo"}));
    assert_eq!(
        serialized(session),
        before,
        "{label}: undo restores the replace"
    );
}

/// Review (major): /ToUnicode was read by the strict decoder alone, so an
/// Identity-H font whose CMap has an EOL inside /Length or a bad or missing
/// Adler-32 lost its code map and every run in it was unsupported.
#[test]
fn an_identity_h_cmap_with_flate_damage_is_still_read() {
    damaged_cmap_round_trips(
        "p0b/edit-embedded.pdf",
        b"F2",
        ("日本語テスト", "日本語テト"),
        ("テスト", "テト", 1),
    );
}

/// The same for a coded symbolic simple font (one-byte /ToUnicode).
#[test]
fn a_coded_font_cmap_with_flate_damage_is_still_read() {
    damaged_cmap_round_trips(
        "p0b/edit-coded.pdf",
        b"F2",
        ("Lorem ipsum", "Lorem ipsu"),
        ("or", "o", 2),
    );
}

const HELVETICA: &[u8] =
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

/// A page whose /Contents are `streams` (objects 5 on), with Helvetica as /F1.
fn page_with(streams: &[Vec<u8>]) -> Vec<u8> {
    let refs: Vec<String> = (0..streams.len())
        .map(|i| format!("{} 0 R", i + 5))
        .collect();
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents [{}] >>",
            refs.join(" ")
        )
        .into_bytes(),
        HELVETICA.to_vec(),
    ];
    objects.extend_from_slice(streams);
    classic(&objects)
}

fn raw(session: &AppSession, object: u32) -> Vec<u8> {
    document(session)
        .stream_data(id(object))
        .unwrap()
        .unwrap()
        .to_vec()
}

/// Review (minor): a /Contents stream neither the strict decoder nor the
/// view reads (an LZW stream without a clear code, JBIG2Decode) refused the
/// whole page, text-add included; reference viewers read it as empty and the view
/// draws nothing from it. A filter name no reader knows is skipped by the
/// view (and reference viewers), so that stream's run is listed, as before.
#[test]
fn a_content_stream_no_decoder_reads_holds_no_targets() {
    let streams = vec![
        stream("/Filter /LZWDecode", b"\xff\xff\xff\xff\xff\xff"),
        stream("", b"BT /F1 12 Tf 72 700 Td (READABLE) Tj ET"),
        stream("/Filter /JBIG2Decode", b"\x97JB2\r\n\x1a\n\x01junk"),
        stream("/Filter /FooDecode", b"BT /F1 12 Tf 72 600 Td (RAW) Tj ET"),
    ];
    let (mut engine, mut session) = session_with(page_with(&streams));
    for object in [5, 7, 8] {
        assert!(
            document(&session)
                .decoded_stream(id(object), StreamLimits::default())
                .is_err()
        );
    }
    let message = document(&session)
        .decoded_stream(id(8), StreamLimits::default())
        .unwrap_err()
        .to_string();
    assert!(
        message.ends_with("unsupported filter name /FooDecode"),
        "{message}"
    );
    let reply = scan(&mut engine, &mut session);
    assert_eq!(items(&reply).len(), 2, "{reply}");
    assert_eq!(target(&reply, "READABLE")["support"], "candidate");
    assert_eq!(target(&reply, "RAW")["support"], "candidate");
    let before = serialized(&session);
    let unread: Vec<Vec<u8>> = [5, 7].iter().map(|&o| raw(&session, o)).collect();
    let edited = edit_text(&mut engine, &mut session, &reply, "READABLE", "READABLY");
    assert_eq!(edited["ok"], true, "{edited}");
    let added = op(
        &mut engine,
        &mut session,
        json!({"op": "text-add", "page": 0, "point": [72, 300], "text": "Added",
            "font": "Helvetica", "size": 12, "color": [0, 0, 0]}),
    );
    assert!(
        added["targetId"].as_str().is_some_and(|t| !t.is_empty()),
        "{added}"
    );
    let after: Vec<Vec<u8>> = [5, 7].iter().map(|&o| raw(&session, o)).collect();
    assert_eq!(after, unread, "the unread streams are kept as they are");
    let rescan = scan(&mut engine, &mut session);
    let texts: Vec<&Value> = items(&rescan).iter().map(|item| &item["text"]).collect();
    assert_eq!(texts, ["READABLY", "RAW", "Added"]);
    op(&mut engine, &mut session, json!({"op": "undo"}));
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);

    // The qpdf shape: the page's one stream is unreadable.
    let (mut engine, mut session) = session_with(page_with(&streams[..1]));
    assert!(items(&scan(&mut engine, &mut session)).is_empty());
    let added = op(
        &mut engine,
        &mut session,
        json!({"op": "text-add", "page": 0, "point": [72, 300], "text": "Added",
            "font": "Helvetica", "size": 12, "color": [0, 0, 0]}),
    );
    assert!(
        added["targetId"].as_str().is_some_and(|t| !t.is_empty()),
        "{added}"
    );
}

/// The codes edit-embedded.pdf's /F2 draws, one character each.
const EMBEDDED_CODES: [&str; 9] = [
    "00a1", "00e7", "00f4", "00f6", "1a02", "1b49", "3c69", "3ebb", "4ac0",
];

/// fixtures/p0b/edit-embedded.pdf's Identity-H font rebuilt with `fonts`
/// Type0 dictionaries (/F2, /F3, ...) sharing one descendant whose /W is
/// `w` and one /ToUnicode; each draws `runs` runs of two characters, every
/// run on the page distinct.
fn embedded_fonts(w: &str, fonts: usize, runs: usize) -> Vec<u8> {
    let source = fixture("p0b/edit-embedded.pdf");
    let (_, session) = session_with(source);
    let doc = document(&session);
    let font = page_font(doc, b"F2");
    let base = entry(doc, font, b"BaseFont").as_name().unwrap().to_vec();
    let base = String::from_utf8(base).unwrap();
    let cmap = doc
        .decoded_stream(reference(doc, font, b"ToUnicode"), StreamLimits::default())
        .unwrap();
    let descendants = entry(doc, font, b"DescendantFonts").as_array().unwrap();
    let descendant = descendants[0].as_reference().unwrap();
    let descriptor = reference(doc, descendant, b"FontDescriptor");
    let program = doc
        .decoded_stream(
            reference(doc, descriptor, b"FontFile2"),
            StreamLimits::default(),
        )
        .unwrap();
    let names: Vec<String> = (0..fonts)
        .map(|i| format!("/F{} {} 0 R", i + 2, i + 10))
        .collect();
    let mut content = String::new();
    for f in 0..fonts {
        for r in 0..runs {
            let index = f * runs + r;
            let codes = format!(
                "{}{}",
                EMBEDDED_CODES[index % 9],
                EMBEDDED_CODES[index / 9 % 9]
            );
            let (x, y) = (72 + 200 * (index / 45), 760 - 15 * (index % 45));
            content.push_str(&format!(
                "BT /F{} 10 Tf {x} {y} Td <{codes}> Tj ET\n",
                f + 2
            ));
        }
    }
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << {} >> >> /Contents 4 0 R >>",
            names.join(" ")
        )
        .into_bytes(),
        stream("", content.as_bytes()),
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 6 0 R /DW 1000 /W {w} >>"
        )
        .into_bytes(),
        format!(
            "<< /Type /FontDescriptor /FontName /{base} /Flags 4 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 /FontFile2 7 0 R >>"
        )
        .into_bytes(),
        stream("", &program),
        stream("", &cmap),
        b"null".to_vec(),
    ];
    for _ in 0..fonts {
        objects.push(
            format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 8 0 R >>"
            )
            .into_bytes(),
        );
    }
    classic(&objects)
}

/// The fixture descendant's own /W, serialized.
fn embedded_w() -> String {
    let (_, session) = session_with(fixture("p0b/edit-embedded.pdf"));
    let doc = document(&session);
    let font = page_font(doc, b"F2");
    let descendant = entry(doc, font, b"DescendantFonts").as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let w = entry(doc, descendant, b"W");
    String::from_utf8(serialize_object(w, WriteLimits::default()).unwrap()).unwrap()
}

/// Review (minor, hostile cost): the 2^20 code ceiling was per font, so
/// fonts sharing one small /W or CMap that expands to 2^20 codes cost 2^20
/// each (64 fonts in 53 KB took 14.5 s). Now one scan generates at most
/// four fonts' ceilings in all (two until round 2) and a font past it
/// stays unproven.
#[test]
fn generated_cmap_and_w_codes_are_charged_to_the_scan() {
    let own = embedded_w();
    // 15 full ranges and the fixture's own (its ranges expand to tens of
    // thousands of codes) are just under one font's ceiling; its own widths
    // come last and win.
    let wide = format!(
        "[{} {}]",
        "0 65535 1000 ".repeat(15),
        &own[1..own.len() - 1]
    );
    let (mut engine, mut session) = session_with(embedded_fonts(&wide, 8, 1));
    let reply = scan(&mut engine, &mut session);
    let runs = items(&reply);
    assert_eq!(runs.len(), 8, "{reply}");
    // The total holds four fonts' ceilings; each font here spends a little
    // over one (its /W and its CMap), so three are proven.
    let proven: Vec<bool> = runs
        .iter()
        .map(|run| run["support"] == "candidate")
        .collect();
    assert_eq!(
        proven,
        [true, true, true, false, false, false, false, false],
        "{reply}"
    );
    // The fixture's own /W: every font is proven.
    let (mut engine, mut session) = session_with(embedded_fonts(&own, 3, 1));
    let reply = scan(&mut engine, &mut session);
    assert!(
        items(&reply)
            .iter()
            .all(|run| run["support"] == "candidate"),
        "{reply}"
    );
}

/// Found while verifying the cost finding: every run of a proven font
/// carried its own copy of the font's widths (up to 65,536 entries), so
/// 512 short runs of one font with a full /W range held 512 copies.
#[test]
fn the_runs_of_one_font_share_its_widths() {
    let own = embedded_w();
    let wide = format!("[0 65535 1000 {}]", &own[1..own.len() - 1]);
    let (_, mut session) = session_with(embedded_fonts(&wide, 1, 40));
    let scan = scan_for(&mut session, 0).unwrap();
    let writes: Vec<_> = scan
        .records
        .iter()
        .filter_map(|record| record.cid_write.as_ref())
        .collect();
    assert_eq!(writes.len(), 40);
    assert!(writes[0].widths.len() > 65_000);
    assert!(
        writes
            .iter()
            .all(|write| Arc::ptr_eq(&write.widths, &writes[0].widths))
    );
}

/// /BaseFont names the view's structured text holds differently from the
/// bytes (round 2): not UTF-8 (GBK, Shift-JIS, Latin-1), UTF-8, and longer
/// than 31 bytes (where reference viewers cut the name), with the cut inside a character.
const FONT_NAMES: [&str; 9] = [
    "/#CB#CE#CC#E5",
    "/#CB#CE#CC#E5,Bold",
    "/ABCDEF+#CB#CE#CC#E5",
    "/#82l#82r#20#96#BE#92#A9",
    "/Caf#E9Sans",
    "/Caf#C3#A9Sans",
    "/#E5#AE#8B#E4#BD#93",
    "/ABCDEF+TimesNewRomanPS-BoldItalicMT",
    "/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA#C3#A9Sans",
];

/// A page in one simple WinAnsi font of `subtype` named `name`: two runs
/// and a three-line paragraph.
fn named_simple_font(subtype: &str, name: &str) -> Vec<u8> {
    let widths = vec!["500"; 95].join(" ");
    classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        format!(
            "<< /Type /Font /Subtype /{subtype} /BaseFont {name} /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 126 /Widths [{widths}] /FontDescriptor 6 0 R >>"
        )
        .into_bytes(),
        stream(
            "",
            b"BT /F1 14 Tf 72 700 Td (Hello font name) Tj ET\nBT /F1 14 Tf 72 650 Td (Second) Tj ET\n\
              BT /F1 12 Tf 14 TL 72 600 Td (The quick brown fox) Tj T* (jumps over the lazy) Tj T* (dog again and again) Tj ET\n",
        ),
        format!(
            "<< /Type /FontDescriptor /FontName {name} /Flags 32 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>"
        )
        .into_bytes(),
    ])
}

/// Review round 2 (major): association compared the scan's Latin-1 reading
/// of /BaseFont with the view's name (hayro's UTF-8 reading, a placeholder
/// when the bytes are not UTF-8, cut at 31 bytes), so every run and
/// paragraph in such a font was `ambiguous-text-association`. The earlier app engine
/// lists them (except names past 31 bytes, which it refused the same way).
#[test]
fn simple_fonts_with_non_ascii_or_long_names_are_edited() {
    for subtype in ["TrueType", "Type1"] {
        let (mut engine, mut session) = session_with(named_simple_font(subtype, "/Arial"));
        let expected = verdicts(&scan(&mut engine, &mut session));
        assert!(
            expected.iter().all(|v| v.contains("\"candidate\"")),
            "{expected:?}"
        );
        for name in FONT_NAMES {
            let label = format!("{subtype} {name}");
            let (mut engine, mut session) = session_with(named_simple_font(subtype, name));
            edits_round_trip(
                &mut engine,
                &mut session,
                &expected,
                &label,
                ("Hello font name", "Hello font"),
                ("Second", "Third", 1),
            );
            let reply = scan(&mut engine, &mut session);
            let paragraphs = reply["paragraphs"].as_array().unwrap();
            assert_eq!(paragraphs.len(), 1, "{label}: {reply}");
            assert_eq!(paragraphs[0]["lines"], 3, "{label}");
            let before = serialized(&session);
            let request = json!({"op": "edit-paragraph", "page": 0,
                "paragraphId": paragraphs[0]["paragraphId"], "generation": reply["generation"],
                "revision": reply["revision"], "text": "A new paragraph text that wraps"});
            op(&mut engine, &mut session, request);
            op(&mut engine, &mut session, json!({"op": "undo"}));
            assert_eq!(serialized(&session), before, "{label}: paragraph undo");
        }
    }
}

/// The reply's font name: UTF-8 as UTF-8, other bytes as Latin-1, the
/// subset prefix stripped and never cut.
#[test]
fn font_names_display_as_utf8_else_latin1() {
    for (name, shown) in [
        ("/Caf#C3#A9Sans", "Caf\u{e9}Sans"),
        ("/Caf#E9Sans", "Caf\u{e9}Sans"),
        ("/ABCDEF+#CB#CE#CC#E5", "\u{cb}\u{ce}\u{cc}\u{e5}"),
        ("/#E5#AE#8B#E4#BD#93", "\u{5b8b}\u{4f53}"),
        (
            "/ABCDEF+TimesNewRomanPS-BoldItalicMT",
            "TimesNewRomanPS-BoldItalicMT",
        ),
    ] {
        let (mut engine, mut session) = session_with(named_simple_font("TrueType", name));
        let reply = scan(&mut engine, &mut session);
        assert_eq!(items(&reply)[0]["font"]["name"], shown, "{name}");
    }
}

/// `bytes` with object `object`'s dictionary given /BaseFont `name` by an
/// appended update.
fn renamed(bytes: &[u8], object: ObjectId, name: &[u8]) -> Vec<u8> {
    let (_, session) = session_with(bytes.to_vec());
    let doc = document(&session);
    let mut dict = doc.resolve_id(object).as_dict().unwrap().clone();
    dict.insert(b"BaseFont".to_vec(), Object::Name(name.to_vec()));
    let body = serialize_object(&Object::Dictionary(dict), WriteLimits::default()).unwrap();
    update(bytes, object.number(), &body)
}

/// The same for an Identity-H font (fixtures/p0b/edit-embedded.pdf /F2)
/// with its Type0 and descendant renamed, and for a Type0 /BaseFont that
/// is the descendant's plus the CMap name (ISO 32000 9.7.6.1): the view
/// names the run by the descendant, which the earlier app engine did not compare.
#[test]
fn identity_h_fonts_with_non_ascii_or_long_names_are_edited() {
    let clean = fixture("p0b/edit-embedded.pdf");
    let (mut engine, mut session) = session_with(clean.clone());
    let expected = verdicts(&scan(&mut engine, &mut session));
    let doc = document(&session);
    let font = page_font(doc, b"F2");
    let descendant = entry(doc, font, b"DescendantFonts").as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let raw = |name: &str| -> Vec<u8> {
        let mut out = Vec::new();
        let bytes = name.as_bytes();
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i] == b'#' {
                out.push(u8::from_str_radix(&name[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        out
    };
    let mut cases: Vec<(Vec<u8>, Vec<u8>)> = FONT_NAMES
        .iter()
        .map(|name| (raw(name), raw(name)))
        .collect();
    cases.push((
        b"ABCDEF+DroidSans-Identity-H".to_vec(),
        b"ABCDEF+DroidSans".to_vec(),
    ));
    for (type0, cid) in cases {
        let label = format!(
            "{} / {}",
            String::from_utf8_lossy(&type0),
            String::from_utf8_lossy(&cid)
        );
        let bytes = renamed(&renamed(&clean, font, &type0), descendant, &cid);
        let (mut engine, mut session) = session_with(bytes);
        edits_round_trip(
            &mut engine,
            &mut session,
            &expected,
            &label,
            ("日本語テスト", "日本語テト"),
            ("テスト", "テト", 1),
        );
    }
}

/// Review round 2 (minor, the p3-edit "/W expansion" timing): the scan
/// skips /W codes past two bytes, but the view (hayro) set every code of a
/// range, so the scan's association paid for 200 ranges above 0xFFFF
/// (13M entries) and `[0 4294967295 500]` asked for 2^32. The view now sets
/// CIDs up to 0xFFFF and at most 2^18 codes per array. Each case here took
/// about a second and up to 1 GB before, a few milliseconds now.
#[test]
fn w_ranges_past_two_bytes_cost_the_view_nothing() {
    let clean = fixture("p0b/edit-embedded.pdf");
    let (_, session) = session_with(clean.clone());
    let doc = document(&session);
    let font = page_font(doc, b"F2");
    let descendant = entry(doc, font, b"DescendantFonts").as_array().unwrap()[0]
        .as_reference()
        .unwrap();
    let w = reference(doc, descendant, b"W");
    let mut above = String::new();
    for index in 1..=200u64 {
        above.push_str(&format!("{} {} 500 ", index << 16, (index << 16) + 0xffff));
    }
    // The scan keeps /DW for the first; it refuses the second's 2^25 codes
    // (the earlier app engine's per-font ceiling), but the view still reads its /W.
    for (array, support) in [
        (format!("[{above}]"), "candidate"),
        (format!("[0 33554431 500 {above}]"), "unsupported"),
    ] {
        let (mut engine, mut session) = session_with(update(&clean, w.number(), array.as_bytes()));
        let started = std::time::Instant::now();
        let reply = scan(&mut engine, &mut session);
        let found = op(
            &mut engine,
            &mut session,
            json!({"op": "search", "query": "テスト"}),
        );
        let elapsed = started.elapsed();
        assert_eq!(items(&reply)[1]["support"], support, "{reply}");
        assert_eq!(found["total"], 1, "{found}");
        assert!(elapsed.as_millis() < 500, "{elapsed:?}");
    }
}
