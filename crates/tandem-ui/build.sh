#!/bin/sh
# Builds the UI into dist/: the crate as WASM, wasm-bindgen's web glue, the
# page and its styles. Usage: sh build.sh [--release]
set -e
cd "$(dirname "$0")"
PROFILE=${1:---release}
# wasm-bindgen-cli 0.2.129: WASM_BINDGEN_BIN, else the tools folder, else the PATH.
WBG=${WASM_BINDGEN_BIN:-}
[ -n "$WBG" ] || { [ -x "$HOME/.local/tpdf-wasm-tools/bin/wasm-bindgen" ] && WBG=$HOME/.local/tpdf-wasm-tools/bin/wasm-bindgen; } || true
[ -n "$WBG" ] || WBG=$(command -v wasm-bindgen || true)
if [ -z "$WBG" ]; then
  echo "build.sh: wasm-bindgen not found: cargo install wasm-bindgen-cli --version 0.2.129 --locked, or set WASM_BINDGEN_BIN" >&2
  exit 1
fi
cargo build -p tandem-ui --lib --target wasm32-unknown-unknown $PROFILE
DIR=release; [ "$PROFILE" = "--release" ] || DIR=debug
rm -rf dist && mkdir -p dist
"$WBG" ../target/wasm32-unknown-unknown/$DIR/tandem_ui.wasm --target web --no-typescript --out-dir dist
cp index.html styles.css dist/
[ -d assets ] && cp -R assets dist/ || true
