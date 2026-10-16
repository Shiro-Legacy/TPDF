// SPDX-License-Identifier: AGPL-3.0-or-later
//! Structured text of synthetic pages against MuPDF 1.28.1 (node `mupdf`,
//! the app's engine): clipping text, clip bounds, bidirectional runs, text
//! spans, ActualText over images, Type3 glyphs drawn directly, lenient
//! ToUnicode CMaps, MuPDF's cp936 fonts and structure lookups. Every expected
//! string was measured with MuPDF on the same content, except where a test
//! says otherwise. See design/app-corpus-text.md.
use std::time::{Duration, Instant};
use tandem_pdf_engine::view::{MAX_SEARCH_QUADS, RenderOptions, ViewDocument};

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

/// Whether the page renders anything but white.
fn has_ink(doc: &ViewDocument) -> bool {
    let image = doc.render(0, 1.0, RenderOptions::default()).unwrap();
    image
        .pixels
        .chunks(4)
        .any(|px| px[..3].iter().any(|&v| v < 200))
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
        // Fill (or stroke) and clip: the drawn text once. MuPDF extracts it
        // again as the clip ("AfiB\nAfB"); search and copy would repeat it.
        (
            "BT /F1 10 Tf 4 Tr 10 100 Td (A\\037B) Tj ET",
            "AfiB\n\n",
            "AfiB\n\n",
        ),
        ("BT /F1 10 Tf 5 Tr 10 100 Td (AB) Tj ET", "AB\n\n", "AB\n\n"),
        ("BT /F1 10 Tf 6 Tr 10 100 Td (AB) Tj ET", "AB\n\n", "AB\n\n"),
        // The clip holds the glyphs drawn in no other way (MuPDF: "CD\nBCD").
        (
            "BT /F1 10 Tf 7 Tr 10 100 Td (AB) Tj 4 Tr (CD) Tj ET",
            "CD\nAB\n\n",
            "CD\nAB\n\n",
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
    // Filled clipping text is found once.
    for mode in 4..=6 {
        let doc = ligature_page(&format!(
            "BT /F1 20 Tf {mode} Tr 10 100 Td (Hello) Tj 0 -30 Td (World) Tj ET"
        ));
        let text = doc.structured_text(0).unwrap();
        assert_eq!(text.as_text(), "Hello\nWorld\n\n", "mode {mode}");
        assert_eq!(text.search("Hello", MAX_SEARCH_QUADS).hits.len(), 1);
    }
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
fn text_shown_before_any_font_is_set_is_extracted_as_drawn() {
    // hayro draws `Tj` and `TJ` without a font in Helvetica (ASCII only),
    // and `'` and `"` not at all; the text is what the page shows. MuPDF
    // draws nothing before a `Tf` (its text is "" for the first three).
    // Text matrices of 40 points: the font size is 1 without a `Tf`.
    let cases = [
        ("BT 40 0 0 40 10 100 Tm (AB) Tj ET", "AB\n\n"),
        (
            "q BT /F1 10 Tf ET Q BT 40 0 0 40 10 50 Tm (CD) Tj ET",
            "CD\n\n",
        ),
        ("BT 40 0 0 40 10 100 Tm [(A) 50 (B)] TJ ET", "AB\n\n"),
        ("BT 40 0 0 40 10 150 Tm 1 TL (AB) ' ET", ""),
        ("BT 40 0 0 40 10 150 Tm 1 TL 0 0 (AB) \" ET", ""),
        (
            "BT 40 0 0 40 10 150 Tm 1 TL (AB) Tj (CD) ' ET",
            "AB\nCD\n\n",
        ),
        (
            "BT 40 0 0 40 10 150 Tm 1 TL (AB) ' ET BT /Missing 40 Tf 10 50 Td (CD) Tj ET",
            "CD\n\n",
        ),
        ("BT /F9 40 Tf 10 100 Td (AB) Tj ET", "AB\n\n"),
        ("BT 40 0 0 40 10 100 Tm 11 Tf (AB) Tj ET", "AB\n\n"),
    ];
    for (content, want) in cases {
        let doc = ligature_page(content);
        assert_eq!(texts(&doc).0, want, "{content}");
        assert_eq!(has_ink(&doc), !want.is_empty(), "{content}");
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
    let form = "BT 40 0 0 40 10 100 Tm (AB) Tj ET";
    for content in ["BT /F1 1 Tf ET /X Do", "/X Do"] {
        let doc = with_form(content, form);
        assert_eq!(texts(&doc).0, "AB\n\n", "{content}");
        assert!(has_ink(&doc), "{content}");
    }
}

#[test]
fn gaps_of_the_space_threshold_read_by_how_they_were_made() {
    // A gap of 0.15 em: below the threshold when a `TJ` adjustment makes it
    // (TeX's ellipsis kerns), above it when letter spacing or positioning
    // does. MuPDF reads these two pages so; it reads such gaps on either
    // side of the threshold by the rounding of its own coordinates.
    let doc = ligature_page("BT /F1 12 Tf 10 100 Td [(A) -150 (B) -150 (A)] TJ ET");
    assert_eq!(texts(&doc).0, "ABA\n\n");
    let doc = page(
        "BT /F1 10 Tf 1.5 Tc 10 100 Td (abc) Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &["<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into()],
    );
    assert_eq!(texts(&doc).0, "a b c\n\n");
}

/// Helvetica as /F1 and a form /Fm showing `form` (with a bounding box
/// when `bbox`).
fn form_page(content: &str, form: &str, bbox: bool) -> ViewDocument {
    let bbox = if bbox { "/BBox [0 0 300 300]" } else { "" };
    page(
        content,
        "<< /Font << /F1 5 0 R >> /XObject << /Fm 6 0 R >> >>",
        &[
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
            stream(&format!("/Type /XObject /Subtype /Form {bbox}"), form),
        ],
    )
}

#[test]
fn each_glyph_run_reads_the_show_operation_that_drew_it() {
    // hayro draws no form without a bounding box: its shows must not lend
    // their text to the page's (which read "XY" at CD's place, and "CD" at
    // EF's). MuPDF draws the form ("XY\n\nCD\n\nEF\n\n").
    let shows = "BT /F1 12 Tf 10 50 Td (CD) Tj ET BT /F1 12 Tf 10 30 Td (EF) Tj ET";
    let form = "BT /F1 12 Tf 10 100 Td (XY) Tj ET";
    let doc = form_page(&format!("/Fm Do {shows}"), form, false);
    assert_eq!(texts(&doc).0, "CD\n\nEF\n\n");
    let text = doc.structured_text(0).unwrap();
    assert!(text.search("XY", MAX_SEARCH_QUADS).hits.is_empty());
    let ef = text.search("EF", MAX_SEARCH_QUADS);
    assert_eq!(ef.hits.len(), 1);
    // Page space, y down: EF's baseline is at 270.
    assert!((ef.hits[0][0][5] - 270.0).abs() < 4.0, "{:?}", ef.hits);
    let doc = form_page(&format!("/Fm Do {shows}"), form, true);
    assert_eq!(texts(&doc).0, "XY\n\nCD\n\nEF\n\n");
    // Shows after one that was not drawn, in the same text object.
    let doc = form_page(
        "/Fm Do BT /F1 12 Tf 10 50 Td (CD) Tj (EF) Tj ET",
        "BT /F1 12 Tf 10 100 Td (AB) Tj (XY) Tj ET",
        false,
    );
    assert_eq!(texts(&doc).0, "CDEF\n\n");
    // hayro reads an operator's last operands: `1 7 Tr` is mode 7, drawn
    // as clipping text (as MuPDF's "AB\n\nCD\n\n", which reads mode 1).
    let doc = form_page(
        "q BT /F1 10 Tf 1 7 Tr 10 200 Td (AB) Tj ET Q BT /F1 10 Tf 0 Tr 10 50 Td (CD) Tj ET",
        "",
        true,
    );
    assert_eq!(texts(&doc).0, "AB\n\nCD\n\n");
    // Mode 7 set by the page reaches the form, which hayro draws so; one
    // a form sets is not drawn (MuPDF: "XY\n\nCD\n\n").
    let doc = form_page(
        "q BT /F1 10 Tf 7 Tr 10 200 Td (AB) Tj ET Q q 7 Tr /Fm Do Q \
         BT /F1 10 Tf 0 Tr 10 50 Td (CD) Tj ET",
        "BT /F1 10 Tf 10 100 Td (XY) Tj ET",
        true,
    );
    assert_eq!(texts(&doc).0, "AB\n\nXY\n\nCD\n\n");
    let doc = form_page(
        "/Fm Do BT /F1 10 Tf 10 50 Td (CD) Tj ET",
        "BT /F1 10 Tf 7 Tr 10 100 Td (XY) Tj ET",
        true,
    );
    assert_eq!(texts(&doc).0, "CD\n\n");
}

#[test]
fn truetype_cmaps_with_many_groups_are_read_by_bisection() {
    // Eight fonts sharing a program whose format 12 subtable has 500,000
    // groups, none for the codes shown: looking each code up by walking
    // the groups took seconds.
    const GROUPS: u32 = 500_000;
    let mut sub = Vec::new();
    for v in [12u16, 0] {
        sub.extend(v.to_be_bytes());
    }
    for v in [16 + 12 * GROUPS, 0, GROUPS] {
        sub.extend(v.to_be_bytes());
    }
    for i in 0..GROUPS {
        for v in [0x10000 + i, 0x10000 + i, 1] {
            sub.extend(v.to_be_bytes());
        }
    }
    let mut cmap = Vec::new();
    for v in [0u16, 1, 3, 1] {
        cmap.extend(v.to_be_bytes());
    }
    cmap.extend(12u32.to_be_bytes());
    cmap.extend(&sub);
    let tables: [(&[u8; 4], Vec<u8>); 2] = [(b"cmap", cmap), (b"glyf", vec![0; 4])];
    let mut font = Vec::new();
    font.extend(0x0001_0000u32.to_be_bytes());
    for v in [2u16, 0, 0, 0] {
        font.extend(v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        font.extend(*tag);
        for v in [0, offset as u32, data.len() as u32] {
            font.extend(v.to_be_bytes());
        }
        offset += data.len();
    }
    for (_, data) in &tables {
        font.extend(data);
    }
    let program = format!(
        "<< /Length {} >>\nstream\n{}\nendstream",
        font.len(),
        font.iter().map(|&b| char::from(b)).collect::<String>()
    );
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        String::new(),
        String::new(),
        program,
    ];
    let (mut fonts, mut content) = (String::new(), String::new());
    for i in 0..8 {
        fonts.push_str(&format!("/F{i} {} 0 R ", objects.len() + 1));
        content.push_str(&format!("BT /F{i} 12 Tf 10 {} Td (A) Tj ET ", 20 + 20 * i));
        objects.push(format!(
            "<< /Type /Font /Subtype /TrueType /BaseFont /T{i} /FirstChar 65 /LastChar 65 \
             /Widths [500] /FontDescriptor << /Type /FontDescriptor /FontName /T{i} /Flags 32 \
             /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 \
             /CapHeight 700 /StemV 80 /FontFile2 5 0 R >> >>"
        ));
    }
    objects[2] = format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << {fonts}>> >> \
         /Contents 4 0 R >>"
    );
    objects[3] = stream("", &content);
    // Latin-1 bytes as chars: `pdf` writes them back as UTF-8, so encode by hand.
    let bytes: Vec<u8> = pdf(&objects);
    let bytes = latin1(&bytes);
    let doc = ViewDocument::open(bytes, "").unwrap();
    let started = Instant::now();
    doc.structured_text(0).unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
}

/// `pdf`'s UTF-8 output with every character below U+0100 as its byte.
fn latin1(utf8: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(utf8)
        .chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect()
}
