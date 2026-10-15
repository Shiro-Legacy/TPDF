// SPDX-License-Identifier: AGPL-3.0-or-later
//! CJK CID fonts without a program (MuPDF's `pdf_add_cjk_font` shape, as
//! the forms group writes them): nothing drawn until the host installs a
//! CJK font, their text extracted all the same; then drawn from it, with
//! what it lacks drawn from URW Helvetica or not at all, never as another
//! glyph. Its own binary: the installed font is process-wide.
use tandem_pdf_engine::view::{RenderOptions, ViewDocument, cjk_font_wanted, set_cjk_font};

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

/// Three 100 pt cells in a Song font: 日 (the test font's square), 1 (not
/// in it: URW Helvetica) and 一 (in neither: nothing).
fn page() -> Vec<u8> {
    let content = "BT /C0 100 Tf 0 10 Td <65E5> Tj 100 0 Td <0031> Tj 100 0 Td <4E00> Tj ET";
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 120] \
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
        "<< /Type /FontDescriptor /FontName (Song) /FontBBox [-200 -200 1200 1200] /Flags 6 \
         /ItalicAngle 0 /Ascent 1000 /Descent -200 /StemV 80 >>"
            .into(),
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

/// Non-white pixels in each 100 pt cell.
fn cells(view: &ViewDocument) -> [usize; 3] {
    let image = view.render(0, 1.0, RenderOptions::default()).unwrap();
    std::array::from_fn(|cell| {
        (0..120)
            .flat_map(|y| (100 * cell..100 * cell + 100).map(move |x| (y * 300 + x) * 4))
            .filter(|&at| image.pixels[at..at + 3] != [255, 255, 255])
            .count()
    })
}

#[test]
fn cjk_text_draws_once_a_font_is_installed_and_never_as_a_wrong_glyph() {
    let view = ViewDocument::open(page(), "").unwrap();
    let text = view.structured_text(0).unwrap().as_text();
    assert_eq!(text.split_whitespace().collect::<String>(), "日1一");
    assert!(!cjk_font_wanted(), "text extraction asks for no font");
    assert_eq!(cells(&view), [0, 0, 0], "nothing drawn without the font");
    assert!(cjk_font_wanted());
    assert!(set_cjk_font(b"not a font".to_vec()).is_err());
    set_cjk_font(square_font()).unwrap();
    assert!(!cjk_font_wanted());
    let [square, one, missing] = cells(&view);
    assert!(square > 5000, "the square: {square}");
    assert!(one > 100, "1 from URW Helvetica: {one}");
    assert_eq!(missing, 0, "a character neither font has draws nothing");
    assert!(!cjk_font_wanted());
}
