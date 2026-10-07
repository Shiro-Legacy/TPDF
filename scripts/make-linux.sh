#!/usr/bin/env bash
# Wrap the packaged Linux build two ways (run `npm run package` first; needs dpkg-deb, so a Debian or Ubuntu host):
#   out/tandem-pdf_<version>_amd64.deb         Ubuntu and Debian: /opt/tandem-pdf, launcher, "Open with", sandbox set up
#   out/Tandem-PDF-<version>-linux-x64.tar.gz  any other glibc x64 distro: unpack, then `bash scripts/install-linux.sh`
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
name='Tandem PDF'
id='tandem-pdf'
version=$(node -p "require('./package.json').version")
from="out/$name-linux-x64"
if [[ ! -x "$from/$name" ]]; then echo "No build at $from. Run: npm run package" >&2; exit 1; fi
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

# The tarball mirrors the repo layout so scripts/install-linux.sh runs from it unchanged.
tgz="$stage/Tandem-PDF-$version-linux-x64"
mkdir -p "$tgz/out" "$tgz/scripts" "$tgz/assets"
cp -a "$from" "$tgz/out/"
chmod -R a+rX "$tgz/out"
cp scripts/install-linux.sh "$tgz/scripts/"
cp assets/icon.png "$tgz/assets/"
tar -C "$stage" -czf "out/Tandem-PDF-$version-linux-x64.tar.gz" "Tandem-PDF-$version-linux-x64"

deb="$stage/deb"
exe="/opt/$id/$name"
mkdir -p "$deb/DEBIAN" "$deb/opt" "$deb/usr/share/applications" "$deb/usr/share/icons/hicolor/512x512/apps"
cp -a "$from" "$deb/opt/$id"
# Installed by root for every user: every directory traversable and every file readable by all. (@electron/packager
# creates the app folder with mkdtemp, mode 0700; scripts/package.mjs widens it, this guards an older build too.)
chmod -R a+rX "$deb/opt/$id"
# Chromium's setuid sandbox helper, for systems with no unprivileged user namespaces at all.
chmod 4755 "$deb/opt/$id/chrome-sandbox"
install -m 644 assets/icon.png "$deb/usr/share/icons/hicolor/512x512/apps/$id.png"
# The same entry scripts/install-linux.sh writes for a per-user install.
cat > "$deb/usr/share/applications/$id.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=$name
Comment=Local, offline PDF editor a person and an AI agent can both drive
Exec="$exe" %f
Icon=$id
Terminal=false
Categories=Office;Viewer;
MimeType=application/pdf;
StartupWMClass=$id
EOF
# The same AppArmor profile too (it confines nothing, it only lets the sandbox make its namespaces). It is kept in
# the app folder and loaded by postinst only where the parser knows `userns` (AppArmor 4, Ubuntu 24.04 and later):
# an older parser would reject the file at every boot, and has no such restriction to lift.
cat > "$deb/opt/$id/$id.apparmor" <<EOF
abi <abi/4.0>,
include <tunables/global>

profile $id "$exe" flags=(unconfined) {
  userns,

  include if exists <local/$id>
}
EOF
cat > "$deb/DEBIAN/control" <<EOF
Package: $id
Version: $version
Architecture: amd64
Maintainer: Shiro-Legacy <307989901+Shiro-Legacy@users.noreply.github.com>
Homepage: https://github.com/Shiro-Legacy/TPDF
Section: text
Priority: optional
Installed-Size: $(du -sk "$deb/opt" | cut -f1)
Depends: libgtk-3-0, libnss3, libnspr4, libasound2, libgbm1, libdrm2, libatk-bridge2.0-0, libatspi2.0-0, libcups2, libxkbcommon0, libxcomposite1, libxdamage1, libxfixes3, libxrandr2, xdg-utils
Description: Local, offline PDF editor a person and an AI agent can both drive
 Read, annotate, fill and design forms, edit text and images, redact, sign
 and compare PDFs. No account and no network at run time. Carries an MCP
 server so an AI agent can drive the same editor.
EOF
cat > "$deb/DEBIAN/postinst" <<EOF
#!/bin/sh
set -e
if [ -e /etc/apparmor.d/abi/4.0 ]; then
  install -m 644 "/opt/$id/$id.apparmor" /etc/apparmor.d/$id
  apparmor_parser -r /etc/apparmor.d/$id || true
fi
if command -v update-desktop-database >/dev/null; then update-desktop-database -q /usr/share/applications || true; fi
EOF
cat > "$deb/DEBIAN/postrm" <<EOF
#!/bin/sh
set -e
if [ "\$1" = remove ] || [ "\$1" = purge ]; then
  if [ -e /etc/apparmor.d/$id ]; then
    apparmor_parser -R /etc/apparmor.d/$id 2>/dev/null || true
    rm -f /etc/apparmor.d/$id
  fi
fi
EOF
chmod 755 "$deb/DEBIAN/postinst" "$deb/DEBIAN/postrm"
dpkg-deb --root-owner-group --build "$deb" "out/${id}_${version}_amd64.deb" >/dev/null
echo "out/${id}_${version}_amd64.deb"
echo "out/Tandem-PDF-$version-linux-x64.tar.gz"
