// SPDX-License-Identifier: AGPL-3.0-or-later
//! Rendering never stops the engine and stays within its budgets on hostile
//! pages: tint transforms whose outputs do not fit their alternate space,
//! tiling patterns with huge or negative steps, meshes, images too wide for a
//! pixmap or drawn far smaller than their samples, the render's pixmap
//! budget, a page written directly into /Kids and marked content looked up by
//! MCID. Every document is generated here and small: each input stays cheap
//! even for the unpatched renderer (vendor/*/PATCH.md), so a regression fails
//! instead of exhausting memory; budgets are proved with small budgets.
use std::sync::Arc;
use tandem_pdf_engine::app::AppDocument;
use tandem_pdf_engine::view::{RenderBudgets, RenderOptions, RgbaImage, ViewDocument, ViewError};

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

fn flate(dict: &str, data: &[u8]) -> Vec<u8> {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
    stream(&format!("{dict} /Filter /FlateDecode"), &packed)
}

/// One page of `width` × `height` with `resources` (dictionary text) and
/// `content`; `extra` objects follow as 5, 6, ...
fn one_page(
    width: f64,
    height: f64,
    resources: &str,
    content: &[u8],
    extra: &[Vec<u8>],
) -> Vec<u8> {
    let mut objects = vec![
        obj("<< /Type /Catalog /Pages 2 0 R >>"),
        obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        obj(&format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources {resources} /Contents 4 0 R >>"
        )),
        stream("", content),
    ];
    objects.extend_from_slice(extra);
    pdf(&objects)
}

fn render(bytes: Vec<u8>, scale: f64) -> RgbaImage {
    ViewDocument::open(bytes, "")
        .expect("open")
        .render(0, scale, RenderOptions::default())
        .expect("render")
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * image.width + x) * 4) as usize;
    [image.pixels[at], image.pixels[at + 1], image.pixels[at + 2]]
}

fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

/// A page filling [10 10 90 90] in colour space /CS0 with tint `tint`.
fn tint_page(space: &str, function: Vec<u8>, tint: &str) -> Vec<u8> {
    one_page(
        100.0,
        100.0,
        &format!("<< /ColorSpace << /CS0 {space} >> >>"),
        format!("/CS0 cs {tint} scn 10 10 80 80 re f").as_bytes(),
        &[function],
    )
}

#[test]
fn a_tint_transform_leaving_extra_values_draws_the_top_ones() {
    // The program leaves four values (t 0 t 1) for a three-component
    // alternate; MuPDF 1.28.1 takes the top three, (0, t, 1): (0, 51, 255)
    // measured. hayro panicked copying four values into three.
    let function = stream(
        "/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1]",
        b"{dup 0 exch 1}",
    );
    let image = render(
        tint_page("[/Separation /Custom /DeviceRGB 5 0 R]", function, "0.2"),
        1.0,
    );
    assert!(
        near(pixel(&image, 50, 50), [0, 51, 255]),
        "{:?}",
        pixel(&image, 50, 50)
    );
}

#[test]
fn tint_outputs_are_cut_or_padded_to_the_alternate_space() {
    // Four outputs into RGB: the first three (MuPDF 255, 127, 0).
    let four = obj("<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [1 0.5 0 1] /N 1 >>");
    let image = render(
        tint_page("[/Separation /A /DeviceRGB 5 0 R]", four, "1"),
        1.0,
    );
    assert!(
        near(pixel(&image, 50, 50), [255, 128, 0]),
        "{:?}",
        pixel(&image, 50, 50)
    );
    // Nothing left on the stack: zeros, black (MuPDF draws stale values).
    let none = stream(
        "/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1]",
        b"{pop}",
    );
    let image = render(
        tint_page("[/Separation /A /DeviceRGB 5 0 R]", none, "1"),
        1.0,
    );
    assert!(
        near(pixel(&image, 50, 50), [0, 0, 0]),
        "{:?}",
        pixel(&image, 50, 50)
    );
    // One value where RGB needs three: (v, 0, 0).
    let one = stream("/FunctionType 4 /Domain [0 1] /Range [0 1]", b"{}");
    let image = render(
        tint_page("[/Separation /A /DeviceRGB 5 0 R]", one, "1"),
        1.0,
    );
    assert!(
        near(pixel(&image, 50, 50), [255, 0, 0]),
        "{:?}",
        pixel(&image, 50, 50)
    );
    // Three outputs into CMYK: K is 0, so cyan.
    let three = obj("<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0] /C1 [1 0 0] /N 1 >>");
    let cyan = render(
        tint_page("[/Separation /A /DeviceCMYK 5 0 R]", three, "1"),
        1.0,
    );
    let c = pixel(&cyan, 50, 50);
    assert!(c[0] < 64 && c[2] > 160, "{c:?}");
    // DeviceN of two colorants into RGB with five values left: the top three.
    let program = stream(
        "/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1]",
        b"{1 0 0}",
    );
    let image = render(
        tint_page("[/DeviceN [/A /B] /DeviceRGB 5 0 R]", program, "1 0"),
        1.0,
    );
    assert!(
        near(pixel(&image, 50, 50), [255, 0, 0]),
        "{:?}",
        pixel(&image, 50, 50)
    );
}

#[test]
fn an_image_in_a_mismatched_separation_space_is_drawn() {
    let function = stream(
        "/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1]",
        b"{dup 0 exch 1}",
    );
    let samples: Vec<u8> = (0..64u8).map(|v| v * 4).collect();
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 8 /BitsPerComponent 8 /ColorSpace [/Separation /Custom /DeviceRGB 5 0 R]",
        &samples,
    );
    let bytes = one_page(
        100.0,
        100.0,
        "<< /XObject << /Im0 6 0 R >> >>",
        b"q 80 0 0 80 10 10 cm /Im0 Do Q",
        &[function, image],
    );
    let image = render(bytes, 1.0);
    let p = pixel(&image, 50, 50);
    assert!(p[0] < 8 && p[2] > 247 && p[1] > 0, "{p:?}");
}

#[test]
fn a_short_devicen_image_draws_its_rows_and_padding() {
    // Two 4-bit colorants into RGB as (a, b, 0), one row of red, red, green,
    // green where four are declared: the missing rows are zeros, black, as
    // MuPDF 1.28.1 draws them. Equal pixels reuse one tint evaluation (a
    // 25-byte DeviceN image declaring 64 × 100,000 samples took 3 s).
    let function = stream(
        "/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1]",
        b"{0}",
    );
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /BitsPerComponent 4 /ColorSpace [/DeviceN [/A /B] /DeviceRGB 5 0 R]",
        &[0xF0, 0xF0, 0x0F, 0x0F],
    );
    let bytes = one_page(
        40.0,
        40.0,
        "<< /XObject << /Im0 6 0 R >> >>",
        b"q 40 0 0 40 0 0 cm /Im0 Do Q",
        &[function, image],
    );
    let image = render(bytes, 1.0);
    for (x, expected) in [
        (5, [255, 0, 0]),
        (15, [255, 0, 0]),
        (25, [0, 255, 0]),
        (35, [0, 255, 0]),
    ] {
        assert_eq!(pixel(&image, x, 5), expected, "{x}");
    }
    for (x, y) in [(5, 25), (35, 35)] {
        assert_eq!(pixel(&image, x, y), [0, 0, 0], "{x} {y}");
    }
}

/// A tiling pattern drawing a red square over its whole `bbox`.
fn red_pattern(bbox: &str, x_step: f64, y_step: f64, matrix: &str) -> Vec<u8> {
    stream(
        &format!(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox {bbox} /XStep {x_step} /YStep {y_step} /Matrix {matrix} /Resources << >>"
        ),
        format!("1 0 0 rg {} re f", bbox.trim_matches(['[', ']'])).as_bytes(),
    )
}

#[test]
fn a_tiling_step_past_vellos_coordinates_is_drawn() {
    // A 90,000-unit step asked hayro for a 65,535-pixel-wide cell pixmap,
    // which overflowed vello's 16-bit tile coordinates (the panic of pdf.js
    // tiling-pattern-large-steps.pdf); the row step keeps the pixmap small.
    let pattern = red_pattern("[0 0 300 20]", 90000.0, 30.0, "[1 0 0 1 0 0]");
    let bytes = one_page(
        400.0,
        40.0,
        "<< /Pattern << /P0 5 0 R >> >>",
        b"/Pattern cs /P0 scn 0 0 400 40 re f",
        &[pattern],
    );
    let image = render(bytes, 1.0);
    // Rows of cells at y 0-20 and 30-40 (image rows 20-40 and 0-10).
    for (x, y) in [(10, 30), (290, 30), (150, 5)] {
        let p = pixel(&image, x, y);
        assert!(p[0] > 200 && p[1] < 80, "{x} {y}: {p:?}");
    }
    assert_eq!(pixel(&image, 150, 15), [255, 255, 255]);
    assert_eq!(pixel(&image, 350, 30), [255, 255, 255]);
}

#[test]
fn a_single_visible_cell_is_drawn_where_the_lattice_puts_it() {
    // Cells of 10 units every 100: the area [95 95 120 120] meets only the
    // cell at (100, 100).
    let pattern = red_pattern("[0 0 10 10]", 100.0, 100.0, "[1 0 0 1 0 0]");
    let bytes = one_page(
        200.0,
        200.0,
        "<< /Pattern << /P0 5 0 R >> >>",
        b"/Pattern cs /P0 scn 95 95 25 25 re f",
        &[pattern],
    );
    let image = render(bytes, 1.0);
    // Page y up; pixel rows down from 200.
    assert_eq!(pixel(&image, 105, 94), [255, 0, 0]);
    assert_eq!(pixel(&image, 97, 94), [255, 255, 255]);
    assert_eq!(pixel(&image, 112, 94), [255, 255, 255]);
    assert_eq!(pixel(&image, 105, 87), [255, 255, 255]);
    // Repeated cells are still drawn where several are visible.
    let pattern = red_pattern("[0 0 10 10]", 100.0, 100.0, "[1 0 0 1 0 0]");
    let bytes = one_page(
        200.0,
        200.0,
        "<< /Pattern << /P0 5 0 R >> >>",
        b"/Pattern cs /P0 scn 0 0 200 200 re f",
        &[pattern],
    );
    let image = render(bytes, 1.0);
    for (x, y) in [(5, 195), (105, 195), (5, 95), (105, 95)] {
        assert_eq!(pixel(&image, x, y), [255, 0, 0], "{x} {y}");
    }
    assert_eq!(pixel(&image, 50, 150), [255, 255, 255]);
}

#[test]
fn negative_steps_place_cells_as_their_magnitudes_do() {
    // A cell red on its left half and blue on its right, at negative steps:
    // MuPDF 1.28.1 draws it unmirrored at multiples of the step's magnitude;
    // hayro mirrored it. Both the repeated and the single-cell paths.
    for (x_step, y_step) in [(-100.0, 100.0), (-40.0, -40.0), (100.0, -100.0)] {
        let pattern = stream(
            &format!(
                "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 30 10] /XStep {x_step} /YStep {y_step} /Matrix [1 0 0 1 10 10] /Resources << >>"
            ),
            b"1 0 0 rg 0 0 15 10 re f 0 0 1 rg 15 0 15 10 re f",
        );
        for area in ["0 0 200 200", "5 5 50 20"] {
            let bytes = one_page(
                200.0,
                200.0,
                "<< /Pattern << /P0 5 0 R >> >>",
                format!("/Pattern cs /P0 scn {area} re f").as_bytes(),
                std::slice::from_ref(&pattern),
            );
            let image = render(bytes, 1.0);
            let case = format!("{x_step} {y_step} {area}");
            assert_eq!(pixel(&image, 15, 185), [255, 0, 0], "{case}");
            assert_eq!(pixel(&image, 35, 185), [0, 0, 255], "{case}");
        }
    }
}

#[test]
fn a_repeating_pattern_with_steps_past_the_tile_edge_is_drawn_reduced() {
    // Three cells 20,000 pixels apart are visible: the step pixmap is
    // reduced to the tile edge, not 20,000 pixels wide.
    let pattern = red_pattern("[0 0 100 10]", 20000.0, 10.0, "[1 0 0 1 0 0]");
    let bytes = one_page(
        60000.0,
        20.0,
        "<< /Pattern << /P0 5 0 R >> >>",
        b"/Pattern cs /P0 scn 0 0 60000 20 re f",
        &[pattern],
    );
    let image = render(bytes, 1.0);
    for x in [50, 20050, 40050] {
        let p = pixel(&image, x, 15);
        assert!(p[0] > 200 && p[1] < 80, "{x}: {p:?}");
    }
    assert_eq!(pixel(&image, 10000, 15), [255, 255, 255]);
}

/// A pattern whose `cell` × `cell` cell is white with a black bar one unit
/// wide at its middle, repeated every `step`.
fn bar_pattern(cell: f64, step: f64) -> Vec<u8> {
    stream(
        &format!(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 {cell} {cell}] /XStep {step} /YStep {step} /Resources << >>"
        ),
        format!(
            "1 g 0 0 {cell} {cell} re f 0 g {} 0 1 {cell} re f",
            cell / 2.0
        )
        .as_bytes(),
    )
}

/// The darkest red level of row `y` between columns `x0` and `x1`.
fn darkest(image: &RgbaImage, y: u32, x0: u32, x1: u32) -> u8 {
    (x0..x1).map(|x| pixel(image, x, y)[0]).min().unwrap_or(255)
}

/// The app's budgets with `pixels` for the render's pixmaps.
fn pixmap_budget(pixels: f64) -> RenderBudgets {
    RenderBudgets {
        pixmap_pixels: pixels,
        ..RenderBudgets::default()
    }
}

/// The app's budgets with `samples` for one image.
fn image_budget(samples: u64) -> RenderBudgets {
    RenderBudgets {
        image_samples: samples,
        ..RenderBudgets::default()
    }
}

fn is_over_budget(result: &Result<RgbaImage, ViewError>) -> bool {
    matches!(result, Err(ViewError::TooComplex(_)))
}

#[test]
fn pattern_pixmaps_past_the_render_budget_are_drawn_reduced_then_refused() {
    // One 100-unit cell repeated over the page at scale 2: a 40,000-pixel
    // step pixmap. Under a budget of 5,000 pixels it keeps 5,000 and its
    // one-unit bar blurs; under 1,000 (less than the 4,096 a pixmap is
    // reduced to at least) the render is refused; under the default it is
    // drawn sharp.
    let bytes = one_page(
        100.0,
        100.0,
        "<< /Pattern << /P0 5 0 R >> >>",
        b"/Pattern cs /P0 scn 0 0 100 100 re f",
        &[bar_pattern(100.0, 100.0)],
    );
    let view = ViewDocument::open(bytes, "").expect("open");
    let sharp = view
        .render(0, 2.0, RenderOptions::default())
        .expect("render");
    assert!(darkest(&sharp, 100, 90, 110) < 16);
    let reduced = view
        .render_with_budgets(0, 2.0, RenderOptions::default(), pixmap_budget(5000.0))
        .expect("render");
    let bar = darkest(&reduced, 100, 90, 110);
    assert!((32..224).contains(&bar), "{bar}");
    let refused = view.render_with_budgets(0, 2.0, RenderOptions::default(), pixmap_budget(1000.0));
    assert!(is_over_budget(&refused));
}

#[test]
fn single_cells_keep_their_pixmaps_small() {
    // Sixty patterns each holding one 100 × 140 cell every 480 units, each
    // painted where one cell shows: drawn alone, the cells take about
    // 15,000 pixels each and stay sharp within a 2-million-pixel budget,
    // where step pixmaps (230,000 pixels each) would exhaust it.
    let mut extra = Vec::new();
    let mut names = String::new();
    let mut content = Vec::new();
    for i in 0..60 {
        extra.push(stream(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 100 140] /XStep 480 /YStep 480 /Resources << >>",
            b"1 g 0 0 100 140 re f 0 g 50 0 1 140 re f",
        ));
        names.push_str(&format!("/P{i} {} 0 R ", 5 + i));
        content.extend_from_slice(
            format!("q /Pattern cs /P{i} scn 40 {} 20 10 re f Q\n", 10 + i * 2).as_bytes(),
        );
    }
    let bytes = one_page(
        200.0,
        200.0,
        &format!("<< /Pattern << {names}>> >>"),
        &content,
        &extra,
    );
    let view = ViewDocument::open(bytes, "").expect("open");
    let image = view
        .render_with_budgets(0, 1.0, RenderOptions::default(), pixmap_budget(2_000_000.0))
        .expect("render");
    // The last pattern's bar, at page y 128-138.
    assert!(darkest(&image, 200 - 133, 45, 56) < 16);
}

/// A `size` × `size` one-bit checkerboard of single samples.
fn checkerboard(size: usize) -> Vec<u8> {
    let row = |y: usize| -> Vec<u8> {
        (0..size.div_ceil(8))
            .map(|_| if y % 2 == 0 { 0b1010_1010 } else { 0b0101_0101 })
            .collect()
    };
    (0..size).flat_map(row).collect()
}

#[test]
fn draws_of_one_image_share_its_pixmap_and_others_are_budgeted() {
    // Six draws of 128 × 128 checkerboards at two pixels a sample keep the
    // image's own 16,384 pixels. Six draws of one image share one pixmap,
    // so all stay sharp under a budget of 20,000 pixels. Three different
    // images under 40,000: the third keeps the 7,232 left and averages its
    // checkers to grey; six are refused once the budget is spent.
    let page = |images: usize| {
        let mut content = Vec::new();
        let mut names = String::new();
        for i in 0..images {
            names.push_str(&format!("/Im{i} {} 0 R ", 5 + i));
        }
        for i in 0..6 {
            content.extend_from_slice(
                format!(
                    "q 256 0 0 256 {} {} cm /Im{} Do Q\n",
                    (i % 3) * 256,
                    (i / 3) * 256,
                    i % images
                )
                .as_bytes(),
            );
        }
        // The images differ only in a key the drawing ignores.
        let extra: Vec<Vec<u8>> = (0..images)
            .map(|i| {
                flate(
                    &format!(
                        "/Type /XObject /Subtype /Image /Width 128 /Height 128 /BitsPerComponent 1 /ColorSpace /DeviceGray /Name /I{i}"
                    ),
                    &checkerboard(128),
                )
            })
            .collect();
        let view = one_page(
            768.0,
            512.0,
            &format!("<< /XObject << {names}>> >>"),
            &content,
            &extra,
        );
        ViewDocument::open(view, "").expect("open")
    };
    let sample = |image: &RgbaImage, draw: u32| {
        let (x0, y0) = ((draw % 3) * 256, 512 - (draw / 3 + 1) * 256);
        pixel(image, x0 + 101, y0 + 101)[0]
    };
    let shared = page(1)
        .render_with_budgets(0, 1.0, RenderOptions::default(), pixmap_budget(20_000.0))
        .expect("render");
    for draw in 0..6 {
        assert!(matches!(sample(&shared, draw), 0..=8 | 247..=255), "{draw}");
    }
    let three = page(3)
        .render_with_budgets(0, 1.0, RenderOptions::default(), pixmap_budget(40_000.0))
        .expect("render");
    assert!(matches!(sample(&three, 0), 0..=8 | 247..=255));
    let grey = sample(&three, 2);
    assert!((64..192).contains(&grey), "{grey}");
    let six =
        page(6).render_with_budgets(0, 1.0, RenderOptions::default(), pixmap_budget(40_000.0));
    assert!(is_over_budget(&six));
}

#[test]
fn an_image_wider_than_a_pixmap_is_drawn() {
    // 70,000 samples drawn at one pixel each: hayro cut the width to 16 bits
    // and vello refused the pixmap.
    let samples = vec![0u8; 70_000];
    let image = flate(
        "/Type /XObject /Subtype /Image /Width 70000 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray",
        &samples,
    );
    let bytes = one_page(
        1000.0,
        20.0,
        "<< /XObject << /Im0 5 0 R >> >>",
        b"q 70000 0 0 10 0 5 cm /Im0 Do Q",
        &[image],
    );
    let image = render(bytes, 1.0);
    assert_eq!(pixel(&image, 500, 10), [0, 0, 0]);
}

#[test]
fn images_and_masks_drawn_far_smaller_than_their_samples_are_reduced() {
    // A one-bit soft mask of 4,000 × 400 samples, opaque on its left half,
    // drawn at 100 × 10 pixels (pdf.js issue16263.pdf: 34,862 × 4,332), and
    // the same mask as 8-bit samples with an inverted /Decode; then an RGB
    // image of 2,000 × 200, red on the left and blue on the right.
    let (w, h) = (4000usize, 400usize);
    let row: Vec<u8> = (0..w / 8)
        .map(|i| if i < w / 16 { 0xFF } else { 0 })
        .collect();
    let one_bit: Vec<u8> = row.iter().copied().cycle().take(row.len() * h).collect();
    let bytes_row: Vec<u8> = (0..w).map(|i| if i < w / 2 { 0 } else { 255 }).collect();
    let eight_bit: Vec<u8> = bytes_row.iter().copied().cycle().take(w * h).collect();
    for smask in [
        flate(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /BitsPerComponent 1 /ColorSpace /DeviceGray"
            ),
            &one_bit,
        ),
        flate(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /BitsPerComponent 8 /ColorSpace /DeviceGray /Decode [1 0]"
            ),
            &eight_bit,
        ),
    ] {
        let image = stream(
            "/Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceRGB /SMask 6 0 R",
            &[0u8; 12],
        );
        let mut content = Vec::new();
        for i in 0..8 {
            content.extend_from_slice(
                format!(
                    "q 100 0 0 10 {} {} cm /Im0 Do Q\n",
                    (i % 2) * 110,
                    (i / 2) * 12
                )
                .as_bytes(),
            );
        }
        let bytes = one_page(
            220.0,
            50.0,
            "<< /XObject << /Im0 5 0 R >> >>",
            &content,
            &[image, smask],
        );
        let image = render(bytes, 1.0);
        for (x0, y) in [(0, 45), (110, 9)] {
            let p = pixel(&image, x0 + 20, y);
            assert!(p[0] < 24, "{p:?}");
            assert_eq!(pixel(&image, x0 + 80, y), [255, 255, 255]);
        }
    }
    let rgb: Vec<u8> = (0..2000 * 200)
        .flat_map(|i| {
            if i % 2000 < 1000 {
                [255, 0, 0]
            } else {
                [0, 0, 255]
            }
        })
        .collect();
    let image = flate(
        "/Type /XObject /Subtype /Image /Width 2000 /Height 200 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
        &rgb,
    );
    let bytes = one_page(
        120.0,
        20.0,
        "<< /XObject << /Im0 5 0 R >> >>",
        b"q 100 0 0 10 10 5 cm /Im0 Do Q",
        &[image],
    );
    let image = render(bytes, 1.0);
    assert!(
        near(pixel(&image, 30, 10), [255, 0, 0]),
        "{:?}",
        pixel(&image, 30, 10)
    );
    assert!(
        near(pixel(&image, 90, 10), [0, 0, 255]),
        "{:?}",
        pixel(&image, 90, 10)
    );
}

#[test]
fn a_lattice_of_fewer_than_two_vertices_per_row_is_read_as_two() {
    // MuPDF 1.28.1 warns and reads two; hayro drew nothing for 1 and read
    // empty rows without end for 0 (not generated here: an unpatched
    // renderer would not stop).
    let data = [
        25, 25, 255, 0, 0, 230, 25, 0, 255, 0, 25, 230, 0, 0, 255, 230, 230, 255, 255, 255,
    ];
    for per_row in ["1", "2"] {
        let shading = stream(
            &format!(
                "/ShadingType 5 /ColorSpace /DeviceRGB /BitsPerCoordinate 8 /BitsPerComponent 8 /VerticesPerRow {per_row} /Decode [0 100 0 100 0 1 0 1 0 1]"
            ),
            &data,
        );
        let image = render(
            one_page(
                100.0,
                100.0,
                "<< /Shading << /Sh0 5 0 R >> >>",
                b"/Sh0 sh",
                &[shading],
            ),
            1.0,
        );
        // MuPDF's pixels.
        for ((x, y), expected) in [
            ((30, 70), [125, 65, 64]),
            ((50, 50), [0, 127, 127]),
            ((80, 20), [190, 223, 222]),
        ] {
            let p = pixel(&image, x, y);
            let close = p.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 4);
            assert!(close, "{per_row}: {p:?}");
        }
    }
}

#[test]
fn a_patch_mesh_is_drawn_within_its_clip() {
    // One Coons patch over [10 90]², drawn through a [20 80]² clip at two
    // scales; MuPDF 1.28.1 draws the points below within a level or two.
    let points = [
        (10, 10),
        (10, 40),
        (10, 70),
        (10, 90),
        (40, 90),
        (70, 90),
        (90, 90),
        (90, 70),
        (90, 40),
        (90, 10),
        (70, 10),
        (40, 10),
    ];
    let mut data = vec![0u8];
    for (x, y) in points {
        data.push((f64::from(x) * 2.55).round() as u8);
        data.push((f64::from(y) * 2.55).round() as u8);
    }
    data.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0]);
    let shading = stream(
        "/ShadingType 6 /ColorSpace /DeviceRGB /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 100 0 100 0 1 0 1 0 1]",
        &data,
    );
    let bytes = one_page(
        100.0,
        100.0,
        "<< /Shading << /Sh0 5 0 R >> >>",
        b"q 20 20 60 60 re W n /Sh0 sh Q",
        &[shading],
    );
    let view = ViewDocument::open(bytes, "").expect("open");
    for scale in [1u32, 3] {
        let image = view
            .render(0, f64::from(scale), RenderOptions::default())
            .expect("render");
        for ((x, y), expected) in [
            ((50, 50), [139, 127, 52]),
            ((25, 25), [57, 174, 33]),
            ((75, 75), [212, 175, 32]),
            ((25, 75), [212, 72, 6]),
            ((10, 10), [255, 255, 255]),
        ] {
            let p = pixel(&image, x * scale, y * scale);
            let close = p.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 3);
            assert!(close, "{scale} {x} {y}: {p:?}");
        }
    }
}

#[test]
fn a_mesh_past_its_triangle_budget_is_drawn_in_part() {
    // A free-form mesh of 262,144 tiny triangles, then one covering the
    // right half: the mesh keeps 262,144 triangles (about 60 MB), so the
    // last is not drawn. Vertices: flag, x, y, grey, a byte each.
    let vertex = |data: &mut Vec<u8>, flag: u8, x: u8, y: u8| {
        data.extend_from_slice(&[flag, x, y, 0]);
    };
    let mut data = Vec::new();
    vertex(&mut data, 0, 10, 10);
    vertex(&mut data, 0, 11, 10);
    vertex(&mut data, 0, 10, 11);
    let tiny = [(11, 11), (10, 10), (11, 10)];
    for i in 0..262_143 {
        let (x, y) = tiny[i % 3];
        vertex(&mut data, 1, x, y);
    }
    let big = |data: &mut Vec<u8>| {
        vertex(data, 0, 140, 10);
        vertex(data, 0, 250, 10);
        vertex(data, 0, 140, 250);
    };
    big(&mut data);
    let dict = "/ShadingType 4 /ColorSpace /DeviceGray /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 100 0 100 0 1]";
    let page = |shading| {
        one_page(
            100.0,
            100.0,
            "<< /Shading << /Sh0 5 0 R >> >>",
            b"/Sh0 sh",
            &[shading],
        )
    };
    let image = render(page(flate(dict, &data)), 1.0);
    assert_eq!(pixel(&image, 65, 80), [255, 255, 255]);
    // Within the budget the same triangle is drawn.
    let mut data = Vec::new();
    big(&mut data);
    let image = render(page(stream(dict, &data)), 1.0);
    assert_eq!(pixel(&image, 65, 80), [0, 0, 0]);
}

#[test]
fn content_encoded_as_image_samples_is_left_out_or_refused() {
    // Page content through an image filter decodes to samples, not
    // operators, and hayro decodes it whole first (pdf.js bomb_giant.pdf:
    // three JBIG2 streams declaring 100,000 × 100,000 pixels, 1.5 GB). The
    // app's view leaves such page streams out and draws the rest, as MuPDF
    // 1.28.1 draws that file; the view of other bytes refuses the page, and
    // a form so encoded is refused in either. The streams here hold a few
    // bytes, so nothing large is decoded either way.
    for filter in ["/JBIG2Decode", "[/FlateDecode /DCTDecode]", "/CCF"] {
        let bytes = pdf(&[
            obj("<< /Type /Catalog /Pages 2 0 R >>"),
            obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            obj(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents [4 0 R 5 0 R] >>",
            ),
            stream("", b"0 0 1 rg 10 10 30 30 re f"),
            stream(&format!("/Filter {filter}"), b"\x00\x00\x00\x00\x30"),
        ]);
        let mut doc = AppDocument::load(Arc::new(bytes.clone()), b"").expect("load");
        let image = doc
            .view()
            .expect("view")
            .render(0, 1.0, RenderOptions::default())
            .expect("render");
        assert_eq!(pixel(&image, 20, 75), [0, 0, 255], "{filter}");
        let view = ViewDocument::open(bytes, "").expect("open");
        let refused = view.render(0, 1.0, RenderOptions::default());
        assert!(matches!(refused, Err(ViewError::TooComplex(_))), "{filter}");
    }
    let form = one_page(
        100.0,
        100.0,
        "<< /XObject << /X 5 0 R >> >>",
        b"0 0 1 rg 10 10 30 30 re f /X Do",
        &[stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Filter /JBIG2Decode",
            b"\x00\x00\x00\x00\x30",
        )],
    );
    let mut doc = AppDocument::load(Arc::new(form), b"").expect("load");
    let refused = doc
        .view()
        .expect("view")
        .render(0, 1.0, RenderOptions::default());
    assert!(matches!(refused, Err(ViewError::TooComplex(_))));
}

/// `pages` pages sharing one resources dictionary of `fonts` fonts.
fn shared_resources(pages: usize, fonts: usize) -> Vec<u8> {
    let kids: String = (0..pages).map(|i| format!("{} 0 R ", 5 + i)).collect();
    let font_names: String = (0..fonts).map(|i| format!("/F{i} 4 0 R ")).collect();
    let mut objects = vec![
        obj("<< /Type /Catalog /Pages 2 0 R >>"),
        obj(&format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>")),
        obj(&format!("<< /Font << {font_names}>> >>")),
        obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
    ];
    for _ in 0..pages {
        objects.push(obj(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources 3 0 R >>",
        ));
    }
    pdf(&objects)
}

#[test]
fn pages_sharing_resources_within_the_view_budget_open() {
    // 10 pages × 4,000 fonts: 40,000 entries, far inside the budget the view
    // checks before hayro opens it (`app::view_cost`, tested there).
    let mut doc = AppDocument::load(Arc::new(shared_resources(10, 4000)), b"").expect("load");
    let view = doc.view().expect("view");
    assert_eq!(view.page_count(), 10);
    view.render(9, 1.0, RenderOptions::default())
        .expect("render");
}

#[test]
fn a_page_written_into_kids_inherits_its_parents_media_box() {
    // The direct page's /Parent is the object holding it, which hayro's
    // reader took for a cycle: the page drew as Letter (pyHanko
    // page-tree-direct-kid.pdf).
    let bytes = pdf(&[
        obj("<< /Type /Catalog /Pages 2 0 R >>"),
        obj(
            "<< /Type /Pages /Count 2 /MediaBox [0 0 595 842] /Kids [4 0 R << /Type /Page /Parent 2 0 R /Resources << >> /Contents 3 0 R >>] >>",
        ),
        stream("", b""),
        obj("<< /Type /Page /Parent 2 0 R /Resources << >> /Contents 3 0 R >>"),
    ]);
    let view = ViewDocument::open(bytes.clone(), "").expect("open");
    for page in 0..2 {
        assert_eq!(
            view.page_size(page).expect("size"),
            (595.0, 842.0),
            "page {page}"
        );
    }
    let mut doc = AppDocument::load(Arc::new(bytes), b"").expect("load");
    let image = doc
        .view()
        .expect("view")
        .render(1, 1.0, RenderOptions::default())
        .expect("render");
    assert_eq!((image.width, image.height), (595, 842));
}

#[test]
fn marked_content_finds_its_structure_element_by_mcid() {
    // 3,000 marked-content sequences look their structure element up in the
    // page's parent array; the last one's /ActualText replaces its text.
    // Each lookup walked the array, resolving every element before it
    // (quadratic: govdocs1 000748.pdf took 27 s for its text, now 0.2 s).
    let n = 3000;
    let mut objects = vec![
        obj("<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 5 0 R >>"),
        obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        obj(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /StructParents 0 /Resources << /Font << /F1 6 0 R >> >> /Contents 4 0 R >>",
        ),
    ];
    let mut content = b"BT /F1 2 Tf ".to_vec();
    for i in 0..n {
        content.extend_from_slice(
            format!(
                "/Span <</MCID {i}>> BDC 1 0 0 1 {} {} Tm (x) Tj EMC ",
                10 + (i % 100) * 5,
                790.0 - (i / 100) as f64 * 3.9
            )
            .as_bytes(),
        );
    }
    content.extend_from_slice(b"ET");
    objects.push(stream("", &content));
    let elements: String = (0..n).map(|i| format!("{} 0 R ", 9 + i)).collect();
    objects.push(obj(
        "<< /Type /StructTreeRoot /ParentTree 7 0 R /K 8 0 R >>",
    ));
    objects.push(obj(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ));
    objects.push(obj(&format!("<< /Nums [0 [{elements}]] >>")));
    objects.push(obj("<< /Type /StructElem /S /Document /P 5 0 R >>"));
    for i in 0..n {
        let actual = if i == n - 1 { " /ActualText (Z)" } else { "" };
        objects.push(obj(&format!(
            "<< /Type /StructElem /S /Span /P 8 0 R /Pg 3 0 R /K {i}{actual} >>"
        )));
    }
    let view = ViewDocument::open(pdf(&objects), "").expect("open");
    let text = view.structured_text(0).expect("text").as_text();
    assert_eq!(text.matches('Z').count(), 1, "{text:?}");
    assert_eq!(text.matches('x').count(), n - 1);
}

#[test]
fn a_page_as_long_as_a_render_allows_drawing_at_its_edges() {
    // vello rounds tiles and spans up in 16-bit pixel coordinates: a
    // 65,535-pixel edge overflowed when drawn at its end.
    let samples = [0u8, 255, 0, 255];
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceGray",
        &samples,
    );
    let pattern = red_pattern("[0 0 3 3]", 5.0, 5.0, "[1 0 0 1 0 0]");
    let resources = "<< /XObject << /Im0 5 0 R >> /Pattern << /P0 6 0 R >> >>";
    for (long, wide) in [(65280.0, true), (65280.0, false), (65281.0, true)] {
        let (w, h): (f64, f64) = if wide { (long, 6.0) } else { (6.0, long) };
        let content = format!(
            "0 0 1 rg {x} {y} 10 10 re f 5 w {x} {y} m {w} {h} l S q 20 0 0 20 {x} {y} cm /Im0 Do Q /Pattern cs /P0 scn {x} {y} 30 30 re f",
            x = w - 4.0,
            y = h - 4.0
        );
        let bytes = one_page(
            w,
            h,
            resources,
            content.as_bytes(),
            &[image.clone(), pattern.clone()],
        );
        let view = ViewDocument::open(bytes, "").expect("open");
        match view.render(0, 1.0, RenderOptions::default()) {
            Ok(image) => {
                assert!(long <= 65280.0);
                assert_ne!(pixel(&image, image.width - 1, 0), [255, 255, 255]);
            }
            Err(e) => {
                assert!(long > 65280.0);
                assert_eq!(e, ViewError::BadScale);
            }
        }
    }
}

#[test]
fn drawing_past_the_render_scene_budget_is_refused() {
    // vello keeps every fill of a page until the page is composed (20,000
    // page-sized fills of a 600 × 600 page held 540 MB); a fill of a 200 ×
    // 200 page is estimated at about 6.6 KB. Under a budget of 1 MB, 100
    // such fills draw and 200 are refused.
    let page = |fills: usize| {
        let content: String = (0..fills)
            .map(|i| format!("{} g 0 0 200 200 re f ", i % 2))
            .collect();
        let bytes = one_page(200.0, 200.0, "<< >>", content.as_bytes(), &[]);
        ViewDocument::open(bytes, "").expect("open")
    };
    let budget = RenderBudgets {
        scene_bytes: 1e6,
        ..RenderBudgets::default()
    };
    let image = page(100)
        .render_with_budgets(0, 1.0, RenderOptions::default(), budget)
        .expect("render");
    assert_eq!(pixel(&image, 100, 100), [255, 255, 255]);
    assert!(is_over_budget(&page(200).render_with_budgets(
        0,
        1.0,
        RenderOptions::default(),
        budget
    )));
}

/// Bytes from hex text.
fn hex(parts: &[&str]) -> Vec<u8> {
    let text: String = parts.concat();
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

/// A 256 × 256 8-bit grey checkerboard of 32-sample squares as a JPEG 2000
/// codestream with five decomposition levels (made for this test with
/// `opj_compress -n 6 -r 200`).
const CHECKER_J2K: &[&str] = &[
    "ff4fff5100290000000001000000010000000000000000000000010000000100000000000000000000010701",
    "01ff52000c00000001000504040001ff5c00134040484850484850484850484850484850ff64002500014372",
    "6561746564206279204f70656e4a5045472076657273696f6e20322e352e34ff90000a0000000000de0001ff",
    "93df00e01234e64475eb03ce167c44631c12b99f8d9610bee5a6c06ea293c301e78302dbc016020dcdd97998",
    "682ece6b27ac8015d8979f8e36783f6d0da1b09b261a983f418e4572dc8ea183f594e73c6044bc335fe8a832",
    "1aa84956a99c5c15d8979f8e36783f6d0d8f20189a9302476880317055bf75e418c332e857c22d8ae93b93c8",
    "e5ec2bc051bb75a84666d93a5ece6ec0216c07e8cd1ca9108be79db7eea4b447f742bf248f23d40488c376c1",
    "74bf7d30bc6b762a7534a47f3055ab57f7faf73e7cfa163d9ba142850ce4948080ffd9",
];

/// A 16 × 16 8-bit grey JPEG, a ramp from black on the left to light grey
/// on the right (made for this test with `cjpeg -grayscale`); its frame
/// header starts at JPEG_SOF.
const RAMP_JPEG: &[&str] = &[
    "ffd8ffe000104a46494600010100000100010000ffdb00430003020203020203030303040303040508050504",
    "04050a070706080c0a0c0c0b0a0b0b0d0e12100d0e110e0b0b1016101113141515150c0f1718161418121415",
    "14ffc0000b080010001001011100ffc4001f0000010501010101010100000000000000000102030405060708",
    "090a0bffc400b5100002010303020403050504040000017d0102030004110512213141061351610722711432",
    "8191a1082342b1c11552d1f02433627282090a161718191a25262728292a3435363738393a43444546474849",
    "4a535455565758595a636465666768696a737475767778797a838485868788898a92939495969798999aa2a3",
    "a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2e3e4e5e6e7e8e9ea",
    "f1f2f3f4f5f6f7f8f9faffda0008010100003f00f8ff00f66dff00975fc2bf557f66dff975fc2bf2abf66dff",
    "00975fc2bf557f66dff975fc2bffd9",
];
const JPEG_SOF: usize = 89;

/// `RAMP_JPEG` with its frame header claiming `width` × `height`.
fn jpeg_claiming(width: u16, height: u16) -> Vec<u8> {
    let mut jpeg = hex(RAMP_JPEG);
    jpeg[JPEG_SOF + 5..JPEG_SOF + 7].copy_from_slice(&height.to_be_bytes());
    jpeg[JPEG_SOF + 7..JPEG_SOF + 9].copy_from_slice(&width.to_be_bytes());
    jpeg
}

/// An embedded JBIG2 stream of one `width` × `height` page whose default
/// pixel is black, with no regions: page information, then end of page.
fn jbig2_page(width: u32, height: u32) -> Vec<u8> {
    let mut data = vec![0, 0, 0, 0, 0x30, 0, 1, 0, 0, 0, 19];
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&[0; 8]);
    data.extend_from_slice(&[0x04, 0, 0]);
    data.extend_from_slice(&[0, 0, 0, 1, 0x31, 0, 1, 0, 0, 0, 0]);
    data
}

/// A page of `size` points drawing image object 5 (`dict`, `data`) over
/// all of it.
fn image_page(size: f64, dict: &str, data: &[u8]) -> ViewDocument {
    let bytes = one_page(
        size,
        size,
        "<< /XObject << /Im0 5 0 R >> >>",
        format!("q {size} 0 0 {size} 0 0 cm /Im0 Do Q").as_bytes(),
        &[stream(
            &format!("/Type /XObject /Subtype /Image {dict}"),
            data,
        )],
    );
    ViewDocument::open(bytes, "").expect("open")
}

#[test]
fn a_jpx_image_its_decoder_reduced_is_not_reduced_again() {
    // Drawn at 16 × 16 pixels, the decoder skips four resolution levels and
    // gives 16 × 16 samples (squares of two); a further box reduction by 4,
    // computed from the declared size, averaged them to flat grey.
    let view = image_page(
        16.0,
        "/Width 256 /Height 256 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /JPXDecode",
        &hex(CHECKER_J2K),
    );
    let image = view
        .render(0, 1.0, RenderOptions::default())
        .expect("render");
    let values: Vec<u8> = image.pixels.chunks_exact(4).map(|p| p[0]).collect();
    let dark = values.iter().filter(|v| **v < 64).count();
    let light = values.iter().filter(|v| **v > 192).count();
    assert!(dark >= 16 && light >= 16, "{dark} {light}");
    // Past its budget the decoder skips more levels instead (256 × 256
    // samples cost 4 bytes each; 2^12 allows 32 × 32 at most): drawn.
    let large = view.render_with_budgets(0, 8.0, RenderOptions::default(), image_budget(1 << 12));
    assert!(large.is_ok(), "{large:?}");
}

#[test]
fn images_past_their_sample_budget_are_decoded_reduced_or_refused() {
    // A 2,000 × 1,000 one-bit image, black on its left half, has 2,000,000
    // samples; under a budget of 2^20 it is decoded reduced a strip at a
    // time when drawn at 100 × 50 pixels (it was left out), and refused
    // when drawn a sample a pixel. MuPDF draws both.
    let (w, h) = (2000usize, 1000usize);
    let row: Vec<u8> = (0..w / 8)
        .map(|i| if i < w / 16 { 0 } else { 0xFF })
        .collect();
    let data: Vec<u8> = row.iter().copied().cycle().take(row.len() * h).collect();
    let dict = format!("/Width {w} /Height {h} /BitsPerComponent 1 /ColorSpace /DeviceGray");
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
    let view = image_page(100.0, &format!("{dict} /Filter /FlateDecode"), &packed);
    let small = image_budget(1 << 20);
    for budgets in [small, RenderBudgets::default()] {
        let image = view
            .render_with_budgets(0, 1.0, RenderOptions::default(), budgets)
            .expect("render");
        assert!(pixel(&image, 20, 50)[0] < 16, "{budgets:?}");
        assert!(pixel(&image, 80, 50)[0] > 240, "{budgets:?}");
    }
    let refused = view.render_with_budgets(0, 20.0, RenderOptions::default(), small);
    assert!(is_over_budget(&refused));

    // An RGB image of 1,000 × 500 (1,500,000 samples), red on its left half
    // and a key colour (blue) masked out by /Mask on its right: reduced with
    // its colour key mask.
    let rgb: Vec<u8> = (0..1000 * 500)
        .flat_map(|i| {
            if i % 1000 < 500 {
                [255, 0, 0]
            } else {
                [0, 0, 255]
            }
        })
        .collect();
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&rgb, 6);
    let view = image_page(
        100.0,
        "/Width 1000 /Height 500 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Mask [0 0 0 0 255 255] /Filter /FlateDecode",
        &packed,
    );
    let image = view
        .render_with_budgets(0, 1.0, RenderOptions::default(), small)
        .expect("render");
    assert!(
        near(pixel(&image, 20, 50), [255, 0, 0]),
        "{:?}",
        pixel(&image, 20, 50)
    );
    assert_eq!(pixel(&image, 80, 50), [255, 255, 255]);

    // A stencil mask of the same 2,000 × 1,000 bits paints its left half;
    // an 8-bit soft mask past the budget is refused (masks other than one
    // bit are unpacked whole).
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
    let bytes = one_page(
        100.0,
        100.0,
        "<< /XObject << /Im0 5 0 R >> >>",
        b"0 0 1 rg q 100 0 0 100 0 0 cm /Im0 Do Q",
        &[stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {w} /Height {h} /BitsPerComponent 1 /ImageMask true /Filter /FlateDecode"
            ),
            &packed,
        )],
    );
    let view = ViewDocument::open(bytes, "").expect("open");
    let image = view
        .render_with_budgets(0, 1.0, RenderOptions::default(), small)
        .expect("render");
    assert_eq!(pixel(&image, 20, 50), [0, 0, 255]);
    assert_eq!(pixel(&image, 80, 50), [255, 255, 255]);
    let smask: Vec<u8> = (0..w * h)
        .map(|i| if i % w < w / 2 { 0 } else { 255 })
        .collect();
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&smask, 6);
    let bytes = one_page(
        100.0,
        100.0,
        "<< /XObject << /Im0 5 0 R >> >>",
        b"q 100 0 0 100 0 0 cm /Im0 Do Q",
        &[
            stream(
                "/Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceGray /SMask 6 0 R",
                &[0; 4],
            ),
            stream(
                &format!(
                    "/Type /XObject /Subtype /Image /Width {w} /Height {h} /BitsPerComponent 8 /ColorSpace /DeviceGray /Decode [1 0] /Filter /FlateDecode"
                ),
                &packed,
            ),
        ],
    );
    let view = ViewDocument::open(bytes, "").expect("open");
    let refused = view.render_with_budgets(0, 1.0, RenderOptions::default(), small);
    assert!(is_over_budget(&refused));
    assert!(view.render(0, 1.0, RenderOptions::default()).is_ok());
}

#[test]
fn decoders_sized_from_declared_sizes_are_held_to_the_budget() {
    // DCT, CCITT and JBIG2 decoders allocate their output from declared
    // sizes before decoding anything: a few bytes asked for gigabytes
    // (and stopped the WASM engine). Under a budget of 2^20 samples,
    // 2,048 × 2,048 is refused before decoding; within it, decoded.
    let small = image_budget(1 << 20);
    let jpeg =
        "/Width 2048 /Height 2048 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode";
    let ccitt = "/Width 2048 /Height 2048 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 2048 /Rows 2048 >>";
    let jbig2 =
        "/Width 2048 /Height 2048 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode";
    for (dict, data) in [
        (jpeg, jpeg_claiming(2048, 2048)),
        (ccitt, vec![0xFF; 6]),
        (jbig2, jbig2_page(2048, 2048)),
    ] {
        let view = image_page(100.0, dict, &data);
        let refused = view.render_with_budgets(0, 1.0, RenderOptions::default(), small);
        assert!(is_over_budget(&refused), "{dict}");
        assert!(
            view.render(0, 1.0, RenderOptions::default()).is_ok(),
            "{dict}"
        );
    }
    // Within the budget they draw: the JPEG's ramp, the JBIG2 page black.
    let view = image_page(
        16.0,
        "/Width 16 /Height 16 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode",
        &hex(RAMP_JPEG),
    );
    let image = view
        .render_with_budgets(0, 1.0, RenderOptions::default(), small)
        .expect("render");
    assert!(pixel(&image, 1, 8)[0] < 40 && pixel(&image, 14, 8)[0] > 180);
    let view = image_page(
        16.0,
        "/Width 16 /Height 16 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /JBIG2Decode",
        &jbig2_page(16, 16),
    );
    let image = view
        .render_with_budgets(0, 1.0, RenderOptions::default(), small)
        .expect("render");
    assert_eq!(pixel(&image, 8, 8), [0, 0, 0]);
    // JBIG2 behind another filter cannot be read before decoding: refused.
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&jbig2_page(16, 16), 6);
    let view = image_page(
        16.0,
        "/Width 16 /Height 16 /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter [/FlateDecode /JBIG2Decode]",
        &packed,
    );
    assert!(is_over_budget(&view.render(
        0,
        1.0,
        RenderOptions::default()
    )));
}

#[test]
fn shading_textures_and_soft_masks_count_against_the_pixmap_budget() {
    // A function-based shading painted 50 times over a 200 × 200 page
    // samples one 40,000-pixel texture the paints share; three different
    // shadings under a budget of 60,000 pixels are refused.
    let function = |n: usize| {
        stream(
            &format!("/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Name /F{n}"),
            b"{pop dup 0}",
        )
    };
    let page = |shadings: usize| {
        let mut names = String::new();
        let mut extra = Vec::new();
        for i in 0..shadings {
            names.push_str(&format!("/Sh{i} {} 0 R ", 5 + 2 * i));
            extra.push(obj(&format!(
                "<< /ShadingType 1 /ColorSpace /DeviceRGB /Domain [0 1 0 1] /Matrix [200 0 0 200 0 0] /Function {} 0 R >>",
                6 + 2 * i
            )));
            extra.push(function(i));
        }
        let content: String = (0..50)
            .map(|i| format!("/Sh{} sh ", i % shadings))
            .collect();
        let bytes = one_page(
            200.0,
            200.0,
            &format!("<< /Shading << {names}>> >>"),
            content.as_bytes(),
            &extra,
        );
        ViewDocument::open(bytes, "").expect("open")
    };
    let budget = pixmap_budget(60_000.0);
    let one = page(1)
        .render_with_budgets(0, 1.0, RenderOptions::default(), budget)
        .expect("render");
    let p = pixel(&one, 150, 100);
    assert!(p[0] > 100 && p[2] < 8, "{p:?}");
    assert!(is_over_budget(&page(3).render_with_budgets(
        0,
        1.0,
        RenderOptions::default(),
        budget
    )));

    // Soft masks hold a byte a pixel (a quarter of a pixmap pixel each):
    // three different 40,000-pixel masks need 30,000 of the budget.
    let masks = |budget: f64| {
        let mut extra = Vec::new();
        let mut states = String::new();
        let mut content = String::new();
        for i in 0..3 {
            states.push_str(&format!("/G{i} {} 0 R ", 5 + 2 * i));
            extra.push(obj(&format!(
                "<< /Type /ExtGState /SMask << /Type /Mask /S /Luminosity /G {} 0 R >> >>",
                6 + 2 * i
            )));
            extra.push(stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 200 200] /Group << /S /Transparency /CS /DeviceGray >>",
                format!("1 g {} 0 50 200 re f", i * 60).as_bytes(),
            ));
            content.push_str(&format!("q /G{i} gs 0 0 1 rg 0 0 200 200 re f Q "));
        }
        let bytes = one_page(
            200.0,
            200.0,
            &format!("<< /ExtGState << {states}>> >>"),
            content.as_bytes(),
            &extra,
        );
        ViewDocument::open(bytes, "")
            .expect("open")
            .render_with_budgets(0, 1.0, RenderOptions::default(), pixmap_budget(budget))
    };
    let image = masks(40_000.0).expect("render");
    assert_eq!(pixel(&image, 25, 100), [0, 0, 255]);
    assert_eq!(pixel(&image, 175, 100), [255, 255, 255]);
    assert!(is_over_budget(&masks(25_000.0)));
}

/// xorshift64*, for generated cases.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }

    fn numbers(&mut self, n: usize) -> String {
        (0..n)
            .map(|_| format!("{} ", self.below(5) as f64 / 4.0))
            .collect()
    }
}

#[test]
fn generated_colour_spaces_and_tint_transforms_render() {
    // Separation and DeviceN over every alternate, with functions of every
    // type whose outputs fit, fall short of or exceed the alternate, used
    // for fills, strokes, text, images and shadings. Images are 2 × 2 and
    // meshes small; wider generators (huge sizes, zero vertices per row) run
    // outside the suite (design/app-corpus-robust.md).
    let alternates = [
        "/DeviceGray",
        "/DeviceRGB",
        "/DeviceCMYK",
        "[/CalRGB << /WhitePoint [0.95 1 1.09] >>]",
        "[/CalGray << /WhitePoint [0.95 1 1.09] >>]",
        "[/Lab << /WhitePoint [0.95 1 1.09] >>]",
        "[/Indexed /DeviceRGB 1 <FF000000FF00>]",
        "/Pattern",
    ];
    let programs = [
        "{}",
        "{pop}",
        "{dup dup}",
        "{dup 0 exch 1}",
        "{1 2 3 4 5 6 7}",
        "{dup 0.5 gt {0} {0 0 0 0} ifelse}",
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..300 {
        let alternate = rng.pick(&alternates);
        let inputs = 1 + rng.below(3);
        let outputs = rng.below(6);
        let domain = "0 1 ".repeat(inputs);
        let function = match rng.below(4) {
            0 => stream(
                &format!(
                    "/FunctionType 4 /Domain [{domain}] /Range [{}]",
                    "0 1 ".repeat(outputs.max(1))
                ),
                rng.pick(&programs).as_bytes(),
            ),
            1 => obj(&format!(
                "<< /FunctionType 2 /Domain [0 1] /C0 [{}] /C1 [{}] /N {} >>",
                rng.numbers(outputs),
                rng.numbers(outputs),
                rng.below(3)
            )),
            2 => {
                let samples: Vec<u8> = (0..2 * outputs.max(1)).map(|_| rng.next() as u8).collect();
                stream(
                    &format!(
                        "/FunctionType 0 /Domain [{domain}] /Range [{}] /Size [{}] /BitsPerSample 8",
                        "0 1 ".repeat(outputs.max(1)),
                        "2 ".repeat(inputs)
                    ),
                    &samples,
                )
            }
            _ => {
                let other = rng.below(5);
                obj(&format!(
                    "<< /FunctionType 3 /Domain [0 1] /Functions [<< /FunctionType 2 /Domain [0 1] /C1 [{}] /N 1 >> << /FunctionType 2 /Domain [0 1] /C1 [{}] /N 1 >>] /Bounds [0.5] /Encode [0 1 0 1] >>",
                    rng.numbers(outputs),
                    rng.numbers(other)
                ))
            }
        };
        let names: String = (0..inputs).map(|i| format!("/C{i} ")).collect();
        let space = if inputs == 1 && rng.below(2) == 0 {
            format!("[/Separation /C0 {alternate} 5 0 R]")
        } else {
            format!("[/DeviceN [{names}] {alternate} 5 0 R]")
        };
        let tint = rng.numbers(inputs);
        let bits = [1, 2, 4, 8, 16][rng.below(5)];
        let samples: Vec<u8> = (0..rng.below(4 * inputs * 2 + 1))
            .map(|_| rng.next() as u8)
            .collect();
        let decode_len = rng.below(2 * inputs + 2);
        let decode = rng.numbers(decode_len);
        let image = stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent {bits} /Decode [{decode}] /ColorSpace 7 0 R"
            ),
            &samples,
        );
        let shading_type = 1 + rng.below(7);
        let mesh: Vec<u8> = (0..rng.below(200)).map(|_| rng.next() as u8).collect();
        let decode_len = rng.below(8);
        let shading = stream(
            &format!(
                "/ShadingType {shading_type} /ColorSpace 7 0 R /Coords [0 0 40 0 10 20] /Domain [0 1 0 1] /Function 5 0 R /Extend [true true] /BitsPerCoordinate {} /BitsPerComponent {} /BitsPerFlag {} /VerticesPerRow {} /Decode [0 40 0 40 {}]",
                [1, 8, 16, 32][rng.below(4)],
                [1, 8, 16][rng.below(3)],
                [2, 8][rng.below(2)],
                1 + rng.below(3),
                rng.numbers(decode_len)
            ),
            &mesh,
        );
        let content = format!(
            "/CS0 cs /CS0 CS {tint} scn {tint} SCN 5 5 30 30 re B BT /F1 12 Tf 5 40 Td (Ab) Tj ET q 20 0 0 20 40 5 cm /Im0 Do Q q 40 40 40 40 re W n /Sh0 sh Q"
        );
        let bytes = one_page(
            100.0,
            100.0,
            "<< /ColorSpace << /CS0 7 0 R >> /XObject << /Im0 6 0 R >> /Shading << /Sh0 8 0 R >> /Font << /F1 9 0 R >> >>",
            content.as_bytes(),
            &[
                function,
                image,
                obj(&space),
                shading,
                obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
            ],
        );
        let view = ViewDocument::open(bytes, "").expect("open");
        let rendered = view.render(0, 0.5, RenderOptions::default());
        assert!(
            rendered.is_ok() || matches!(rendered, Err(ViewError::TooComplex(_))),
            "case {case}"
        );
        view.structured_text(0).expect("text");
    }
}

#[test]
fn generated_tiling_patterns_render() {
    // Steps negative, zero, tiny and up to the page, cells thin or past
    // their steps, skewed and singular matrices, coloured and uncoloured,
    // filled, stroked and shown as text. Every step pixmap the unpatched
    // renderer would ask for stays near 1,000 pixels a side; huge steps and
    // empty or inverted cells (which ask it for 65,535) are the targeted
    // tests' and the outside generator's.
    let steps = ["0", "-5", "0.001", "1", "7", "40", "-30", "100"];
    let boxes = ["[0 0 10 10]", "[-5 -5 5 5]", "[0 0 50 3]", "[0 0 200 200]"];
    let matrices = [
        "[1 0 0 1 0 0]",
        "[0.01 0 0 0.01 0 0]",
        "[2 0 0 2 0 0]",
        "[1 2 3 4 5 6]",
        "[0 0 0 0 0 0]",
        "[1 0 0 -1 0 100]",
        "[5 0 0 0.001 0 0]",
    ];
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for case in 0..200 {
        let paint = 1 + rng.below(2);
        let bbox = rng.pick(&boxes);
        let pattern = stream(
            &format!(
                "/Type /Pattern /PatternType 1 /PaintType {paint} /TilingType 1 /BBox {bbox} /XStep {} /YStep {} /Matrix {} /Resources << >>",
                rng.pick(&steps),
                rng.pick(&steps),
                rng.pick(&matrices)
            ),
            b"0 0 1 rg 0 0 4 4 re f 1 w 0 0 m 8 8 l S",
        );
        let color = if paint == 2 {
            "/Pattern /DeviceRGB"
        } else {
            "/Pattern"
        };
        let content = format!(
            "/CS0 cs /CS0 CS {} /P0 scn {} /P0 SCN 0 0 60 60 re B 4 w 70 10 m 95 90 l S BT /F1 30 Tf 2 Tr 10 70 Td (W) Tj ET",
            if paint == 2 { "1 0 0" } else { "" },
            if paint == 2 { "0 1 0" } else { "" }
        );
        let bytes = one_page(
            100.0,
            100.0,
            &format!(
                "<< /ColorSpace << /CS0 [{color}] >> /Pattern << /P0 5 0 R >> /Font << /F1 6 0 R >> >>"
            ),
            content.as_bytes(),
            &[
                pattern,
                obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"),
            ],
        );
        let view = ViewDocument::open(bytes, "").expect("open");
        for scale in [0.25, 1.0, 2.0] {
            let rendered = view.render(0, scale, RenderOptions::default());
            assert!(
                rendered.is_ok() || matches!(rendered, Err(ViewError::TooComplex(_))),
                "case {case}"
            );
        }
    }
}
