#!/bin/sh
# SPDX-License-Identifier: MIT
# Copies the licence files of every crate tandem-sign compiles (cargo tree for wasm32-unknown-unknown, the
# platform-independent set; normal edges, no proc macros) from the cargo registry into licenses/<crate>-<version>/,
# and lists them with SHA-256 in
# licenses/license-files.json. A crate that ships none takes the files kept in licenses-upstream/<crate>-<version>/
# (fetched from its repository; SOURCE.txt says from where). Rerun after changing Cargo.lock; NOTICE names the
# same crates.
set -eu
SIGN_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SIGN_DIR"
REGISTRY=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/ | head -1)
if command -v sha256sum >/dev/null 2>&1; then SHA256='sha256sum'; else SHA256='shasum -a 256'; fi
rm -rf licenses
mkdir licenses
cargo tree --locked -e normal,no-proc-macro --target wasm32-unknown-unknown --prefix none --format '{p}' |
  sed 's/ (\*)//' | grep -v '^tandem-sign ' | LC_ALL=C sort -u | while read -r name version; do
  version=${version#v}
  dir="licenses/$name-$version"
  mkdir -p "$dir"
  found=0
  for file in "$REGISTRY/$name-$version"/*; do
    case "$(basename "$file")" in
      LICENSE*|LICENCE*|COPYING*|COPYRIGHT*|UNLICENSE*|NOTICE*) cp "$file" "$dir/"; found=1 ;;
    esac
  done
  # A package that ships no licence file: the upstream files kept in licenses-upstream/ (with SOURCE.txt).
  if [ -d "licenses-upstream/$name-$version" ]; then
    cp "licenses-upstream/$name-$version"/* "$dir/"
    found=1
  fi
  if [ "$found" = 0 ]; then
    echo "licenses.sh: $name $version ships no licence file and licenses-upstream/$name-$version is missing" >&2
    exit 1
  fi
done
{
  echo '['
  first=1
  find licenses -type f ! -name license-files.json | LC_ALL=C sort | while read -r file; do
    [ "$first" = 1 ] || echo ','
    first=0
    printf ' {"file": "%s", "sha256": "%s"}' "$file" "$($SHA256 "$file" | cut -c1-64)"
  done
  echo
  echo ']'
} > licenses/license-files.json
