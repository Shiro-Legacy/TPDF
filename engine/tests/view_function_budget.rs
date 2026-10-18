// SPDX-License-Identifier: MIT
//! PostScript (type 4) functions kept for a render (the integration
//! re-review, fix round 3). hayro builds a function once per render and
//! keeps it, its program parsed into operators; one program has at most
//! 2^14, but a stitching function may name 4,096 functions, so 256 programs
//! of 2^14 operators held 218 MB from a 74 KB file (4,096 about 3.4 GB). The
//! operators of one render's type 4 functions are now counted together, at
//! most `MAX_PROGRAM_OPS` (2^18, about 8 MB); past it the function is not
//! built and the render fails as over budget. Proved by the count: 16 such
//! programs draw, 17 do not.
use tandem_pdf_engine::view::{RenderOptions, ViewDocument, ViewError};

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

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// A page filled by an axial shading whose stitching function names
/// `functions` distinct type 4 functions of `ops` operators each.
fn shading_page(functions: usize, ops: usize) -> ViewDocument {
    let mut program = b"{ ".to_vec();
    for _ in 0..ops / 2 {
        program.extend_from_slice(b"dup pop ");
    }
    program.extend_from_slice(b"}");
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(&program, 9);
    let refs: Vec<String> = (0..functions).map(|i| format!("{} 0 R", 7 + i)).collect();
    let bounds: Vec<String> = (1..functions)
        .map(|i| format!("{:.6}", i as f64 / functions as f64))
        .collect();
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Shading << /Sh 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream("", b"/Sh sh"),
        b"<< /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function 6 0 R /Extend [true true] >>".to_vec(),
        format!(
            "<< /FunctionType 3 /Domain [0 1] /Functions [{}] /Bounds [{}] /Encode [{}] >>",
            refs.join(" "),
            bounds.join(" "),
            vec!["0 1"; functions].join(" ")
        )
        .into_bytes(),
    ];
    for _ in 0..functions {
        objects.push(stream(
            "/FunctionType 4 /Domain [0 1] /Range [0 1] /Filter /FlateDecode",
            &packed,
        ));
    }
    ViewDocument::open(pdf(&objects), "").expect("open")
}

#[test]
fn the_type4_functions_of_a_render_keep_a_bounded_number_of_operators() {
    // 2^14 operators a program (8,191 `dup pop` pairs and the input).
    let ops = (1 << 14) - 2;
    let drawn = shading_page(16, ops)
        .render(0, 1.0, RenderOptions::default())
        .expect("render");
    // Each function maps its part of the axis to a ramp: the page is shaded.
    let row: Vec<u8> = (0..100)
        .map(|x| drawn.pixels[((50 * drawn.width + x) * 4) as usize])
        .collect();
    let (low, high) = (row.iter().min(), row.iter().max());
    assert!(low < Some(&50) && high > Some(&200), "{row:?}");
    assert!(matches!(
        shading_page(17, ops).render(0, 1.0, RenderOptions::default()),
        Err(ViewError::TooComplex(_))
    ));
    // Many small programs, as files have them, draw.
    assert!(
        shading_page(1000, 40)
            .render(0, 1.0, RenderOptions::default())
            .is_ok()
    );
}
