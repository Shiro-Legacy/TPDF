// SPDX-License-Identifier: AGPL-3.0-or-later
use std::io::Write;
use std::process::{Command, Stdio};

fn pdf(contents: &[&str]) -> Vec<u8> {
    let mut objects = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string()];
    let kids = (0..contents.len())
        .map(|i| format!("{} 0 R", 3 + i * 2))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!(
        "<< /Type /Pages /Count {} /Kids [{kids}] >>",
        contents.len()
    ));
    for (i, body) in contents.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 30] /Contents {} 0 R >>",
            4 + i * 2
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{body}\nendstream",
            body.len()
        ));
    }
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
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
fn run(mode: &str, bytes: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
        .args([mode, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn selected_page_preserves_source_and_applies_page_transform() {
    let result = run("--page-program=2", &pdf(&["", "1 2 3 4 re f"]));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("\"page\":2,\"status\":\"accepted\""));
    assert!(output.contains("\"content_hex\":\"312032203320342072652066\""));
    assert!(output.contains("\"bounds\":[0.0, 0.0, 20.0, 30.0]"));
    assert!(output.contains("\"metric_policy\":\"AllowPinnedBase14\""));
}
#[test]
fn corpus_continues_after_refusal_without_publishing_partial_page() {
    let result = run(
        "--page-programs",
        &pdf(&["1 2 3 4 re f", "1 2 3 4 re f /Bad Do", "2 2 1 1 re f"]),
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("\"page\":1,\"status\":\"accepted\""));
    assert!(output.contains("\"page\":2,\"status\":\"refused\""));
    assert!(output.contains("\"page\":3,\"status\":\"accepted\""));
    assert_eq!(output.matches("\"program\":").count(), 2);
    assert!(output.contains("\"page_attempts\":3"));
    assert!(!output.contains("content_hex"));
}
#[test]
fn single_page_refuses_atomically_and_rejects_bad_indices() {
    let bytes = pdf(&["1 2 3 4 re f /Bad Do"]);
    for mode in [
        "--page-program=1",
        "--page-program=0",
        "--page-program=2",
        "--page-program=18446744073709551616",
    ] {
        let result = run(mode, &bytes);
        assert!(!result.status.success(), "{mode}");
        assert!(result.stdout.is_empty(), "{mode}: partial stdout");
        assert!(!result.stderr.is_empty());
    }
}
