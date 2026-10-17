// SPDX-License-Identifier: AGPL-3.0-or-later
//! Decoded content hayro holds through what a form inherits (the
//! integration acceptance review, round 3). hayro decodes a content stream
//! whole and holds it while what it invokes runs, and a form inherits its
//! caller's fill pattern, Type3 font and soft mask: a pattern cell or glyph
//! procedure set outside a form runs inside it, and soft masks stay held
//! with the graphics states that set them. The engine's walk counts such a
//! run where it is set, so forms of 120 MiB chained through inherited
//! patterns or Type3 fonts held 128 MiB a level (1,157 MB of WASM at eight
//! levels from a 6,923-byte file). hayro-interpret now counts the content
//! it holds and reads (`hayro_interpret::MAX_HELD_CONTENT_BYTES`,
//! `MAX_READ_CONTENT_BYTES`) and refuses what would pass either; these
//! tests prove the bound with that counter (`RenderStats::content_peak`),
//! each document generated here and refused after holding one form, not
//! eight. WASM: tests/rust-engine-inherited.test.mjs at the app root.
use std::sync::{Mutex, MutexGuard, PoisonError};
use tandem_pdf_engine::view::{
    MAX_HELD_CONTENT_BYTES, RenderBudgets, RenderOptions, RenderStats, RgbaImage, ViewDocument,
    ViewError,
};

/// One test at a time: each decodes forms of 120 MiB a few times, and the
/// macOS allocator keeps much of what a 120 MiB decode frees (about 1 GB of
/// RSS for eight in a row; WASM reuses it).
static HEAVY: Mutex<()> = Mutex::new(());

fn heavy() -> MutexGuard<'static, ()> {
    HEAVY.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A classic-xref PDF from object bodies numbered from 1 (object 1 the catalog).
fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
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

fn obj(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// `data` behind FlateDecode, already deflated.
fn deflated(dict: &str, packed: &[u8]) -> Vec<u8> {
    stream(&format!("{dict} /Filter /FlateDecode"), packed)
}

fn flate(dict: &str, data: &[u8]) -> Vec<u8> {
    deflated(dict, &miniz_oxide::deflate::compress_to_vec_zlib(data, 6))
}

/// `megabytes` MiB of spaces, then `tail`, deflated (about a thousandth of
/// the size).
fn padded(megabytes: usize, tail: &str) -> Vec<u8> {
    let mut data = vec![b' '; megabytes << 20];
    data.extend_from_slice(tail.as_bytes());
    miniz_oxide::deflate::compress_to_vec_zlib(&data, 1)
}

/// A 100 × 100 page with `resources` and Flate `content`; `extra` objects
/// follow as 5, 6, ...
fn page(resources: &str, content: &str, extra: Vec<Vec<u8>>) -> ViewDocument {
    let mut objects = vec![
        obj("<< /Type /Catalog /Pages 2 0 R >>"),
        obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        obj(&format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources {resources} /Contents 4 0 R >>"
        )),
        flate("", content.as_bytes()),
    ];
    objects.extend(extra);
    ViewDocument::open(pdf(&objects), "").expect("open")
}

fn render(doc: &ViewDocument) -> (Result<RgbaImage, ViewError>, RenderStats) {
    doc.render_with_stats(0, 1.0, RenderOptions::default(), RenderBudgets::default())
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * image.width + x) * 4) as usize;
    [image.pixels[at], image.pixels[at + 1], image.pixels[at + 2]]
}

const TOO_LARGE: &str = "page content decodes past the size limit";
const TOO_MUCH_WORK: &str = "page content expands past the work limit";
const HELD: usize = MAX_HELD_CONTENT_BYTES as usize;

fn refused_with(result: &Result<RgbaImage, ViewError>, why: &str) -> bool {
    matches!(result, Err(ViewError::TooComplex(message)) if *message == why)
}

const FORM: &str = "/Type /XObject /Subtype /Form /BBox [0 0 100 100]";
const CELL: &str = "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10";

/// The review's chain: the page sets fill pattern P1 and draws form G1;
/// G1 (`form`: 120 MiB of spaces, then a fill) fills with the inherited P1;
/// P1's cell sets P2 and draws G2; and so on, `levels` deep. The last cell
/// paints a blue square.
fn pattern_chain(levels: usize, form: &[u8]) -> ViewDocument {
    let mut extra = Vec::new();
    for level in 1..=levels {
        let (g, p) = (5 + 2 * (level - 1), 6 + 2 * (level - 1));
        extra.push(deflated(FORM, form));
        extra.push(if level < levels {
            flate(
                &format!(
                    "{CELL} /Resources << /Pattern << /P {} 0 R >> /XObject << /G {} 0 R >> >>",
                    p + 2,
                    g + 2
                ),
                b"/Pattern cs /P scn /G Do",
            )
        } else {
            flate(
                &format!("{CELL} /Resources << >>"),
                b"0 0 1 rg 0 0 5 5 re f",
            )
        });
    }
    page(
        "<< /Pattern << /P 6 0 R >> /XObject << /G 5 0 R >> >>",
        "/Pattern cs /P scn /G Do",
        extra,
    )
}

/// The same through Type3 fonts: the page selects T1 and draws G1, which
/// shows a glyph with the inherited T1; T1's glyph selects T2 and draws G2;
/// and so on. The last glyph is a filled square.
fn type3_chain(levels: usize, form: &[u8]) -> ViewDocument {
    let mut extra = Vec::new();
    for level in 1..=levels {
        let (g, t, proc) = (
            5 + 3 * (level - 1),
            6 + 3 * (level - 1),
            7 + 3 * (level - 1),
        );
        extra.push(deflated(FORM, form));
        let resources = if level < levels {
            format!(
                "/Resources << /Font << /T {} 0 R >> /XObject << /G {} 0 R >> >>",
                t + 3,
                g + 3
            )
        } else {
            String::new()
        };
        extra.push(obj(&format!(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 100 100] /FontMatrix [0.01 0 0 0.01 0 0] /CharProcs << /a {proc} 0 R >> /Encoding << /Type /Encoding /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [100] {resources} >>"
        )));
        extra.push(flate(
            "",
            if level < levels {
                b"100 0 0 0 100 100 d1 BT /T 50 Tf ET /G Do"
            } else {
                b"100 0 0 0 100 100 d1 0 0 100 100 re f"
            },
        ));
    }
    page(
        "<< /Font << /T 6 0 R >> /XObject << /G 5 0 R >> >>",
        "BT /T 20 Tf ET /G Do",
        extra,
    )
}

#[test]
fn pattern_cells_inherited_into_forms_hold_content_within_the_budget() {
    // Each level holds its form while the inherited cell draws the next:
    // 128 MiB a level before. One level holds 120 MiB and draws; at eight
    // levels hayro refuses the second form's content instead of decoding
    // it, holding no more than the budget.
    let _heavy = heavy();
    let form = padded(120, " 0 0 100 100 re f");
    let (drawn, stats) = render(&pattern_chain(1, &form));
    assert_eq!(pixel(&drawn.expect("render"), 2, 97), [0, 0, 255]);
    assert!(stats.content_peak >= 120 << 20, "{stats:?}");
    let doc = pattern_chain(8, &form);
    let (result, stats) = render(&doc);
    assert!(refused_with(&result, TOO_LARGE), "{result:?}");
    assert!(
        (120 << 20..=HELD).contains(&stats.content_peak),
        "{stats:?}"
    );
    // Text runs no pattern cells: it reads the first form only.
    assert!(doc.structured_text(0).is_ok());
}

#[test]
fn type3_glyphs_inherited_into_forms_hold_content_within_the_budget() {
    let _heavy = heavy();
    let form = padded(120, " BT 10 10 Td (a) Tj ET");
    let (drawn, stats) = render(&type3_chain(1, &form));
    // The glyph: a 20-point square at (10, 10).
    assert_eq!(pixel(&drawn.expect("render"), 20, 80), [0, 0, 0]);
    assert!(stats.content_peak >= 120 << 20, "{stats:?}");
    let (result, stats) = render(&type3_chain(8, &form));
    assert!(refused_with(&result, TOO_LARGE), "{result:?}");
    assert!(
        (120 << 20..=HELD).contains(&stats.content_peak),
        "{stats:?}"
    );
}

#[test]
fn soft_masks_kept_with_their_graphics_states_hold_content_within_the_budget() {
    // hayro decodes a soft mask's group when `gs` sets it and keeps it with
    // the state: eight nested states each setting a mask of 60 MiB held
    // them all (the walk counts each mask where it is set). Render and text
    // (hayro sets soft masks there too) are refused within the budget; one
    // mask draws.
    let _heavy = heavy();
    let group = deflated(
        "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Group << /S /Transparency /CS /DeviceGray >>",
        &padded(60, " 1 g 0 0 100 100 re f"),
    );
    let masked = |levels: usize| {
        page(
            "<< /ExtGState << /S << /SMask << /Type /Mask /S /Luminosity /G 5 0 R >> >> >> >>",
            &format!(
                "{}0 0 50 50 re f{}",
                "q /S gs ".repeat(levels),
                " Q".repeat(levels)
            ),
            vec![group.clone()],
        )
    };
    let (drawn, stats) = render(&masked(2));
    assert_eq!(pixel(&drawn.expect("render"), 25, 75), [0, 0, 0]);
    assert!(stats.content_peak >= 120 << 20, "{stats:?}");
    let doc = masked(8);
    let (result, stats) = render(&doc);
    assert!(refused_with(&result, TOO_LARGE), "{result:?}");
    assert!(
        (120 << 20..=HELD).contains(&stats.content_peak),
        "{stats:?}"
    );
    assert!(matches!(
        doc.structured_text(0),
        Err(ViewError::TooComplex(message)) if message == TOO_LARGE
    ));
    // A text reader keeps one cache for many pages; the next page starts
    // its content budget afresh.
    let resources =
        "<< /ExtGState << /S << /SMask << /Type /Mask /S /Luminosity /G 7 0 R >> >> >> >>";
    let two = ViewDocument::open(
        pdf(&[
            obj("<< /Type /Catalog /Pages 2 0 R >>"),
            obj("<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>"),
            obj(&format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources {resources} /Contents 5 0 R >>"
            )),
            obj("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents 6 0 R >>"),
            flate("", format!("{}0 0 50 50 re f{}", "q /S gs ".repeat(8), " Q".repeat(8)).as_bytes()),
            flate("", b"0 0 50 50 re f"),
            group,
        ]),
        "",
    )
    .expect("open");
    let reader = two.text_reader();
    assert!(reader.structured_text(0).is_err());
    assert!(reader.structured_text(1).is_ok());
}

#[test]
fn inherited_cells_read_again_and_again_are_held_to_the_read_budget() {
    // A tiny form drawn 300 times fills with the inherited pattern, whose
    // cell draws a form of 4 MiB: hayro decoded it on every fill (the
    // review's 60 MiB drawn 30 times: 1.8 GB to inflate, uncounted by the
    // walk). A page reads at most 1 GiB.
    let _heavy = heavy();
    let big = deflated(FORM, &padded(4, " 0 0 5 5 re f"));
    let repeated = |times: usize| {
        let content = format!("/Pattern cs /P scn {}", "/G1 Do ".repeat(times));
        page(
            "<< /Pattern << /P 6 0 R >> /XObject << /G1 5 0 R >> >>",
            &content,
            vec![
                flate(FORM, b"0 0 10 10 re f"),
                flate(
                    &format!("{CELL} /Resources << /XObject << /G 7 0 R >> >>"),
                    b"/G Do",
                ),
                big.clone(),
            ],
        )
    };
    let (drawn, _) = render(&repeated(2));
    assert_eq!(pixel(&drawn.expect("render"), 2, 97), [0, 0, 0]);
    let (result, stats) = render(&repeated(300));
    assert!(refused_with(&result, TOO_MUCH_WORK), "{result:?}");
    assert!(stats.content_peak <= HELD, "{stats:?}");
}
