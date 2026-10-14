// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent PDF glyph policy and ownership/budget review.
use tandem_pdf_engine::{
    document::Document,
    embedded_glyph::{GlyphError, GlyphFontLoader, GlyphLoadLimits},
    object::ObjectId,
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
fn pdf(
    ttf: &[u8],
    flags: u32,
    encoding: &str,
    descriptor_extra: &str,
    stream_extra: &str,
) -> Vec<u8> {
    let mut stream = format!("<< /Length {} {stream_extra} >>\nstream\n", ttf.len()).into_bytes();
    stream.extend_from_slice(ttf);
    stream.extend_from_slice(b"\nendstream");
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_vec(),
        format!("<< /Type /Font /Subtype /TrueType /FontDescriptor 5 0 R {encoding} >>")
            .into_bytes(),
        format!("<< /Type /FontDescriptor /Flags {flags} /FontFile2 6 0 R {descriptor_extra} >>")
            .into_bytes(),
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

fn symbolic() -> Vec<u8> {
    font(cmap(&[(3, 0, subtable(&[(65, 65, -64, None)]))]), 16)
}
#[test]
fn loading_limits_are_cumulative_and_admit_before_work() {
    let ttf = symbolic();
    let bytes = pdf(&ttf, 4, "", "", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    for limits in [
        GlyphLoadLimits {
            max_attempts: 0,
            ..Default::default()
        },
        GlyphLoadLimits {
            max_reference_hops: 0,
            ..Default::default()
        },
        GlyphLoadLimits {
            max_metadata_items: 0,
            ..Default::default()
        },
    ] {
        let mut loader = GlyphFontLoader::new(&doc, limits);
        assert!(matches!(loader.load(id(4)), Err(GlyphError::Limit(_))));
        assert_eq!(loader.usage().encoded_bytes, 0);
    }
    let mut loader = GlyphFontLoader::new(
        &doc,
        GlyphLoadLimits {
            max_attempts: 2,
            max_metadata_items: 1,
            ..Default::default()
        },
    );
    assert!(loader.load(id(4)).is_err());
    assert_eq!(loader.usage().metadata_items, 1);
    assert!(loader.load(id(4)).is_err());
    assert_eq!(loader.usage().metadata_items, 1);
    assert_eq!(loader.usage().attempts, 2);
}
#[test]
fn failed_stream_decode_keeps_reserved_work_and_retry_is_bounded() {
    let bytes = pdf(b"invalid flate", 4, "", "", "/Filter /FlateDecode");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut loader = GlyphFontLoader::new(
        &doc,
        GlyphLoadLimits {
            max_attempts: 2,
            max_encoded_bytes: 100,
            max_decoded_work_bytes: 200,
            ..Default::default()
        },
    );
    assert!(matches!(loader.load(id(4)), Err(GlyphError::Stream(_))));
    assert_eq!(loader.usage().encoded_bytes, 100);
    assert_eq!(loader.usage().decoded_work_bytes, 200);
    assert!(loader.load(id(4)).is_err());
    assert!(loader.load(id(4)).is_err());
    assert_eq!(loader.usage().attempts, 2);
}
#[test]
fn strict_policy_refuses_implicit_encodings_alternate_programs_and_symbolic_overrides() {
    let ttf = symbolic();
    for (flags, encoding, extra) in [
        (32, "", ""),
        (4, "/Encoding /WinAnsiEncoding", ""),
        (4, "", "/FontFile 6 0 R"),
        (4, "", "/FontFile3 6 0 R"),
        (32, "/Encoding /UnknownEncoding", ""),
        (32, "/Encoding << /Differences [65 /A] >>", ""),
    ] {
        let bytes = pdf(&ttf, flags, encoding, extra, "");
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let mut loader = GlyphFontLoader::new(&doc, Default::default());
        assert!(matches!(
            loader.load(id(4)),
            Err(GlyphError::Unsupported(_))
        ));
        assert_eq!(loader.usage().encoded_bytes, 0);
    }
}
#[test]
fn differences_names_charge_before_validation_or_allocation() {
    let ttf = symbolic();
    let bytes = pdf(
        &ttf,
        32,
        "/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /LongName] >>",
        "",
        "",
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut loader = GlyphFontLoader::new(
        &doc,
        GlyphLoadLimits {
            max_name_bytes: 7,
            ..Default::default()
        },
    );
    assert!(matches!(loader.load(id(4)), Err(GlyphError::Limit(_))));
    assert_eq!(loader.usage().encoded_bytes, 0);
    let mut limits = GlyphLoadLimits::default();
    limits.encoding.max_name_bytes = 7;
    let mut loader = GlyphFontLoader::new(&doc, limits);
    assert!(matches!(loader.load(id(4)), Err(GlyphError::Encoding(_))));
    assert_eq!(loader.usage().name_bytes, 8);
    assert!(loader.load(id(4)).is_err());
    assert_eq!(loader.usage().name_bytes, 16);
}
#[test]
fn indirect_differences_and_stream_metadata_are_refused() {
    let ttf = symbolic();
    for encoding in [
        "/Encoding 6 0 R",
        "/Encoding << /BaseEncoding 6 0 R >>",
        "/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 4 0 R] >>",
    ] {
        let bytes = pdf(&ttf, 32, encoding, "", "");
        let doc = Document::open(&bytes, Default::default()).unwrap();
        assert!(
            GlyphFontLoader::new(&doc, Default::default())
                .load(id(4))
                .is_err()
        );
    }
}
#[test]
fn inspection_cli_does_not_publish_partial_output_for_late_cmap_failure() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let ttf = font(
        cmap(&[
            (3, 0, subtable(&[(65, 65, -64, None)])),
            (3, 1, words(&[12, 0])),
        ]),
        16,
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
        .arg("--tt-cmap=all")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&ttf).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
