// SPDX-License-Identifier: AGPL-3.0-or-later
//! Structured text of synthetic pages against MuPDF 1.28.1 (node `mupdf`,
//! the app's engine): clipping text, clip bounds, bidirectional runs, text
//! spans, ActualText over images, Type3 glyphs drawn directly, lenient
//! ToUnicode CMaps, MuPDF's cp936 fonts and structure lookups. Every expected
//! string was measured with MuPDF on the same content, except where a test
//! says otherwise. See design/app-corpus-text.md.
use std::time::{Duration, Instant};
use tandem_pdf_engine::view::{MAX_SEARCH_QUADS, ViewDocument};

/// A classic-xref PDF of `objects` (numbered from 1).
fn pdf(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    out
}

fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

/// One 300 × 300 page with `resources` and `content`; `extra` objects are
/// numbered from 5.
fn page(content: &str, resources: &str, extra: &[String]) -> ViewDocument {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources {resources} /Contents 4 0 R >>"
        ),
        stream("", content),
    ];
    objects.extend(extra.iter().cloned());
    ViewDocument::open(pdf(&objects), "").unwrap()
}

fn cmap(entries: &str, count: usize) -> String {
    stream(
        "",
        &format!(
            "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /X def \
             1 begincodespacerange <00> <FF> endcodespacerange\n{count} beginbfchar\n{entries}\n\
             endbfchar endcmap CMapName currentdict /CMap defineresource pop end end"
        ),
    )
}

/// Helvetica as /F1 whose ToUnicode reads code 0x1F as "fi".
fn ligature_page(content: &str) -> ViewDocument {
    page(
        content,
        "<< /Font << /F1 5 0 R >> >>",
        &[
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>".into(),
            cmap("<1f> <00660069> <41> <0041>", 2),
        ],
    )
}

/// (whitespace layout, preserve-spans layout) as MuPDF's `asText`.
fn texts(doc: &ViewDocument) -> (String, String) {
    let text = doc.structured_text(0).unwrap();
    (text.as_text(), text.spans_text())
}

#[test]
fn clipping_text_is_extracted_at_the_end_of_its_text_object() {
    let cases = [
        // Clipping text keeps a glyph's first character only.
        (
            "BT /F1 10 Tf 7 Tr 10 100 Td (A\\037B) Tj ET",
            "AfB\n\n",
            "AfB\n\n",
        ),
        // Fill and clip: the filled text, then the clip.
        (
            "BT /F1 10 Tf 4 Tr 10 100 Td (A\\037B) Tj ET",
            "AfiB\nAfB\n\n",
            "AfiB\nAfB\n\n",
        ),
        // A last show outside the clipping modes drops the clip text.
        (
            "BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj 0 Tr (CD) Tj ET",
            "CD\n\n",
            "CD\n\n",
        ),
        // One in the invisible mode keeps it, after the shows.
        (
            "BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj 3 Tr (CD) Tj ET",
            "CD\nAB\n\n",
            "CD\nAB\n\n",
        ),
        (
            "q BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj ET 0 0 1 rg 0 0 300 300 re f Q \
             BT /F1 10 Tf 10 50 Td (CD) Tj ET",
            "AB\n\nCD\n\n",
            "AB\n\nCD\n\n",
        ),
        // Without ET nothing is extracted.
        ("BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj", "", ""),
    ];
    for (content, ws, spans) in cases {
        let doc = ligature_page(content);
        assert_eq!(
            texts(&doc),
            (ws.to_string(), spans.to_string()),
            "{content}"
        );
    }
    // The text that only clips a fill is searchable (the common use).
    let doc = ligature_page("q BT /F1 30 Tf 7 Tr 20 100 Td (AB) Tj ET 0 0 1 rg 0 0 300 300 re f Q");
    let hits = doc
        .structured_text(0)
        .unwrap()
        .search("AB", MAX_SEARCH_QUADS);
    assert_eq!(hits.hits.len(), 1);
}

#[test]
fn glyphs_are_clipped_by_their_bounds_and_inkless_ones_by_their_origin() {
    // The space is kerned back, so a dropped space leaves no gap behind.
    let shown = "BT /F1 10 Tf 10 100 Td [(A) ( ) 278 (B)] TJ ET Q";
    let cases = [
        ("q 10 99 100 5 re W n", "A B\n\n"),
        ("q 10 101 100 5 re W n", "AB\n\n"),
        ("q 10 98 6.5 9 re W n", "A\n\n"),
        ("q 10 98 6.8 9 re W n", "A \n\n"),
        ("q 10 90 100 9.9 re W n", ""),
    ];
    for (clip, want) in cases {
        let doc = ligature_page(&format!("{clip} {shown}"));
        assert_eq!(texts(&doc).0, want, "{clip}");
    }
}

/// Helvetica whose ToUnicode reads a..e as Hebrew letters, f as an Arabic
/// alef and g as an Old Hungarian letter.
fn hebrew_page(content: &str) -> ViewDocument {
    let entries = "<61> <05d0>\n<62> <05d1>\n<63> <05d2>\n<64> <05d3>\n<65> <05d4>\n\
                   <66> <0627>\n<67> <d803dc80>";
    page(
        content,
        "<< /Font << /F1 5 0 R /F2 7 0 R >> >>",
        &[
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>".into(),
            cmap(entries, 7),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /ToUnicode 6 0 R >>".into(),
        ],
    )
}

#[test]
fn right_to_left_runs_take_neutral_characters_as_mupdf_does() {
    // Letters as drawn: a..e are U+05D0..U+05D4 (A..E below).
    let hebrew = |s: &str| {
        s.chars()
            .map(|c| match c {
                'A'..='E' => char::from_u32(0x5d0 + (c as u32 - 'A' as u32)).unwrap(),
                'G' => '\u{10c80}',
                c => c,
            })
            .collect::<String>()
    };
    let shown = |s: &str| format!("BT /F1 10 Tf 10 100 Td ({s}) Tj ET");
    let cases = [
        (shown("abc de"), "ED CBA\n\n", "ED CBA\n\n"),
        (shown("? abc de"), "? ED CBA\n\n", "? \nED CBA\n\n"),
        // European digits read left to right; a direction change starts a span.
        (shown("abc 12 de"), " CBA12 ED\n\n", " CBA\n12 \nED\n\n"),
        (
            shown("XY abc de ZW"),
            "XY  ED CBAZW\n\n",
            "XY \n ED CBA\nZW\n\n",
        ),
        (shown("abc. de"), "ED .CBA\n\n", "ED .CBA\n\n"),
        (shown("ggg ggg"), "GGG GGG\n\n", "GGG GGG\n\n"),
        (shown("a+b"), "A+B\n\n", "A\n+\nB\n\n"),
        // Neutral characters keep the direction across shows and fonts...
        (
            "BT /F1 10 Tf 10 100 Td (a) Tj /F2 10 Tf ( ) Tj /F1 10 Tf (b) Tj ET".into(),
            "B A\n\n",
            "A\n \nB\n\n",
        ),
        // ...but a text object starts left to right.
        (
            "BT /F1 10 Tf 10 100 Td (a) Tj ET BT /F1 10 Tf 15.1 100 Td ( ) Tj ET \
             BT /F1 10 Tf 17.9 100 Td (b) Tj ET"
                .into(),
            "A B\n\n",
            "A\n \nB\n\n",
        ),
    ];
    for (content, ws, spans) in cases {
        let doc = hebrew_page(&content);
        assert_eq!(texts(&doc), (hebrew(ws), hebrew(spans)), "{content}");
    }
}

#[test]
fn text_spans_start_where_mupdf_flushes_its_text() {
    let split = [
        "0 g",
        "0 G",
        "1 w",
        "0 J",
        "0 j",
        "10 M",
        "[] 0 d",
        "/Perceptual ri",
    ];
    let joined = ["/F1 10 Tf", "0 Tc", "0 Tw", "100 Tz", "1 i", "0 Tr", "BX"];
    for (op, spans) in split
        .iter()
        .map(|op| (op, "AB\nCD\n\n"))
        .chain(joined.iter().map(|op| (op, "ABCD\n\n")))
        .chain([(&"/K MP", "AB\nCD\n\n"), (&"/K << >> DP", "AB\nCD\n\n")])
    {
        let doc = ligature_page(&format!("BT /F1 10 Tf 10 100 Td (AB) Tj {op} (CD) Tj ET"));
        assert_eq!(
            texts(&doc),
            ("ABCD\n\n".to_string(), spans.to_string()),
            "{op}"
        );
    }
    // Another rendering mode is another span; setting the same one is not.
    let doc = ligature_page("BT /F1 10 Tf 10 100 Td (AB) Tj 2 Tr (CD) Tj ET");
    assert_eq!(texts(&doc).1, "AB\nCD\n\n");
    let doc = ligature_page("BT /F1 10 Tf 3 Tr 10 100 Td (AB) Tj 0 Tr 3 Tr (CD) Tj ET");
    assert_eq!(texts(&doc).1, "ABCD\n\n");
}

#[test]
fn actual_text_over_no_text_is_placed_on_the_images_it_covers() {
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray \
         /BitsPerComponent 8 /Filter /ASCIIHexDecode",
        "80402010>",
    );
    let doc = |content: &str| {
        page(
            content,
            "<< /Font << /F1 6 0 R >> /XObject << /Im 5 0 R >> >>",
            &[
                image.clone(),
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
            ],
        )
    };
    let over_image =
        doc("/Span << /ActualText (Bookkeeper) >> BDC q 50 0 0 20 10 100 cm /Im Do Q EMC");
    // Repeated letters stay: the characters do not read as fake bold.
    assert_eq!(texts(&over_image).0, "Bookkeeper\n\n");
    // At the image's bottom left (page space, y down), an em of its size.
    let text = over_image.structured_text(0).unwrap();
    let first = &text.blocks()[0].lines[0].chars[0];
    assert_eq!(first.origin, [10.0, 200.0]);
    assert!((first.size - 1000f64.sqrt()).abs() < 1e-6);
    // Paths do not carry ActualText.
    let over_path = doc("/Span << /ActualText (Hello) >> BDC 10 100 50 20 re f EMC");
    assert_eq!(texts(&over_path).0, "");
}

#[test]
fn type3_glyphs_whose_forms_fill_without_a_colour_are_drawn_directly() {
    // Each directly drawn glyph is a span of its own in MuPDF.
    let doc = |procedure: &str, form: &str| {
        page(
            "BT /F1 12 Tf 50 100 Td (aaa) Tj ET",
            "<< /Font << /F1 5 0 R >> >>",
            &[
                "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 750 750] \
                  /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 6 0 R >> \
                  /Encoding << /Differences [97 /a] >> /FirstChar 97 /LastChar 97 \
                  /Widths [800] /Resources << /XObject << /X0 7 0 R >> >> >>"
                    .into(),
                stream("", procedure),
                stream(
                    "/Type /XObject /Subtype /Form /BBox [0 0 1000 1000] /Resources << >>",
                    form,
                ),
            ],
        )
    };
    let fill = "0 0 m 750 750 l 375 0 l f";
    let cases = [
        ("800 0 d0 /X0 Do", fill, "a\na\na\n\n"),
        ("800 0 d0 0 g /X0 Do", fill, "aaa\n\n"),
        (
            "800 0 d0 /X0 Do",
            "0 g 0 0 m 750 750 l 375 0 l f",
            "aaa\n\n",
        ),
        ("800 0 0 0 750 750 d1 /X0 Do", fill, "aaa\n\n"),
    ];
    for (procedure, form, spans) in cases {
        assert_eq!(
            texts(&doc(procedure, form)).1,
            spans,
            "{procedure} / {form}"
        );
    }
}

#[test]
fn to_unicode_cmaps_read_as_leniently_as_mupdf() {
    let font = |to_unicode: &str| {
        page(
            "BT /F1 10 Tf 10 100 Td <0001000200030003> Tj ET",
            "<< /Font << /F1 5 0 R >> >>",
            &[
                "<< /Type /Font /Subtype /Type0 /BaseFont /Helvetica /Encoding /Identity-H \
                  /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>"
                    .into(),
                "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Helvetica \
                  /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
                  /FontDescriptor 8 0 R >>"
                    .into(),
                stream("", to_unicode),
                "<< /Type /FontDescriptor /FontName /Helvetica /Flags 32 \
                  /FontBBox [0 -200 1000 800] /ItalicAngle 0 /Ascent 800 /Descent -200 \
                  /CapHeight 700 /StemV 80 >>"
                    .into(),
            ],
        )
    };
    // A name cut short by a stray `>` outside the mapping blocks.
    let stray = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering (UC>) /Supplement 0 >> def \
                 /CMapName /Adobe-Identity-UC> def 1 begincodespacerange <0000> <FFFF> \
                 endcodespacerange 3 beginbfchar <0001> <0048> <0002> <0065> <0003> <006C> \
                 endbfchar endcmap";
    assert_eq!(texts(&font(stray)).0, "Hell\n\n");
    // CID mappings read as the characters.
    let cids = "begincmap 1 begincodespacerange <0000> <FFFF> endcodespacerange \
                1 begincidrange <0001> <0003> 72 endcidrange endcmap";
    assert_eq!(texts(&font(cids)).0, "HIJJ\n\n");
    // A malformed mapping block fails the CMap, as in MuPDF.
    let broken = "begincmap 1 beginbfchar <0001> /H endbfchar endcmap";
    assert_eq!(
        texts(&font(broken)).0,
        "\u{fffd}\u{fffd}\u{fffd}\u{fffd}\n\n"
    );
}

#[test]
fn s22pdf_chinese_fonts_read_as_gbk_with_full_em_advances() {
    // A WinAnsi font of Flags 4 named 宋体 in GBK: MuPDF reads GBK-EUC-H
    // codes through Adobe-GB1-UCS2, every code an em wide.
    let doc = page(
        "BT /F1 12 Tf 50 100 Td <D6D020> Tj 12 0 Td <CEC420> Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &[
            "<< /Type /Font /Subtype /TrueType /BaseFont /#CB#CE#CC#E5 \
              /Encoding /WinAnsiEncoding /FontDescriptor 6 0 R >>"
                .into(),
            "<< /Type /FontDescriptor /FontName /#CB#CE#CC#E5 /Flags 4 \
              /FontBBox [0 -140 1000 860] /ItalicAngle 0 /Ascent 860 /Descent -140 \
              /CapHeight 700 /StemV 80 >>"
                .into(),
        ],
    );
    let (ws, spans) = texts(&doc);
    // Each code with the space after it is a line, the next code starting
    // an em before the pen. hayro's Adobe-GB1-UCS2 reads the space's CID as
    // U+2002, MuPDF's as U+0020.
    assert_eq!(spans, "\u{4e2d} \n\u{6587} \n\n");
    assert_eq!(ws, "\u{4e2d}\u{2002}\n\u{6587}\u{2002}\n\n");
}

#[test]
fn structure_elements_are_found_by_mcid_in_linear_time() {
    // 6,000 marked-content sequences; the last one's element has ActualText,
    // behind a null entry. Resolving every element before each one took
    // 8.7 s natively for such a page. (Each element's /K names its MCID:
    // MuPDF takes the first element of the array whose /K does, the
    // specification and the engine the element at the MCID.)
    const N: usize = 6000;
    let mut content = String::from("BT /F1 0.04 Tf 10 100 Td ");
    for mcid in 0..N {
        content.push_str(&format!("/P << /MCID {mcid} >> BDC (x) Tj EMC "));
    }
    content.push_str("ET");
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /StructParents 0 \
         /Resources << /Font << /F1 6 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        stream("", &content),
    ];
    let first = 8;
    let parents: Vec<String> = (0..N)
        .map(|i| match i {
            i if i == N - 2 => "null".to_string(),
            i => format!("{} 0 R", first + i),
        })
        .collect();
    objects.push("<< /Type /StructTreeRoot /ParentTree << /Nums [0 7 0 R] >> >>".to_string());
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());
    objects.push(format!("[{}]", parents.join(" ")));
    for i in 0..N {
        objects.push(if i == N - 1 {
            format!("<< /Type /StructElem /S /Span /K {i} /ActualText (Last) >>")
        } else {
            format!("<< /Type /StructElem /S /Span /K {i} >>")
        });
    }
    let doc = ViewDocument::open(pdf(&objects), "").unwrap();
    let started = Instant::now();
    let text = doc.structured_text(0).unwrap().as_text();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(
        text.ends_with("xxLast\n\n"),
        "{:?}",
        &text[text.len() - 10..]
    );
    assert_eq!(text.matches('x').count(), N - 1);
}

#[test]
fn text_shown_before_any_font_is_set_is_not_extracted() {
    // hayro draws ASCII in Helvetica without a font; MuPDF draws nothing
    // before a `Tf`, and a default font after one naming no font.
    let cases = [
        ("BT 10 100 Td (AB) Tj ET", ""),
        ("q BT /F1 10 Tf ET Q BT 10 50 Td (CD) Tj ET", ""),
        ("BT /F9 10 Tf 10 100 Td (AB) Tj ET", "AB\n\n"),
        ("BT 11 Tf 10 100 Td (AB) Tj ET", "AB\n\n"),
    ];
    for (content, want) in cases {
        assert_eq!(texts(&ligature_page(content)).0, want, "{content}");
    }
    // A form draws with the font of the content that draws it.
    let with_form = |content: &str, form: &str| {
        page(
            content,
            "<< /Font << /F1 5 0 R >> /XObject << /X 6 0 R >> >>",
            &[
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
                stream("/Type /XObject /Subtype /Form /BBox [0 0 300 300]", form),
            ],
        )
    };
    let form = "BT 10 100 Td (AB) Tj ET";
    assert_eq!(texts(&with_form("BT /F1 10 Tf ET /X Do", form)).0, "AB\n\n");
    assert_eq!(texts(&with_form("/X Do", form)).0, "");
}
