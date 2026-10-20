#!/usr/bin/env bash
# Packages the Rust desktop app (crates/tandem-desktop) for Linux: a .deb and an AppImage.
#
#   bash scripts/package-linux.sh
#
# Needs: Rust with the wasm32-unknown-unknown target, wasm-bindgen-cli 0.2.129 (WASM_BINDGEN_BIN, else
# ~/.local/tpdf-wasm-tools/bin/wasm-bindgen), tauri-cli 2.12.3 (TAURI_CLI, else `cargo tauri`), python3,
# and the Tauri build packages (Debian/Ubuntu: libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libdbus-1-dev
# libssl-dev pkg-config build-essential file patchelf). Build on the oldest supported Ubuntu (22.04): the
# binary then runs on the newer ones too (glibc is backwards compatible, not forwards).
#
# Output (Tauri's default places), printed at the end:
#   crates/target/release/bundle/deb/Tandem PDF_<version>_amd64.deb          (package tandem-pdf, /usr/bin/tandem-pdf)
#   crates/target/release/bundle/appimage/Tandem PDF_<version>_amd64.AppImage
#
# Licence texts: ours, the engine's NOTICE and every linked crate's go into both packages
# (/usr/lib/Tandem PDF/licenses). The AppImage also carries the system libraries it bundles (GTK, WebKitGTK,
# GLib and what they pull in; mostly LGPL), so this script lists them after a first AppImage build, copies
# each one's Debian copyright file and /usr/share/common-licenses into licenses/system-libraries, and builds the
# AppImage again with them. That needs dpkg (Debian/Ubuntu builders); elsewhere it stops unless
# TANDEM_APPIMAGE_WITHOUT_SYSTEM_LICENSES=1 (then the AppImage lacks those texts and the script says so).
#
# Signing: none. (An AppImage can carry a GPG signature and a .deb can be signed with debsigs; neither is set up.)
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
if [ -n "${TAURI_CLI:-}" ]; then read -r -a TAURI <<<"$TAURI_CLI"; else TAURI=(cargo tauri); fi
TARGET=$(rustc -vV | sed -n 's/^host: //p')
cd "$ROOT/crates"

# 1. The UI (WASM, release); tandem-desktop embeds it at compile time.
sh tandem-ui/build.sh --release

# 2. Licence texts that ship inside both packages: ours, the engine's NOTICE, and every crate linked on this target.
LIC=tandem-desktop/.stage/licenses
rm -rf "$LIC" && mkdir -p "$LIC"
cp "$ROOT/LICENSE" "$LIC/LICENSE-Tandem-PDF.txt"
cp "$ROOT/engine/NOTICE" "$LIC/NOTICE-engine.txt"
cp "$ROOT/docs/third-party/droid-sans-fallback-NOTICE.txt" "$LIC/"
if [ -f tandem-desktop/NOTICE ]; then cp tandem-desktop/NOTICE "$LIC/NOTICE.txt"; fi
python3 "$ROOT/scripts/crate-licenses.py" "$ROOT/crates" "$LIC" --target "$TARGET"

# 3. Build and bundle (tauri.linux.conf.json: deb + AppImage, the .desktop template with %F and the PDF type).
rm -rf target/release/bundle/deb target/release/bundle/appimage
"${TAURI[@]}" build --bundles deb,appimage -- -p tandem-desktop

# 4. The AppImage's system libraries and their licence texts, then the AppImage again with them.
APPDIR="target/release/bundle/appimage/Tandem PDF.AppDir"
SYS="$ROOT/crates/tandem-desktop/.stage/appimage-licenses"
rm -rf "$SYS" && mkdir -p "$SYS/packages"
if command -v dpkg-query >/dev/null && [ -d "$APPDIR" ]; then
  {
    echo "System libraries inside the Tandem PDF AppImage, with the Debian/Ubuntu package each was copied from."
    echo "Built on: $(. /etc/os-release && echo "$PRETTY_NAME"). Each package's copyright file (its licences and"
    echo "authors) is in packages/<package>/copyright; the licence texts they refer to are in common-licenses/."
    echo "The source of each package version is available from the distribution (apt-get source <package>=<version>)."
    echo "AppRun (the AppImage's start program) and the AppImage runtime come from AppImageKit (MIT)."
    echo
  } >"$SYS/SYSTEM-LIBRARIES.txt"
  : >"$SYS/list.tmp"
  unknown=()
  nocopyright=()
  # ELF files only (libraries and WebKit's helper processes); our own binary is not a system library.
  while IFS= read -r -d '' f; do
    rel=${f#"$APPDIR"}
    case "$rel" in /usr/bin/tandem-pdf | "/usr/lib/Tandem PDF/"* | /AppRun) continue ;; esac
    [ "$(head -c4 "$f" | od -An -tx1 | tr -d ' \n')" = 7f454c46 ] || continue
    # The same path on this system, else a file of that name (linuxdeploy flattens libraries into usr/lib).
    pkg=$(dpkg-query -S "$rel" 2>/dev/null | head -1 | sed 's/[:,].*//' || true)
    [ -n "$pkg" ] || pkg=$(dpkg-query -S "*/$(basename "$rel")" 2>/dev/null | grep -v -- '-dev[:,]' | head -1 | sed 's/[:,].*//' || true)
    if [ -z "$pkg" ]; then
      unknown+=("$rel")
      continue
    fi
    ver=$(dpkg-query -W -f '${Version}' "$pkg" 2>/dev/null || echo '?')
    printf '%s\t%s %s\n' "$rel" "$pkg" "$ver" >>"$SYS/list.tmp"
    if [ ! -d "$SYS/packages/$pkg" ]; then
      if [ -f "/usr/share/doc/$pkg/copyright" ]; then
        mkdir -p "$SYS/packages/$pkg" && cp "/usr/share/doc/$pkg/copyright" "$SYS/packages/$pkg/copyright"
      else
        nocopyright+=("$pkg")
      fi
    fi
  done < <(find "$APPDIR" -type f -print0)
  sort "$SYS/list.tmp" >>"$SYS/SYSTEM-LIBRARIES.txt"
  count=$(wc -l <"$SYS/list.tmp")
  rm -f "$SYS/list.tmp"
  cp -R /usr/share/common-licenses "$SYS/common-licenses"
  if [ ${#nocopyright[@]} -gt 0 ]; then
    printf '\nPackages without a copyright file on the builder (find their licence by hand):\n' >>"$SYS/SYSTEM-LIBRARIES.txt"
    printf '%s\n' "${nocopyright[@]}" | sort -u | sed 's/^/  /' >>"$SYS/SYSTEM-LIBRARIES.txt"
    echo "WARNING: ${#nocopyright[@]} package(s) without a copyright file; see SYSTEM-LIBRARIES.txt" >&2
  fi
  if [ ${#unknown[@]} -gt 0 ]; then
    printf '\nNot traced to a package (check by hand):\n' >>"$SYS/SYSTEM-LIBRARIES.txt"
    printf '  %s\n' "${unknown[@]}" >>"$SYS/SYSTEM-LIBRARIES.txt"
    echo "WARNING: ${#unknown[@]} AppImage file(s) not traced to a package; see SYSTEM-LIBRARIES.txt" >&2
  fi
  echo "AppImage system libraries: $count files from $(find "$SYS/packages" -mindepth 1 -maxdepth 1 | wc -l) packages"
elif [ "${TANDEM_APPIMAGE_WITHOUT_SYSTEM_LICENSES:-}" = 1 ]; then
  echo "WARNING: no dpkg-query (or no AppDir): the AppImage ships WITHOUT its system libraries' licence texts." >&2
else
  echo "error: listing the AppImage's system libraries needs dpkg-query (a Debian/Ubuntu builder) and $APPDIR." >&2
  echo "Set TANDEM_APPIMAGE_WITHOUT_SYSTEM_LICENSES=1 to build anyway (the AppImage then lacks those licence texts)." >&2
  exit 1
fi
cat >"$SYS/NOTE.txt" <<'NOTE'
The AppImage bundles system libraries from the Linux distribution it was built on (GTK 3, WebKitGTK 4.1, GLib,
libsoup, Pango, Cairo, HarfBuzz, ICU and others), so it runs where they are missing or older. Most are under the
GNU LGPL (2.1 or later); some under other free licences. SYSTEM-LIBRARIES.txt lists each file, its package and
version; packages/<package>/copyright holds each package's licence statement and common-licenses/ the full texts.
You may replace these libraries: extract the AppImage (--appimage-extract) and swap the files in usr/lib.
The .deb does not bundle them (it depends on the system's packages).
NOTE
CONF=$(mktemp)
python3 -c 'import json, sys; print(json.dumps({"bundle": {"linux": {"appimage": {"files": {"/usr/lib/Tandem PDF/licenses/system-libraries": sys.argv[1]}}}}}))' "$SYS" >"$CONF"
"${TAURI[@]}" build --bundles appimage --config "$CONF" -- -p tandem-desktop
rm -f "$CONF"

# 5. What was built.
DEB=$(ls target/release/bundle/deb/*.deb)
IMG=$(ls target/release/bundle/appimage/*.AppImage)
if command -v dpkg-deb >/dev/null; then
  dpkg-deb --field "$DEB" Package Version Depends Recommends
  files=$(dpkg-deb --contents "$DEB")
  grep -q ' \./usr/bin/tandem-pdf$' <<<"$files" || { echo "error: the .deb has no /usr/bin/tandem-pdf" >&2; exit 1; }
  grep -q '/licenses/THIRD-PARTY.txt$' <<<"$files" || { echo "error: the .deb has no licence texts" >&2; exit 1; }
  desktop=$(dpkg-deb --fsys-tarfile "$DEB" | tar -xO --wildcards './usr/share/applications/*.desktop')
  echo "$desktop"
  grep -qx 'Exec=tandem-pdf %F' <<<"$desktop" || { echo "error: the .desktop file does not take files (Exec ... %F)" >&2; exit 1; }
  grep -qx 'MimeType=application/pdf;' <<<"$desktop" || { echo "error: the .desktop file does not name PDF" >&2; exit 1; }
fi
du -h "$DEB" "$IMG"
echo "Installer: $ROOT/crates/$DEB"
echo "Installer: $ROOT/crates/$IMG"
