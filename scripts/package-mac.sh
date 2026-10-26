#!/bin/zsh
# Packages the Rust Mac app (crates/tandem-desktop) as Tandem PDF.app and a .dmg.
#
#   scripts/package-mac.sh            # ad-hoc signed, not notarized (no Developer ID needed)
#   APPLE_SIGNING_IDENTITY="Developer ID Application: Name (TEAMID)" \
#   APPLE_ID=you@example.com APPLE_PASSWORD=app-specific-password APPLE_TEAM_ID=TEAMID \
#   scripts/package-mac.sh            # signed with the Developer ID, hardened runtime, notarized
#
# The Mac App Store build (App Sandbox, tandem-desktop/entitlements-appstore.plist):
#   MAS=local scripts/package-mac.sh  # sandboxed .app to try on this Mac (Apple Development or ad hoc)
#   MAS=1 APPLE_PROVISIONING_PROFILE=~/Downloads/Tandem_PDF_App_Store.provisionprofile \
#   APPLE_SIGNING_IDENTITY="Apple Distribution: Name (TEAMID)" \
#   APPLE_INSTALLER_IDENTITY="3rd Party Mac Developer Installer: Name (TEAMID)" \
#   BUILD_NUMBER=3 scripts/package-mac.sh   # a signed .pkg for App Store Connect (not uploaded)
#   (BUILD_NUMBER must grow with every upload; the version stays tauri.conf.json's.)
#
# UNIVERSAL=1 builds one app for Apple silicon and Intel Macs (universal-apple-darwin; needs
# `rustup target add x86_64-apple-darwin`). It is the default for MAS=1, so the App Store
# offers the app to every Mac; UNIVERSAL=0 turns it off. Output then goes to
# crates/target/universal-apple-darwin/release/bundle/.
#
# On the 8 GB development Mac heavy steps go through tpdf-wt/tools/slot; elsewhere (CI) they run directly.
# Tools: TAURI_CLI (else tpdf-wt's cargo-root cargo-tauri 2.12.3, else `cargo tauri`), WASM_BINDGEN_BIN.
# Output: crates/target/release/bundle/ (macos/Tandem PDF.app, dmg/Tandem PDF_<version>_<arch>.dmg).
[[ $MAS = 1 ]] && UNIVERSAL=${UNIVERSAL:-1}
OUT=target/release; TARGET=()
[[ $UNIVERSAL = 1 ]] && { OUT=target/universal-apple-darwin/release; TARGET=(--target universal-apple-darwin) }
set -e
ROOT=${0:A:h:h}
TOOLS=${TANDEM_TOOLS:-$HOME/Documents/tpdf-wt/tools}
SLOT=; [[ -x $TOOLS/slot ]] && SLOT=$TOOLS/slot
if [[ -n $TAURI_CLI ]]; then TAURI=(${=TAURI_CLI})
elif [[ -x $TOOLS/cargo-root/bin/cargo-tauri ]]; then TAURI=($TOOLS/cargo-root/bin/cargo-tauri)
else TAURI=(cargo tauri); fi
cd $ROOT/crates

# 1. The UI (WASM, release).
$SLOT sh tandem-ui/build.sh --release

# 2. Licence texts that ship inside the app: ours, the engine's NOTICE, and every crate's.
LIC=tandem-desktop/.stage/licenses
rm -rf $LIC && mkdir -p $LIC
cp $ROOT/LICENSE $LIC/LICENSE-Tandem-PDF.txt
cp $ROOT/engine/NOTICE $LIC/NOTICE-engine.txt
cp $ROOT/docs/third-party/droid-sans-fallback-NOTICE.txt $LIC/
[ -f $ROOT/crates/tandem-desktop/NOTICE ] && cp $ROOT/crates/tandem-desktop/NOTICE $LIC/NOTICE.txt
python3 $ROOT/scripts/crate-licenses.py $ROOT/crates $LIC --target $(rustc -vV | sed -n 's/^host: //p')

# 3. The app icon as an asset catalog too (Assets.car, named by CFBundleIconName): App Store Connect
#    and Transporter read the icon from there. CFBundleIconFile stays icon.icns, which has every
#    size up to 512@2x (actool's own AppIcon.icns stops at 256, too small for App Store checks).
#    Needs Xcode's actool; without it only icon.icns ships (an App Store upload refuses to go on).
STAGE=$ROOT/crates/tandem-desktop/.stage
ICONS=$STAGE/icon
rm -rf $ICONS
if xcrun --find actool >/dev/null 2>&1; then
  SET=$ICONS/Assets.xcassets/AppIcon.appiconset
  mkdir -p $SET $ICONS/out
  iconutil -c iconset tandem-desktop/icons/icon.icns -o $ICONS/icon.iconset
  cp $ICONS/icon.iconset/*.png $SET/
  python3 - $ICONS/Assets.xcassets <<'PY'
import json, sys
info = {"version": 1, "author": "xcode"}
images = [{"idiom": "mac", "size": f"{n}x{n}", "scale": f"{s}x",
           "filename": f"icon_{n}x{n}{'@2x' if s == 2 else ''}.png"}
          for n in (16, 32, 128, 256, 512) for s in (1, 2)]
json.dump({"images": images, "info": info}, open(sys.argv[1] + "/AppIcon.appiconset/Contents.json", "w"))
json.dump({"info": info}, open(sys.argv[1] + "/Contents.json", "w"))
PY
  MINOS=$(python3 -c "import json;print(json.load(open('tandem-desktop/tauri.conf.json'))['bundle']['macOS']['minimumSystemVersion'])")
  xcrun actool --compile $ICONS/out --platform macosx --target-device mac --minimum-deployment-target $MINOS \
    --app-icon AppIcon --output-partial-info-plist $ICONS/out/partial.plist $ICONS/Assets.xcassets > $ICONS/actool.log
  plutil -create xml1 $ICONS/out/Info.plist
  plutil -insert CFBundleIconName -string AppIcon $ICONS/out/Info.plist
elif [[ $MAS = 1 ]]; then
  echo "MAS=1 needs Xcode's actool (the asset catalog icon)"; exit 1
else
  echo "No actool (Xcode): the app ships icon.icns only"
fi

# 4. Build and bundle. Without a Developer ID the app is signed ad hoc ("-") so it runs on this Mac.
export APPLE_SIGNING_IDENTITY=${APPLE_SIGNING_IDENTITY:--}
ENT= PROFILE_FILE= BUNDLE_VERSION=
if [[ -n $MAS ]]; then
  # The sandbox entitlements; an upload also names the app and team the profile was made for.
  ENT=$STAGE/entitlements-appstore.plist
  cp tandem-desktop/entitlements-appstore.plist $ENT
  if [[ $MAS != local ]]; then
    [[ -f $APPLE_PROVISIONING_PROFILE ]] || { echo "MAS=1 needs APPLE_PROVISIONING_PROFILE (a Mac App Store profile)"; exit 1; }
    [[ -n $APPLE_INSTALLER_IDENTITY && $APPLE_SIGNING_IDENTITY != - ]] || { echo "MAS=1 needs APPLE_SIGNING_IDENTITY and APPLE_INSTALLER_IDENTITY"; exit 1; }
    [[ -n $BUILD_NUMBER ]] || { echo "MAS=1 needs BUILD_NUMBER (higher than the last upload's)"; exit 1; }
    PROFILE=$(mktemp)
    security cms -D -i $APPLE_PROVISIONING_PROFILE > $PROFILE
    for KEY in com.apple.application-identifier com.apple.developer.team-identifier; do
      VALUE=$(/usr/libexec/PlistBuddy -c "Print :Entitlements:$KEY" $PROFILE)
      /usr/libexec/PlistBuddy -c "Add :$KEY string $VALUE" $ENT
    done
    rm -f $PROFILE
    PROFILE_FILE=${APPLE_PROVISIONING_PROFILE:A}
    # An upload is checked by App Review, not notarized.
    unset APPLE_ID APPLE_PASSWORD APPLE_API_KEY APPLE_API_ISSUER
  fi
  BUNDLE_VERSION=$BUILD_NUMBER
fi
# What this build adds to tauri.conf.json: the icon catalog, and for the App Store its entitlements,
# profile and build number.
ICON_OUT=$ICONS/out ENT=$ENT PROFILE_FILE=$PROFILE_FILE BUNDLE_VERSION=$BUNDLE_VERSION \
  python3 - $STAGE/tauri.overlay.json <<'PY'
import json, os, sys
env = lambda k: os.environ.get(k) or None
mac, files = {}, {}
icons = env("ICON_OUT")
if icons and os.path.isfile(icons + "/Assets.car"):
    files["Resources/Assets.car"] = icons + "/Assets.car"
    mac["infoPlist"] = icons + "/Info.plist"
if env("ENT"):
    mac["entitlements"] = env("ENT")
if env("PROFILE_FILE"):
    files["embedded.provisionprofile"] = env("PROFILE_FILE")
if env("BUNDLE_VERSION"):
    mac["bundleVersion"] = env("BUNDLE_VERSION")
if files:
    mac["files"] = files
json.dump({"bundle": {"macOS": mac}}, open(sys.argv[1], "w"), indent=1)
PY
OVERLAY=(--config $STAGE/tauri.overlay.json)
$SLOT $TAURI build --bundles app $TARGET --config tandem-desktop/tauri.conf.json $OVERLAY -- -p tandem-desktop

APP="$OUT/bundle/macos/Tandem PDF.app"
echo "Built: $ROOT/crates/$APP"
codesign --verify --deep --strict --verbose=2 "$APP" && echo "codesign: ok (${APPLE_SIGNING_IDENTITY})"

if [[ -n $MAS ]]; then
  codesign -d --entitlements - "$APP" 2>/dev/null | grep -q app-sandbox || { echo "not sandboxed"; exit 1; }
  [[ $MAS = local ]] && { echo "Sandboxed app to try here: $ROOT/crates/$APP"; exit 0; }
  # The installer package App Store Connect takes. Upload it with Transporter, or
  #   xcrun altool --upload-app -t macos -f <pkg> --apiKey <key id> --apiIssuer <issuer id>
  VERSION=$(python3 -c "import json;print(json.load(open('tandem-desktop/tauri.conf.json'))['version'])")
  PKG="$OUT/bundle/appstore/Tandem PDF_${VERSION}_${BUILD_NUMBER}.pkg"
  mkdir -p $OUT/bundle/appstore && rm -f "$PKG"
  productbuild --component "$APP" /Applications --sign "$APPLE_INSTALLER_IDENTITY" "$PKG"
  echo "App Store package (not uploaded): $ROOT/crates/$PKG"
  exit 0
fi

# The disk image: the app and a link to /Applications. (Tauri's own .dmg step styles the window
# through Finder AppleScript, which fails without an interactive session; plain hdiutil does not.)
VERSION=$(python3 -c "import json;print(json.load(open('tandem-desktop/tauri.conf.json'))['version'])")
ARCH=$(uname -m); [[ $ARCH = arm64 ]] && ARCH=aarch64; [[ $ARCH = x86_64 ]] && ARCH=x64
[[ $UNIVERSAL = 1 ]] && ARCH=universal
DMG="$OUT/bundle/dmg/Tandem PDF_${VERSION}_${ARCH}.dmg"
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/" && ln -s /Applications "$STAGE/Applications"
mkdir -p $OUT/bundle/dmg && rm -f "$DMG"
hdiutil create -quiet -volname "Tandem PDF" -srcfolder "$STAGE" -fs HFS+ -format UDZO "$DMG"
rm -rf "$STAGE"
[ "$APPLE_SIGNING_IDENTITY" != "-" ] && codesign --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$DMG"
du -sh "$APP" "$DMG"

# 5. Notarization happens inside `tauri build` when APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID are set
#    (with a real Developer ID). Check the result with:
#      spctl -a -vv "$APP"; xcrun stapler validate "$APP"
echo "Installer: $ROOT/crates/$DMG"
