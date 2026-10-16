// SPDX-License-Identifier: AGPL-3.0-or-later
//! Small hostile pages from the integration acceptance review (and the sweep
//! that followed it) stop neither the engine nor the machine: PostScript
//! functions, colour spaces and stitching functions nested thousands deep
//! (they overflowed the WASM stack), sampled function tables, nested
//! graphics states, paths and dash patterns that cost vello gigabytes, and
//! predictor parameters hayro divided by. Every document is generated here
//! and a few kilobytes to a few megabytes; the decoding limit itself
//! (128 MiB) is proved on the WASM build (tests/rust-engine-bounds.test.mjs
//! at the app root), where memory matters.
use tandem_pdf_engine::view::{RenderOptions, RgbaImage, ViewDocument, ViewError};

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

/// A 100 × 100 page with `resources` and Flate `content`; `extra` objects
/// follow as 5, 6, ...
fn page(resources: &str, content: &[u8], extra: Vec<Vec<u8>>) -> ViewDocument {
    let mut objects = vec![
        obj("<< /Type /Catalog /Pages 2 0 R >>"),
        obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        obj(&format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources {resources} /Contents 4 0 R >>"
        )),
        flate("", content),
    ];
    objects.extend(extra);
    ViewDocument::open(pdf(&objects), "").expect("open")
}

fn render(doc: &ViewDocument) -> Result<RgbaImage, ViewError> {
    doc.render(0, 1.0, RenderOptions::default())
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * image.width + x) * 4) as usize;
    [image.pixels[at], image.pixels[at + 1], image.pixels[at + 2]]
}

fn too_complex(result: &Result<RgbaImage, ViewError>) -> bool {
    matches!(result, Err(ViewError::TooComplex(_)))
}

const WHITE: [u8; 3] = [255, 255, 255];
const BLACK: [u8; 3] = [0, 0, 0];

/// A page filled by an axial shading whose function is object 6.
fn shading_page(function: Vec<u8>) -> ViewDocument {
    page(
        "<< /Shading << /Sh 5 0 R >> >>",
        b"/Sh sh",
        vec![
            obj("<< /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function 6 0 R >>"),
            function,
        ],
    )
}

/// A PostScript function mapping everything to 0 whose program carries an
/// unused procedure nested `depth` levels below its own.
fn nested_program(depth: usize) -> Vec<u8> {
    let program = format!("{{ pop 0 {}{} }}", "{ ".repeat(depth), "} ".repeat(depth));
    stream(
        "/FunctionType 4 /Domain [0 1] /Range [0 1]",
        program.as_bytes(),
    )
}

#[test]
fn postscript_procedures_nest_at_most_as_deep_as_mupdf_reads() {
    // MuPDF 1.28.1 draws the shading at 100 levels and refuses its function
    // at 101 (the shading is then not drawn); hayro recursed without a
    // limit, and about 4,000 levels overflowed the WASM stack.
    let at = |depth| render(&shading_page(nested_program(depth))).expect("render");
    assert_eq!(pixel(&at(100), 50, 50), BLACK);
    assert_eq!(pixel(&at(101), 50, 50), WHITE);
    assert_eq!(pixel(&at(20_000), 50, 50), WHITE);
}

#[test]
fn a_postscript_program_past_its_operator_budget_is_refused() {
    let program = |ops: usize| {
        let body = "1 pop ".repeat(ops / 2);
        stream(
            "/FunctionType 4 /Domain [0 1] /Range [0 1]",
            format!("{{ pop {body} 0 }}").as_bytes(),
        )
    };
    let at = |ops| render(&shading_page(program(ops))).expect("render");
    assert_eq!(pixel(&at(16_000), 50, 50), BLACK);
    assert_eq!(pixel(&at(17_000), 50, 50), WHITE);
}

/// A page filling with colour space object 5 at tint `tint`; `spaces` are
/// objects 5, 6, ...
fn colour_page(tint: &str, spaces: Vec<Vec<u8>>) -> ViewDocument {
    page(
        "<< /ColorSpace << /C 5 0 R >> >>",
        format!("/C cs {tint} sc 10 10 80 80 re f").as_bytes(),
        spaces,
    )
}

#[test]
fn colour_space_chains_are_cut_where_they_nest_too_deeply() {
    // About 2,000 Indexed, Separation or ICCBased spaces, each the base or
    // alternate of the one before, overflowed the WASM stack. MuPDF refuses
    // an Indexed base that is Indexed and a Separation alternate that is a
    // Separation (the page fails); here such a chain draws until it nests
    // past eight.
    let n = 3_000;
    let chain = |make: &dyn Fn(usize) -> String, last: &str| -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| {
                obj(&if i + 1 < n {
                    make(i + 6)
                } else {
                    last.to_string()
                })
            })
            .collect()
    };
    let indexed = chain(
        &|next| format!("[/Indexed {next} 0 R 1 <000000ffffff>]"),
        "/DeviceRGB",
    );
    assert!(render(&colour_page("1", indexed)).is_ok());
    let separation = chain(
        &|next| {
            format!(
                "[/Separation /S {next} 0 R << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>]"
            )
        },
        "/DeviceGray",
    );
    assert!(render(&colour_page("1", separation)).is_ok());
    let mut icc: Vec<Vec<u8>> = (0..n)
        .map(|i| obj(&format!("[/ICCBased {} 0 R]", n + 5 + i + 1)))
        .collect();
    for i in 0..n {
        let alternate = if i + 1 < n {
            format!("{} 0 R", i + 6)
        } else {
            "/DeviceRGB".to_string()
        };
        icc.push(stream(&format!("/N 3 /Alternate {alternate}"), b"junk"));
    }
    assert!(render(&colour_page("1 0 0", icc)).is_ok());
}

#[test]
fn colour_spaces_nested_as_files_nest_them_draw() {
    // An Indexed space over an ICCBased one whose profile does not read
    // falls back to its alternate: index 1 is green.
    let doc = colour_page(
        "1",
        vec![
            obj("[/Indexed 6 0 R 1 <ff0000 00ff00>]"),
            obj("[/ICCBased 7 0 R]"),
            stream("/N 3 /Alternate /DeviceRGB", b"not a profile"),
        ],
    );
    assert_eq!(pixel(&render(&doc).expect("render"), 50, 50), [0, 255, 0]);
    // Eight Separations deep (each an alternate of the one before, as hayro
    // reads them) still draw: the tint passes down unchanged to gray.
    let n = 8;
    let spaces = (0..n)
        .map(|i| {
            let alternate = if i + 1 < n {
                format!("{} 0 R", i + 6)
            } else {
                "/DeviceGray".to_string()
            };
            obj(&format!(
                "[/Separation /S{i} {alternate} << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>]"
            ))
        })
        .collect();
    assert_eq!(
        pixel(&render(&colour_page("0", spaces)).expect("render"), 50, 50),
        BLACK
    );
}

/// A stitching function (object 6 on) over `functions`.
fn stitch(functions: &str, bounds: &str, encode: &str) -> Vec<u8> {
    obj(&format!(
        "<< /FunctionType 3 /Domain [0 1] /Functions [{functions}] /Bounds [{bounds}] /Encode [{encode}] >>"
    ))
}

const BLACK_FUNCTION: &str = "<< /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [0] /N 1 >>";

#[test]
fn stitching_functions_are_bounded_in_depth_and_parts() {
    // A chain of 3,000 overflowed the WASM stack; one that refers to itself
    // recursed forever. Past 32 levels the function is refused and the
    // shading not drawn (MuPDF draws 128 levels and not 256).
    let chain = |n: usize| {
        let mut objects: Vec<Vec<u8>> = (0..n)
            .map(|i| stitch(&format!("{} 0 R", i + 7), "", "0 1"))
            .collect();
        objects.push(obj(BLACK_FUNCTION));
        render(&shading_page_with(objects)).expect("render")
    };
    assert_eq!(pixel(&chain(31), 50, 50), BLACK);
    assert_eq!(pixel(&chain(40), 50, 50), WHITE);
    assert_eq!(pixel(&chain(3_000), 50, 50), WHITE);
    let cycle = render(&shading_page(stitch("6 0 R", "", "0 1"))).expect("render");
    assert_eq!(pixel(&cycle, 50, 50), WHITE);
    // Every level refers to the next twice: built once each (2^30 builds
    // without the cache), and drawn.
    let levels = 30;
    let mut objects: Vec<Vec<u8>> = (0..levels)
        .map(|i| stitch(&format!("{0} 0 R {0} 0 R", i + 7), "0.5", "0 1 0 1"))
        .collect();
    objects.push(obj(BLACK_FUNCTION));
    let shared = render(&shading_page_with(objects)).expect("render");
    assert_eq!(pixel(&shared, 50, 50), BLACK);
}

/// `shading_page` with its function and the objects after it (6, 7, ...).
fn shading_page_with(functions: Vec<Vec<u8>>) -> ViewDocument {
    let mut extra = vec![obj(
        "<< /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function 6 0 R >>",
    )];
    extra.extend(functions);
    page("<< /Shading << /Sh 5 0 R >> >>", b"/Sh sh", extra)
}

/// A one-input, one-output sampled function declaring `entries` one-bit
/// samples, with `bytes` of zero data, Flate-encoded.
fn sampled_with(entries: u64, bytes: usize) -> Vec<u8> {
    flate(
        &format!("/FunctionType 0 /Domain [0 1] /Range [0 1] /Size [{entries}] /BitsPerSample 1"),
        &vec![0u8; bytes],
    )
}

/// A sampled function of `entries` one-bit samples, all 0.
fn sampled(entries: u64) -> Vec<u8> {
    sampled_with(entries, usize::try_from(entries.div_ceil(8)).expect("size"))
}

#[test]
fn sampled_function_tables_are_held_to_a_budget() {
    // hayro kept a 4-byte entry for every sample bit the data held, read
    // past the declared size: 64 KB of Flate asked for gigabytes. A table
    // is now its declared entries, at most what the data holds, within 64
    // MiB per render.
    let small = render(&shading_page(sampled(256))).expect("render");
    assert_eq!(pixel(&small, 50, 50), BLACK);
    // Declared far past its data: only the data's entries are kept.
    assert!(render(&shading_page(sampled_with(u64::from(u32::MAX), 64))).is_ok());
    // 2^24 + 64 entries need more than 64 MiB.
    assert!(too_complex(&render(&shading_page(sampled((1 << 24) + 64)))));
}

#[test]
fn a_function_shared_by_many_colour_spaces_is_built_once() {
    // Twenty Separation spaces share one sampled tint transform of 2^22
    // entries (16 MiB): built once, it fits the 64 MiB budget; built per
    // space it would not.
    let n = 20;
    let mut objects: Vec<Vec<u8>> = vec![sampled(1 << 22)];
    let names: String = (0..n).map(|i| format!("/C{i} {} 0 R ", i + 6)).collect();
    for i in 0..n {
        objects.push(obj(&format!("[/Separation /S{i} /DeviceGray 5 0 R]")));
    }
    let content: String = (0..n)
        .map(|i| format!("/C{i} cs 1 sc {} 10 4 80 re f ", 10 + 4 * i))
        .collect();
    let doc = page(
        &format!("<< /ColorSpace << {names} >> >>"),
        content.as_bytes(),
        objects,
    );
    assert_eq!(pixel(&render(&doc).expect("render"), 11, 50), BLACK);
}

#[test]
fn graphics_states_nested_past_the_limit_are_refused() {
    // 600,000 nested `q` (2.8 KB of Flate) held 1.2 GB of WASM memory;
    // MuPDF refuses past about 2,048 states. 4,096 are kept, counting one
    // for each content run.
    let nested = |n: usize| format!("{}0 0 50 50 re f{}", "q ".repeat(n), " Q".repeat(n));
    let ok = page("<< >>", nested(4_000).as_bytes(), vec![]);
    assert_eq!(pixel(&render(&ok).expect("render"), 25, 75), BLACK);
    let deep = page("<< >>", nested(5_000).as_bytes(), vec![]);
    assert!(too_complex(&render(&deep)));
    assert!(matches!(
        deep.structured_text(0),
        Err(ViewError::TooComplex(_))
    ));
    // Along a form: 3,000 on the page and 2,000 in the form.
    let form = flate(
        "/Type /XObject /Subtype /Form /BBox [0 0 100 100]",
        nested(2_000).as_bytes(),
    );
    let along = page(
        "<< /XObject << /X 5 0 R >> >>",
        format!("{}/X Do{}", "q ".repeat(3_000), " Q".repeat(3_000)).as_bytes(),
        vec![form],
    );
    assert!(too_complex(&render(&along)));
}

#[test]
fn a_path_past_the_element_limit_is_refused() {
    // hayro builds a path whole (about 56 bytes an element) and vello
    // flattens it at once: a 24 KB page stroking 2,000,000 segments took
    // 2.3 GB of WASM memory, 8,000,000 aborted the engine.
    let zigzag = |n: usize, paint: &str| {
        format!("0 0 m {}{paint}", "1 1 l 2 0 l ".repeat(n / 2)).into_bytes()
    };
    assert!(render(&page("<< >>", &zigzag(100_000, "f"), vec![])).is_ok());
    assert!(render(&page("<< >>", &zigzag(100_000, "S"), vec![])).is_ok());
    assert!(too_complex(&render(&page(
        "<< >>",
        &zigzag((1 << 20) + 2, "f"),
        vec![]
    ))));
    // Text extraction does not build paths.
    let doc = page("<< >>", &zigzag((1 << 20) + 2, "f"), vec![]);
    assert!(doc.structured_text(0).is_ok());
    // A stroke costs vello about 1,240 bytes a segment on WASM: 800,000 of
    // them pass the per-path budget (512 MiB) before vello sees them.
    assert!(too_complex(&render(&page(
        "<< >>",
        &zigzag(800_000, "S"),
        vec![]
    ))));
}

#[test]
fn a_wide_stroke_counts_its_outline_against_the_path_budget() {
    // Each segment of a stroke 1,000 units wide adds an outline across the
    // whole page (both sides and the join): 20,000 tiny segments draw,
    // 200,000 pass the per-path budget, where without the width they would
    // be estimated at half of it.
    let zigzag = |n: usize| format!("1000 w 0 0 m {}S", "1 1 l 2 0 l ".repeat(n / 2)).into_bytes();
    assert!(render(&page("<< >>", &zigzag(20_000), vec![])).is_ok());
    assert!(too_complex(&render(&page(
        "<< >>",
        &zigzag(200_000),
        vec![]
    ))));
    // A stroke wider than the page around a short path stays cheap.
    let wide = page("<< >>", b"1000000000000 w 0 50 m 100 50 l S", vec![]);
    assert!(render(&wide).is_ok());
}

#[test]
fn dash_patterns_are_drawn_within_the_path_budget() {
    // A dash pattern finer than half a pixel strokes solid, as MuPDF does
    // (50 bytes of content took 1.4 GB natively); a long dashed path whose
    // dashes would pass the per-path budget is refused before kurbo expands
    // them.
    let fine = page("<< >>", b"2 w [0.001] 0 d 0 50 m 100 50 l S", vec![]);
    let image = render(&fine).expect("render");
    assert!(pixel(&image, 50, 49)[0].min(pixel(&image, 50, 50)[0]) < 64);
    let dashed = page("<< >>", b"2 w [2] 0 d 0 50 m 100 50 l S", vec![]);
    let image = render(&dashed).expect("render");
    let row = |x: u32| pixel(&image, x, 49)[0].min(pixel(&image, x, 50)[0]);
    assert!(row(1) < 128 && row(3) > 128, "{} {}", row(1), row(3));
    let long = page("<< >>", b"[1] 0 d 0 50 m 1000000000 50 l S", vec![]);
    assert!(too_complex(&render(&long)));
}

#[test]
fn predictor_parameters_hayro_divided_by_or_allocated_from_draw() {
    // /Columns 0 made a row of no bytes (hayro divided by it), a huge
    // /Columns allocated a zero row of that size first, and 32 colors of 16
    // bits wrapped a u8.
    let content = miniz_oxide::deflate::compress_to_vec_zlib(b"0 0 50 50 re f", 6);
    for params in [
        "/Predictor 2 /Columns 0",
        "/Predictor 2 /Colors 0",
        "/Predictor 12 /Columns 4294967295",
        "/Predictor 15 /Colors 32 /BitsPerComponent 16 /Columns 3",
        "/Predictor 2 /Colors 32 /BitsPerComponent 16 /Columns 3",
    ] {
        let objects = vec![
            obj("<< /Type /Catalog /Pages 2 0 R >>"),
            obj("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            obj(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents 4 0 R >>",
            ),
            stream(
                &format!("/Filter /FlateDecode /DecodeParms << {params} >>"),
                &content,
            ),
        ];
        let doc = ViewDocument::open(pdf(&objects), "").expect("open");
        let image = render(&doc).expect(params);
        assert!(doc.structured_text(0).is_ok(), "{params}");
        // A row of no bytes leaves the data unpredicted, as MuPDF reads it:
        // the square is drawn.
        if params.ends_with(" 0") {
            assert_eq!(pixel(&image, 25, 75), BLACK, "{params}");
        }
    }
}
