#!/usr/bin/env bash
# Wrap the packaged macOS app in a drag-to-Applications disk image: out/Tandem-PDF-<version>-mac-arm64.dmg.
# Run `npm run package` first. Signing and notarization happen there (scripts/package.mjs, APPLE_SIGN_IDENTITY):
# Gatekeeper checks the app, which carries its own stapled ticket, so the image itself is left unsigned.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
name='Tandem PDF'
version=$(node -p "require('./package.json').version")
app="out/$name-darwin-arm64/$name.app"
dmg="out/Tandem-PDF-$version-mac-arm64.dmg"
if [[ ! -d $app ]]; then echo "No build at $app. Run: npm run package" >&2; exit 1; fi
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
ditto "$app" "$stage/$name.app"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "$name" -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null
echo "$dmg"
