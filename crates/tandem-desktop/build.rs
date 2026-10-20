//! Stages what the app bundle carries besides the binary, then runs Tauri's
//! build step (which checks the bundle's resources exist):
//! (The MCP server is the binary's own `--mcp` mode, with docs/agent.md compiled
//! in, so the bundle carries no agent scripts.)
//! - `fixtures/`: the built-in samples a `--smoke`/`--drive` run opens first;
//! - `fonts/`: the CJK fallback font;
//! - `licenses/`: filled by scripts/package-mac.sh, package-windows.ps1 or package-linux.sh
//!   (a README only in a dev build).
//!
//! Paths are joined with `Path::join`, so the same staging runs on Windows.

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
    // Staged by earlier builds, when the bundle carried the Node MCP server.
    let _ = fs::remove_dir_all(stage.join("agent"));
    for name in ["hello.pdf", "form.pdf", "large.pdf"] {
        copy(
            &repo.join("fixtures").join(name),
            &stage.join("fixtures").join(name),
        );
    }
    // The CJK fallback font the engine draws non-embedded CJK text with (Apache-2.0; see
    // assets/fonts/README.md, staged with the licences).
    copy(
        &repo.join("assets/fonts/DroidSansFallback.ttf"),
        &stage.join("fonts/DroidSansFallback.ttf"),
    );
    copy(
        &repo.join("assets/fonts/README.md"),
        &stage.join("licenses/fonts-README.md"),
    );
    let _ = fs::create_dir_all(stage.join("licenses"));
    if fs::read_dir(stage.join("licenses"))
        .map(|mut d| d.next().is_none())
        .unwrap_or(true)
    {
        let _ = fs::write(
            stage.join("licenses/README.txt"),
            "Licence texts are added by scripts/package-mac.sh, package-windows.ps1 or package-linux.sh.\n",
        );
    }
    // tauri-build merges tauri.<os>.conf.json for the target OS but only watches tauri.conf.json.
    for os in ["macos", "windows", "linux"] {
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tauri.{os}.conf.json"));
        if file.is_file() {
            println!("cargo:rerun-if-changed={}", file.display());
        }
    }
    tauri_build::build()
}
