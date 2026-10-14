// SPDX-License-Identifier: AGPL-3.0-or-later
use std::io::Write;
use std::process::{Command, Stdio};
fn pdf(content: &[u8]) -> Vec<u8> {
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend(content);
    stream.extend(b"\nendstream");
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 2 2] /Contents 4 0 R >>".to_vec(),
        stream,
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(obj);
        out.extend(b"\nendobj\n");
    }
    let start = out.len();
    out.extend(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n").bytes());
    out
}
fn cli(selector: &str, input: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
        .arg(format!("--page-render={selector}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn page_pixels_have_declared_ppm_dimensions_scale_and_row_order() {
    let input = pdf(b"1 0 0 rg 0 0 1 2 re f");
    for (dpi, size) in [(72, 2), (144, 4)] {
        let result = cli(&format!("1@{dpi}"), &input);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut expected = format!("P6\n{size} {size}\n255\n").into_bytes();
        for _ in 0..size {
            for x in 0..size {
                expected.extend(if x < size / 2 {
                    [255, 0, 0]
                } else {
                    [255, 255, 255]
                });
            }
        }
        assert_eq!(result.stdout, expected);
    }
}
#[test]
fn unsupported_suffix_and_invalid_page_or_dpi_never_publish_pixels() {
    for content in [
        b"1 0 0 rg 0 0 1 2 re f 0 w 0 0 m 2 2 l S".as_slice(),
        b"0 w 0 0 1 2 re B",
        b"0 1 1 0 k 0 0 1 2 re f",
    ] {
        let result = cli("1@72", &pdf(content));
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
    }
    let input = pdf(b"0 0 1 2 re f");
    for selector in [
        "0@72", "2@72", "1@0", "1@-72", "1@NaN", "1@inf", "1@1e300", "1@72@1", "1", "@72",
    ] {
        let result = cli(selector, &input);
        assert!(!result.status.success(), "{selector}");
        assert!(result.stdout.is_empty(), "{selector}");
    }
}
#[test]
fn embedded_truetype_page_produces_owned_opaque_image() {
    let result = cli(
        "1@36",
        include_bytes!("../../fixtures/p3/simple-subset.pdf"),
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.starts_with(b"P6\n"));
    let header_end = result
        .stdout
        .windows(5)
        .position(|b| b == b"\n255\n")
        .unwrap()
        + 5;
    assert!(result.stdout[header_end..].iter().any(|&b| b < 255));
}

#[test]
fn solid_stroke_outputs_atomic_color_pixels_at_requested_resolution() {
    let input = pdf(b"0 0 1 RG 2 w 1 0 m 1 2 l S");
    for (dpi, size) in [(72, 2), (144, 4)] {
        let result = cli(&format!("1@{dpi}"), &input);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut expected = format!("P6\n{size} {size}\n255\n").into_bytes();
        for _ in 0..size * size {
            expected.extend([0, 0, 255]);
        }
        assert_eq!(result.stdout, expected);
    }
}
