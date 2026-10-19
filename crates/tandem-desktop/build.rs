//! Stages what the app bundle carries besides the binary, then runs Tauri's
//! build step (which checks the bundle's resources exist):
//! - `agent/`: the MCP server and the session driver, so an agent needs only
//!   the app (`Contents/Resources/agent/scripts/tandem-mcp.mjs`, docs/agent.md);
//! - `fixtures/`: the built-in samples a `--smoke`/`--drive` run opens first;
//! - `licenses/`: filled by scripts/package-mac.sh (empty in a dev build).

use std::fs;
use std::path::Path;

fn copy(from: &Path, to: &Path) {
    if let Some(dir) = to.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if fs::copy(from, to).is_err() {
        println!("cargo:warning=could not stage {}", from.display());
    }
    println!("cargo:rerun-if-changed={}", from.display());
}

fn main() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let stage = Path::new(env!("CARGO_MANIFEST_DIR")).join(".stage");
    for file in ["scripts/tandem-mcp.mjs", "scripts/session.mjs", "docs/agent.md", "package.json"] {
        copy(&repo.join(file), &stage.join("agent").join(file));
    }
    for name in ["hello.pdf", "form.pdf", "large.pdf"] {
        copy(&repo.join("fixtures").join(name), &stage.join("fixtures").join(name));
    }
    let _ = fs::create_dir_all(stage.join("licenses"));
    if fs::read_dir(stage.join("licenses")).map(|mut d| d.next().is_none()).unwrap_or(true) {
        let _ = fs::write(stage.join("licenses/README.txt"), "Licence texts are added by scripts/package-mac.sh.\n");
    }
    tauri_build::build()
}
