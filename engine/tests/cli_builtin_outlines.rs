// SPDX-License-Identifier: AGPL-3.0-or-later
use std::process::Command;
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
