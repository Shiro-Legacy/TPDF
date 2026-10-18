// SPDX-License-Identifier: MIT
use std::io::Write;
use std::process::{Command, Stdio};

fn inspect(mode: &str, bytes: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
        .arg(mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch CLI");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(bytes)
        .expect("write input");
    child.wait_with_output().expect("read CLI output")
}

#[test]
fn token_output_preserves_binary_payload_and_types() {
    let output = inspect(
        "--tokens",
        b"% comment\n<< /A#20B [42 -.5 <00ff> (a\\n)] >>",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "dict-start\nname\t412042\narray-start\ninteger\t42\nreal\t-0.5\nstring\t00ff\nstring\t610a\narray-end\ndict-end\n"
    );
}

#[test]
fn malformed_input_returns_nonzero_with_offset() {
    let output = inspect("--tokens", b"(unterminated");
    assert_eq!(output.status.code(), Some(2));
    assert!(!output.stderr.is_empty());
}

#[test]
fn object_mode_rejects_trailing_object() {
    let output = inspect("--object", b"42 43");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn object_mode_accepts_dictionary() {
    let output = inspect("--object", b"<< /Type /Page /Parent 2 0 R >>");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.is_empty());
}

#[test]
fn help_and_bad_mode_are_distinct() {
    let help = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8_lossy(&help.stdout)
            .contains("Inspect at most 16 MiB of PDF syntax; defaults to stdin.")
    );
    assert_eq!(inspect("--render", b"").status.code(), Some(2));
}

#[test]
fn filter_mode_decodes_binary_and_refuses_unknown_filter() {
    let result = inspect("--decode=asciihex", b"00ff>");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, vec![0, 255]);
    assert_eq!(inspect("--decode=imaginary", b"").status.code(), Some(2));
}

#[test]
fn lzw_cli_exposes_both_early_change_modes() {
    // Hand-packed MSB-first 9-bit codes: clear256, literal65, EOD257.
    for mode in ["--decode=lzw", "--decode=lzw0"] {
        let output = inspect(mode, &[0x80, 0x10, 0x60, 0x20]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"A");
    }
    let output = inspect("--decode=lzw", &[0x80, 0x10, 0x60]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}
