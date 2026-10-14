// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent PDF glyph policy and ownership/budget review.
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    embedded_glyph::{GlyphFontLoader, GlyphLoadLimits, GlyphRoute},
    object::ObjectId,
    truetype_outline::{FontLimits, OutlineLimits},
};
fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
}
fn words(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}
fn subtable(segments: &[(u16, u16, i16, Option<Vec<u16>>)]) -> Vec<u8> {
    let mut segments = segments.to_vec();
    segments.push((65535, 65535, 1, None));
    let n = segments.len();
    let mut offsets = Vec::new();
    let mut glyphs = Vec::new();
    for (i, (start, end, _, array)) in segments.iter().enumerate() {
        if let Some(array) = array {
            assert_eq!(array.len(), usize::from(end - start) + 1);
            offsets.push((2 * (n - i + glyphs.len())) as u16);
            glyphs.extend_from_slice(array);
        } else {
            offsets.push(0);
        }
    }
    let search = 1usize << (usize::BITS - 1 - n.leading_zeros());
    let mut values = vec![
        4,
        (16 + 8 * n + 2 * glyphs.len()) as u16,
        0,
        (2 * n) as u16,
        (2 * search) as u16,
        search.trailing_zeros() as u16,
        (2 * (n - search)) as u16,
    ];
    values.extend(segments.iter().map(|s| s.1));
    values.push(0);
    values.extend(segments.iter().map(|s| s.0));
    values.extend(segments.iter().map(|s| s.2 as u16));
    values.extend(offsets);
    values.extend(glyphs);
    words(&values)
}
fn cmap(subtables: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut result = words(&[0, subtables.len() as u16]);
    let mut offset = 4 + 8 * subtables.len();
    for (platform, encoding, bytes) in subtables {
        result.extend(words(&[*platform, *encoding]));
        result.extend((offset as u32).to_be_bytes());
        offset += bytes.len();
    }
    for (_, _, bytes) in subtables {
        result.extend(bytes);
    }
    result
}
fn font(cmap: Vec<u8>, glyphs: u16) -> Vec<u8> {
    let mut head = vec![0; 54];
    head[0..4].copy_from_slice(&65536u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5f0f3cf5u32.to_be_bytes());
    put16(&mut head, 18, 1000);
    let mut maxp = vec![0; 32];
    maxp[0..4].copy_from_slice(&65536u32.to_be_bytes());
    put16(&mut maxp, 4, glyphs);
    let tables: [(&[u8; 4], Vec<u8>); 5] = [
        (b"cmap", cmap),
        (b"head", head),
        (b"maxp", maxp),
        (b"loca", vec![0; 2 * (usize::from(glyphs) + 1)]),
        (b"glyf", vec![]),
    ];
    let mut bytes = vec![0; 12 + 16 * tables.len()];
    bytes[0..4].copy_from_slice(&65536u32.to_be_bytes());
    put16(&mut bytes, 4, tables.len() as u16);
    for (index, (tag, data)) in tables.into_iter().enumerate() {
        let at = 12 + 16 * index;
        bytes[at..at + 4].copy_from_slice(tag);
        let offset = bytes.len() as u32;
        bytes[at + 8..at + 12].copy_from_slice(&offset.to_be_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
        bytes.extend(data);
    }
    bytes
}
fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}
fn pdf(ttf: &[u8], flags: u32, encoding: &str, extra: &str) -> Vec<u8> {
    let mut stream = format!("<< /Length {} >>\nstream\n", ttf.len()).into_bytes();
    stream.extend_from_slice(ttf);
    stream.extend_from_slice(b"\nendstream");
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_vec(),
        format!("<< /Type /Font /Subtype /TrueType /FontDescriptor 5 0 R {encoding} {extra} >>")
            .into_bytes(),
        format!("<< /Type /FontDescriptor /Flags {flags} /FontFile2 6 0 R >>").into_bytes(),
        stream,
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
#[test]
fn symbolic_raw_byte_precedes_private_use_and_tounicode_is_never_read() {
    let ttf = font(
        cmap(&[(
            3,
            0,
            subtable(&[(65, 65, -64, None), (0xf041, 0xf042, 0x0fc1, None)]),
        )]),
        16,
    );
    let loaded = {
        let bytes = pdf(&ttf, 4, "", "/ToUnicode 999 0 R /Widths /Malformed");
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        GlyphFontLoader::new(&doc, Default::default())
            .load(id(4))
            .unwrap()
    };
    assert_eq!(loaded.font_id(), id(4));
    assert_eq!(loaded.program_id(), id(6));
    let map = loaded.map(Default::default(), Default::default()).unwrap();
    assert_eq!(map.choice(65).glyph_id().unwrap().get(), 1);
    assert_eq!(map.choice(65).route(), GlyphRoute::RawByte);
    assert_eq!(map.choice(66).glyph_id().unwrap().get(), 3);
    assert_eq!(map.choice(66).lookup_code(), Some(0xf042));
    assert_eq!(map.choice(66).route(), GlyphRoute::PrivateUse);
    assert!(map.choice(67).glyph_id().is_none());
}
#[test]
fn nonsymbolic_flag_wins_and_encoding_override_is_independent_of_raw_seed() {
    let ttf = font(
        cmap(&[
            (3, 0, subtable(&[(65, 66, -60, None)])),
            (3, 1, subtable(&[(65, 66, -64, None)])),
        ]),
        16,
    );
    let bytes = pdf(
        &ttf,
        36,
        "/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /B] >>",
        "",
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let loaded = GlyphFontLoader::new(&doc, Default::default())
        .load(id(4))
        .unwrap();
    let map = loaded.map(Default::default(), Default::default()).unwrap();
    assert_eq!(map.selection().record().encoding_id(), 1);
    assert_eq!(map.choice(65).glyph_id().unwrap().get(), 2);
    assert_eq!(map.choice(65).lookup_code(), Some(66));
    assert_eq!(map.choice(65).route(), GlyphRoute::EncodingUnicode);
}
#[test]
fn macroman_duplicate_space_name_uses_first_reverse_encoding_code() {
    let ttf = font(
        cmap(&[(
            1,
            0,
            subtable(&[(32, 32, -31, None), (202, 202, -200, None)]),
        )]),
        16,
    );
    let bytes = pdf(&ttf, 32, "/Encoding /MacRomanEncoding", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let loaded = GlyphFontLoader::new(&doc, Default::default())
        .load(id(4))
        .unwrap();
    let map = loaded.map(Default::default(), Default::default()).unwrap();
    assert_eq!(map.choice(202).source(), 202);
    assert_eq!(map.choice(202).glyph_id().unwrap().get(), 1);
    assert_eq!(map.choice(202).lookup_code(), Some(32));
    assert_eq!(map.choice(202).route(), GlyphRoute::EncodingMacRoman);
}
#[test]
fn ambiguous_selector_and_unknown_name_fallback_never_publish_guessed_gid() {
    let table = subtable(&[(65, 67, -64, None)]);
    let ttf = font(cmap(&[(3, 1, table.clone())]), 16);
    let bytes = pdf(
        &ttf,
        32,
        "/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /UnknownReviewGlyph 66 /.notdef 67 /A] >>",
        "",
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let loaded = GlyphFontLoader::new(&doc, Default::default())
        .load(id(4))
        .unwrap();
    let map = loaded.map(Default::default(), Default::default()).unwrap();
    assert_eq!(map.choice(65).route(), GlyphRoute::UnsupportedNameFallback);
    assert!(map.choice(65).glyph_id().is_none());
    assert_eq!(map.choice(66).glyph_id().unwrap().get(), 2);
    assert_eq!(map.choice(67).glyph_id().unwrap().get(), 1);
    let ttf = font(cmap(&[(3, 1, table.clone()), (3, 1, table)]), 16);
    let bytes = pdf(&ttf, 32, "/Encoding /WinAnsiEncoding", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let loaded = GlyphFontLoader::new(&doc, Default::default())
        .load(id(4))
        .unwrap();
    assert!(loaded.map(Default::default(), Default::default()).is_err());
}
#[test]
fn repeated_owned_font_loads_consume_cumulative_work_even_after_results_drop() {
    let ttf = font(cmap(&[(3, 0, subtable(&[(65, 65, -64, None)]))]), 16);
    let bytes = pdf(&ttf, 4, "", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut loader = GlyphFontLoader::new(
        &doc,
        GlyphLoadLimits {
            max_attempts: 2,
            max_encoded_bytes: 2 * ttf.len(),
            max_decoded_work_bytes: 2 * ttf.len(),
            ..Default::default()
        },
    );
    for attempt in 1..=2 {
        drop(loader.load(id(4)).unwrap());
        assert_eq!(loader.usage().attempts, attempt);
        assert_eq!(loader.usage().encoded_bytes, attempt * ttf.len());
        assert_eq!(loader.usage().decoded_work_bytes, attempt * ttf.len());
    }
    assert!(loader.load(id(4)).is_err());
    assert_eq!(loader.usage().attempts, 2);
}
#[test]
fn missing_outline_attempts_are_bounded_and_empty_nonzero_outline_is_success() {
    let ttf = font(cmap(&[(3, 0, subtable(&[(65, 65, -64, None)]))]), 16);
    let bytes = pdf(&ttf, 4, "", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let loaded = GlyphFontLoader::new(&doc, Default::default())
        .load(id(4))
        .unwrap();
    let outlines = loaded
        .outlines(FontLimits::default(), Default::default())
        .unwrap();
    let mut session = outlines.session(OutlineLimits {
        max_decode_attempts: 2,
        ..Default::default()
    });
    assert!(session.decode(0).is_err());
    assert!(session.decode(0).is_err());
    assert!(
        session.decode(65).is_err(),
        "missing source-code retries must consume admission"
    );
    let mut fresh = outlines.session(Default::default());
    let outline = fresh.decode(65).unwrap();
    assert_eq!(outline.id().get(), 1);
    assert!(outline.commands().is_empty());
}

#[test]
fn contradictory_font_and_descriptor_type_names_are_refused() {
    let ttf = font(cmap(&[(3, 0, subtable(&[(65, 65, -64, None)]))]), 16);
    let original = pdf(&ttf, 4, "", "");
    for (from, to) in [
        (
            b"/Type /Font /Subtype".as_slice(),
            b"/Type /Page /Subtype".as_slice(),
        ),
        (
            b"/Type /FontDescriptor".as_slice(),
            b"/Type /NotADescriptor".as_slice(),
        ),
    ] {
        assert_eq!(from.len(), to.len());
        let mut bytes = original.clone();
        let at = bytes.windows(from.len()).position(|v| v == from).unwrap();
        bytes[at..at + from.len()].copy_from_slice(to);
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let mut loader = GlyphFontLoader::new(&doc, Default::default());
        assert!(loader.load(id(4)).is_err());
        assert_eq!(loader.usage().decoded_work_bytes, 0);
    }
}

#[test]
fn failed_stream_admission_retains_conservative_decode_partition_reservations() {
    let ttf = font(cmap(&[(3, 0, subtable(&[(65, 65, -64, None)]))]), 16);
    let bytes = pdf(&ttf, 4, "", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut limits = GlyphLoadLimits {
        max_encoded_bytes: 2,
        max_decoded_work_bytes: 14,
        ..Default::default()
    };
    limits.stream.max_encoded_bytes = 1;
    limits.stream.max_total_decoded_bytes = 7;
    let mut loader = GlyphFontLoader::new(&doc, limits);
    for attempt in 1..=2 {
        assert!(loader.load(id(4)).is_err());
        assert_eq!(loader.usage().attempts, attempt);
        assert_eq!(loader.usage().encoded_bytes, attempt);
        assert_eq!(loader.usage().decoded_work_bytes, 7 * attempt);
    }
    assert!(loader.load(id(4)).is_err());
    assert_eq!(loader.usage().encoded_bytes, 2);
    assert_eq!(loader.usage().decoded_work_bytes, 14);
}
