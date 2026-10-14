// SPDX-License-Identifier: AGPL-3.0-or-later
use std::{
    io::Write,
    process::{Command, Output, Stdio},
};
fn input(mode: &str, pdf: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
        .arg(mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(pdf).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn static_asset_inspection_needs_no_pdf_and_rejects_unknown_face() {
    for face in ["Helvetica", "Times-Roman"] {
        let result = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
            .arg(format!("--builtin-outlines={face}"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let json = String::from_utf8(result.stdout).unwrap();
        assert_eq!(json.matches("\"native_gid\":").count(), 654);
        assert_eq!(json.matches("\"units_per_em\":1000").count(), 654);
        assert!(json.contains("\"name\":\"space\""));
    }
    let invalid = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
        .arg("--builtin-outlines=Arial")
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
}
#[test]
fn explicit_combined_mode_retains_distinct_builtin_and_embedded_provenance() {
    let hello = include_bytes!("../../fixtures/hello.pdf");
    let old = input("--page-glyph=1", hello);
    assert_eq!(old.status.code(), Some(2));
    assert!(old.stdout.is_empty());
    let new = input("--page-outlines=1", hello);
    assert!(
        new.status.success(),
        "{}",
        String::from_utf8_lossy(&new.stderr)
    );
    let json = String::from_utf8(new.stdout).unwrap();
    let glyphs = json
        .split_once(",\"glyphs\":[")
        .unwrap()
        .1
        .split_once("],\"commands\":[")
        .unwrap()
        .0;
    assert!(glyphs.contains("\"source_kind\":\"pinned_base14\""));
    assert!(glyphs.contains("\"asset_sha256\":"));
    assert!(!glyphs.contains("\"program\":"));
    assert!(!glyphs.contains("\"selection\":"));
    let embedded = include_bytes!("../../fixtures/p3/simple-subset.pdf");
    let old = input("--page-glyph=1", embedded);
    assert!(old.status.success());
    assert!(!String::from_utf8_lossy(&old.stdout).contains("\"source_kind\""));
    let new = input("--page-outlines=1", embedded);
    assert!(new.status.success());
    assert!(
        String::from_utf8_lossy(&new.stdout).contains("\"source_kind\":\"embedded_true_type\"")
    );
}
