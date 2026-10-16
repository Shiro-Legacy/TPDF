// SPDX-License-Identifier: AGPL-3.0-or-later
//! The installed CJK font's metrics and coverage as hayro uses them: a Latin
//! character it lacks, drawn from the URW glyph appended at install, keeps
//! its width where the CID font gives one in /W (hayro stretches a fallback
//! glyph to /W by its advance), and a character past the BMP it lacks draws
//! nothing rather than the glyph numbered like its CID. Its own binary: the
//! installed font is process-wide.
use tandem_pdf_engine::view::{RenderOptions, ViewDocument, set_cjk_font};

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

/// Five 100 pt cells: four in a GB1 font without a program, Identity-H with
/// a ToUnicode (1 with a /W width, URW's; 1 without one; U+20000, in neither
/// font; 日, the test font's square), then 1 in Helvetica.
fn page() -> Vec<u8> {
    let content = "BT /C0 100 Tf 0 10 Td <0001> Tj 100 0 Td <0002> Tj \
                   100 0 Td <0003> Tj 100 0 Td <0004> Tj /H0 100 Tf 100 0 Td (1) Tj ET";
    let to_unicode = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                      /CMapName /T def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> \
                      endcodespacerange 4 beginbfchar <0001> <0031> <0002> <0031> \
                      <0003> <D840DC00> <0004> <65E5> endbfchar endcmap \
                      CMapName currentdict /CMap defineresource pop end end";
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 500 120] \
         /Resources << /Font << /C0 5 0 R /H0 9 0 R >> >> /Contents 4 0 R >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type0 /BaseFont /Song /Encoding /Identity-H \
         /DescendantFonts [6 0 R] /ToUnicode 8 0 R >>"
            .into(),
        "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Song /CIDSystemInfo \
         << /Registry (Adobe) /Ordering (GB1) /Supplement 5 >> /FontDescriptor 7 0 R \
         /W [1 [556]] >>"
            .into(),
        "<< /Type /FontDescriptor /FontName (Song) /FontBBox [-200 -200 1200 1200] /Flags 6 \
         /ItalicAngle 0 /Ascent 1000 /Descent -200 /StemV 80 >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{to_unicode}\nendstream",
            to_unicode.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ])
}

/// A TrueType font of two glyphs: .notdef (empty) and a square from 100 to
/// 900 units, mapped from U+65E5.
fn square_font() -> Vec<u8> {
    let be16 = |v: u16| v.to_be_bytes();
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
    maxp[4..6].copy_from_slice(&be16(2));
    let hmtx = [0u8, 0, 0, 0, 0x03, 0xE8, 0, 0];
    let mut glyph = Vec::new();
    for v in [1i16, 100, 100, 900, 900, 3, 0] {
        glyph.extend_from_slice(&v.to_be_bytes());
    }
    glyph.extend_from_slice(&[1, 1, 1, 1]);
    // x deltas, then y deltas, of (100 100) (900 100) (900 900) (100 900).
    for v in [100i16, 800, 0, -800, 100, 0, 800, 0] {
        glyph.extend_from_slice(&v.to_be_bytes());
    }
    let loca: Vec<u8> = [0u32, 0, glyph.len() as u32]
        .iter()
        .flat_map(|v| v.to_be_bytes())
        .collect();
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
        (b"glyf", &glyph),
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
fn cells(view: &ViewDocument) -> [(usize, usize); 5] {
    let image = view.render(0, 1.0, RenderOptions::default()).unwrap();
    std::array::from_fn(|cell| {
        let inked: Vec<usize> = (0..120)
            .flat_map(|y| (100 * cell..100 * cell + 100).map(move |x| (x, (y * 500 + x) * 4)))
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
fn fallback_latin_keeps_its_width_and_characters_past_the_bmp_draw_nothing() {
    let view = ViewDocument::open(page(), "").unwrap();
    set_cjk_font(square_font()).unwrap();
    let [with_width, without, past_bmp, square, helvetica] = cells(&view);
    assert!(helvetica.0 > 100, "1 in Helvetica: {helvetica:?}");
    // Both drew squeezed to 556/1000 (a glyph's outline is drawn once per
    // render) before the appended glyphs kept URW's advances.
    for cell in [with_width, without] {
        assert!(
            cell.1.abs_diff(helvetica.1) <= 1,
            "1 from URW Helvetica as wide as in Helvetica: {cell:?} {helvetica:?}"
        );
    }
    assert_eq!(past_bmp.0, 0, "U+20000 draws nothing");
    assert!(square.0 > 5000, "the square: {square:?}");
}
