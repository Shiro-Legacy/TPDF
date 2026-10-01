#!/usr/bin/env bash
# Install the packaged Linux build for this user (no root): copy it under ~/.local/share/tandem-pdf (or --to=<dir>),
# add a launcher (.desktop file + icon) and list it under "Open with" for PDFs. The default PDF app is left as it is.
# The launcher and its icon always go under ~/.local/share: the desktop looks for them nowhere else.
# Run `npm run package` first, quit a running Tandem PDF, then from the repo root:
#   bash scripts/install-linux.sh [--uninstall] [--dry-run] [--from=<dir>] [--to=<dir>]
# --to is the app folder itself (made, or replaced if it holds an earlier install); give it to --uninstall as well.
# The one step that needs root is not taken here: where AppArmor restricts unprivileged user namespaces
# (Ubuntu 23.10 and later) Chromium's sandbox cannot start and the app aborts with "No usable sandbox" /
# "SUID sandbox helper … not configured correctly". The script writes the AppArmor profile that allows it and
# prints the two sudo commands that load it. Never start the app with --no-sandbox instead.
set -euo pipefail
name='Tandem PDF'
id='tandem-pdf'
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
from="$root/out/$name-linux-x64"
uninstall=0
dry=0
to=
for arg in "$@"; do
  case $arg in
    --uninstall) uninstall=1 ;;
    --dry-run) dry=1 ;;
    --from=*) from=${arg#--from=} ;;
    --to=*) to=${arg#--to=} ;;
    *) echo "usage: bash scripts/install-linux.sh [--uninstall] [--dry-run] [--from=<dir>] [--to=<dir>]" >&2; exit 2 ;;
  esac
done
data=${XDG_DATA_HOME:-$HOME/.local/share}
dest=$(realpath -m -- "${to:-$data/$id}")
exe="$dest/$name"
# $dest is removed whole below, so it must be ours: absent, empty, or an earlier install (it holds the binary).
if [[ -e $dest && ! -x $exe && -n $(ls -A -- "$dest" 2>/dev/null) ]]; then
  echo "$dest exists and is not a Tandem PDF install. Give --to a folder of its own." >&2; exit 1
fi
desktop="$data/applications/$id.desktop"
icon="$data/icons/hicolor/512x512/apps/$id.png"
profile="$dest/$id.apparmor"
run() { if ((dry)); then printf 'would run:'; printf ' %q' "$@"; printf '\n'; else "$@"; fi; }
refresh() { if command -v update-desktop-database >/dev/null; then run update-desktop-database "$data/applications"; fi; }

if ((uninstall)); then
  run rm -rf -- "$dest"
  run rm -f -- "$desktop" "$icon"
  refresh
  echo "Removed $dest (app data stays in ${XDG_CONFIG_HOME:-$HOME/.config}/$name)"
  if [[ -e /etc/apparmor.d/$id ]]; then echo "The AppArmor profile stays until you run: sudo apparmor_parser -R /etc/apparmor.d/$id && sudo rm /etc/apparmor.d/$id"; fi
  exit 0
fi

if [[ ! -x "$from/$name" ]]; then echo "No build at $from. Run: npm run package" >&2; exit 1; fi
if pgrep -f -- "^$exe( |\$)" >/dev/null; then echo "Tandem PDF is running from $dest. Quit it first." >&2; exit 1; fi
run mkdir -p "$(dirname "$dest")" "$(dirname "$desktop")" "$(dirname "$icon")"
# A mirror, as robocopy /MIR does on Windows: files a newer build dropped must not stay behind.
run rm -rf -- "$dest"
run cp -a -- "$from" "$dest"
run install -m 644 -- "$root/assets/icon.png" "$icon"
# Exec: the path is quoted, and inside the quotes the Desktop Entry spec wants \ " ` $ escaped. %f, not %F:
# one document per window, a second start hands its PDF to the running instance.
quoted=$(printf '%s' "$exe" | sed -e 's/[\\"`$]/\\&/g')
entry="[Desktop Entry]
Type=Application
Name=$name
Comment=Local, offline PDF editor a person and an AI agent can both drive
Exec=\"$quoted\" %f
Icon=$id
Terminal=false
Categories=Office;Viewer;
MimeType=application/pdf;
StartupWMClass=$id
"
# AppArmor names the binary by path; the profile confines nothing, it only lets the sandbox make its namespaces.
rules="abi <abi/4.0>,
include <tunables/global>

profile $id \"$exe\" flags=(unconfined) {
  userns,

  include if exists <local/$id>
}
"
if ((dry)); then
  printf 'would write %s:\n%s\nwould write %s:\n%s' "$desktop" "$entry" "$profile" "$rules"
else
  printf '%s' "$entry" > "$desktop"
  printf '%s' "$rules" > "$profile"
fi
refresh
echo "Installed $exe"
echo "MCP server: $dest/resources/agent/scripts/tandem-mcp.mjs"
restricted=$(sysctl -n kernel.apparmor_restrict_unprivileged_userns 2>/dev/null || echo 0)
if [[ $restricted == 1 ]] && ! grep -qsF -- "\"$exe\"" "/etc/apparmor.d/$id"; then
  echo
  echo "This system restricts unprivileged user namespaces, so the app cannot start its sandbox yet."
  echo "Load the AppArmor profile once (needs root):"
  printf '  sudo install -m 644 %q /etc/apparmor.d/%s\n' "$profile" "$id"
  printf '  sudo apparmor_parser -r /etc/apparmor.d/%s\n' "$id"
fi
