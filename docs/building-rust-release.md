# Building a Rust-engine release (macOS ARM64)

How to rebuild the Tandem PDF package with the Rust PDF engine from source, and how far the result is reproducible. Written October 17, 2026 for commit `de398ab` (branch `rust-engine-app`) plus the M10 package-check commits on `app-p10-pkg`. Checked on darwin-arm64 only. Windows and Linux packages have never been built with the Rust engine: nothing is pushed, so no CI has run (`engine/design/m10-macos-package.md`, "Windows and Linux").

This is the build recipe the AGPL notice in the package points to (`licenses/NOTICE.txt`: "Corresponding source ... build steps"). The checklist results for the package are in `engine/design/m10-macos-package.md`.

## Toolchains (exact versions used)

| Tool | Version | Notes |
| --- | --- | --- |
| macOS | 26.6.2 (25G83), Apple Silicon | the package targets macOS 13+ (`LSMinimumSystemVersion` 13.0) |
| Xcode command line tools | Apple clang 21.0.0 | only for the native engine tests, not the WASM |
| Rust (builds the WASM) | `rustc 1.98.1 (48a229cea 2026-09-01)`, `cargo 1.98.1` | rustup `stable`; the gates also check the minimum, 1.92.0 (`rustc 1.92.0 (ded5c06cf 2025-12-08)`) |
| Rust target | `wasm32-unknown-unknown` | `rustup target add wasm32-unknown-unknown` |
| wasm-bindgen-cli | 0.2.129 | must equal the `wasm-bindgen` crate pin in `engine/bridge/Cargo.toml` (`=0.2.129`) |
| Node.js | 22.23.1 (README: 22.12+) | npm 10.9.8 |
| npm dependencies | `package-lock.json` | Electron 44.4.0, @electron/packager 20.3.0, @electron/asar 4.3.0, TypeScript 7.0.2 |

Install the WASM tools outside the repository:

```sh
rustup toolchain install 1.98.1 1.92.0
rustup target add wasm32-unknown-unknown --toolchain 1.98.1
cargo install wasm-bindgen-cli --version 0.2.129 --locked --root "$HOME/.local/tpdf-wasm-tools"
```

The Rust dependencies are pinned in `engine/Cargo.lock` and `engine/bridge/Cargo.lock` (the WASM is built `--locked`); the patched hayro crates are vendored in `engine/vendor/` (`PATCH.md` in each). The CJK fallback font (`assets/fonts/DroidSansFallback.ttf`, sha256 `21b96a03…0b0949`) is committed, so the build needs no network after `npm ci` and the Electron download.

## Commands

From a clean checkout of the tagged commit:

```sh
npm ci
npm exec -- install-electron          # Electron 44.4.0 for node_modules/electron (checksums: node_modules/electron/checksums.json)

# 1. The WASM package (engine/web/pkg, not tracked): cargo build --release --target wasm32-unknown-unknown, then wasm-bindgen.
WASM_BINDGEN_BIN="$HOME/.local/tpdf-wasm-tools/bin/wasm-bindgen" sh engine/tools/build-workflow.sh

# 2. The app with the Rust worker engine, then the package (npm run build runs inside package:rust).
npm run package:rust                  # → out/Tandem PDF-darwin-arm64/Tandem PDF.app (unsigned: ad-hoc linker signature only)

# 3. The packaged self-test and the M10 checklist on the package.
"out/Tandem PDF-darwin-arm64/Tandem PDF.app/Contents/MacOS/Tandem PDF" --user-data-dir="$(mktemp -d)" --smoke --report=smoke.json
node scripts/check-macos-package.mjs --work=<scratch folder>
```

Order matters: `npm run build` refuses an `engine/web/pkg` whose `sources.sha256` does not match `engine/` (`TPDF_ALLOW_STALE_ENGINE=1` overrides). `@electron/packager` takes the Electron zip from `~/Library/Caches/electron/` or downloads it, verified against the lockfile's checksums.

A release build signs and notarizes when `APPLE_SIGN_IDENTITY` (and `APPLE_ID`, `APPLE_APP_PASSWORD`, `APPLE_TEAM_ID`) are set (`scripts/package.mjs`). That path has not been run for a Rust package. `bash scripts/make-dmg.sh` makes the DMG.

Machine limits on the 8 GB build Mac: run the cargo build with `CARGO_BUILD_JOBS=2`. A WASM build from an empty target directory took 2 min 47 s at a peak of 1,257 MB; the package step peaked at 546 MB.

## Expected outputs for this commit

| Output | sha256 | Size |
| --- | --- | --- |
| `engine/web/pkg/tandem_rust_workflow_bg.wasm`, built in `/Users/owner/Documents/tpdf-wt/p10-pkg` | `651fa2437f20917eead7e9487a38da4abfbe340dd28ed27e185eb3854be1e762` | 13,518,693 bytes |
| the same, built from an empty target directory with the checkout path remapped to `/Users/owner/Documents/TPDF` | `cf37c03ebe9089dca370606cb7552ff75d221dedee756ec3a07b06ec8e50a122` | 13,517,349 bytes |
| the gate's build in `/Users/owner/Documents/TPDF` (final-head gates, 11bf970; same Rust sources) | `6e90096945cf733485e57651e734b9b78659b4c21aaba36bce2914e62ce2aae4` | |
| `engine/web/pkg/tandem_rust_workflow.js` (all three builds) | `a9261f14f6da838d6f032db78f68b84fd2486980c03540526656a06f523cb890` | |
| `engine/web/pkg/sources.sha256` (sha256 of the listing) | `a515e15e20b33afbd7416b629d17dadbd69e13ae908472a7a1bd662b55c41103` | |

**The WASM hash depends on where the source tree is checked out.** rustc writes the absolute path of every non-std source file that can panic into the module's data (panic locations): 120 strings name the checkout (`/Users/<user>/…/engine/src/…`, `…/engine/vendor/hayro-…`) and 225 the cargo registry (`$HOME/.cargo/registry/src/index.crates.io-…/`). So a rebuild reproduces the hash above only from the same checkout path, the same `$HOME` and the same toolchain. This also puts the builder's user name and folder layout into the shipped WASM.

What was verified (October 17, 2026; logs in `tpdf-wt/scratch/p8/pkg/logs/`, not committed):

- The `.js` glue and `.d.ts` files are byte-identical across all three builds.
- The module built in this worktree (cached target directory) and the one built from an empty target directory (with `--remap-path-prefix` to the gate's checkout path) agree in every WASM section except data, code and globals: type, import, function, table, memory, export, element and the name section (1.74 MB, every symbol and crate hash) are identical, and the code section has the same length. The data section differs by the path strings (120 × 10 bytes shorter, plus 144 bytes of alignment), and the code and globals by the addresses that move with them. No other difference was found.
- The empty-target build remapped to the gate's path does **not** reproduce the gate's hash `6e900969…`, although the Rust sources are the same (11bf970 to de398ab changes only `vendor/hayro-interpret-0.8.0/PATCH.md` prose) and the toolchain is the same. The cause is not established: the gate's module was not compared section by section (it is in the main checkout, outside this lane's worktree). One difference in the inputs is that `--remap-path-prefix` itself, passed through `RUSTFLAGS`, is a tracked rustc option.

So reproducibility is shown for the parts the path does not touch, not bit for bit. To make the hash path-independent (recommended before a release; it changes the hash, so the owner decides): build with `RUSTFLAGS="--remap-path-prefix=$PWD/engine=/tpdf/engine --remap-path-prefix=$HOME/.cargo/registry/src=/cargo"` in `engine/tools/build-workflow.sh`, record the resulting sha256 here, and confirm it with one build from a second checkout path. That also removes the user name from the shipped module.

The package as a whole is not reproducible byte for byte: `app.asar` and the bundle carry build-time metadata (`ElectronAsarIntegrity` hash of this asar, file times), and the bundle is ad-hoc signed. Compare the WASM, then the files inside `app.asar` (`npx @electron/asar extract`).
