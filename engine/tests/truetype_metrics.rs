// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::{truetype_metrics::HorizontalMetrics, truetype_outline::TrueTypeFont};
fn put16(b: &mut [u8], at: usize, n: u16) {
    b[at..at + 2].copy_from_slice(&n.to_be_bytes());
}
fn put32(b: &mut [u8], at: usize, n: u32) {
    b[at..at + 4].copy_from_slice(&n.to_be_bytes());
}
fn hhea(count: u16) -> Vec<u8> {
    let mut b = vec![0; 36];
    put32(&mut b, 0, 0x10000);
    put16(&mut b, 34, count);
    b
}
fn font(header: Vec<u8>, metrics: Vec<u8>, count: u16) -> Vec<u8> {
    let mut head = vec![0; 54];
    put32(&mut head, 0, 0x10000);
    put32(&mut head, 12, 0x5f0f3cf5);
    put16(&mut head, 18, 1000);
    let mut maxp = vec![0; 32];
    put32(&mut maxp, 0, 0x10000);
    put16(&mut maxp, 4, count);
    let tables = [
        (*b"head", head),
        (*b"maxp", maxp),
        (*b"loca", vec![0; 2 * (usize::from(count) + 1)]),
        (*b"glyf", vec![]),
        (*b"hhea", header),
        (*b"hmtx", metrics),
    ];
    let mut bytes = vec![0; 12 + 16 * tables.len()];
    put32(&mut bytes, 0, 0x10000);
    put16(&mut bytes, 4, tables.len() as u16);
    for (i, (tag, value)) in tables.iter().enumerate() {
        let offset = bytes.len();
        let at = 12 + 16 * i;
        bytes[at..at + 4].copy_from_slice(tag);
        put32(&mut bytes, at + 8, offset as u32);
        put32(&mut bytes, at + 12, value.len() as u32);
        bytes.extend_from_slice(value);
    }
    bytes
}
fn words(values: &[i32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|v| (*v as u16).to_be_bytes())
        .collect()
}
#[test]
fn compressed_metrics_repeat_last_advance_and_keep_individual_signed_bearings() {
    let bytes = font(hhea(2), words(&[600, -32768, 900, 32767, -40, 100]), 4);
    let f = TrueTypeFont::open(&bytes, Default::default()).unwrap();
    let m = HorizontalMetrics::open(&f).unwrap();
    assert_eq!(m.number_of_h_metrics(), 2);
    assert_eq!(m.hmtx_range().len(), 12);
    assert_eq!(m.hhea_range().len(), 36);
    for (gid, (advance, lsb)) in [(600, -32768), (900, 32767), (900, -40), (900, 100)]
        .into_iter()
        .enumerate()
    {
        let metric = m.get(f.glyph_id(gid as u32).unwrap()).unwrap();
        assert_eq!(metric.advance_width(), advance);
        assert_eq!(metric.left_side_bearing(), lsb);
    }
}
#[test]
fn all_metric_truncations_are_rejected_without_querying() {
    let full = hhea(2);
    let entries = words(&[600, 10, 900, -10, 30]);
    for n in 0..36 {
        let bytes = font(full[..n].to_vec(), entries.clone(), 3);
        let f = TrueTypeFont::open(&bytes, Default::default()).unwrap();
        assert!(HorizontalMetrics::open(&f).is_err());
    }
    for n in 0..entries.len() {
        let bytes = font(full.clone(), entries[..n].to_vec(), 3);
        let f = TrueTypeFont::open(&bytes, Default::default()).unwrap();
        assert!(HorizontalMetrics::open(&f).is_err());
    }
}
#[test]
fn metric_header_counts_versions_reserved_words_and_padding_are_strict() {
    let entries = words(&[600, 10, 900, -10, 30]);
    for at in [0, 2, 24, 26, 28, 30, 32] {
        let mut h = hhea(2);
        h[at] ^= 1;
        let bytes = font(h, entries.clone(), 3);
        let f = TrueTypeFont::open(&bytes, Default::default()).unwrap();
        assert!(HorizontalMetrics::open(&f).is_err());
    }
    for count in [0, 4, 65535] {
        let bytes = font(hhea(count), entries.clone(), 3);
        let f = TrueTypeFont::open(&bytes, Default::default()).unwrap();
        assert!(HorizontalMetrics::open(&f).is_err());
    }
    for suffix in [vec![1], vec![0, 0, 0, 0]] {
        let mut m = entries.clone();
        m.extend_from_slice(&suffix);
        let bytes = font(hhea(2), m, 3);
        let f = TrueTypeFont::open(&bytes, Default::default()).unwrap();
        assert!(HorizontalMetrics::open(&f).is_err());
    }
    let mut h = hhea(2);
    h.extend([0, 0, 0]);
    let mut m = entries;
    m.extend([0, 0, 0]);
    let bytes = font(h, m, 3);
    let f = TrueTypeFont::open(&bytes, Default::default()).unwrap();
    assert!(HorizontalMetrics::open(&f).is_ok());
}
#[test]
fn gid_from_a_larger_font_is_range_checked_again() {
    let large = font(hhea(1), words(&[600, 0, 10, 20, 30]), 4);
    let lf = TrueTypeFont::open(&large, Default::default()).unwrap();
    let foreign = lf.glyph_id(3).unwrap();
    let small = font(hhea(1), words(&[600, -40]), 1);
    let sf = TrueTypeFont::open(&small, Default::default()).unwrap();
    let m = HorizontalMetrics::open(&sf).unwrap();
    assert!(m.get(foreign).is_err());
    assert_eq!(
        m.get(sf.glyph_id(0).unwrap()).unwrap().left_side_bearing(),
        -40
    );
}
