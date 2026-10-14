// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::truetype_cmap::*;
use tandem_pdf_engine::truetype_outline::{FontLimits, TrueTypeFont};
fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
}
fn format4(segments: &[(u16, u16, u16, u16)], glyphs: &[u16]) -> Vec<u8> {
    let n = segments.len();
    let mut bytes = vec![0; 16 + 8 * n + 2 * glyphs.len()];
    let length = bytes.len() as u16;
    put16(&mut bytes, 0, 4);
    put16(&mut bytes, 2, length);
    put16(&mut bytes, 6, (n * 2) as u16);
    // Accelerator hints intentionally zero: reader derives search itself.
    for (i, &(start, end, delta, offset)) in segments.iter().enumerate() {
        put16(&mut bytes, 14 + 2 * i, end);
        put16(&mut bytes, 16 + 2 * n + 2 * i, start);
        put16(&mut bytes, 16 + 4 * n + 2 * i, delta);
        put16(&mut bytes, 16 + 6 * n + 2 * i, offset);
    }
    for (i, &gid) in glyphs.iter().enumerate() {
        put16(&mut bytes, 16 + 8 * n + 2 * i, gid);
    }
    bytes
}
fn cmap(subtables: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![0; 4 + 8 * subtables.len()];
    put16(&mut bytes, 2, subtables.len() as u16);
    for (i, (platform, encoding, sub)) in subtables.iter().enumerate() {
        let at = 4 + 8 * i;
        let offset = bytes.len() as u32;
        put16(&mut bytes, at, *platform);
        put16(&mut bytes, at + 2, *encoding);
        put32(&mut bytes, at + 4, offset);
        bytes.extend_from_slice(sub);
    }
    bytes
}
fn font(cmap: Vec<u8>, glyph_count: u16) -> Vec<u8> {
    let mut head = vec![0; 54];
    put32(&mut head, 0, 0x10000);
    put32(&mut head, 12, 0x5f0f3cf5);
    put16(&mut head, 18, 1000);
    put16(&mut head, 50, 1);
    let mut maxp = vec![0; 32];
    put32(&mut maxp, 0, 0x10000);
    put16(&mut maxp, 4, glyph_count);
    let tables = [
        (*b"head", head),
        (*b"maxp", maxp),
        (*b"loca", vec![0; 4 * (usize::from(glyph_count) + 1)]),
        (*b"glyf", vec![]),
        (*b"cmap", cmap),
    ];
    let mut bytes = vec![0; 12 + 16 * tables.len()];
    put32(&mut bytes, 0, 0x10000);
    put16(&mut bytes, 4, tables.len() as u16);
    for (i, (tag, value)) in tables.iter().enumerate() {
        let offset = bytes.len() as u32;
        let at = 12 + 16 * i;
        bytes[at..at + 4].copy_from_slice(tag);
        put32(&mut bytes, at + 8, offset);
        put32(&mut bytes, at + 12, value.len() as u32);
        bytes.extend_from_slice(value);
    }
    bytes
}
fn direct() -> Vec<u8> {
    format4(
        &[(65, 67, 1u16.wrapping_sub(65), 0), (65535, 65535, 1, 0)],
        &[],
    )
}
fn gid(value: CmapLookup) -> u16 {
    match value {
        CmapLookup::Missing => 0,
        CmapLookup::Glyph(id) => id.get(),
    }
}
#[test]
fn exhaustive_direct_delta_and_provenance() {
    let bytes = font(cmap(&[(3, 1, direct())]), 5);
    let font = TrueTypeFont::open(&bytes, FontLimits::default()).unwrap();
    let mut session = TrueTypeCmapSession::new(&font, CmapLimits::default()).unwrap();
    let selection = session.select(0).unwrap();
    assert_eq!(selection.record().platform_id(), 3);
    assert_eq!(selection.segment_count(), 2);
    assert_eq!(&bytes[selection.subtable_range()][..2], &[0, 4]);
    for code in 0..=u16::MAX {
        assert_eq!(
            gid(session.lookup(code).unwrap()),
            if (65..=67).contains(&code) {
                code - 64
            } else {
                0
            }
        );
    }
    assert_eq!(session.usage().validated_codes, 4);
    assert_eq!(session.usage().queries, 65536);
}
#[test]
fn glyph_array_uses_own_offset_word_and_zero_skips_delta() {
    let sub = format4(&[(65, 67, u16::MAX, 4), (65535, 65535, 1, 0)], &[2, 0, 4]);
    let bytes = font(cmap(&[(3, 1, sub)]), 5);
    let font = TrueTypeFont::open(&bytes, FontLimits::default()).unwrap();
    let mut s = TrueTypeCmapSession::new(&font, CmapLimits::default()).unwrap();
    s.select(0).unwrap();
    assert_eq!(
        [
            gid(s.lookup(65).unwrap()),
            gid(s.lookup(66).unwrap()),
            gid(s.lookup(67).unwrap())
        ],
        [1, 0, 3]
    );
}
#[test]
fn explicit_record_choice_preserves_duplicates_and_does_not_fallback() {
    let bytes = font(
        cmap(&[(3, 0, direct()), (3, 0, vec![0, 12]), (1, 0, direct())]),
        5,
    );
    let font = TrueTypeFont::open(&bytes, FontLimits::default()).unwrap();
    let mut s = TrueTypeCmapSession::new(&font, CmapLimits::default()).unwrap();
    assert_eq!(
        s.records().map(|r| r.format()).collect::<Vec<_>>(),
        [4, 12, 4]
    );
    s.select(0).unwrap();
    assert_eq!(
        s.select(1).unwrap_err().kind,
        CmapErrorKind::UnsupportedFormat(12)
    );
    assert!(s.selection().is_none());
    assert_eq!(s.lookup(65).unwrap_err().kind, CmapErrorKind::NoSelection);
    assert_eq!(s.usage().queries, 1);
    s.select(2).unwrap();
    assert_eq!(gid(s.lookup(65).unwrap()), 1);
}
#[test]
fn exact_validation_query_and_failed_selection_limits_are_cumulative() {
    let bytes = font(cmap(&[(3, 1, direct())]), 5);
    let font = TrueTypeFont::open(&bytes, FontLimits::default()).unwrap();
    let mut s = TrueTypeCmapSession::new(
        &font,
        CmapLimits {
            max_validation_codes: 4,
            max_queries: 1,
            ..CmapLimits::default()
        },
    )
    .unwrap();
    s.select(0).unwrap();
    s.lookup(65).unwrap();
    assert!(s.lookup(65).is_err());
    assert_eq!(s.usage().queries, 1);
    assert!(s.select(0).is_err());
    assert!(s.selection().is_none());
    assert_eq!(s.usage().validated_codes, 4);
    assert_eq!(s.usage().selection_attempts, 2);
}
#[test]
fn malformed_entire_mapping_refuses_before_first_query() {
    let mut cases = vec![
        format4(
            &[
                (65, 67, 1u16.wrapping_sub(65), 0),
                (67, 68, 0, 0),
                (65535, 65535, 1, 0),
            ],
            &[],
        ),
        format4(&[(65, 67, 0, 0), (65535, 65535, 1, 0)], &[]), // out-of-range GIDs
        format4(&[(65, 67, 0, 2), (65535, 65535, 1, 0)], &[1, 2, 3]), // points into metadata
        format4(&[(65, 67, 0, 5), (65535, 65535, 1, 0)], &[1, 2, 3]), // odd range offset
        format4(&[(65, 67, 0, 4), (65535, 65535, 1, 0)], &[1, 2]), // last range member truncated
        format4(
            &[(65, 67, 1u16.wrapping_sub(65), 0), (65535, 65535, 2, 0)],
            &[],
        ), // nonmissing sentinel
    ];
    let mut reserved = direct();
    put16(&mut reserved, 18, 1);
    cases.push(reserved);
    for sub in cases {
        let bytes = font(cmap(&[(3, 1, sub)]), 5);
        let font = TrueTypeFont::open(&bytes, FontLimits::default()).unwrap();
        let mut s = TrueTypeCmapSession::new(&font, CmapLimits::default()).unwrap();
        assert!(s.select(0).is_err());
        assert!(s.selection().is_none());
        assert_eq!(s.usage().queries, 0);
    }
}
#[test]
fn modulo_delta_wrap_and_shared_subtable_offsets_are_supported() {
    let sub = format4(&[(65534, 65534, 3, 0), (65535, 65535, 1, 0)], &[]);
    let mut table = cmap(&[(1, 0, sub.clone()), (3, 0, sub)]);
    put32(&mut table, 16, 20);
    let bytes = font(table, 5);
    let font = TrueTypeFont::open(&bytes, FontLimits::default()).unwrap();
    let mut s = TrueTypeCmapSession::new(&font, CmapLimits::default()).unwrap();
    for i in 0..2 {
        s.select(i).unwrap();
        assert_eq!(gid(s.lookup(65534).unwrap()), 1);
        assert_eq!(gid(s.lookup(65535).unwrap()), 0);
    }
}
#[test]
fn all_subtable_truncations_refuse_without_panicking() {
    let sub = direct();
    for end in 0..sub.len() {
        let bytes = font(cmap(&[(3, 1, sub[..end].to_vec())]), 5);
        let font = TrueTypeFont::open(&bytes, FontLimits::default()).unwrap();
        if let Ok(mut s) = TrueTypeCmapSession::new(&font, CmapLimits::default()) {
            assert!(s.select(0).is_err());
        }
    }
}
#[test]
fn nonzero_language_is_explicit_unsupported() {
    let mut sub = direct();
    put16(&mut sub, 4, 1);
    let bytes = font(cmap(&[(1, 0, sub)]), 5);
    let font = TrueTypeFont::open(&bytes, FontLimits::default()).unwrap();
    let mut s = TrueTypeCmapSession::new(&font, CmapLimits::default()).unwrap();
    assert_eq!(
        s.select(0).unwrap_err().kind,
        CmapErrorKind::UnsupportedLanguage(1)
    );
}
