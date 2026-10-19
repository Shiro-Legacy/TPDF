#!/bin/zsh
# Packages the Rust Mac app (crates/tandem-desktop) as Tandem PDF.app and a .dmg.
#
#   scripts/package-mac.sh            # ad-hoc signed, not notarized (no Developer ID needed)
#   APPLE_SIGNING_IDENTITY="Developer ID Application: Name (TEAMID)" \
#   APPLE_ID=you@example.com APPLE_PASSWORD=app-specific-password APPLE_TEAM_ID=TEAMID \
#   scripts/package-mac.sh            # signed with the Developer ID, hardened runtime, notarized
#
# Heavy steps go through tpdf-wt/tools/slot (8 GB Mac). Output: crates/target/release/bundle/.
set -e
ROOT=${0:A:h:h}
SLOT=/Users/owner/Documents/tpdf-wt/tools/slot
TAURI=${TAURI_CLI:-/Users/owner/Documents/tpdf-wt/tools/cargo-root/bin/cargo-tauri}
WBG=${WASM_BINDGEN_BIN:-$HOME/.local/tpdf-wasm-tools/bin/wasm-bindgen}
cd $ROOT/crates

# 1. The UI (WASM, release).
WASM_BINDGEN_BIN=$WBG $SLOT sh tandem-ui/build.sh --release

# 2. Licence texts that ship inside the app: ours, the engine's NOTICE, and every crate's.
LIC=tandem-desktop/.stage/licenses
rm -rf $LIC && mkdir -p $LIC
cp $ROOT/LICENSE $LIC/LICENSE-Tandem-PDF.txt
cp $ROOT/engine/NOTICE $LIC/NOTICE-engine.txt
[ -f $ROOT/crates/tandem-desktop/NOTICE ] && cp $ROOT/crates/tandem-desktop/NOTICE $LIC/NOTICE.txt
python3 $ROOT/scripts/crate-licenses.py $ROOT/crates $LIC

# 3. Build and bundle. Without a Developer ID the app is signed ad hoc ("-") so it runs on this Mac.
export APPLE_SIGNING_IDENTITY=${APPLE_SIGNING_IDENTITY:--}
$SLOT $TAURI build --bundles app,dmg --config tandem-desktop/tauri.conf.json -- -p tandem-desktop

APP="target/release/bundle/macos/Tandem PDF.app"
echo "Built: $ROOT/crates/$APP"
du -sh "$APP" target/release/bundle/dmg/*.dmg 2>/dev/null || true

# 4. Notarization happens inside `tauri build` when APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID are set
#    (with a real Developer ID). Check the result with:
#      spctl -a -vv "$APP"; xcrun stapler validate "$APP"
codesign --verify --deep --strict --verbose=2 "$APP" && echo "codesign: ok (${APPLE_SIGNING_IDENTITY})"
