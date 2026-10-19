#!/bin/sh
# SPDX-License-Identifier: MIT
# Copies the licence files of every crate compiled into the tandem-sign WASM module (cargo tree, normal edges,
# no proc macros) from the cargo registry into licenses/<crate>-<version>/, and lists them with SHA-256 in
# licenses/license-files.json. Rerun after changing Cargo.lock; NOTICE names the same crates.
set -eu
SIGN_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$SIGN_DIR"
REGISTRY=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/ | head -1)
if command -v sha256sum >/dev/null 2>&1; then SHA256='sha256sum'; else SHA256='shasum -a 256'; fi
rm -rf licenses
mkdir licenses
cargo tree --locked -e normal,no-proc-macro --target wasm32-unknown-unknown --features wasm --prefix none --format '{p}' |
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
  if [ "$found" = 0 ]; then
    printf '%s %s ships no licence file. Its Cargo.toml licence is "%s"; the text is the RustCrypto formats\nrepository'"'"'s LICENSE-APACHE and LICENSE-MIT, as retained here for der-0.7.10 (Copyright (c) the RustCrypto\nProject Developers).\n' \
      "$name" "$version" "$(sed -n 's/^license = "\(.*\)"/\1/p' "$REGISTRY/$name-$version/Cargo.toml" | head -1)" > "$dir/NOTE.txt"
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
