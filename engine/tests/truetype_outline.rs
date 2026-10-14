use tandem_pdf_engine::truetype_outline::*;

fn word(out: &mut Vec<u8>, x: u16) {
    out.extend_from_slice(&x.to_be_bytes());
}
fn signed(out: &mut Vec<u8>, x: i16) {
    word(out, x as u16);
}
fn put16(bytes: &mut [u8], at: usize, x: u16) {
    bytes[at..at + 2].copy_from_slice(&x.to_be_bytes());
}
fn put32(bytes: &mut [u8], at: usize, x: u32) {
    bytes[at..at + 4].copy_from_slice(&x.to_be_bytes());
}
fn simple(points: &[(i16, i16, bool)], ends: &[u16]) -> Vec<u8> {
    let mut b = Vec::new();
    word(&mut b, ends.len() as u16);
    for x in [
        points.iter().map(|p| p.0).min().unwrap_or(0),
        points.iter().map(|p| p.1).min().unwrap_or(0),
        points.iter().map(|p| p.0).max().unwrap_or(0),
        points.iter().map(|p| p.1).max().unwrap_or(0),
    ] {
        signed(&mut b, x);
    }
    for &end in ends {
        word(&mut b, end);
    }
    word(&mut b, 0);
    for p in points {
        b.push(u8::from(p.2));
    }
    let mut x = 0;
    for p in points {
        signed(&mut b, p.0 - x);
        x = p.0;
    }
    let mut y = 0;
    for p in points {
        signed(&mut b, p.1 - y);
        y = p.1;
    }
    b
}
fn font(glyphs: &[Vec<u8>], long: bool) -> Vec<u8> {
    let mut head = vec![0; 54];
    put32(&mut head, 0, 0x10000);
    put32(&mut head, 12, 0x5f0f3cf5);
    put16(&mut head, 18, 1000);
    put16(&mut head, 50, u16::from(long));
    let mut maxp = vec![0; 32];
    put32(&mut maxp, 0, 0x10000);
    put16(&mut maxp, 4, glyphs.len() as u16);
    put16(&mut maxp, 6, 32767);
    put16(&mut maxp, 8, 4096);
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in glyphs {
        if long {
            loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        } else {
            word(&mut loca, (glyf.len() / 2) as u16);
        }
        glyf.extend_from_slice(g);
        if !long && glyf.len() % 2 != 0 {
            glyf.push(0);
        }
    }
    if long {
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
    } else {
        word(&mut loca, (glyf.len() / 2) as u16);
    }
    let tables = [
        (*b"head", head),
        (*b"maxp", maxp),
        (*b"loca", loca),
        (*b"glyf", glyf),
    ];
    let mut out = vec![0; 76];
    put32(&mut out, 0, 0x10000);
    put16(&mut out, 4, 4);
    for (i, (tag, bytes)) in tables.iter().enumerate() {
        while out.len() % 4 != 0 {
            out.push(0);
        }
        let at = 12 + 16 * i;
        let offset = out.len();
        out[at..at + 4].copy_from_slice(tag);
        put32(&mut out, at + 8, offset as u32);
        put32(&mut out, at + 12, bytes.len() as u32);
        out.extend_from_slice(bytes);
    }
    out
}
fn table(bytes: &[u8], tag: &[u8; 4]) -> usize {
    (0..4)
        .find_map(|i| {
            let p = 12 + 16 * i;
            if &bytes[p..p + 4] == tag {
                Some(u32::from_be_bytes(bytes[p + 8..p + 12].try_into().unwrap()) as usize)
            } else {
                None
            }
        })
        .unwrap()
}
fn decode(bytes: &[u8], id: u32) -> Result<GlyphOutline, OutlineError> {
    let f = TrueTypeFont::open(bytes, FontLimits::default())?;
    f.session(OutlineLimits::default()).decode(f.glyph_id(id)?)
}
fn commands(o: &GlyphOutline) -> Vec<(&'static str, Vec<i32>)> {
    o.commands()
        .iter()
        .map(|c| match c {
            OutlineCommand::MoveTo(p) => ("m", vec![p.x2(), p.y2()]),
            OutlineCommand::LineTo(p) => ("l", vec![p.x2(), p.y2()]),
            OutlineCommand::QuadTo { control, end } => {
                ("q", vec![control.x2(), control.y2(), end.x2(), end.y2()])
            }
            OutlineCommand::Close => ("h", vec![]),
        })
        .collect()
}

#[test]
fn metadata_and_empty_short_long() {
    for long in [false, true] {
        let bytes = font(&[vec![], simple(&[(1, 2, true), (4, 5, true)], &[1])], long);
        let f = TrueTypeFont::open(&bytes, FontLimits::default()).unwrap();
        assert_eq!(f.units_per_em(), 1000);
        assert_eq!(f.glyph_count(), 2);
        assert_eq!(f.table_records().len(), 4);
        assert!(f.glyph_id(2).is_err());
        let mut s = f.session(OutlineLimits::default());
        let empty = s.decode(f.glyph_id(0).unwrap()).unwrap();
        assert_eq!(empty.kind(), OutlineKind::EmptyLocaRange);
        assert!(empty.points().is_empty());
        assert_eq!(empty.header_bounds(), None);
        let o = s.decode(f.glyph_id(1).unwrap()).unwrap();
        assert_eq!(o.kind(), OutlineKind::Simple);
        assert_eq!(o.control_bounds().unwrap().coordinates(), [1, 2, 4, 5]);
        assert_eq!(o.contour_endpoints(), [1]);
        assert_eq!(s.usage().decode_attempts, 2);
        assert_eq!(
            commands(&o),
            vec![
                ("m", vec![2, 4]),
                ("l", vec![8, 10]),
                ("l", vec![2, 4]),
                ("h", vec![])
            ]
        );
    }
}
#[test]
fn implied_midpoints_and_off_curve_endpoints_are_exact() {
    let cases = [
        (
            vec![(1, 1, false), (4, 6, false)],
            vec![
                ("m", vec![5, 7]),
                ("q", vec![2, 2, 5, 7]),
                ("q", vec![8, 12, 5, 7]),
                ("h", vec![]),
            ],
        ),
        (
            vec![(1, 2, false), (4, 5, true)],
            vec![("m", vec![8, 10]), ("q", vec![2, 4, 8, 10]), ("h", vec![])],
        ),
        (
            vec![(1, 2, true), (4, 5, false)],
            vec![("m", vec![2, 4]), ("q", vec![8, 10, 2, 4]), ("h", vec![])],
        ),
        (
            vec![(1, 2, false)],
            vec![("m", vec![2, 4]), ("q", vec![2, 4, 2, 4]), ("h", vec![])],
        ),
        (
            vec![(1, 2, true)],
            vec![("m", vec![2, 4]), ("l", vec![2, 4]), ("h", vec![])],
        ),
    ];
    for (p, expected) in cases {
        let bytes = font(&[simple(&p, &[(p.len() - 1) as u16])], false);
        assert_eq!(commands(&decode(&bytes, 0).unwrap()), expected);
    }
}
#[test]
fn coordinate_accumulation_continues_between_contours() {
    let bytes = font(
        &[simple(
            &[(10, 10, true), (20, 30, false), (40, 50, true)],
            &[0, 2],
        )],
        false,
    );
    let o = decode(&bytes, 0).unwrap();
    assert_eq!(
        o.points()
            .iter()
            .map(|p| (p.x(), p.y(), p.on_curve()))
            .collect::<Vec<_>>(),
        vec![(10, 10, true), (20, 30, false), (40, 50, true)]
    );
    assert_eq!(
        o.commands()
            .iter()
            .filter(|c| matches!(c, OutlineCommand::Close))
            .count(),
        2
    );
}
#[test]
fn repeated_short_and_unchanged_coordinates() {
    // Three points: (5,0), (10,0), (15,0). Repeat flag is additional count.
    let mut g = Vec::new();
    for n in [1, 5, 0, 15, 0, 2, 0] {
        word(&mut g, n);
    }
    g.extend_from_slice(&[0x3b, 2, 5, 5, 5]);
    let o = decode(&font(&[g.clone()], true), 0).unwrap();
    assert_eq!(
        o.points().iter().map(|p| p.x()).collect::<Vec<_>>(),
        [5, 10, 15]
    );
    g[15] = 3;
    assert!(decode(&font(&[g], true), 0).is_err());
}
#[test]
fn overlap_flag_is_current_spec_bit_and_reserved_flag_refused() {
    let mut g = simple(&[(1, 2, true), (3, 4, true)], &[1]);
    g[14] |= 0x40;
    assert!(decode(&font(&[g.clone()], true), 0).unwrap().overlap());
    g[14] &= !0x40;
    g[15] |= 0x40;
    assert!(decode(&font(&[g.clone()], true), 0).is_err());
    g[15] = 0x81;
    assert!(decode(&font(&[g], true), 0).is_err());
}
#[test]
fn zero_contours_and_bounded_instructions() {
    for extra in [
        vec![],
        vec![0],
        vec![0, 0],
        vec![0, 3, 1, 2, 3],
        vec![0, 3, 1, 2, 3, 0, 0],
    ] {
        let mut g = vec![0; 10];
        g.extend_from_slice(&extra);
        let o = decode(&font(&[g], true), 0).unwrap();
        assert_eq!(o.kind(), OutlineKind::ZeroContours);
        assert!(o.commands().is_empty());
        assert_eq!(o.instruction_bytes(), if extra.len() > 2 { 3 } else { 0 });
    }
    let mut g = vec![0; 10];
    g.extend_from_slice(&[0, 2, 1]);
    assert!(decode(&font(&[g], true), 0).is_err());
    let mut g = vec![0; 10];
    g.push(1);
    assert!(decode(&font(&[g], true), 0).is_err());
}
/// Composite glyph: (flags, glyph, dx, dy, F2Dot14 matrix words).
fn composite(components: &[(u16, u16, i16, i16, &[i16])]) -> Vec<u8> {
    let mut g = vec![];
    signed(&mut g, -1);
    for v in [0, 0, 0, 0] {
        signed(&mut g, v);
    }
    for (i, (flags, glyph, dx, dy, matrix)) in components.iter().enumerate() {
        let more = if i + 1 < components.len() { 0x20 } else { 0 };
        word(&mut g, flags | more | 0x1 | 0x2);
        word(&mut g, *glyph);
        signed(&mut g, *dx);
        signed(&mut g, *dy);
        for m in *matrix {
            signed(&mut g, *m);
        }
    }
    g
}
fn points(o: &GlyphOutline) -> Vec<(i32, i32)> {
    o.points().iter().map(|p| (p.x(), p.y())).collect()
}
#[test]
fn composites_flatten_offset_scaled_and_nested_components() {
    let square = simple(
        &[(0, 0, true), (10, 0, true), (10, 10, true), (0, 10, true)],
        &[3],
    );
    let glyphs = [
        square.clone(),
        composite(&[(0, 0, 5, 7, &[])]),
        // Uniform 0.5 scale, then offset; xy scale 2 x 1; 2x2 quarter turn.
        composite(&[(0x8, 0, 1, 1, &[8192])]),
        composite(&[(0x40, 0, 0, 0, &[32767, 16384])]),
        composite(&[(0x80, 0, 0, 0, &[0, 16384, -16384, 0])]),
        // Nested composite plus a second component; contours concatenate.
        composite(&[(0, 1, 100, 0, &[]), (0, 0, 0, 0, &[])]),
    ];
    let font = font(&glyphs, true);
    let o = decode(&font, 1).unwrap();
    assert_eq!(o.kind(), OutlineKind::Composite);
    assert_eq!(points(&o), [(5, 7), (15, 7), (15, 17), (5, 17)]);
    assert_eq!(o.contour_endpoints(), [3]);
    assert_eq!(commands(&o).len(), 6);
    assert_eq!(
        points(&decode(&font, 2).unwrap()),
        [(1, 1), (6, 1), (6, 6), (1, 6)]
    );
    assert_eq!(
        points(&decode(&font, 3).unwrap()),
        [(0, 0), (20, 0), (20, 10), (0, 10)]
    );
    assert_eq!(
        points(&decode(&font, 4).unwrap()),
        [(0, 0), (0, 10), (-10, 10), (-10, 0)]
    );
    let nested = decode(&font, 5).unwrap();
    assert_eq!(nested.contour_endpoints(), [3, 7]);
    assert_eq!(
        points(&nested)[..4],
        [(105, 7), (115, 7), (115, 17), (105, 17)]
    );
    assert_eq!(
        nested.control_bounds().unwrap().coordinates(),
        [0, 0, 115, 17]
    );
}
#[test]
fn composite_anchor_matching_cycles_and_bad_children_are_refused() {
    let square = simple(&[(0, 0, true), (10, 0, true), (10, 10, true)], &[2]);
    let mut anchored = composite(&[(0, 0, 0, 0, &[])]);
    anchored[11] &= !0x2;
    let glyphs = [
        square,
        anchored,
        composite(&[(0, 2, 0, 0, &[])]),
        composite(&[(0, 9, 0, 0, &[])]),
    ];
    let bytes = font(&glyphs, true);
    for (id, expected) in [
        (
            1,
            OutlineErrorKind::Unsupported("composite anchor-point matching"),
        ),
        (2, OutlineErrorKind::Limit("composite depth")),
    ] {
        let e = decode(&bytes, id).unwrap_err();
        assert_eq!(e.kind, expected, "{id}");
        assert_eq!(e.table, Some(*b"glyf"));
    }
    assert!(decode(&bytes, 3).is_err());
    let mut truncated = composite(&[(0, 0, 0, 0, &[])]);
    truncated.truncate(14);
    assert!(decode(&font(&[simple(&[(0, 0, true)], &[0]), truncated], true), 1).is_err());
}
#[test]
fn every_simple_payload_truncation_and_padding_boundary() {
    let g = simple(&[(1, 2, true), (5, 6, false)], &[1]);
    for len in 1..g.len() {
        assert!(
            decode(&font(&[g[..len].to_vec()], true), 0).is_err(),
            "{len}"
        );
    }
    for n in 0..=3 {
        let mut p = g.clone();
        p.resize(p.len() + n, 0);
        decode(&font(&[p], true), 0).unwrap();
    }
    let mut p = g.clone();
    p.extend_from_slice(&[0; 4]);
    assert!(decode(&font(&[p], true), 0).is_err());
    let mut p = g;
    p.push(1);
    assert!(decode(&font(&[p], true), 0).is_err());
}
#[test]
fn strict_bbox_endpoints_and_maxp() {
    let g = simple(&[(1, 2, true), (5, 6, true)], &[1]);
    let mut b = font(&[g], true);
    let p = table(&b, b"glyf");
    put16(&mut b, p + 2, 0);
    assert!(decode(&b, 0).is_err());
    put16(&mut b, p + 2, 1);
    let m = table(&b, b"maxp");
    put16(&mut b, m + 6, 1);
    assert!(decode(&b, 0).is_err());
    let mut g = simple(&[(1, 2, true), (5, 6, true)], &[0, 1]);
    put16(&mut g, 12, 0);
    assert!(decode(&font(&[g], true), 0).is_err());
}
#[test]
fn structural_directory_head_loca_and_container_refusals() {
    let good = font(&[vec![], simple(&[(1, 1, true)], &[0])], true);
    for n in 0..good.len() {
        assert!(TrueTypeFont::open(&good[..n], FontLimits::default()).is_err());
    }
    for signature in [*b"OTTO", *b"ttcf", *b"wOFF", *b"true"] {
        let mut b = good.clone();
        b[..4].copy_from_slice(&signature);
        assert!(matches!(
            TrueTypeFont::open(&b, FontLimits::default())
                .unwrap_err()
                .kind,
            OutlineErrorKind::Unsupported(_)
        ));
    }
    let mut b = good.clone();
    b[28..32].copy_from_slice(b"head");
    assert!(TrueTypeFont::open(&b, FontLimits::default()).is_err());
    let mut b = good.clone();
    let h = table(&b, b"head");
    put32(&mut b, 36, h as u32);
    assert!(TrueTypeFont::open(&b, FontLimits::default()).is_err());
    let mut b = good.clone();
    let h = table(&b, b"head");
    put16(&mut b, h + 18, 15);
    assert!(TrueTypeFont::open(&b, FontLimits::default()).is_err());
    let mut b = good.clone();
    let l = table(&b, b"loca");
    put32(&mut b, l, 4);
    assert!(TrueTypeFont::open(&b, FontLimits::default()).is_err());
    let mut b = good;
    b[12..16].copy_from_slice(b"gvar");
    assert!(matches!(
        TrueTypeFont::open(&b, FontLimits::default())
            .unwrap_err()
            .kind,
        OutlineErrorKind::Unsupported(_)
    ));
}
#[test]
fn font_open_limits_exact_boundary() {
    let b = font(&[vec![]], false);
    let f = TrueTypeFont::open(&b, FontLimits::default()).unwrap();
    let u = f.usage();
    let l = FontLimits {
        max_input_bytes: b.len(),
        max_tables: 4,
        max_glyphs: 1,
        max_index_allocation_bytes: u.index_allocation_bytes,
        max_work: u.work,
    };
    TrueTypeFont::open(&b, l).unwrap();
    for which in 0..5 {
        let mut x = l;
        match which {
            0 => x.max_input_bytes -= 1,
            1 => x.max_tables -= 1,
            2 => x.max_glyphs -= 1,
            3 => x.max_index_allocation_bytes -= 1,
            _ => x.max_work -= 1,
        };
        assert!(matches!(
            TrueTypeFont::open(&b, x).unwrap_err().kind,
            OutlineErrorKind::Limit(_)
        ));
    }
}
#[test]
fn aggregate_work_allocation_and_failed_attempts_do_not_reset() {
    let b = font(&[simple(&[(1, 2, true), (3, 4, false)], &[1])], true);
    let f = TrueTypeFont::open(&b, FontLimits::default()).unwrap();
    let id = f.glyph_id(0).unwrap();
    let mut s = f.session(OutlineLimits::default());
    s.decode(id).unwrap();
    let u = s.usage();
    let mut exact = OutlineLimits {
        max_decode_attempts: 1,
        max_total_glyph_bytes: u.glyph_bytes,
        max_total_contours: u.contours,
        max_total_points: u.points,
        max_total_commands: u.commands,
        max_allocation_bytes: u.allocation_bytes,
        max_work: u.work,
        ..OutlineLimits::default()
    };
    let mut s = f.session(exact);
    s.decode(id).unwrap();
    assert!(s.decode(id).is_err());
    assert_eq!(s.usage().commands, u.commands);
    assert!(s.usage().allocation_bytes <= u.allocation_bytes);
    exact.max_allocation_bytes = s.usage().allocation_bytes;
    for which in 0..7 {
        let mut x = exact;
        match which {
            0 => x.max_decode_attempts -= 1,
            1 => x.max_total_glyph_bytes -= 1,
            2 => x.max_total_contours -= 1,
            3 => x.max_total_points -= 1,
            4 => x.max_total_commands -= 1,
            5 => x.max_allocation_bytes -= 1,
            _ => x.max_work -= 1,
        };
        assert!(matches!(
            f.session(x).decode(id).unwrap_err().kind,
            OutlineErrorKind::Limit(_)
        ));
    }
    let mut s = f.session(OutlineLimits {
        max_total_commands: 0,
        ..OutlineLimits::default()
    });
    assert!(s.decode(id).is_err());
    let a = s.usage();
    assert!(s.decode(id).is_err());
    let z = s.usage();
    assert_eq!(z.decode_attempts, 2);
    assert_eq!(z.glyph_bytes, a.glyph_bytes * 2);
    assert_eq!(z.allocation_bytes, a.allocation_bytes * 2);
}
#[test]
fn per_glyph_limits_and_cross_font_id_checked() {
    let b = font(
        &[vec![], simple(&[(1, 2, true), (3, 4, true)], &[1])],
        false,
    );
    let f = TrueTypeFont::open(&b, FontLimits::default()).unwrap();
    let id = f.glyph_id(1).unwrap();
    for l in [
        OutlineLimits {
            max_points_per_glyph: 1,
            ..OutlineLimits::default()
        },
        OutlineLimits {
            max_contours_per_glyph: 0,
            ..OutlineLimits::default()
        },
    ] {
        assert!(matches!(
            f.session(l).decode(id).unwrap_err().kind,
            OutlineErrorKind::Limit(_)
        ));
    }
    let small = font(&[vec![]], false);
    let f2 = TrueTypeFont::open(&small, FontLimits::default()).unwrap();
    assert!(f2.session(OutlineLimits::default()).decode(id).is_err());
    let mut g = vec![0; 10];
    g.extend_from_slice(&[0, 1, 7]);
    let b = font(&[g], true);
    let f = TrueTypeFont::open(&b, FontLimits::default()).unwrap();
    assert!(
        f.session(OutlineLimits {
            max_instruction_bytes_per_glyph: 0,
            ..OutlineLimits::default()
        })
        .decode(f.glyph_id(0).unwrap())
        .is_err()
    );
}

#[test]
fn safely_reads_unaligned_nonoverlapping_tables() {
    let original = font(&[simple(&[(1, 2, true), (3, 4, false)], &[1])], true);
    let mut packed = original[..76].to_vec();
    packed.push(0);
    for i in 0..4 {
        let p = 12 + 16 * i;
        let start = u32::from_be_bytes(original[p + 8..p + 12].try_into().unwrap()) as usize;
        let len = u32::from_be_bytes(original[p + 12..p + 16].try_into().unwrap()) as usize;
        let offset = packed.len();
        put32(&mut packed, p + 8, offset as u32);
        packed.extend_from_slice(&original[start..start + len]);
    }
    let o = decode(&packed, 0).unwrap();
    assert_eq!(o.points().len(), 2);
    let f = TrueTypeFont::open(&packed, FontLimits::default()).unwrap();
    assert!(f.table_records().iter().any(|t| t.offset() % 4 != 0));
}
