use tandem_pdf_engine::truetype_outline::*;

// Independent tiny encoder: long signed deltas, one point in each contour.
fn font(glyphs: &[Vec<u8>], aligned: bool) -> Vec<u8> {
    let mut head = vec![0; 54];
    head[0..4].copy_from_slice(&0x10000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5f0f3cf5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    let mut maxp = vec![0; 32];
    maxp[0..4].copy_from_slice(&0x10000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&(glyphs.len() as u16).to_be_bytes());
    maxp[6..8].copy_from_slice(&100u16.to_be_bytes());
    maxp[8..10].copy_from_slice(&100u16.to_be_bytes());
    let mut loca = Vec::new();
    let mut glyf = Vec::new();
    for g in glyphs {
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        glyf.extend_from_slice(g);
    }
    loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
    let mut out = vec![0; 76];
    out[0..4].copy_from_slice(&0x10000u32.to_be_bytes());
    out[4..6].copy_from_slice(&4u16.to_be_bytes());
    for (i, (tag, data)) in [
        (b"head", head),
        (b"maxp", maxp),
        (b"loca", loca),
        (b"glyf", glyf),
    ]
    .into_iter()
    .enumerate()
    {
        if aligned {
            while out.len() % 4 != 0 {
                out.push(0);
            }
        }
        let p = 12 + i * 16;
        out[p..p + 4].copy_from_slice(tag);
        let offset = out.len() as u32;
        out[p + 8..p + 12].copy_from_slice(&offset.to_be_bytes());
        out[p + 12..p + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend(data);
    }
    out
}
fn glyph(on: bool) -> Vec<u8> {
    // One contour: (1,3) on or off curve. No instructions.
    let mut g = vec![0, 1, 0, 1, 0, 3, 0, 1, 0, 3, 0, 0, 0, 0];
    g.extend([u8::from(on), 0, 1, 0, 3]);
    g
}
fn open(bytes: &[u8]) -> TrueTypeFont<'_> {
    TrueTypeFont::open(bytes, FontLimits::default()).unwrap()
}
#[test]
fn single_off_point_retains_quadratic_and_exact_unaligned_coordinates() {
    let bytes = font(&[glyph(false)], false);
    let original = bytes.clone();
    let f = open(&bytes);
    assert!(f.table_records().iter().any(|t| t.offset() % 4 != 0));
    let g = f
        .session(OutlineLimits::default())
        .decode(f.glyph_id(0).unwrap())
        .unwrap();
    assert_eq!(g.header_bounds().unwrap().coordinates(), [1, 3, 1, 3]);
    assert_eq!(g.control_bounds(), g.header_bounds());
    match g.commands() {
        [
            OutlineCommand::MoveTo(a),
            OutlineCommand::QuadTo { control, end },
            OutlineCommand::Close,
        ] => {
            assert_eq!((a.x2(), a.y2()), (2, 6));
            assert_eq!(control, a);
            assert_eq!(end, a);
        }
        x => panic!("unexpected {x:?}"),
    }
    assert_eq!(bytes, original);
}
#[test]
fn failed_glyph_cannot_borrow_coordinate_bytes_from_the_next_glyph() {
    let mut truncated = glyph(true);
    truncated.pop();
    let bytes = font(&[truncated, glyph(false)], true);
    let f = open(&bytes);
    let mut s = f.session(OutlineLimits::default());
    let e = s.decode(f.glyph_id(0).unwrap()).unwrap_err();
    assert_eq!(e.glyph, Some(0));
    assert_eq!(e.table, Some(*b"glyf"));
    assert!(matches!(e.kind, OutlineErrorKind::Malformed(_)));
    assert_eq!(s.decode(f.glyph_id(1).unwrap()).unwrap().points().len(), 1);
    assert_eq!(s.usage().decode_attempts, 2);
}
#[test]
fn zero_command_allowance_allocates_no_command_arena_and_exact_three_succeeds() {
    let bytes = font(&[glyph(false)], true);
    let f = open(&bytes);
    let id = f.glyph_id(0).unwrap();
    let mut a = f.session(OutlineLimits {
        max_total_commands: 0,
        ..OutlineLimits::default()
    });
    assert!(matches!(
        a.decode(id).unwrap_err().kind,
        OutlineErrorKind::Limit(_)
    ));
    assert_eq!(a.usage().commands, 0);
    let mut b = f.session(OutlineLimits {
        max_total_commands: 3,
        ..OutlineLimits::default()
    });
    assert_eq!(b.decode(id).unwrap().commands().len(), 3);
    assert_eq!(
        b.usage().allocation_bytes - a.usage().allocation_bytes,
        3 * std::mem::size_of::<OutlineCommand>()
    );
    let before = b.usage();
    assert!(b.decode(id).is_err());
    assert_eq!(b.usage().commands, 3);
    assert_eq!(b.usage().decode_attempts, before.decode_attempts + 1);
}
#[test]
fn exact_session_cumulative_work_allocation_and_point_edges() {
    let bytes = font(&[glyph(false)], true);
    let f = open(&bytes);
    let id = f.glyph_id(0).unwrap();
    let mut baseline = f.session(OutlineLimits::default());
    baseline.decode(id).unwrap();
    let used = baseline.usage();
    let limits = OutlineLimits {
        max_decode_attempts: used.decode_attempts,
        max_total_glyph_bytes: used.glyph_bytes,
        max_total_contours: used.contours,
        max_total_points: used.points,
        max_total_commands: used.commands,
        max_allocation_bytes: used.allocation_bytes,
        max_work: used.work,
        ..OutlineLimits::default()
    };
    let mut s = f.session(limits);
    s.decode(id).unwrap();
    assert!(s.decode(id).is_err());
    for field in 0..5 {
        let mut l = limits;
        match field {
            0 => l.max_total_glyph_bytes -= 1,
            1 => l.max_total_points -= 1,
            2 => l.max_total_commands -= 1,
            3 => {
                l.max_allocation_bytes = 2
                    + 1
                    + std::mem::size_of::<FontPoint>()
                    + 3 * std::mem::size_of::<OutlineCommand>()
                    - 1
            }
            _ => l.max_work -= 1,
        }
        assert!(
            matches!(
                f.session(l).decode(id).unwrap_err().kind,
                OutlineErrorKind::Limit(_)
            ),
            "{field}"
        );
    }
}
#[test]
fn malformed_repeats_charge_attempts_and_fail_closed_without_panics() {
    let mut g = glyph(true);
    g[14] = 9;
    g[15] = 255;
    let bytes = font(&[g], true);
    let f = open(&bytes);
    let id = f.glyph_id(0).unwrap();
    let mut s = f.session(OutlineLimits {
        max_decode_attempts: 2,
        ..OutlineLimits::default()
    });
    for _ in 0..2 {
        assert!(matches!(
            s.decode(id).unwrap_err().kind,
            OutlineErrorKind::Malformed(_)
        ));
    }
    assert_eq!(s.usage().decode_attempts, 2);
    assert!(matches!(
        s.decode(id).unwrap_err().kind,
        OutlineErrorKind::Limit(_)
    ));
}
#[test]
fn foreign_glyph_id_is_rechecked_and_all_prefixes_refuse() {
    let a = font(&[vec![], glyph(true)], true);
    let f = open(&a);
    let foreign = f.glyph_id(1).unwrap();
    let b = font(&[glyph(true)], true);
    let small = open(&b);
    assert!(
        small
            .session(OutlineLimits::default())
            .decode(foreign)
            .is_err()
    );
    for n in 0..b.len() {
        assert!(
            TrueTypeFont::open(&b[..n], FontLimits::default()).is_err(),
            "prefix {n}"
        );
    }
}
#[test]
fn exact_font_index_budgets_and_zero_contour_instruction_opacity() {
    let mut zero = vec![0; 10];
    zero.extend([0, 3, 0xff, 0x89, 0x2d]);
    let bytes = font(&[vec![], zero], true);
    let f = open(&bytes);
    let u = f.usage();
    let exact = FontLimits {
        max_input_bytes: bytes.len(),
        max_tables: 4,
        max_glyphs: 2,
        max_index_allocation_bytes: u.index_allocation_bytes,
        max_work: u.work,
    };
    TrueTypeFont::open(&bytes, exact).unwrap();
    assert!(
        TrueTypeFont::open(
            &bytes,
            FontLimits {
                max_work: u.work - 1,
                ..exact
            }
        )
        .is_err()
    );
    assert!(
        TrueTypeFont::open(
            &bytes,
            FontLimits {
                max_index_allocation_bytes: u.index_allocation_bytes - 1,
                ..exact
            }
        )
        .is_err()
    );
    let mut s = f.session(OutlineLimits::default());
    let empty = s.decode(f.glyph_id(0).unwrap()).unwrap();
    assert_eq!(empty.kind(), OutlineKind::EmptyLocaRange);
    let zero = s.decode(f.glyph_id(1).unwrap()).unwrap();
    assert_eq!(zero.kind(), OutlineKind::ZeroContours);
    assert_eq!(zero.instruction_bytes(), 3);
    assert!(zero.commands().is_empty());
}
