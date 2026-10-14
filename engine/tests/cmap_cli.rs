// SPDX-License-Identifier: AGPL-3.0-or-later
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct QueryFile(std::path::PathBuf);
impl QueryFile {
    fn new(bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "tandem-cmap-queries-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(bytes).unwrap();
        Self(path)
    }
}
impl Drop for QueryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn query(cmap: &[u8], lines: &[u8]) -> std::process::Output {
    let file = QueryFile::new(lines);
    let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
        .arg(format!("--cmap={}", file.0.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(cmap).unwrap();
    child.wait_with_output().unwrap()
}
const MAP:&[u8]=b"begincmap 2 begincodespacerange <80> <ff> <0000> <007f> endcodespacerange 2 beginbfchar <80> <d801dc00> <0001> <00660069> endbfchar endcmap";
#[test]
fn batched_queries_preserve_width_unicode_and_missing_distinctions() {
    let out = query(
        MAP,
        b"lookup 80\nlookup 0001\nlookup 81\ndecode 0001ff\ndecode 00\ndecode -\ndecode 017f\n",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("\"width\":1,\"value\":128,\"scalars\":[66560]"));
    assert!(text.contains("\"width\":2,\"value\":1,\"scalars\":[102,105]"));
    assert!(text.contains("\"value\":129,\"scalars\":null"));
    assert!(text.contains("\"consumed\":2,\"matched_codespace\":true"));
    assert!(text.contains("{\"kind\":\"decode\",\"input\":\"00\",\"error\":\"Incomplete\"}"));
    assert!(text.contains("{\"kind\":\"decode\",\"input\":\"\",\"error\":\"Incomplete\"}"));
    assert!(text.contains("{\"kind\":\"decode\",\"input\":\"017f\",\"error\":\"Invalid\"}"));
}
#[test]
fn malformed_query_aborts_atomically_after_prior_valid_queries() {
    for tail in [
        "lookup -",
        "lookup 0",
        "lookup zz",
        "lookup 0102030405",
        "lookup 01 extra",
        "execute 80",
        "decode",
    ] {
        let lines = format!("lookup 80\n{tail}\n");
        let out = query(MAP, lines.as_bytes());
        assert_eq!(out.status.code(), Some(2), "{tail}");
        assert!(out.stdout.is_empty(), "{tail}");
    }
}
#[test]
fn malformed_destination_fails_before_any_query_results() {
    let bad=b"begincmap 1 begincodespacerange <00> <ff> endcodespacerange 1 beginbfchar <01> 10400 endbfchar endcmap";
    let out = query(bad, b"lookup 01\n");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}
#[test]
fn blank_query_lines_are_bounded_inert_input() {
    let out = query(MAP, b"\n  \r\n\t\n");
    assert!(out.status.success());
    assert_eq!(out.stdout, b"{\"usecmap_ignored\":false,\"queries\":[]}\n");
}
