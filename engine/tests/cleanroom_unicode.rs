// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acceptance for spec 05 (character codes to Unicode): every case builds a
//! page whose font is the subject, and reads what the view's text
//! extraction gives each code. Expected values come from ISO 32000-2
//! §9.10.2, the Adobe Glyph List specification, Adobe's CMaps and mapping
//! files, the OpenType `cmap` and `post` tables, and UnicodeData.txt.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tandem_pdf_engine::view::{MAX_SEARCH_QUADS, ViewDocument};

include!("data/adobe-symbol-encodings.rs");

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

fn page(content: &str, resources: &str, extra: &[Vec<u8>]) -> ViewDocument {
    page_doc(300, 300, content, resources, extra)
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

/// A Type3 font whose `codes` name these glyphs, every glyph the same
/// procedure, 500 units wide.
fn type3_font(codes: &[(u8, &str)]) -> Vec<u8> {
    let mut procs = String::new();
    let mut differences = String::new();
    for (code, name) in codes {
        procs.push_str(&format!("/{name} 6 0 R "));
        differences.push_str(&format!("{code} /{name} "));
    }
    format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 500 700] \
         /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << {procs}>> \
         /Encoding << /Type /Encoding /Differences [{differences}] >> \
         /FirstChar 0 /LastChar 255 /Widths [{}] >>",
        ["500"; 256].join(" ")
    )
    .into_bytes()
}

/// Each code of `codes` shown alone on its own line (baseline y = 20 + 14 i
/// from the top) in the Type3 font naming it; the text read at each.
fn type3_texts(codes: &[(u8, &str)]) -> Vec<String> {
    let height = 40 + 14 * codes.len() as u32;
    let mut content = String::new();
    for (i, (code, _)) in codes.iter().enumerate() {
        content.push_str(&format!(
            "BT /T3 10 Tf 20 {} Td <{code:02X}> Tj ET ",
            height - 20 - 14 * i as u32
        ));
    }
    let doc = page_doc(
        300,
        height,
        &content,
        "<< /Font << /T3 5 0 R >> >>",
        &[type3_font(codes), stream("", b"500 0 d0 0 0 400 600 re f")],
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

fn u(values: &[u32]) -> String {
    values.iter().map(|&v| char::from_u32(v).unwrap()).collect()
}

const FFFD: &str = "\u{fffd}";

// ------------------------------------------------------------------ N1 glyph names

#[test]
fn glyph_names_read_by_the_glyph_list_and_the_uni_and_u_forms() {
    let rows: [(&str, &str); 25] = [
        ("A", "A"),
        ("Euro", "€"),
        ("Acircumflex", "Â"),
        ("afii10017", "\u{410}"),
        ("uni0041", "A"),
        ("uni00e9", "é"),
        ("u00E9", "é"),
        ("u0041", "A"),
        ("uni004", FFFD),
        ("uFFFFFF", FFFD),
        ("foo", FFFD),
        ("g65", FFFD),
        ("G41", FFFD),
        ("cid65", FFFD),
        ("a1", FFFD),
        ("A.sc", "A"),
        ("f_i", "fi"),
        ("f_f_i", "ffi"),
        ("a5", FFFD),
        ("space", " "),
        ("Omega", "\u{2126}"),
        ("Delta", "\u{2206}"),
        ("mu", "\u{b5}"),
        ("fi", "fi"),
        ("uni0000", FFFD),
    ];
    let codes: Vec<(u8, &str)> = (rows.iter().enumerate())
        .map(|(i, (name, _))| (0x80 + i as u8, *name))
        .collect();
    let texts = type3_texts(&codes);
    let wrong: Vec<String> = (rows.iter().zip(&texts))
        .filter(|((_, want), got)| got != want)
        .map(|((name, want), got)| format!("/{name}: {got:?} not {want:?}"))
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
    let texts = type3_texts(&[(0x41, "sfthyphen"), (0x42, "ff")]);
    assert_eq!(texts, ["\u{ad}", "ff"]);
}

#[test]
#[ignore = "clean-room: 05§4.1 glyph names by the AGL specification; 05§3.7 controls"]
fn glyph_names_follow_the_agl_specification() {
    let rows: [(u8, &str, &str); 12] = [
        (0x80, "uni00410042", "AB"),
        (0x81, "uniD800", FFFD),
        (0x82, "u1F600", "\u{1F600}"),
        (0x83, "T_h", "Th"),
        (0x84, "uni0041_uni0042", "AB"),
        (0xA2, "a67", FFFD),
        (0x86, "dalethatafpatah", "\u{5d3}\u{5b2}"),
        (0x87, "uni0009", " "),
        (0x88, "u0007", FFFD),
        (0x89, "uni0041_foo", FFFD),
        (0x8A, "nbspace", "\u{a0}"),
        (0x8B, "delta", "\u{3b4}"),
    ];
    let codes: Vec<(u8, &str)> = rows.iter().map(|(code, name, _)| (*code, *name)).collect();
    let texts = type3_texts(&codes);
    let wrong: Vec<String> = (rows.iter().zip(&texts))
        .filter(|((_, _, want), got)| got != want)
        .map(|((_, name, want), got)| format!("/{name}: {got:?} not {want:?}"))
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

// ------------------------------------------------------------------ N10 Type3 a<n>

#[test]
fn type3_names_that_are_their_own_code_read_as_latin_1() {
    let texts = type3_texts(&[(65, "a65"), (200, "a200")]);
    assert_eq!(texts, ["A", "\u{c8}"]);
}

#[test]
#[ignore = "clean-room: 05§4.2 a<n> reads as code n only at code n"]
fn type3_names_of_another_code_read_as_unknown() {
    let texts = type3_texts(&[(162, "a67"), (65, "g65"), (66, "a65"), (67, "g67")]);
    assert_eq!(texts, [FFFD, FFFD, FFFD, FFFD]);
}

// ------------------------------------------------------------------ N2, N9 ToUnicode

/// Helvetica WinAnsi showing `ABCD`, with `to_unicode` as its /ToUnicode
/// stream (or, when it is a single `/Name`, as a name).
fn to_unicode_text(to_unicode: &str) -> String {
    let (entry, extra) = if to_unicode.starts_with('/') && !to_unicode.contains(' ') {
        (to_unicode.to_string(), vec![])
    } else {
        ("6 0 R".to_string(), vec![stream("", to_unicode.as_bytes())])
    };
    let mut objects = vec![
        format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding \
             /ToUnicode {entry} >>"
        )
        .into_bytes(),
    ];
    objects.extend(extra);
    let doc = page(
        "BT /F1 10 Tf 10 100 Td (ABCD) Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &objects,
    );
    doc.structured_text(0)
        .unwrap()
        .as_text()
        .trim_end()
        .to_string()
}

/// A ToUnicode CMap with a one-byte codespace and `body` as its blocks.
fn cmap(body: &str) -> String {
    format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T def \
         1 begincodespacerange <00> <FF> endcodespacerange\n{body}\n\
         endcmap CMapName currentdict /CMap defineresource pop end end"
    )
}

#[test]
fn to_unicode_maps_codes_and_falls_back_for_the_rest() {
    let cases = [
        // Strings, sequences, surrogate pairs.
        (
            cmap("1 beginbfchar <41> <0058> endbfchar"),
            "XBCD".to_string(),
        ),
        (
            cmap("1 beginbfchar <41> <00660069> endbfchar"),
            "fiBCD".into(),
        ),
        (
            cmap("1 beginbfchar <41> <D83DDE00> endbfchar"),
            "\u{1F600}BCD".into(),
        ),
        // Ranges: the last unit increases; arrays give one string per code.
        (
            cmap("1 beginbfrange <41> <43> <0061> endbfrange"),
            "abcD".into(),
        ),
        (
            cmap("1 beginbfrange <41> <43> <00660069> endbfrange"),
            "fifjfkD".into(),
        ),
        (
            cmap("1 beginbfrange <41> <43> [<0078> <00790079> <007A>] endbfrange"),
            "xyyzD".into(),
        ),
        // Only B mapped: the others read through the encoding.
        (cmap("1 beginbfchar <42> <0058> endbfchar"), "AXCD".into()),
        // A later mapping of a code replaces an earlier one.
        (
            cmap("2 beginbfchar <41> <0058> <41> <0059> endbfchar"),
            "YBCD".into(),
        ),
        // CID mappings read the CID as the character.
        (cmap("1 begincidchar <41> 72 endcidchar"), "HBCD".into()),
        // No codespace range.
        (
            "begincmap 1 beginbfchar <41> <0058> endbfchar endcmap".into(),
            "XBCD".into(),
        ),
        // NUL and other controls fall back; whitespace controls are spaces.
        (cmap("1 beginbfchar <41> <0000> endbfchar"), "ABCD".into()),
        (cmap("1 beginbfchar <41> <0007> endbfchar"), "ABCD".into()),
        (cmap("1 beginbfchar <41> <000A> endbfchar"), " BCD".into()),
        (cmap("1 beginbfchar <41> <0009> endbfchar"), " BCD".into()),
        // An empty destination falls back.
        (cmap("1 beginbfchar <41> <> endbfchar"), "ABCD".into()),
        // A one-byte destination reads as Latin-1.
        (cmap("1 beginbfchar <41> <5A> endbfchar"), "ZBCD".into()),
        // An unpaired surrogate is no mapping.
        (cmap("1 beginbfchar <41> <D800> endbfchar"), "ABCD".into()),
        // A carry out of the last byte goes into the unit.
        (
            cmap("1 beginbfrange <41> <43> <00FF> endbfrange"),
            "\u{ff}\u{100}\u{101}D".into(),
        ),
        // A two-byte codespace still maps a one-byte code by its value.
        (
            "begincmap 1 begincodespacerange <0000> <FFFF> endcodespacerange \
             1 beginbfchar <0041> <0058> endbfchar endcmap"
                .into(),
            "XBCD".into(),
        ),
        // Junk outside the mapping blocks is skipped.
        (
            cmap("> junk 1 beginbfchar <41> <0058> endbfchar ] stray"),
            "XBCD".into(),
        ),
        // Not a CMap at all.
        ("this is not a cmap".into(), "ABCD".into()),
    ];
    let wrong: Vec<String> = (cases.iter())
        .filter(|(to_unicode, want)| to_unicode_text(to_unicode) != *want)
        .map(|(to_unicode, want)| {
            format!(
                "{to_unicode}: {:?} not {want:?}",
                to_unicode_text(to_unicode)
            )
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
    // A ToUnicode name on a simple font is ignored.
    assert_eq!(to_unicode_text("/Identity-H"), "ABCD");
}

#[test]
#[ignore = "clean-room: 05§3.3 tolerant ToUnicode entries; 05§3.7 controls in sequences"]
fn to_unicode_keeps_good_entries_beside_bad_ones() {
    let cases = [
        // One malformed entry is skipped, the others map.
        (cmap("2 beginbfchar <41> /H <42> <0058> endbfchar"), "AXCD"),
        // A NUL inside a sequence is dropped, the rest kept.
        (cmap("1 beginbfchar <41> <004100000042> endbfchar"), "ABBCD"),
    ];
    let wrong: Vec<String> = (cases.iter())
        .filter(|(to_unicode, want)| to_unicode_text(to_unicode) != *want)
        .map(|(to_unicode, want)| {
            format!(
                "{to_unicode}: {:?} not {want:?}",
                to_unicode_text(to_unicode)
            )
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// A Type0 font over a CIDFontType2 Helvetica without a program, with the
/// ToUnicode stream `to_unicode`, showing <0001000200030003>.
fn identity_font_text(to_unicode: &str) -> String {
    let doc = page(
        "BT /F1 10 Tf 10 100 Td <0001000200030003> Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &[
            b"<< /Type /Font /Subtype /Type0 /BaseFont /Helvetica /Encoding /Identity-H \
              /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Helvetica \
              /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
              /FontDescriptor 8 0 R >>"
                .to_vec(),
            stream("", to_unicode.as_bytes()),
            b"<< /Type /FontDescriptor /FontName /Helvetica /Flags 32 \
              /FontBBox [0 -200 1000 800] /ItalicAngle 0 /Ascent 800 /Descent -200 \
              /CapHeight 700 /StemV 80 >>"
                .to_vec(),
        ],
    );
    doc.structured_text(0).unwrap().as_text()
}

#[test]
fn to_unicode_cmaps_read_leniently() {
    // A name cut short by a stray `>` outside the mapping blocks.
    let stray = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering (UC>) /Supplement 0 >> def \
                 /CMapName /Adobe-Identity-UC> def 1 begincodespacerange <0000> <FFFF> \
                 endcodespacerange 3 beginbfchar <0001> <0048> <0002> <0065> <0003> <006C> \
                 endbfchar endcmap";
    assert_eq!(identity_font_text(stray), "Hell\n\n");
    // CID mappings read as the characters.
    let cids = "begincmap 1 begincodespacerange <0000> <FFFF> endcodespacerange \
                1 begincidrange <0001> <0003> 72 endcidrange endcmap";
    assert_eq!(identity_font_text(cids), "HIJJ\n\n");
}

#[test]
#[ignore = "clean-room: 05§3.3 a malformed entry is skipped; the codes fall back to the Macintosh glyph order"]
fn a_malformed_to_unicode_entry_falls_back_per_code() {
    // CIDs 1, 2, 3 of an Identity CIDFontType2 without a program are the
    // standard Macintosh glyphs .null, nonmarkingreturn and space.
    let broken = "begincmap 1 beginbfchar <0001> /H endbfchar endcmap";
    let text = identity_font_text(broken);
    assert_eq!(text, "\u{fffd}\u{fffd}  \n\n");
}

// ------------------------------------------------------------------ N3 Symbol, ZapfDingbats

/// Each code 0x20..=0xFE of the non-embedded standard font `base` on its
/// own line; the text read at each.
fn standard_font_texts(base: &str) -> BTreeMap<u8, String> {
    let codes: Vec<u8> = (0x20..=0xFE).collect();
    let height = 40 + 14 * codes.len() as u32;
    let mut content = String::new();
    for (i, code) in codes.iter().enumerate() {
        content.push_str(&format!(
            "BT /S 10 Tf 20 {} Td <{code:02X}> Tj ET ",
            height - 20 - 14 * i as u32
        ));
    }
    let doc = page_doc(
        300,
        height,
        &content,
        "<< /Font << /S 5 0 R >> >>",
        &[format!("<< /Type /Font /Subtype /Type1 /BaseFont /{base} >>").into_bytes()],
    );
    let lines = by_baseline(&doc);
    (codes.iter().enumerate())
        .map(|(i, code)| {
            let text = lines
                .get(&(20 + 14 * i as i64))
                .cloned()
                .unwrap_or_default();
            (*code, text)
        })
        .collect()
}

fn check_built_in(base: &str, table: &[(u8, &[u32])]) {
    let texts = standard_font_texts(base);
    let mut wrong = Vec::new();
    for (code, values) in table {
        if *code > 0xFE {
            continue;
        }
        let got = &texts[code];
        if !values.iter().any(|&v| *got == u(&[v])) {
            wrong.push(format!("{code:#04X}: {got:?} not {values:X?}"));
        }
    }
    assert!(wrong.is_empty(), "{base}: {}", wrong.join(", "));
}

#[test]
fn symbol_reads_through_its_built_in_encoding() {
    check_built_in("Symbol", &SYMBOL);
}

#[test]
#[ignore = "clean-room: 05§5.4 ZapfDingbats reads as the dingbats its built-in encoding names"]
fn zapf_dingbats_reads_through_its_built_in_encoding() {
    assert_eq!(standard_font_texts("ZapfDingbats")[&0x34], "\u{2714}");
    check_built_in("ZapfDingbats", &DINGBATS);
}

// ------------------------------------------------------------------ N4, N5 TrueType programs

/// A TrueType font program of `tables` (tag, data), sorted by tag.
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

/// A `post` table of format 2 naming each glyph (glyph 0 `.notdef`).
fn post_v2(names: &[&str]) -> Vec<u8> {
    let mut post = Vec::new();
    post.extend(0x0002_0000u32.to_be_bytes());
    post.extend([0u8; 28]);
    post.extend((names.len() as u16 + 1).to_be_bytes());
    post.extend(0u16.to_be_bytes());
    for i in 0..names.len() {
        post.extend((258 + i as u16).to_be_bytes());
    }
    for name in names {
        post.push(name.len() as u8);
        post.extend(name.as_bytes());
    }
    post
}

/// A cmap table of one subtable (platform, encoding, data).
fn cmap_table(platform: u16, encoding: u16, subtable: Vec<u8>) -> Vec<u8> {
    let mut cmap = be16s(&[0, 1, platform, encoding]);
    cmap.extend(12u32.to_be_bytes());
    cmap.extend(subtable);
    cmap
}

/// A format 4 subtable of `segments` (start, end, delta) in this order,
/// followed by the closing 0xFFFF segment.
fn format4(segments: &[(u16, u16, i16)]) -> Vec<u8> {
    let mut all = segments.to_vec();
    all.push((0xFFFF, 0xFFFF, 1));
    let n = all.len() as u16;
    let mut data = be16s(&[0, 0, 0, 2 * n, 0, 0, 0]);
    data.extend(all.iter().flat_map(|s| s.1.to_be_bytes()));
    data.extend([0, 0]);
    data.extend(all.iter().flat_map(|s| s.0.to_be_bytes()));
    data.extend(all.iter().flat_map(|s| s.2.to_be_bytes()));
    data.extend(all.iter().flat_map(|_| 0u16.to_be_bytes()));
    let length = data.len() as u16;
    data[2..4].copy_from_slice(&length.to_be_bytes());
    data[0..2].copy_from_slice(&4u16.to_be_bytes());
    data
}

/// A format 12 subtable claiming `claimed` groups and holding `groups`
/// (start, end, first glyph).
fn format12(groups: &[(u32, u32, u32)], claimed: u32) -> Vec<u8> {
    let mut data = be16s(&[12, 0]);
    data.extend((16 + 12 * groups.len() as u32).to_be_bytes());
    data.extend(0u32.to_be_bytes());
    data.extend(claimed.to_be_bytes());
    for (start, end, glyph) in groups {
        for v in [start, end, glyph] {
            data.extend(v.to_be_bytes());
        }
    }
    data
}

/// A TrueType font (descriptor /Flags `flags`, no /Encoding) embedding
/// `program`, showing `shown`; the text read.
fn embedded_truetype_text(program: Vec<u8>, flags: u32, shown: &str) -> String {
    let doc = page(
        &format!("BT /F1 10 Tf 10 100 Td ({shown}) Tj ET"),
        "<< /Font << /F1 5 0 R >> >>",
        &[
            format!(
                "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Test /FirstChar 32 \
                 /LastChar 126 /Widths [{}] /FontDescriptor << /Type /FontDescriptor \
                 /FontName /ABCDEF+Test /Flags {flags} /FontBBox [0 -200 1000 800] \
                 /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 \
                 /FontFile2 6 0 R >> >>",
                ["500"; 95].join(" ")
            )
            .into_bytes(),
            stream("", &program),
        ],
    );
    doc.structured_text(0)
        .unwrap()
        .as_text()
        .trim_end()
        .to_string()
}

#[test]
fn format_4_segments_decide_in_table_order() {
    // D..F in the first segment (glyphs 4..6); A..E in the second, out of
    // order and overlapping (glyphs 1..5). A, B and C meet the first
    // segment first, which does not hold them: glyph 0, no name.
    let names = ["alpha", "beta", "gamma", "Delta", "Epsilon", "Phi"];
    let mut tables = base_tables(7);
    tables.push((
        b"cmap",
        cmap_table(
            3,
            1,
            format4(&[(0x44, 0x46, 4 - 0x44), (0x41, 0x45, 1 - 0x41)]),
        ),
    ));
    tables.push((b"post", post_v2(&names)));
    let text = embedded_truetype_text(truetype(tables), 32, "ABCDEF");
    assert_eq!(text, "\u{fffd}\u{fffd}\u{fffd}\u{2206}\u{395}\u{3a6}");
}

#[test]
fn format_4_names_glyphs_through_post_format_2() {
    let names = ["alpha", "beta", "gamma"];
    let mut tables = base_tables(4);
    tables.push((
        b"cmap",
        cmap_table(3, 1, format4(&[(0x41, 0x43, 1 - 0x41)])),
    ));
    tables.push((b"post", post_v2(&names)));
    let text = embedded_truetype_text(truetype(tables), 32, "ABC");
    assert_eq!(text, "\u{3b1}\u{3b2}\u{3b3}");
}

#[test]
fn format_12_groups_read_what_the_table_holds() {
    let names = ["alpha", "beta", "gamma"];
    let mut tables = base_tables(4);
    // Five groups claimed, two present.
    tables.push((
        b"cmap",
        cmap_table(3, 10, format12(&[(0x41, 0x42, 1), (0x43, 0x43, 3)], 5)),
    ));
    tables.push((b"post", post_v2(&names)));
    let text = embedded_truetype_text(truetype(tables), 32, "ABC");
    assert_eq!(text, "\u{3b1}\u{3b2}\u{3b3}");
}

#[test]
fn a_symbolic_font_reads_ascii_codes_through_its_3_0_subtable() {
    let names = ["Alpha", "Beta", "Gamma"];
    let mut tables = base_tables(4);
    tables.push((
        b"cmap",
        cmap_table(3, 0, format4(&[(0xF041, 0xF043, (1i32 - 0xF041) as i16)])),
    ));
    tables.push((b"post", post_v2(&names)));
    let text = embedded_truetype_text(truetype(tables), 4, "ABC");
    assert_eq!(text, "\u{391}\u{392}\u{393}");
}

#[test]
fn truetype_cmaps_with_many_groups_are_read_by_bisection() {
    // Eight fonts sharing a program whose format 12 subtable has 500,000
    // groups, none for the codes shown: looking each code up by walking
    // the groups took seconds.
    const GROUPS: u32 = 500_000;
    let groups: Vec<(u32, u32, u32)> = (0..GROUPS).map(|i| (0x10000 + i, 0x10000 + i, 1)).collect();
    let program = truetype(vec![
        (b"cmap", cmap_table(3, 1, format12(&groups, GROUPS))),
        (b"glyf", vec![0; 4]),
    ]);
    let mut objects: Vec<Vec<u8>> = vec![stream("", &program)];
    let (mut fonts, mut content) = (String::new(), String::new());
    for i in 0..8 {
        fonts.push_str(&format!("/F{i} {} 0 R ", objects.len() + 5));
        content.push_str(&format!("BT /F{i} 12 Tf 10 {} Td (A) Tj ET ", 20 + 20 * i));
        objects.push(
            format!(
                "<< /Type /Font /Subtype /TrueType /BaseFont /T{i} /FirstChar 65 /LastChar 65 \
                 /Widths [500] /FontDescriptor << /Type /FontDescriptor /FontName /T{i} /Flags 32 \
                 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 \
                 /CapHeight 700 /StemV 80 /FontFile2 5 0 R >> >>"
            )
            .into_bytes(),
        );
    }
    let doc = page(&content, &format!("<< /Font << {fonts}>> >>"), &objects);
    let started = Instant::now();
    doc.structured_text(0).unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
}

// ------------------------------------------------------------------ N6, N7 composite fonts

/// A Type0 font with `encoding` over a `subtype` CIDFont of `ordering`
/// without a program or ToUnicode, showing the hex string `shown`.
fn composite_text(
    encoding: &str,
    subtype: &str,
    base: &str,
    ordering: &str,
    shown: &str,
) -> String {
    let doc = page(
        &format!("BT /F1 10 Tf 10 100 Td <{shown}> Tj ET"),
        "<< /Font << /F1 5 0 R >> >>",
        &[
            format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /{encoding} \
                 /DescendantFonts [6 0 R] >>"
            )
            .into_bytes(),
            format!(
                "<< /Type /Font /Subtype /{subtype} /BaseFont /{base} \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering ({ordering}) /Supplement 2 >> \
                 /FontDescriptor 7 0 R >>"
            )
            .into_bytes(),
            format!(
                "<< /Type /FontDescriptor /FontName /{base} /Flags 4 \
                 /FontBBox [0 -120 1000 880] /ItalicAngle 0 /Ascent 880 /Descent -120 \
                 /CapHeight 700 /StemV 80 >>"
            )
            .into_bytes(),
        ],
    );
    let text = doc.structured_text(0).unwrap().as_text();
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn cid_fonts_without_to_unicode_read_through_their_collection() {
    // CIDs 0x22, 0x410, 0x3042 through each collection's UCS2 CMap.
    let rows = [
        ("Japan1", [0x41, 0x3B6, 0x308D]),
        ("GB1", [0x41, 0x62B1, 0x5DFC]),
        ("CNS1", [0x41, 0x8840, 0x9A04]),
        ("Korea1", [0x41, 0x423, 0xBB23]),
    ];
    for (ordering, want) in rows {
        assert_eq!(
            composite_text(
                "Identity-H",
                "CIDFontType0",
                "Song",
                ordering,
                "002204103042"
            ),
            u(&want),
            "{ordering}"
        );
    }
    // A predefined multi-byte CMap splits the codes.
    assert_eq!(
        composite_text(
            "90ms-RKSJ-H",
            "CIDFontType0",
            "Song",
            "Japan1",
            "82A0414242"
        ),
        "\u{3042}ABB"
    );
    // A TrueType CIDFont without a program: CIDs are the standard
    // Macintosh glyph order.
    assert_eq!(
        composite_text(
            "Identity-H",
            "CIDFontType2",
            "Arial",
            "Identity",
            "00240044"
        ),
        "Aa"
    );
}

#[test]
#[ignore = "clean-room: 05§3.1 a CFF CIDFont with Identity ordering has no reading"]
fn cff_cid_fonts_with_identity_ordering_read_as_unknown() {
    assert_eq!(
        composite_text(
            "Identity-H",
            "CIDFontType0",
            "Song",
            "Identity",
            "002204103042"
        ),
        "\u{fffd}\u{fffd}\u{fffd}"
    );
}

#[test]
fn identity_to_unicode_reads_codes_as_utf_16() {
    let doc = page(
        "BT /F1 10 Tf 10 100 Td <00410042> Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &[
            b"<< /Type /Font /Subtype /Type0 /BaseFont /Song /Encoding /Identity-H \
              /DescendantFonts [6 0 R] /ToUnicode /Identity-H >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Song \
              /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
              /FontDescriptor << /Type /FontDescriptor /FontName /Song /Flags 4 \
              /FontBBox [0 -120 1000 880] /ItalicAngle 0 /Ascent 880 /Descent -120 \
              /CapHeight 700 /StemV 80 >> >>"
                .to_vec(),
        ],
    );
    assert_eq!(doc.structured_text(0).unwrap().as_text(), "AB\n\n");
}

// ------------------------------------------------------------------ N8 normalisation

/// Text read for each code point of `range`, through Type3 glyphs named
/// `uniXXXX`, 256 per font.
fn presentation_texts(range: std::ops::RangeInclusive<u32>) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    let all: Vec<u32> = range.collect();
    for chunk in all.chunks(256) {
        let names: Vec<String> = chunk.iter().map(|v| format!("uni{v:04X}")).collect();
        let codes: Vec<(u8, &str)> = (names.iter().enumerate())
            .map(|(i, n)| (i as u8, n.as_str()))
            .collect();
        for (v, text) in chunk.iter().zip(type3_texts(&codes)) {
            out.insert(*v, text);
        }
    }
    out
}

#[test]
fn ligatures_read_as_their_letters() {
    let texts = presentation_texts(0xFB00..=0xFB06);
    assert_eq!(texts[&0xFB00], "ff");
    assert_eq!(texts[&0xFB01], "fi");
    assert_eq!(texts[&0xFB03], "ffi");
}

#[test]
#[ignore = "clean-room: 05§4.3 every presentation form in U+FB00-U+FEFF reads as its full decomposition"]
fn presentation_forms_read_as_their_decompositions() {
    let texts = presentation_texts(0xFB00..=0xFEFF);
    assert_eq!(texts[&0xFB2C], "\u{5e9}\u{5bc}\u{5c1}");
    assert_eq!(texts[&0xFE8D], "\u{627}");
    assert_eq!(texts[&0xFE10], ",");
    assert_eq!(texts[&0xFEFF], "");
    let changed: Vec<u32> = (texts.iter())
        .filter(|(v, text)| **v != 0xFEFF && **text != u(&[**v]))
        .map(|(v, _)| *v)
        .collect();
    assert_eq!(changed.len(), 854, "{changed:X?}");
    // Fully decomposed: nothing read lies in the range itself.
    for v in &changed {
        let text = &texts[v];
        assert!(
            !text.is_empty()
                && text
                    .chars()
                    .all(|c| !(0xFB00..=0xFEFF).contains(&u32::from(c))),
            "{v:X}: {text:?}"
        );
    }
    // Fullwidth forms stay.
    assert_eq!(presentation_texts(0xFF21..=0xFF21)[&0xFF21], "\u{ff21}");
}

#[test]
fn a_ligature_is_found_by_its_letters() {
    let doc = page(
        "BT /F1 10 Tf 10 100 Td (o\\036ce) Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &[
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>".to_vec(),
            stream(
                "",
                cmap("4 beginbfchar <1e> <FB03> <6f> <006F> <63> <0063> <65> <0065> endbfchar")
                    .as_bytes(),
            ),
        ],
    );
    let text = doc.structured_text(0).unwrap();
    assert!(text.as_text().starts_with("office"), "{:?}", text.as_text());
    assert_eq!(text.search("office", MAX_SEARCH_QUADS).hits.len(), 1);
}

#[test]
#[ignore = "clean-room: 05§3.1-3.2 a UTF-16 CMap code of four bytes is one character"]
fn a_utf16_cmap_reads_a_surrogate_pair_as_one_character() {
    assert_eq!(
        composite_text(
            "UniGB-UTF16-H",
            "CIDFontType0",
            "STSong-Light",
            "GB1",
            "65E5D840DC003002"
        ),
        "\u{65e5}\u{20000}\u{3002}"
    );
}
