// SPDX-License-Identifier: MIT
//! Small hostile pages from the integration acceptance review (and the sweep
//! that followed it) stop neither the engine nor the machine: PostScript
//! functions, colour spaces and stitching functions nested thousands deep
//! (they overflowed the WASM stack), sampled function tables, nested
//! graphics states, paths and dash patterns that cost vello gigabytes, and
//! predictor parameters hayro divided by; and (round 2) decoded streams
//! held together: forms along a chain, content read again and again, a
//! page's fonts, and every page's kept content. Every document is generated
//! here and a few kilobytes to a few megabytes; the decoding limit itself
//! (128 MiB) and the memory these bounds save are proved on the WASM build
//! (tests/rust-engine-bounds.test.mjs at the app root), where memory matters.
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
fn postscript_procedures_nest_at_most_as_deep_as_reference_viewers_read() {
    // Reference viewers draw the shading at 100 levels and refuse its function
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
    // alternate of the one before, overflowed the WASM stack. Reference viewers
    // refuse an Indexed base that is Indexed and a Separation alternate that is a
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
    // shading not drawn (reference viewers draw 128 levels and not 256).
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
    // reference viewers refuse past about 2,048 states. 4,096 are kept, counting one
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
    // A dash pattern finer than half a pixel strokes solid, as reference viewers do
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
        // A row of no bytes leaves the data unpredicted, as reference viewers read it:
        // the square is drawn.
        if params.ends_with(" 0") {
            assert_eq!(pixel(&image, 25, 75), BLACK, "{params}");
        }
    }
}

/// `megabytes` MiB of spaces, then `tail`.
fn padded(megabytes: usize, tail: &str) -> Vec<u8> {
    let mut data = vec![b' '; megabytes << 20];
    data.extend_from_slice(tail.as_bytes());
    data
}

fn refused_text(doc: &ViewDocument) -> bool {
    matches!(doc.structured_text(0), Err(ViewError::TooComplex(_)))
}

#[test]
fn nested_forms_hold_their_decoded_content_within_one_budget() {
    // hayro decodes a form whole and holds it while the forms it draws run:
    // forms of 120 MiB (within hayro-syntax's 128 MiB a stream), each
    // drawing the next, held 128 MiB of WASM a level (the integration
    // review, round 2). A page, its forms and what they invoke now hold at
    // most 128 MiB at once: two forms of 70 MiB are refused, two of 30 MiB
    // draw.
    let forms = |megabytes| {
        page(
            "<< /XObject << /F0 5 0 R >> >>",
            b"/F0 Do",
            vec![
                flate(
                    "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /F1 6 0 R >> >>",
                    &padded(megabytes, " /F1 Do"),
                ),
                flate(
                    "/Type /XObject /Subtype /Form /BBox [0 0 100 100]",
                    &padded(megabytes, " 0 0 50 50 re f"),
                ),
            ],
        )
    };
    let held = forms(70);
    assert!(too_complex(&render(&held)));
    assert!(refused_text(&held));
    let drawn = forms(30);
    assert_eq!(pixel(&render(&drawn).expect("render"), 25, 75), BLACK);
    assert!(drawn.structured_text(0).is_ok());
    // A pattern cell drawn inside a form counts along the same chain.
    let cell = flate(
        "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >>",
        &padded(70, " 0 0 10 10 re f"),
    );
    let pattern = page(
        "<< /XObject << /F0 5 0 R >> >>",
        b"/F0 Do",
        vec![
            flate(
                "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Pattern << /P 6 0 R >> >>",
                &padded(70, " /Pattern cs /P scn 0 0 50 50 re f"),
            ),
            cell,
        ],
    );
    assert!(too_complex(&render(&pattern)));
}

#[test]
fn content_read_again_and_again_is_held_to_a_budget() {
    // hayro decodes a form each time it draws it, and a form it then skips
    // (no bounding box, or nested too deeply) too: a form of 120 MiB drawn
    // a thousand times was 120 GB to inflate. A page reads at most 1 GiB of
    // content, each run counted each time.
    let drawn = |times: usize, bbox: &str| {
        page(
            "<< /XObject << /F 5 0 R >> >>",
            "/F Do ".repeat(times).as_bytes(),
            vec![flate(
                &format!("/Type /XObject /Subtype /Form {bbox}"),
                &padded(4, " 0 0 50 50 re f"),
            )],
        )
    };
    let bbox = "/BBox [0 0 100 100]";
    assert_eq!(
        pixel(&render(&drawn(20, bbox)).expect("render"), 25, 75),
        BLACK
    );
    for doc in [drawn(300, bbox), drawn(300, "")] {
        assert!(too_complex(&render(&doc)));
        assert!(refused_text(&doc));
    }
    // A form without a bounding box is not drawn; read 20 times it is fine.
    assert_eq!(
        pixel(&render(&drawn(20, "")).expect("render"), 25, 75),
        WHITE
    );
}

#[test]
fn the_fonts_a_page_selects_decode_within_one_budget() {
    // hayro keeps every font of a render with its program copied whole:
    // three fonts whose programs a page of kilobytes padded to 100 MiB held
    // 490 MB of WASM. The fonts one page selects decode to at most 256 MiB
    // together, programs and CMaps; one program is counted for each font
    // dictionary that names it, as hayro copies it for each.
    let fonts = |count: usize| {
        let names: String = (0..count).map(|i| format!("/F{i} 6 0 R ")).collect();
        let shows: String = (0..count)
            .map(|i| format!("BT /F{i} 12 Tf 10 {} Td (A) Tj ET ", 10 * i + 10))
            .collect();
        page(
            &format!("<< /Font << {names}>> >>"),
            shows.as_bytes(),
            vec![
                flate("", &padded(100, "")),
                obj(
                    "<< /Type /Font /Subtype /TrueType /BaseFont /Padded /FirstChar 65 /LastChar 65 /Widths [600] /FontDescriptor 7 0 R >>",
                ),
                obj(
                    "<< /Type /FontDescriptor /FontName /Padded /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 5 0 R >>",
                ),
            ],
        )
    };
    // One dictionary selected by two names: one font.
    let one = fonts(2);
    assert!(render(&one).is_ok());
    assert!(one.structured_text(0).is_ok());
    let distinct = |count: usize| {
        let names: String = (0..count)
            .map(|i| format!("/F{i} {} 0 R ", 6 + 2 * i))
            .collect();
        let shows: String = (0..count)
            .map(|i| format!("BT /F{i} 12 Tf 10 {} Td (A) Tj ET ", 10 * i + 10))
            .collect();
        let mut extra = vec![flate("", &padded(100, ""))];
        for i in 0..count {
            extra.push(obj(&format!("<< /Type /Font /Subtype /TrueType /BaseFont /Padded{i} /FirstChar 65 /LastChar 65 /Widths [600] /FontDescriptor {} 0 R >>", 7 + 2 * i)));
            extra.push(obj(&format!("<< /Type /FontDescriptor /FontName /Padded{i} /Flags 32 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 900 /Descent -200 /CapHeight 700 /StemV 80 /FontFile2 5 0 R >>")));
        }
        page(
            &format!("<< /Font << {names}>> >>"),
            shows.as_bytes(),
            extra,
        )
    };
    let two = distinct(2);
    assert!(render(&two).is_ok());
    let three = distinct(3);
    assert!(too_complex(&render(&three)));
    assert!(refused_text(&three));
}

#[test]
fn pages_past_the_kept_content_draw_from_content_decoded_again() {
    // hayro-syntax kept every page's decoded content for the life of the
    // document: pages sharing one stream of a kilobyte that decodes to 128
    // MiB held 128 MiB each once drawn (thumbnails and search read them
    // all). The pages of a document now keep 32 MiB together; past it a
    // page decodes its content each time it is read, and draws and reads
    // the same.
    let content = flate(
        "",
        &padded(20, " 0 0 50 50 re f BT /F 12 Tf 60 60 Td (Hi) Tj ET"),
    );
    let pages = 4;
    let kids: String = (0..pages).map(|i| format!("{} 0 R ", 3 + i)).collect();
    let mut objects = vec![
        obj("<< /Type /Catalog /Pages 2 0 R >>"),
        obj(&format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>")),
    ];
    let shared = 3 + pages;
    for _ in 0..pages {
        objects.push(obj(&format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Font << /F {} 0 R >> >> /Contents {shared} 0 R >>",
            shared + 1
        )));
    }
    objects.push(content);
    objects.push(obj(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ));
    let doc = ViewDocument::open(pdf(&objects), "").expect("open");
    for _ in 0..2 {
        for page in 0..pages {
            let image = doc
                .render(page, 1.0, RenderOptions::default())
                .expect("render");
            assert_eq!(pixel(&image, 25, 75), BLACK, "page {page}");
            let text = doc.structured_text(page).expect("text");
            assert_eq!(text.as_text().trim(), "Hi", "page {page}");
        }
    }
}

#[test]
fn a_measurer_loads_fonts_within_the_font_budget() {
    // Text edits and redaction measure codes with a page's fonts through one
    // hayro cache a page, which keeps every font it loads: it holds them to
    // the same 256 MiB. Type3 fonts whose ToUnicode streams decode to
    // 100 MiB: two load, the third is not loaded (as an unloadable font).
    let names: String = (0..3).map(|i| format!("/F{i} {} 0 R ", 7 + i)).collect();
    let mut extra = vec![
        flate("", &padded(100, "")),
        stream("", b"500 0 d0 0 0 500 700 re f"),
    ];
    for _ in 0..3 {
        extra.push(obj(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 500 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 6 0 R >> /Encoding << /Differences [65 /a] >> /FirstChar 65 /LastChar 65 /Widths [500] /ToUnicode 5 0 R >>",
        ));
    }
    let doc = page(&format!("<< /Font << {names}>> >>"), b"", extra);
    let measurer = doc.measurer(0).expect("measurer");
    let codes = [b"A".to_vec()];
    let measured = |font: &[u8]| measurer.measure(&[], font, &codes)[0].is_some();
    assert!(measured(b"F0"));
    assert!(measured(b"F1"));
    assert!(!measured(b"F2"));
    assert!(measured(b"F0"));
}
