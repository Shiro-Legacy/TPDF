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
$SLOT $TAURI build --bundles app --config tandem-desktop/tauri.conf.json -- -p tandem-desktop

APP="target/release/bundle/macos/Tandem PDF.app"
echo "Built: $ROOT/crates/$APP"

# The disk image: the app and a link to /Applications. (Tauri's own .dmg step styles the window
# through Finder AppleScript, which fails without an interactive session; plain hdiutil does not.)
VERSION=$(python3 -c "import json;print(json.load(open('tandem-desktop/tauri.conf.json'))['version'])")
DMG="target/release/bundle/dmg/Tandem PDF_${VERSION}_aarch64.dmg"
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/" && ln -s /Applications "$STAGE/Applications"
mkdir -p target/release/bundle/dmg && rm -f "$DMG"
hdiutil create -quiet -volname "Tandem PDF" -srcfolder "$STAGE" -fs HFS+ -format UDZO "$DMG"
rm -rf "$STAGE"
[ "$APPLE_SIGNING_IDENTITY" != "-" ] && codesign --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$DMG"
du -sh "$APP" "$DMG"

# 4. Notarization happens inside `tauri build` when APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID are set
#    (with a real Developer ID). Check the result with:
#      spctl -a -vv "$APP"; xcrun stapler validate "$APP"
codesign --verify --deep --strict --verbose=2 "$APP" && echo "codesign: ok (${APPLE_SIGNING_IDENTITY})"
