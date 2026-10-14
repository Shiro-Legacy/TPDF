// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent format4 address arithmetic, missing-GID, and cumulative-budget tests.
use tandem_pdf_engine::truetype_cmap::{CmapLimits, CmapLookup, TrueTypeCmapSession};
use tandem_pdf_engine::truetype_outline::{FontLimits, TrueTypeFont};

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
fn open(bytes: &[u8]) -> TrueTypeFont<'_> {
    TrueTypeFont::open(bytes, FontLimits::default()).unwrap()
}
fn gid(value: CmapLookup) -> u16 {
    match value {
        CmapLookup::Missing => 0,
        CmapLookup::Glyph(gid) => gid.get(),
    }
}
fn fixture(table: Vec<u8>, glyphs: u16) -> Vec<u8> {
    font(cmap(&[(3, 1, table)]), glyphs)
}

#[test]
fn raw_zero_bypasses_delta_nonzero_wraps_and_offsets_are_word_relative() {
    let table = subtable(&[
        (65, 68, -5, Some(vec![0, 5, 6, 7])),
        (80, 81, 5, Some(vec![65533, 65534])),
        (65530, 65534, 10, None),
    ]);
    let bytes = fixture(table, 16);
    let original = bytes.clone();
    let font = open(&bytes);
    let mut session = TrueTypeCmapSession::new(&font, Default::default()).unwrap();
    session.select(0).unwrap();
    for (code, expected) in [
        (64, 0),
        (65, 0),
        (66, 0),
        (67, 1),
        (68, 2),
        (69, 0),
        (80, 2),
        (81, 3),
        (82, 0),
        (65530, 4),
        (65534, 8),
        (65535, 0),
    ] {
        assert_eq!(gid(session.lookup(code).unwrap()), expected, "code={code}");
    }
    assert_eq!(bytes, original);
}

#[test]
fn entire_segment_is_validated_even_if_only_first_code_would_be_requested() {
    for table in [
        subtable(&[(65, 67, 0, Some(vec![1, 2, 16]))]),
        subtable(&[(65, 67, -51, None)]),
    ] {
        let bytes = fixture(table, 16);
        let font = open(&bytes);
        let mut session = TrueTypeCmapSession::new(&font, Default::default()).unwrap();
        assert!(session.select(0).is_err());
        assert!(session.selection().is_none());
        assert!(session.lookup(65).is_err());
    }
}

#[test]
fn array_pointer_cannot_read_structural_words_odd_bytes_or_beyond_declared_length() {
    let valid = subtable(&[(65, 67, 0, Some(vec![1, 2, 3]))]);
    // Two segments => first idRangeOffset at28, glyphIdArray at32.
    for (at, value) in [(28, 2), (28, 3), (28, 6), (2, 36)] {
        let mut broken = valid.clone();
        put16(&mut broken, at, value);
        let bytes = fixture(broken, 16);
        let font = open(&bytes);
        let mut session = TrueTypeCmapSession::new(&font, Default::default()).unwrap();
        assert!(session.select(0).is_err(), "at={at}, value={value}");
    }
}

#[test]
fn canonical_sentinel_sorted_disjoint_segments_and_padding_are_strict() {
    let valid = subtable(&[(10, 12, -9, None), (20, 22, -15, None)]);
    // n3: ends14,16,18; pad20; starts22,24,26; delta28,30,32; offsets34,36,38.
    for (at, value) in [
        (6, 0),
        (6, 5),
        (14, 9),
        (16, 11),
        (24, 12),
        (26, 65534),
        (18, 65534),
        (32, 0),
        (38, 65535),
        (20, 1),
        (4, 1),
    ] {
        let mut broken = valid.clone();
        put16(&mut broken, at, value);
        let bytes = fixture(broken, 32);
        let font = open(&bytes);
        let mut session = TrueTypeCmapSession::new(&font, Default::default()).unwrap();
        assert!(session.select(0).is_err(), "at={at},value={value}");
    }
}

#[test]
fn untrusted_search_accelerators_do_not_change_mapping() {
    let mut table = subtable(&[(10, 12, -9, None)]);
    for at in [8, 10, 12] {
        put16(&mut table, at, 65535);
    }
    let bytes = fixture(table, 16);
    let font = open(&bytes);
    let mut session = TrueTypeCmapSession::new(&font, Default::default()).unwrap();
    session.select(0).unwrap();
    assert_eq!(gid(session.lookup(11).unwrap()), 2);
}

#[test]
fn record_order_aliases_and_unused_unsupported_formats_remain_explicit() {
    let valid = subtable(&[(65, 65, -64, None)]);
    let mut table = cmap(&[(3, 1, valid.clone()), (1, 0, valid), (3, 9, words(&[42]))]);
    // Alias second encoding record to exact first subtable bytes.
    let offset = table[8..12].to_vec();
    table[16..20].copy_from_slice(&offset);
    let bytes = font(table, 16);
    let font = open(&bytes);
    let mut session = TrueTypeCmapSession::new(&font, Default::default()).unwrap();
    assert_eq!(session.records().len(), 3);
    session.select(1).unwrap();
    assert_eq!(gid(session.lookup(65).unwrap()), 1);
    assert!(session.select(2).is_err());
    assert!(session.selection().is_none());
    assert!(session.lookup(65).is_err());
    session.select(0).unwrap();
    assert_eq!(gid(session.lookup(65).unwrap()), 1);
    assert!(session.select(usize::MAX).is_err());
    assert!(session.selection().is_none());
}

#[test]
fn missing_selection_failed_selection_and_exhausted_queries_charge_attempts() {
    let bytes = fixture(subtable(&[(65, 65, -64, None)]), 16);
    let font = open(&bytes);
    let mut session = TrueTypeCmapSession::new(
        &font,
        CmapLimits {
            max_queries: 3,
            max_selection_attempts: 2,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(session.lookup(65).is_err());
    assert_eq!(session.usage().queries, 1);
    session.select(0).unwrap();
    assert_eq!(gid(session.lookup(0).unwrap()), 0);
    assert_eq!(gid(session.lookup(65).unwrap()), 1);
    assert!(session.lookup(65).is_err());
    assert_eq!(session.usage().queries, 3);
    assert!(session.select(99).is_err());
    assert_eq!(session.usage().selection_attempts, 2);
    assert!(session.selection().is_none());
    assert!(session.select(0).is_err());
    assert_eq!(session.usage().selection_attempts, 2);
}

#[test]
fn selection_segments_validation_and_work_have_exact_cumulative_edges() {
    let bytes = fixture(subtable(&[(65, 67, -64, None)]), 16);
    let font = open(&bytes);
    let mut baseline = TrueTypeCmapSession::new(&font, Default::default()).unwrap();
    baseline.select(0).unwrap();
    let used = baseline.usage();
    for kind in 0..3 {
        for delta in [0, 1] {
            let mut limits = CmapLimits::default();
            match kind {
                0 => limits.max_segments = used.segments - delta,
                1 => limits.max_validation_codes = used.validated_codes - delta,
                _ => limits.max_work = used.work - delta,
            }
            let mut session = TrueTypeCmapSession::new(&font, limits).unwrap();
            assert_eq!(
                session.select(0).is_ok(),
                delta == 0,
                "kind={kind},delta={delta}"
            );
            assert!(session.select(0).is_err());
            assert!(session.selection().is_none());
            let after = session.usage();
            assert!(after.segments <= limits.max_segments);
            assert!(after.validated_codes <= limits.max_validation_codes);
            assert!(after.work <= limits.max_work);
        }
    }
}

#[test]
fn directory_cannot_alias_its_own_header_or_a_truncated_format_word() {
    let valid = cmap(&[(3, 1, subtable(&[(65, 65, -64, None)]))]);
    for offset in [0, 4, 11, valid.len() - 1, valid.len()] {
        let mut table = valid.clone();
        table[8..12].copy_from_slice(&(offset as u32).to_be_bytes());
        let bytes = font(table, 16);
        let font = open(&bytes);
        assert!(
            TrueTypeCmapSession::new(&font, Default::default()).is_err(),
            "offset={offset}"
        );
    }
    let bytes = font(valid, 16);
    let font = open(&bytes);
    assert!(
        TrueTypeCmapSession::new(
            &font,
            CmapLimits {
                max_records: 0,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn full_u16_domain_terminates_and_retains_maximum_valid_gid() {
    let bytes = fixture(subtable(&[(0, 65534, 0, None)]), 65535);
    let font = open(&bytes);
    let mut session = TrueTypeCmapSession::new(
        &font,
        CmapLimits {
            max_validation_codes: 65536,
            max_queries: 65536,
            ..Default::default()
        },
    )
    .unwrap();
    session.select(0).unwrap();
    assert_eq!(session.usage().validated_codes, 65536);
    for code in 0..=u16::MAX {
        assert_eq!(
            gid(session.lookup(code).unwrap()),
            if code == 65535 { 0 } else { code }
        );
    }
    assert_eq!(session.usage().queries, 65536);
    assert!(session.lookup(1).is_err());
    let mut short = TrueTypeCmapSession::new(
        &font,
        CmapLimits {
            max_validation_codes: 65535,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(short.select(0).is_err());
    assert_eq!(short.usage().validated_codes, 65535);
    assert!(short.selection().is_none());
}

#[test]
fn every_truncated_cmap_prefix_refuses_without_borrowing_adjacent_sfnt_tables() {
    let table = cmap(&[(3, 1, subtable(&[(65, 69, -5, Some(vec![0, 5, 6, 7, 8]))]))]);
    for len in 0..table.len() {
        let bytes = font(table[..len].to_vec(), 16);
        let font = open(&bytes);
        if let Ok(mut session) = TrueTypeCmapSession::new(&font, Default::default()) {
            assert!(session.select(0).is_err(), "prefix={len}");
        }
    }
}
