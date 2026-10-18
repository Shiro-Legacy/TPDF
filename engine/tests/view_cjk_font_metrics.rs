// SPDX-License-Identifier: AGPL-3.0-or-later
//! The installed CJK font as hayro uses it. A Latin character it lacks,
//! drawn from URW Helvetica, keeps its width where the CID font gives one in
//! /W (hayro stretches a fallback glyph to /W by its advance). A code whose
//! Unicode hayro cannot use (no ToUnicode entry, two characters, U+FFFF, a
//! lone surrogate) draws its CID's character, not the font's
//! glyph numbered like the CID; a character neither font has, and a CID past
//! the collection, draw nothing. Text kept from a clipped run is the same
//! before and after the font is installed. Its own binary: the installed
//! font is process-wide.
use hayro_interpret::hayro_cmap::{BfString, CMap, CMapName, load_embedded};
use tandem_pdf_engine::view::{RenderOptions, ViewDocument, set_cjk_font};

/// The first GB1 CID whose character is `c` (its UCS2 CMap).
fn gb1_cid(c: char) -> u16 {
    let ucs2 = CMap::parse(
        load_embedded(CMapName::AdobeGb1Ucs2).unwrap(),
        load_embedded,
    )
    .unwrap();
    (0..=u16::MAX)
        .find(|&cid| ucs2.lookup_bf_string(u32::from(cid)) == Some(BfString::Char(c)))
        .unwrap()
}

fn classic(objects: &[String]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
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

/// Cells of 100 pt in a GB1 font without a program, its codes mapped to
/// CIDs by an embedded CMap and to Unicode by a ToUnicode: 1 with a /W width
/// (URW's); 1 without one; U+20000 for CID 4000 (in neither font); 日 for
/// CID 4 (the test font's square); then codes for 日's CID without a
/// ToUnicode entry, with 日 and A, with U+FFFF and with a lone surrogate; a
/// CID past the collection; and 1 in Helvetica.
fn page() -> Vec<u8> {
    let ri = gb1_cid('日');
    let content = "BT /C0 100 Tf 0 10 Td <0001> Tj 100 0 Td <0002> Tj 100 0 Td <0FA0> Tj \
                   100 0 Td <0004> Tj 100 0 Td <0010> Tj 100 0 Td <0011> Tj 100 0 Td <0012> Tj \
                   100 0 Td <0013> Tj 100 0 Td <FFF0> Tj /H0 100 Tf 100 0 Td (1) Tj ET";
    let encoding = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T def \
         /CMapType 1 def 1 begincodespacerange <0000> <FFFF> endcodespacerange \
         2 begincidrange <0000> <000F> 0 <0014> <FFFF> 20 endcidrange \
         4 begincidchar <0010> {ri} <0011> {ri} <0012> {ri} <0013> {ri} endcidchar \
         endcmap CMapName currentdict /CMap defineresource pop end end"
    );
    let to_unicode = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                      /CMapName /U def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> \
                      endcodespacerange 7 beginbfchar <0001> <0031> <0002> <0031> \
                      <0FA0> <D840DC00> <0004> <65E5> <0011> <65E50041> <0012> <FFFF> \
                      <0013> <D800> endbfchar endcmap \
                      CMapName currentdict /CMap defineresource pop end end";
    let stream = |data: &str| format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len());
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1000 120] \
         /Resources << /Font << /C0 5 0 R /H0 9 0 R >> >> /Contents 4 0 R >>"
            .into(),
        stream(content),
        "<< /Type /Font /Subtype /Type0 /BaseFont /Song /Encoding 10 0 R \
         /DescendantFonts [6 0 R] /ToUnicode 8 0 R >>"
            .into(),
        "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Song /CIDSystemInfo \
         << /Registry (Adobe) /Ordering (GB1) /Supplement 5 >> /FontDescriptor 7 0 R \
         /W [1 [556]] >>"
            .into(),
        "<< /Type /FontDescriptor /FontName (Song) /FontBBox [0 -120 1000 880] /Flags 4 \
         /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>"
            .into(),
        stream(to_unicode),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        stream(&encoding),
    ])
}

/// 日本語文 at 40 pt from x = 10 in a GB1 font without a program, clipped
/// to x below 70 (日 inside, 本 across, 語 and 文 outside).
fn clipped() -> Vec<u8> {
    let content = "q 0 0 70 120 re W n BT /C0 40 Tf 10 40 Td <65E5672C8A9E6587> Tj ET Q";
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 120] \
         /Resources << /Font << /C0 5 0 R >> >> /Contents 4 0 R >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type0 /BaseFont /Song /Encoding /UniGB-UTF16-H \
         /DescendantFonts [6 0 R] >>"
            .into(),
        "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Song /CIDSystemInfo \
         << /Registry (Adobe) /Ordering (GB1) /Supplement 5 >> /FontDescriptor 7 0 R >>"
            .into(),
        "<< /Type /FontDescriptor /FontName (Song) /FontBBox [0 -120 1000 880] /Flags 4 \
         /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 700 /StemV 80 >>"
            .into(),
    ])
}

/// A TrueType font: .notdef (empty), a square from 100 to 900 units mapped
/// from U+65E5, and a bar, mapped from nothing, numbered like 日's GB1 CID
/// and like CID 4000 (the glyph hayro takes when it finds no Unicode).
fn square_font() -> Vec<u8> {
    let be16 = |v: u16| v.to_be_bytes();
    let wrong = [gb1_cid('日'), 4000];
    let count = wrong.iter().max().unwrap() + 1;
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&be16(1000));
    head[50..52].copy_from_slice(&be16(1));
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&be16(2));
    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&be16(count));
    let mut hmtx = vec![0u8, 0, 0, 0, 0x03, 0xE8, 0, 0];
    hmtx.resize(8 + 2 * usize::from(count - 2), 0);
    // One contour from (x0, y0) to (x1, y1): x deltas, then y deltas.
    let rectangle = |x0: i16, y0: i16, x1: i16, y1: i16| {
        let mut glyph = Vec::new();
        for v in [1, x0, y0, x1, y1, 3, 0] {
            glyph.extend_from_slice(&v.to_be_bytes());
        }
        glyph.extend_from_slice(&[1, 1, 1, 1]);
        for v in [x0, x1 - x0, 0, x0 - x1, y0, 0, y1 - y0, 0] {
            glyph.extend_from_slice(&v.to_be_bytes());
        }
        glyph
    };
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    for gid in 0..count {
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        if gid == 1 {
            glyf.extend(rectangle(100, 100, 900, 900));
        } else if wrong.contains(&gid) {
            glyf.extend(rectangle(100, 0, 900, 150));
        }
    }
    loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
    // One format 4 subtable: U+65E5 to glyph 1, then the 0xFFFF segment.
    let mut cmap = Vec::new();
    for v in [0u16, 1, 3, 1] {
        cmap.extend_from_slice(&be16(v));
    }
    cmap.extend_from_slice(&12u32.to_be_bytes());
    let delta = 1u16.wrapping_sub(0x65E5);
    for v in [
        4u16, 32, 0, 4, 4, 1, 0, 0x65E5, 0xFFFF, 0, 0x65E5, 0xFFFF, delta, 1, 0, 0,
    ] {
        cmap.extend_from_slice(&be16(v));
    }
    let tables: [(&[u8; 4], &[u8]); 7] = [
        (b"cmap", &cmap),
        (b"glyf", &glyf),
        (b"head", &head),
        (b"hhea", &hhea),
        (b"hmtx", &hmtx),
        (b"loca", &loca),
        (b"maxp", &maxp),
    ];
    let mut out = 0x0001_0000u32.to_be_bytes().to_vec();
    for v in [7u16, 64, 2, 48] {
        out.extend_from_slice(&be16(v));
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

/// Each 100 pt cell's non-white pixels and the width they span.
fn cells(view: &ViewDocument) -> [(usize, usize); 10] {
    let image = view.render(0, 1.0, RenderOptions::default()).unwrap();
    std::array::from_fn(|cell| {
        let inked: Vec<usize> = (0..120)
            .flat_map(|y| (100 * cell..100 * cell + 100).map(move |x| (x, (y * 1000 + x) * 4)))
            .filter(|&(_, at)| image.pixels[at..at + 3] != [255, 255, 255])
            .map(|(x, _)| x)
            .collect();
        let span = match (inked.iter().min(), inked.iter().max()) {
            (Some(min), Some(max)) => max - min + 1,
            _ => 0,
        };
        (inked.len(), span)
    })
}

#[test]
fn every_cid_draws_its_collections_character_or_nothing() {
    // Text of a clipped run before the font: kept by em boxes, as after it.
    let clip = ViewDocument::open(clipped(), "").unwrap();
    let text = |view: &ViewDocument| view.structured_text(0).unwrap().as_text();
    let before = text(&clip);
    assert_eq!(before.trim(), "日本");
    let view = ViewDocument::open(page(), "").unwrap();
    set_cjk_font(square_font()).unwrap();
    let [
        with_width,
        without,
        past_bmp,
        square,
        no_entry,
        two,
        nonchar,
        surrogate,
        past_collection,
        helvetica,
    ] = cells(&view);
    assert!(helvetica.0 > 100, "1 in Helvetica: {helvetica:?}");
    // Both drew squeezed to 556/1000 (a glyph's outline is drawn once per
    // render) before URW's glyphs kept their advances.
    for cell in [with_width, without] {
        assert!(
            cell.1.abs_diff(helvetica.1) <= 1,
            "1 from URW Helvetica as wide as in Helvetica: {cell:?} {helvetica:?}"
        );
    }
    assert_eq!(
        past_bmp.0, 0,
        "U+20000 for a CID neither font has draws nothing"
    );
    assert!(square.0 > 5000, "the square: {square:?}");
    // hayro finds no Unicode for these and takes the glyph numbered like the
    // CID: the bar before the font was rebuilt by CID.
    for cell in [no_entry, two, nonchar, surrogate] {
        assert_eq!(cell, square, "日 by its CID");
    }
    assert_eq!(past_collection.0, 0, "a CID past the collection");
    view.render(0, 1.0, RenderOptions::default()).unwrap();
    assert_eq!(text(&clip), before, "the same text with the font");
}
