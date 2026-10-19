// SPDX-License-Identifier: MIT
//! Acceptance for spec 05 section 12 (integration addenda U1 to U7, found
//! by the corpus run): every case builds its page and font here and reads
//! what the view's text extraction gives each code. Expected values come
//! from ISO 32000-2 §9.10.2, Annex D (WinAnsiEncoding), the
//! OpenType `cmap` table and the spec's own rules.
use std::collections::BTreeMap;
use tandem_pdf_engine::view::ViewDocument;

// ------------------------------------------------------------------ builders

/// A classic-xref PDF of `objects` (numbered from 1), bodies as bytes.
fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
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

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend(data);
    out.extend(b"\nendstream");
    out
}

/// One page of `width` × `height` with `resources` and `content`; `extra`
/// objects are numbered from 5.
fn page_doc(
    width: u32,
    height: u32,
    content: &str,
    resources: &str,
    extra: &[Vec<u8>],
) -> ViewDocument {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources {resources} /Contents 4 0 R >>"
        )
        .into_bytes(),
        stream("", content.as_bytes()),
    ];
    objects.extend(extra.iter().cloned());
    ViewDocument::open(pdf(&objects), "").unwrap()
}

/// Every character drawn, grouped by baseline (page-space y rounded), in
/// drawing order within each.
fn by_baseline(doc: &ViewDocument) -> BTreeMap<i64, String> {
    let text = doc.structured_text(0).unwrap();
    let mut out: BTreeMap<i64, String> = BTreeMap::new();
    for block in text.unclipped_blocks() {
        for line in &block.lines {
            for c in &line.chars {
                if !c.synthetic {
                    out.entry(c.origin[1].round() as i64).or_default().push(c.c);
                }
            }
        }
    }
    out
}

/// A Type3 font: each code of `codes` with `Some(name)` is named in
/// /Differences, a code with `None` has no /Differences entry; every glyph
/// the same box, 500 units wide. `extra` goes into the font dictionary.
fn type3_font(codes: &[(u8, Option<&str>)], extra: &str) -> Vec<u8> {
    let mut procs = String::new();
    let mut differences = String::new();
    for (code, name) in codes {
        if let Some(name) = name {
            procs.push_str(&format!("/{name} 6 0 R "));
            differences.push_str(&format!("{code} /{name} "));
        }
    }
    format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 500 700] \
         /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /box 6 0 R {procs}>> \
         /Encoding << /Type /Encoding /Differences [{differences}] >> \
         /FirstChar 0 /LastChar 255 /Widths [{}] {extra} >>",
        ["500"; 256].join(" ")
    )
    .into_bytes()
}

/// Each code of `codes` shown alone on its own line in the Type3 font of
/// `codes` (with `extra` in its dictionary and `more` objects numbered from
/// 7); the text read at each, in order.
fn type3_texts_with(codes: &[(u8, Option<&str>)], extra: &str, more: &[Vec<u8>]) -> Vec<String> {
    let height = 40 + 14 * codes.len() as u32;
    let mut content = String::new();
    for (i, (code, _)) in codes.iter().enumerate() {
        content.push_str(&format!(
            "BT /T3 10 Tf 20 {} Td <{code:02X}> Tj ET ",
            height - 20 - 14 * i as u32
        ));
    }
    let mut objects = vec![
        type3_font(codes, extra),
        stream("", b"500 0 d0 0 0 400 600 re f"),
    ];
    objects.extend(more.iter().cloned());
    let doc = page_doc(
        300,
        height,
        &content,
        "<< /Font << /T3 5 0 R >> >>",
        &objects,
    );
    let lines = by_baseline(&doc);
    (0..codes.len())
        .map(|i| {
            lines
                .get(&(20 + 14 * i as i64))
                .cloned()
                .unwrap_or_default()
        })
        .collect()
}

fn type3_texts(codes: &[(u8, Option<&str>)]) -> Vec<String> {
    type3_texts_with(codes, "", &[])
}

/// The rows whose text is not the expected one, for the assertion message.
fn wrong<'a>(rows: impl IntoIterator<Item = (String, &'a str)>, texts: &[String]) -> Vec<String> {
    rows.into_iter()
        .zip(texts)
        .filter(|((_, want), got)| got.as_str() != *want)
        .map(|((label, want), got)| format!("{label}: {got:?} not {want:?}"))
        .collect()
}

const FFFD: &str = "\u{fffd}";

/// A TrueType program of `tables` (tag, data), sorted by tag.
fn truetype(mut tables: Vec<(&[u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(tag, _)| **tag);
    let mut font = Vec::new();
    font.extend(0x0001_0000u32.to_be_bytes());
    for v in [tables.len() as u16, 0, 0, 0] {
        font.extend(v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        font.extend(*tag);
        for v in [0, offset as u32, data.len() as u32] {
            font.extend(v.to_be_bytes());
        }
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    font
}

fn be16s(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// The tables every glyph program needs: head, hhea, maxp, hmtx, loca and
/// glyf of `glyphs` empty glyphs, 500 units wide.
fn base_tables(glyphs: u16) -> Vec<(&'static [u8; 4], Vec<u8>)> {
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&glyphs.to_be_bytes());
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&glyphs.to_be_bytes());
    let hmtx: Vec<u8> = (0..glyphs).flat_map(|_| be16s(&[500, 0])).collect();
    let loca = vec![0u8; 2 * (usize::from(glyphs) + 1)];
    vec![
        (b"head", head),
        (b"hhea", hhea),
        (b"maxp", maxp),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"glyf", vec![]),
    ]
}

/// A `post` table of format 3 (no glyph names).
fn post_v3() -> Vec<u8> {
    let mut post = 0x0003_0000u32.to_be_bytes().to_vec();
    post.extend([0u8; 28]);
    post
}

/// A cmap table of one subtable (platform, encoding, data).
fn cmap_table(platform: u16, encoding: u16, subtable: Vec<u8>) -> Vec<u8> {
    let mut cmap = be16s(&[0, 1, platform, encoding]);
    cmap.extend(12u32.to_be_bytes());
    cmap.extend(subtable);
    cmap
}

/// A format 4 subtable mapping each segment (first code, last code, first
/// glyph), given in code order, followed by the closing 0xFFFF segment.
fn format4(segments: &[(u16, u16, u16)]) -> Vec<u8> {
    let mut all: Vec<(u16, u16, i16)> = segments
        .iter()
        .map(|&(start, end, glyph)| (start, end, (i32::from(glyph) - i32::from(start)) as i16))
        .collect();
    all.push((0xFFFF, 0xFFFF, 1));
    let n = all.len() as u16;
    let mut data = be16s(&[4, 0, 0, 2 * n, 0, 0, 0]);
    data.extend(all.iter().flat_map(|s| s.1.to_be_bytes()));
    data.extend([0, 0]);
    data.extend(all.iter().flat_map(|s| s.0.to_be_bytes()));
    data.extend(all.iter().flat_map(|s| s.2.to_be_bytes()));
    data.extend(all.iter().flat_map(|_| 0u16.to_be_bytes()));
    let length = data.len() as u16;
    data[2..4].copy_from_slice(&length.to_be_bytes());
    data
}

/// A TrueType font with descriptor /Flags `flags` and `encoding` (a
/// dictionary entry such as `/Encoding /WinAnsiEncoding`, or empty)
/// embedding `program`, showing the hex string `shown`; the text read,
/// whitespace removed.
fn embedded_truetype_text(program: Vec<u8>, flags: u32, encoding: &str, shown: &str) -> String {
    let doc = page_doc(
        300,
        300,
        &format!("BT /F1 10 Tf 10 100 Td <{shown}> Tj ET"),
        "<< /Font << /F1 5 0 R >> >>",
        &[
            format!(
                "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Test {encoding} \
                 /FirstChar 0 /LastChar 255 /Widths [{}] /FontDescriptor << \
                 /Type /FontDescriptor /FontName /ABCDEF+Test /Flags {flags} \
                 /FontBBox [0 -200 1000 800] /ItalicAngle 0 /Ascent 800 /Descent -200 \
                 /CapHeight 700 /StemV 80 /FontFile2 6 0 R >> >>",
                ["500"; 256].join(" ")
            )
            .into_bytes(),
            stream("", &program),
        ],
    );
    let text = doc.structured_text(0).unwrap().as_text();
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn hex(text: &str) -> Vec<String> {
    text.chars()
        .map(|c| format!("U+{:04X}", c as u32))
        .collect()
}

// ------------------------------------------------------------------ U1 Type3 codes without a usable name

#[test]
fn type3_codes_without_a_usable_name_read_as_ascii() {
    let rows: [(u8, Option<&str>, &str); 14] = [
        // Names outside the glyph list, at printable ASCII codes.
        (0x2E, Some("leaderdot"), "."),
        (0x7E, Some("thindash"), "~"),
        (0x72, Some("C72"), "r"),
        (0x74, Some("D8"), "t"),
        (0x41, Some("g65"), "A"),
        (0x42, Some("a65"), "B"),
        // Codes without a name.
        (0x21, None, "!"),
        (0x27, None, "'"),
        (0x39, None, "9"),
        (0x60, None, "`"),
        // Outside printable ASCII: unknown.
        (0x1F, None, FFFD),
        (0x7F, Some("foo"), FFFD),
        (0xA1, Some("foo"), FFFD),
        (0xE9, None, FFFD),
    ];
    let codes: Vec<(u8, Option<&str>)> = rows.iter().map(|(c, n, _)| (*c, *n)).collect();
    let texts = type3_texts(&codes);
    let labels = rows.iter().map(|(code, name, want)| {
        (
            format!("code {code:#04x} /{}", name.unwrap_or("(none)")),
            *want,
        )
    });
    let wrong = wrong(labels, &texts);
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn type3_glyph_list_names_win_over_the_code() {
    // U1 applies only after 4.1 and 4.2: a resolvable name decides.
    let texts = type3_texts(&[
        (0x41, Some("Omega")),
        (0x42, Some("a66")),
        (0x43, Some("uni0041")),
    ]);
    assert_eq!(texts, ["\u{2126}", "B", "A"]);
}

// ------------------------------------------------------------------ U2 Unicode cmap without glyph names

/// A program of 4 glyphs whose (3,1) subtable maps A, B, C to glyphs 1-3;
/// with a `post` table of format 3 when `post`.
fn unicode_program(post: bool) -> Vec<u8> {
    let mut tables = base_tables(4);
    tables.push((b"cmap", cmap_table(3, 1, format4(&[(0x41, 0x43, 1)]))));
    if post {
        tables.push((b"post", post_v3()));
    }
    truetype(tables)
}

#[test]
fn unnamed_glyphs_of_a_unicode_cmap_read_as_the_code() {
    let cases = [
        ("no post, no /Encoding", unicode_program(false), ""),
        ("post format 3, no /Encoding", unicode_program(true), ""),
        (
            "post format 3, /Differences not covering the codes",
            unicode_program(true),
            "/Encoding << /Type /Encoding /Differences [24 /breve /caron] >>",
        ),
    ];
    let mut wrong = Vec::new();
    for (label, program, encoding) in cases {
        let got = embedded_truetype_text(program, 32, encoding, "414243");
        if got != "ABC" {
            wrong.push(format!("{label}: {:?} not \"ABC\"", hex(&got)));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

// ------------------------------------------------------------------ U3 symbolic TrueType, unnamed glyphs

#[test]
fn symbolic_truetype_unnamed_codes_read_as_win_ansi() {
    // A (3,0) subtable maps 0xF020-0xF0FF to glyphs 1-224; no glyph names.
    let mut tables = base_tables(225);
    tables.push((b"cmap", cmap_table(3, 0, format4(&[(0xF020, 0xF0FF, 1)]))));
    tables.push((b"post", post_v3()));
    let got = embedded_truetype_text(truetype(tables), 4, "", "929641B0");
    assert_eq!(
        hex(&got),
        ["U+2019", "U+2013", "U+0041", "U+00B0"],
        "WinAnsi quoteright, endash, A, degree"
    );
}

// ------------------------------------------------------------------ U4 inverted cmap, glyphs of several characters

#[test]
fn an_inverted_cmap_glyph_of_several_characters_reads_the_lowest() {
    // Glyph 1: A only. 2: U+03A9 and U+2126. 3: U+002D and U+00AD.
    // 4: U+0042 and U+E001 (private use). 5: U+E002 and U+E005 (both
    // private use).
    let mut tables = base_tables(6);
    tables.push((
        b"cmap",
        cmap_table(
            3,
            1,
            format4(&[
                (0x2D, 0x2D, 3),
                (0x41, 0x41, 1),
                (0x42, 0x42, 4),
                (0xAD, 0xAD, 3),
                (0x3A9, 0x3A9, 2),
                (0x2126, 0x2126, 2),
                (0xE001, 0xE002, 4),
                (0xE005, 0xE005, 5),
            ]),
        ),
    ));
    let program = truetype(tables);
    let doc = page_doc(
        300,
        300,
        "BT /F1 10 Tf 10 100 Td <00010002000300040005> Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &[
            b"<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Test /Encoding /Identity-H \
               /DescendantFonts [6 0 R] >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+Test \
               /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
               /CIDToGIDMap /Identity /DW 500 /FontDescriptor << /Type /FontDescriptor \
               /FontName /ABCDEF+Test /Flags 32 /FontBBox [0 -200 1000 800] /ItalicAngle 0 \
               /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 7 0 R >> >>"
                .to_vec(),
            stream("", &program),
        ],
    );
    let got: String = doc
        .structured_text(0)
        .unwrap()
        .as_text()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    // Glyph 4 reads U+0042, not the private-use U+E001 that also maps to
    // it; glyph 5 has only private-use characters, so the lowest of those.
    assert_eq!(
        hex(&got),
        ["U+0041", "U+03A9", "U+002D", "U+0042", "U+E002"]
    );
}

// ------------------------------------------------------------------ U5 controls and noncharacters stay unknown

#[test]
fn type3_a_number_names_at_control_codes_read_as_unknown() {
    // 4.2 gives U+0010 and U+001C, controls, which 3.7 drops.
    let texts = type3_texts(&[(16, Some("a16")), (28, Some("a28")), (0x41, Some("a65"))]);
    assert_eq!(texts, [FFFD, FFFD, "A"]);
}

#[test]
fn to_unicode_noncharacters_and_controls_are_not_text() {
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                /CMapName /X def 1 begincodespacerange <00> <FF> endcodespacerange\n\
                3 beginbfchar\n<A1> <FFFF>\n<A2> <FFFE>\n<A3> <0007>\nendbfchar\n\
                endcmap CMapName currentdict /CMap defineresource pop end end";
    let texts = type3_texts_with(
        &[
            (0xA1, Some("foo")),
            (0xA2, Some("foo2")),
            (0xA3, Some("Omega")),
        ],
        "/ToUnicode 7 0 R",
        &[stream("", cmap.as_bytes())],
    );
    // U+FFFF and U+FFFE give no text and nothing else maps 0xA1 or 0xA2;
    // U+0007 gives no text and the name reads 0xA3.
    assert_eq!(texts, [FFFD, FFFD, "\u{2126}"]);
}

// ------------------------------------------------------------------ U6 control codes of a Unicode cmap

/// `base_tables(glyphs)` with the glyphs of `outlined` drawn as a filled
/// triangle (one contour of three on-curve points); the others stay empty.
fn tables_with_outlines(glyphs: u16, outlined: &[u16]) -> Vec<(&'static [u8; 4], Vec<u8>)> {
    let mut triangle = be16s(&[1, 100, 0, 500, 600, 2, 0]);
    triangle.extend([1u8, 1, 1]);
    for v in [100i16, 400, -200, 0, 0, 600] {
        triangle.extend(v.to_be_bytes());
    }
    triangle.resize(triangle.len().next_multiple_of(2), 0);
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for glyph in 0..glyphs {
        loca.extend(((glyf.len() / 2) as u16).to_be_bytes());
        if outlined.contains(&glyph) {
            glyf.extend(&triangle);
        }
    }
    loca.extend(((glyf.len() / 2) as u16).to_be_bytes());
    let mut tables = base_tables(glyphs);
    for (tag, data) in &mut tables {
        match *tag {
            b"loca" => *data = loca.clone(),
            b"glyf" => *data = glyf.clone(),
            _ => {}
        }
    }
    tables
}

/// U6: through a Unicode subtable with no glyph names (U2), a code whose
/// character is a whitespace control (0x09 to 0x0D) reads a space only when
/// its glyph draws nothing; a glyph that draws a mark there is not a space
/// (subset fonts that reuse low codes for letters) and reads U+FFFD. Other
/// controls read U+FFFD (U2, 3.7). Each code is drawn on its own line.
#[test]
fn unicode_cmap_whitespace_controls_read_a_space_only_for_blank_glyphs() {
    // Glyph 1 (outlined) at 0x0B and 0x0D, glyph 2 (blank) at 0x09 and
    // 0x0A, glyph 3 (outlined) at 0x04 and 0x41.
    let mut tables = tables_with_outlines(4, &[1, 3]);
    tables.push((
        b"cmap",
        cmap_table(
            3,
            1,
            format4(&[
                (0x04, 0x04, 3),
                (0x09, 0x09, 2),
                (0x0A, 0x0A, 2),
                (0x0B, 0x0B, 1),
                (0x0D, 0x0D, 1),
                (0x41, 0x41, 3),
            ]),
        ),
    ));
    tables.push((b"post", post_v3()));
    let program = truetype(tables);
    let rows: [(u8, &str); 6] = [
        (0x0B, FFFD),
        (0x0D, FFFD),
        (0x09, " "),
        (0x0A, " "),
        (0x04, FFFD),
        (0x41, "A"),
    ];
    let mut all_wrong = Vec::new();
    // For the symbolic flags only the rows that read U+FFFD or a letter:
    // which of U2 and U3 reads an empty glyph of a symbolic font is U3's
    // question (owner, T11), not this one.
    for flags in [32, 4] {
        let rows: Vec<(u8, &str)> = rows
            .iter()
            .copied()
            .filter(|(_, want)| flags == 32 || *want != " ")
            .collect();
        let mut content = String::new();
        for (i, (code, _)) in rows.iter().enumerate() {
            content.push_str(&format!(
                "BT /F1 10 Tf 20 {} Td <{code:02X}> Tj ET ",
                120 - 14 * i
            ));
        }
        let doc = page_doc(
            300,
            140,
            &content,
            "<< /Font << /F1 5 0 R >> >>",
            &[
                format!(
                    "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Test \
                     /FirstChar 0 /LastChar 255 /Widths [{}] /FontDescriptor << \
                     /Type /FontDescriptor /FontName /ABCDEF+Test /Flags {flags} \
                     /FontBBox [0 -200 1000 800] /ItalicAngle 0 /Ascent 800 /Descent -200 \
                     /CapHeight 700 /StemV 80 /FontFile2 6 0 R >> >>",
                    ["500"; 256].join(" ")
                )
                .into_bytes(),
                stream("", &program),
            ],
        );
        let lines = by_baseline(&doc);
        let texts: Vec<String> = (0..rows.len())
            .map(|i| {
                lines
                    .get(&(20 + 14 * i as i64))
                    .cloned()
                    .unwrap_or_default()
            })
            .collect();
        let labels = rows
            .iter()
            .map(|(code, want)| (format!("/Flags {flags}, code {code:#04x}"), *want));
        all_wrong.extend(wrong(labels, &texts));
    }
    assert!(all_wrong.is_empty(), "{all_wrong:#?}");
}

// ------------------------------------------------------------------ U7 Private Use codes of a Unicode encoding CMap

/// U7: a composite font whose encoding is a predefined Unicode CMap and
/// whose CIDFont names an Adobe collection reads a code in the Private Use
/// Area through its CID and the collection's UCS2 CMap when that gives a
/// character outside the Private Use Area (3.1 step 2). Here
/// U+E687, the Hong Kong supplementary character set's Private Use code,
/// is CID-mapped in Adobe-CNS1 to U+2555B; U+4E2D reads as itself.
#[test]
fn u7_a_private_use_code_of_a_unicode_cmap_reads_through_its_cid() {
    let mut wrong = Vec::new();
    for encoding in ["/UniCNS-UTF16-H", "/UniCNS-UCS2-H"] {
        let doc = page_doc(
            300,
            300,
            "BT /F1 20 Tf 20 100 Td <4E2DE687> Tj ET",
            "<< /Font << /F1 5 0 R >> >>",
            &[
                format!(
                    "<< /Type /Font /Subtype /Type0 /BaseFont /MingLiU /Encoding {encoding} \
                     /DescendantFonts [6 0 R] >>"
                )
                .into_bytes(),
                b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /MingLiU /CIDSystemInfo \
                  << /Registry (Adobe) /Ordering (CNS1) /Supplement 4 >> /DW 1000 \
                  /FontDescriptor 7 0 R >>"
                    .to_vec(),
                b"<< /Type /FontDescriptor /FontName /MingLiU /Flags 4 \
                  /FontBBox [0 -200 1000 800] /ItalicAngle 0 /Ascent 800 /Descent -200 \
                  /CapHeight 700 /StemV 80 >>"
                    .to_vec(),
            ],
        );
        let text: String = doc
            .structured_text(0)
            .unwrap()
            .as_text()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if text != "\u{4E2D}\u{2555B}" {
            wrong.push(format!(
                "{encoding}: {:?}, want [U+4E2D, U+2555B]",
                hex(&text)
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
