# Building a Rust-engine release (macOS ARM64)

How to rebuild the Tandem PDF package with the Rust PDF engine from source, and how far the result is reproducible. Written October 17, 2026 for commit `de398ab` (branch `rust-engine-app`) plus the M10 package-check commits on `app-p10-pkg`; the WASM was rebuilt and reproduced at the final head ba70615 ("Expected outputs" below). Checked on darwin-arm64 only. Windows and Linux packages have never been built with the Rust engine: nothing is pushed, so no CI has run (`engine/design/m10-macos-package.md`, "Windows and Linux").

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

Machine limits on the 8 GB build Mac: run the cargo build with `CARGO_BUILD_JOBS=2`. `build-workflow.sh` from an empty target directory took 6.5 to 7.5 minutes (the wasm32 build about 3.5 minutes, then the native `tandem-rust-workflow` binary), at a peak of about 1,170 MB. The package step peaked at 546 MB.

## Reproducible WASM

`engine/tools/build-workflow.sh` makes the WASM independent of the checkout path, the target folder and the builder's home folder. It does this in two ways, for the wasm32 build only (the native binary it builds afterwards is unchanged):

1. **Path remapping.** rustc writes the path of every source file that can panic into the module (panic locations). The script appends `--remap-path-prefix` flags through `CARGO_ENCODED_RUSTFLAGS`: `$RUSTUP_HOME` → `/rustup`, `$CARGO_HOME` (registry sources) → `/cargo`, the repository root → `/tpdf`, `$CARGO_TARGET_DIR` → `/target`. The paths are physical (`pwd -P`). A caller's `CARGO_ENCODED_RUSTFLAGS` or `RUSTFLAGS` are kept and come first. As always with cargo, either variable replaces `build.rustflags` from a cargo config.
2. **Crate hashes.** cargo hashes a path dependency that lies outside the workspace root by its absolute path into the crate's `-C metadata`. The bridge (`engine/bridge`) is its own workspace, so this applies to the engine (`..`) and the vendored hayro crates. rustc derives symbol names and item order from that hash, so remapping alone left the code different between checkouts: in a first try, five crates' hashes and the type, function, element, code and name sections differed. `engine/tools/rustc-stable-metadata.sh`, set as `RUSTC_WRAPPER` for that build, replaces each crate's `-C metadata` with a hash of its name, version and rustc arguments. The machine roots are replaced, and cargo's own hashes are left out, including the `-<16 hex>` in dependency file names, so a dependency counts by its crate name. An earlier version named dependencies by their new hashes instead. It reproduced across clean builds but not in an incremental target folder, where host proc macros built before the wrapper keep their old hashes: the gates' build gave `ea10af80…`. Output file names are unchanged. A caller's `RUSTC_WRAPPER` (for example sccache) still runs the compiler, behind it. The flags also carry a remap that matches no path, `/tpdf-stable-metadata-<wrapper sha256, 16 hex>`. cargo fingerprints the flags, so a changed wrapper rebuilds the wasm32 crates instead of reusing ones hashed by the old rule.

   macOS drops `DYLD_*` variables when `/bin/sh` starts, so the wrapper restores the toolchain's `lib` folder in `DYLD_FALLBACK_LIBRARY_PATH`, as rustup sets it. Without it, `rust-lld` cannot load `libLLVM.dylib`.

   Known limits of the wrapper (final-head verification, recorded rather than changed: any edit to the wrapper changes every crate's flags through the remap above, so it changes the published WASM hash):
   - A joined `-Cmetadata=X` argument would be rewritten to a bare `metadata=X` and the build would fail loudly. cargo always passes `-C metadata=X` as two arguments.
   - A `-C incremental=<dir>` argument stays in the hash. Release builds are not incremental; setting `CARGO_INCREMENTAL=1` would make the WASM depend on the target folder again.
   - Setting `RUSTC_WRAPPER` to this script itself before calling `build-workflow.sh` makes it call itself without end. Leave `RUSTC_WRAPPER` unset or set it to another wrapper.

`tests/m10-package.test.mjs` fails when `engine/web/pkg`'s WASM contains the home folder, the checkout path, `/Users/` or `/home/`. `scripts/check-macos-package.mjs` checks the packaged WASM the same way (item `wasm-paths`).

The staleness check is unchanged: `sources.sha256` lists the sources (Cargo manifests, the bridge lockfile, `src/`, `bridge/src/`, `vendor/`), not `tools/`. A package built before this change therefore still counts as fresh, with the old, path-dependent hash. Rebuild it once.

## Expected outputs for this commit

Built with the commands above: `rustc 1.98.1 (48a229cea 2026-09-01)`, wasm-bindgen-cli 0.2.129, macOS 26.6.2 arm64.

| Output | sha256 | Size |
| --- | --- | --- |
| `engine/web/pkg/tandem_rust_workflow_bg.wasm` | `31e0f62b95b28d5a0881563ad6f4f7bfa7fa1d1e2ff807c4d96891280d102a1a` | 13,506,487 bytes |
| `engine/web/pkg/tandem_rust_workflow.js` | `a9261f14f6da838d6f032db78f68b84fd2486980c03540526656a06f523cb890` | |
| `engine/web/pkg/sources.sha256` (sha256 of the listing) | `a515e15e20b33afbd7416b629d17dadbd69e13ae908472a7a1bd662b55c41103` | |

Verified on October 17, 2026 (logs in `tpdf-wt/scratch/p8/pkg/logs/repro*.log`, not committed). Three builds, one after the other, gave byte-identical WASM, `.js` and `.d.ts` files. A and B started from empty target folders, B from a copy of `engine/` at another path. C was the worktree's own incremental `engine/bridge/target`, last used by the gates with the earlier wrapper:

```sh
export WASM_BINDGEN_BIN=$HOME/.local/tpdf-wasm-tools/bin/wasm-bindgen
CARGO_TARGET_DIR=<scratch>/target-A sh /Users/owner/Documents/tpdf-wt/p10-pkg/engine/tools/build-workflow.sh   # A
CARGO_TARGET_DIR=<scratch>/target-B sh <scratch>/repro-b/engine/tools/build-workflow.sh                       # B, another path
sh /Users/owner/Documents/tpdf-wt/p10-pkg/engine/tools/build-workflow.sh                                 # C, incremental
```

The module contains no `/Users/` path: 120 source paths read `/tpdf/engine/…`, 225 read `/cargo/registry/src/…` and 45 read `/rustc/<commit>/…` (std, as shipped). The gates rebuilt it in `engine/bridge/target` again (`gates-out/p10-pkg`) with the same result.

The hash follows the engine's sources, so it changes with them. The final review's fixes (merged at 39b0699) changed the engine, and the gate build on 39b0699 gave `3c18b0e8cb8dcf868fa13c4aacb4f5a75e29efe3744b353245c298e94783de93`; the benchmarks used the same bytes (`engine/reference/bench-20261017/README.md`). b6bc7b9 does not change the sources. The three-build check above was not repeated on 39b0699.

**Final head ba70615 (reproduced).** `git diff 39b0699 ba70615` touches only documentation, `engine/reference/` and `engine/tools/bench/`, none of which the WASM is built from. On October 17, 2026 the WASM was built twice at ba70615 in the worktree `tpdf-wt/p10-verify`, a different checkout path from the main checkout where the gates built `3c18b0e8…`:

| Build | Target folder | Time | Peak (memcap) |
| --- | --- | --- | --- |
| incremental | `engine/bridge/target`, a copy of an earlier lane's cache (cargo recompiled 85 crates) | 4 min 34 s | 1,132 MB |
| clean | an empty `CARGO_TARGET_DIR` in a scratch folder, deleted afterwards | 5 min 59 s | 1,110 MB |

Both used `WASM_BINDGEN_BIN=$HOME/.local/tpdf-wasm-tools/bin/wasm-bindgen sh engine/tools/build-workflow.sh` with `rustc 1.98.1` and wasm-bindgen 0.2.129. Both gave the same files, byte for byte, as the main checkout's package:

| Output | sha256 | Size |
| --- | --- | --- |
| `engine/web/pkg/tandem_rust_workflow_bg.wasm` | `2de5d021c63352915f50d72890d9bc7bc824f7065506f398d1129217845f0d40` | 13,614,507 bytes |
| `engine/web/pkg/tandem_rust_workflow.js` | `a9261f14f6da838d6f032db78f68b84fd2486980c03540526656a06f523cb890` | |
| `engine/web/pkg/sources.sha256` (sha256 of the listing) | `905a7959224875a1fdb2148434ac53cd3c149e4af6cf5af0d3992be44987652f` | |

The module contains no `/Users/` path. `npm run build` copied the same bytes to `build/renderer/vendor/`.

Before this change the hash depended on the checkout path: `651fa243…` in this worktree and `6e900969…` in the gates' checkout, for the same sources. With remapping only, two checkouts gave `c3ec3b81…` and `427eabb4…`; the cause is in point 2 above.

The package as a whole is not reproducible byte for byte. `app.asar` and the bundle carry build-time metadata (the `ElectronAsarIntegrity` hash of this asar, file times), and the bundle is ad-hoc signed. Compare the WASM, then the files inside `app.asar` (`npx @electron/asar extract`).

## In CI (`.github/workflows/release.yml`)

The release workflow's `engine` job builds the WASM with this same script, `sh engine/tools/build-workflow.sh --wasm-only` (`--wasm-only` skips the native CLI; the wasm32 build, its remaps and the metadata wrapper are the same), on ubuntu-24.04 with Rust 1.98.1 and wasm-bindgen-cli 0.2.129. The job fails if the module names a build-machine path (`$HOME`, the workspace, `/home/`, `/Users/`). `engine/reference/wasm-release.sha256` records, in `sha256sum` format, the hashes of `sources.sha256`, the WASM and the glue from a local build. When the CI build has the same `sources.sha256`, its WASM and glue must match the record; when the sources differ, the record is stale and the job only warns. After an engine change, refresh it from a build made with the commands above:

```sh
cd engine/web/pkg && shasum -a 256 sources.sha256 tandem_rust_workflow_bg.wasm tandem_rust_workflow.js > ../../reference/wasm-release.sha256
```

Every installer is then checked by `scripts/check-package.mjs` (also used by `scripts/check-macos-package.mjs`): its WASM must be the engine job's, with no build-machine path, and the draft release's notes name its sha256. No CI run has happened yet, so a Linux build reproducing the macOS record is expected, not observed.

**Head 3d8a6ff (scanner port; reproduced).** The `scan-compose` and `scan-measure` ops changed the engine. The gates built `2de5d021c63352915f50d72890d9bc7bc824f7065506f398d1129217845f0d40` (13,614,507 bytes) in the main checkout on October 17, 2026, and a clean build into an empty target folder at a second checkout path (`tpdf-wt/p11-docs`) gave the same bytes; the glue `.js` (`a9261f14…`) and `sources.sha256` (`ca6ea9bc…`) match too. `engine/reference/wasm-release.sha256` records these values for `release.yml`. Refresh it whenever the engine changes.
